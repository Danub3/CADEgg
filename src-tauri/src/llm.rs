use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::tools::{self, SessionObject, ToolCall, ToolResult};

const MAX_TURNS: usize = 8;
const SESSION_OBJECT_CONTEXT_LIMIT: usize = 8;

const SYSTEM_PROMPT: &str = "你是 CADEgg，一名 AutoCAD 助理。\n\
- 用户讲中文，你也用中文回答。\n\
- 当用户的请求需要操作 CAD 时，调用提供的工具，不要凭空编造对象信息。\n\
- 坐标和尺寸的默认单位是毫米，除非用户明确说米。\n\
- 优先使用结构化工具（draw_line/draw_circle/move 等）；只有当结构化工具明显不足以表达需求时才用 run_lisp。\n\
- 工具库是分层的：能用高层语义几何工具时，不要退回到底层原子工具去自己算；需要读对象信息时优先用查询工具。\n\
- 如果一个请求可以拆成多步且这些步骤都已确定，尽量在同一轮里一次性返回完整的 tool_calls，不要只做第一步。\n\
- 当前会话如果额外提供了对象表，就把它当作可引用对象清单；对象不在表里时，不要假设它还存在。\n\
- 如果用户要求引用当前在 AutoCAD 里手工选中的对象，且这些对象还不在对象表里，先用 import_selection 把它们导入再继续操作。\n\
- 如果消息里出现“系统引用解析”，把其中给出的 handle 视为对用户指代的显式解析结果，优先按它执行。\n\
- 如果工具结果里出现 handle，后续继续操作同一对象时优先使用按 handle 的工具（如 move_handle / erase_handle），不要退回 last。\n\
- 在调用工具前，先用一句话说明你打算做什么；调用结束后，用一句简短的确认结尾。\n\
- 不要重复调用同一个工具去验证结果；工具的返回字符串就是事实。";

// ---------------- Wire format (frontend ↔ backend) ----------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TaskTier {
    /// 低成本模型：纯问答、分类、摘要、格式化。
    Cheap,
    /// 强模型：规划、出图、工具选择、复核。
    Strong,
}

/// 确定性任务分级（不靠模型自己判断，符合「规则优先」路线）。
///
/// 强模型触发条件：涉及绘图/编辑/生成等需要规划与工具选择的动作，
/// 或复核/校核类请求，或复杂几何（楼梯等）。其余纯问答/解释走便宜模型。
fn classify_task(user_input: &str) -> TaskTier {
    let text = user_input.to_lowercase();
    let wants_action = contains_any(
        &text,
        &[
            "画", "绘制", "生成", "做", "出图", "创建", "设计", "建模", "加", "改", "修", "调整",
            "移动", "旋转", "复制", "镜像", "删除", "偏移", "修剪", "延伸", "标注", "布置",
        ],
    );
    let wants_review = contains_any(
        &text,
        &["校核", "复核", "检查", "审查", "验证", "核对", "验收"],
    );
    // 注意：不在这里放「防护」，因为「电梯井口防护要满足什么规范」这类纯问答不该走强模型；
    // 安全防护请求由 is_safety_request 在上层强制 Strong。这里只保留需要规划的复杂几何场景。
    let wants_complex = contains_any(&text, &["楼梯", "双跑", "折返", "休息平台", "临边"]);

    if wants_action || wants_review || wants_complex {
        TaskTier::Strong
    } else {
        TaskTier::Cheap
    }
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

// ---------------- domain-intent 规则表（本地确定性守卫，不调用模型） ----------------
//
// 评估顺序：
//   1) 命中任一「允许类」关键词 → 放行（进入模型，模糊表达由模型继续澄清）。
//   2) 未命中允许类，但命中「拒绝类」关键词 → 直接拒绝，不消耗 token。
//   3) 都没命中 → 放行（宁可放行让模型澄清，也不误拒 CAD 表达）。
//
// 维护方式：新增领域词汇时按类别追加到对应数组即可；拒绝类只在「没有任何允许类
// 信号」时才生效，因此不会误伤带领域词汇的请求。禁止把会调用模型的分类器放在这里。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DomainRuleKind {
    Allow,
    Reject,
}

struct DomainRule {
    label: &'static str,
    kind: DomainRuleKind,
    patterns: &'static [&'static str],
}

const DOMAIN_RULES: &[DomainRule] = &[
    // ---- 允许类：CAD 绘图 / 编辑动作 ----
    DomainRule {
        label: "cad-action",
        kind: DomainRuleKind::Allow,
        patterns: &[
            "cad", "autocad", "draw", "drawing", "绘", "画", "出图", "生成图", "建模", "草图",
            "图纸", "多段线", "polyline", "矩形", "rectangle", "圆", "circle", "直线", "line",
            "标注", "dimension", "图层", "layer", "模型空间", "modelspace", "handle", "对象",
            "object", "偏移", "offset", "镜像", "mirror", "旋转", "rotate", "复制", "copy",
            "删除", "erase", "修剪", "trim", "延伸", "extend", "移动", "move", "阵列", "array",
            "块", "block", "布局", "layout", "视口", "viewport", "门", "窗",
        ],
    },
    // ---- 允许类：图纸检查 / 图面查询 ----
    DomainRule {
        label: "cad-query",
        kind: DomainRuleKind::Allow,
        patterns: &[
            "检查", "校核", "复核", "审查", "查询", "验收", "核对", "读图", "图面", "面积",
            "长度", "距离", "数量", "统计", "属性", "坐标", "颜色", "线型", "线宽", "比例",
            "单位", "标高", "尺寸", "测量", "measure",
        ],
    },
    // ---- 允许类：施工安全防护与建筑规范 ----
    DomainRule {
        label: "safety-standard",
        kind: DomainRuleKind::Allow,
        patterns: &[
            "安全", "safety", "防护", "施工", "construction", "临边", "洞口", "井口", "电梯",
            "楼梯", "stair", "立杆", "横杆", "踢脚板", "警示", "材料表", "规范", "标准", "图集",
            "jgj", "gb/t", "gb ", "脚手架", "基坑", "塔吊", "安全带", "护栏", "挡脚板", "密目网",
            "防水", "混凝土", "钢筋", "模板", "砌体", "屋面", "无障碍",
        ],
    },
    // ---- 允许类：模型配置 ----
    DomainRule {
        label: "model-config",
        kind: DomainRuleKind::Allow,
        patterns: &[
            "模型", "model", "key", "api", "glm", "deepseek", "qwen", "kimi", "gemini",
            "供应商", "provider", "轮转", "failover", "切换模型", "配置", "密钥",
        ],
    },
    // ---- 允许类：应用设置与使用 ----
    DomainRule {
        label: "app-settings",
        kind: DomainRuleKind::Allow,
        patterns: &[
            "设置", "setting", "字体", "语言", "窗口", "置顶", "会话", "导出", "记忆",
            "bridge", "连接", "刷新", "帮助", "版本", "快捷键", "深色", "外观", "界面",
        ],
    },
    // ---- 拒绝类：纯数学计算 ----
    DomainRule {
        label: "unrelated-math",
        kind: DomainRuleKind::Reject,
        patterns: &[
            "1+1", "2+2", "等于几", "几加几", "几乘几", "算数", "口算", "九九乘法", "解方程",
            "微积分", "求导", "积分题",
        ],
    },
    // ---- 拒绝类：文学创作与办公文案 ----
    DomainRule {
        label: "unrelated-creative",
        kind: DomainRuleKind::Reject,
        patterns: &[
            "写一首", "写首诗", "写诗", "诗歌", "作诗", "歌词", "写小说", "写作文", "散文",
            "押韵", "周报", "简历", "ppt", "海报",
        ],
    },
    // ---- 拒绝类：闲聊 ----
    DomainRule {
        label: "unrelated-chitchat",
        kind: DomainRuleKind::Reject,
        patterns: &[
            "天气", "笑话", "讲个故事", "你好吗", "吃了吗", "聊聊天", "无聊", "讲段子",
            "脑筋急转弯",
        ],
    },
    // ---- 拒绝类：通用百科 ----
    DomainRule {
        label: "unrelated-encyclopedia",
        kind: DomainRuleKind::Reject,
        patterns: &[
            "量子力学", "相对论", "宇宙大爆炸", "哲学问题", "历史人物", "娱乐圈", "足球比赛",
            "nba", "游戏攻略",
        ],
    },
];

fn scoped_response_for_unrelated_request(user_input: &str) -> Option<&'static str> {
    let trimmed = user_input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let text = trimmed.to_lowercase();

    let allowed = DOMAIN_RULES
        .iter()
        .any(|rule| rule.kind == DomainRuleKind::Allow && contains_any(&text, rule.patterns));
    if allowed {
        return None;
    }

    let rejected = DOMAIN_RULES
        .iter()
        .any(|rule| rule.kind == DomainRuleKind::Reject && contains_any(&text, rule.patterns));
    if rejected {
        return Some("这个问题不属于 CADEgg 的 AutoCAD/施工安全绘图范围。我可以帮你处理 CAD 绘制、编辑、图面查询、施工安全防护规范、模型 Key 和应用设置相关任务。");
    }

    // 未命中任何规则：默认放行，让模型澄清（模糊 CAD 表达不能误拒）。
    None
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum MessageView {
    User {
        content: String,
    },
    Assistant {
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        tool_calls: Vec<ToolCall>,
    },
    Tool {
        id: String,
        name: String,
        ok: bool,
        content: String,
    },
}

#[derive(Deserialize, Clone, Debug)]
pub struct ModelSelection {
    provider: String,
    model: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ProviderTokenUsage {
    pub provider: String,
    pub model: String,
    /// 缺失（None）与真实 0 必须区分：前端据此显示「未返回」而不是 0。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
}

/// 模型路由链路中的单次候选（provider + model + 状态 + 原因）。
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ModelRouteAttempt {
    pub provider: String,
    pub model: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// 一次任务内的完整模型路由遥测：候选池、跳过原因、回退次数、最终命中。
#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ModelRouteTelemetry {
    pub selected_provider: String,
    pub selected_model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_model: Option<String>,
    pub fallback_count: usize,
    pub attempts: Vec<ModelRouteAttempt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// failover 候选：provider 实例 + 跳过原因（未配 key 等）。
/// 未配置的候选保留在链里用于路由遥测展示「跳过」，实际请求时被跳过。
#[derive(Clone, Debug)]
struct ProviderPlan {
    provider: Provider,
    skip_reason: Option<String>,
}

// ---------------- Provider trait via enum (no async_trait dep) ----------------

#[derive(Clone, Debug)]
pub enum StepOutput {
    /// Final natural-language answer with no further tool calls.
    Text {
        text: String,
        usage: Option<ProviderTokenUsage>,
    },
    /// Optional accompanying text + tool invocations to execute.
    ToolCalls {
        text: Option<String>,
        calls: Vec<ToolCall>,
        usage: Option<ProviderTokenUsage>,
    },
}

#[derive(Clone)]
struct ExecutedBatch {
    signature: String,
    summary: String,
}

#[derive(Clone, Debug)]
enum Provider {
    Glm(GlmProvider),
}

impl Provider {
    async fn step(&self, history: &[MessageView], app: &AppHandle) -> Result<StepOutput, String> {
        match self {
            Provider::Glm(g) => g.step(history, app).await,
        }
    }

    /// 路由身份：(provider_id, model)。用于遥测 attempt 匹配与候选去重。
    fn identity(&self) -> (String, String) {
        match self {
            Provider::Glm(g) => (g.provider_id.clone(), g.model.clone()),
        }
    }

    fn display_name(&self) -> String {
        match self {
            Provider::Glm(g) => format!("{} / {}", g.label, g.model),
        }
    }
}

/// 构建 provider 候选链（主模型 + 备用模型），供 failover 依次尝试。
///
/// 顺序：
///   1) 主 provider 的指定档位模型；
///   2) 同 provider 的另一档位（strong↔cheap 降级/升级，覆盖"强模型临时不可用"场景）；
///   3) 另一个 provider（若已配 key，覆盖"整个 provider 断连"场景）。
///
/// 未配 key 的候选会保留在链里并标记 skip_reason，供模型路由遥测展示「跳过」，
/// 实际请求时由 step_with_failover 跳过。
fn build_provider_chain(
    settings: &crate::settings::Settings,
    tool_names: &[String],
    tier: TaskTier,
    auto_failover: bool,
) -> Vec<ProviderPlan> {
    let mut chain: Vec<ProviderPlan> = Vec::new();
    let selected = match settings.provider.as_str() {
        "deepseek" | "qwen" | "kimi" | "glm" => settings.provider.as_str(),
        _ => "glm",
    };

    append_openai_compatible_provider(
        &mut chain,
        selected,
        settings,
        tool_names,
        tier,
        auto_failover,
    );
    if auto_failover {
        for provider in ["glm", "deepseek", "qwen", "kimi"] {
            if provider != selected {
                append_openai_compatible_provider(
                    &mut chain, provider, settings, tool_names, tier, true,
                );
            }
        }
    }

    // 去重：保留顺序，(provider_id, model) 相同的只保留第一个。
    let mut seen = std::collections::HashSet::new();
    chain
        .into_iter()
        .filter(|p| seen.insert(p.provider.identity()))
        .collect()
}

fn append_openai_compatible_provider(
    chain: &mut Vec<ProviderPlan>,
    provider: &str,
    settings: &crate::settings::Settings,
    tool_names: &[String],
    tier: TaskTier,
    include_fallback: bool,
) {
    let (label, api_key, cheap_model, strong_model, base_url) = match provider {
        "deepseek" => (
            "DeepSeek",
            settings.deepseek_api_key.clone(),
            settings.deepseek_model.clone(),
            settings.deepseek_strong_model.clone(),
            settings.deepseek_base_url.clone(),
        ),
        "qwen" => (
            "通义千问",
            settings.qwen_api_key.clone(),
            settings.qwen_model.clone(),
            settings.qwen_strong_model.clone(),
            settings.qwen_base_url.clone(),
        ),
        "kimi" => (
            "Kimi",
            settings.kimi_api_key.clone(),
            settings.kimi_model.clone(),
            settings.kimi_strong_model.clone(),
            settings.kimi_base_url.clone(),
        ),
        _ => (
            "GLM",
            settings.glm_api_key.clone(),
            settings.glm_model.clone(),
            settings.glm_strong_model.clone(),
            settings.glm_base_url.clone(),
        ),
    };
    let (primary_model, fallback_model) = match tier {
        TaskTier::Strong => (strong_model, cheap_model),
        TaskTier::Cheap => (cheap_model, strong_model),
    };
    let skip_reason = if api_key.trim().is_empty() {
        Some(format!("{} API Key 未配置", label))
    } else {
        None
    };

    chain.push(ProviderPlan {
        provider: Provider::Glm(GlmProvider {
            provider_id: provider.to_string(),
            label: label.to_string(),
            api_key: api_key.clone(),
            model: primary_model,
            base_url: base_url.clone(),
            selected_tools: tool_names.to_vec(),
        }),
        skip_reason: skip_reason.clone(),
    });
    if include_fallback {
        chain.push(ProviderPlan {
            provider: Provider::Glm(GlmProvider {
                provider_id: provider.to_string(),
                label: label.to_string(),
                api_key,
                model: fallback_model,
                base_url,
                selected_tools: tool_names.to_vec(),
            }),
            skip_reason,
        });
    }
}

fn apply_model_selection(
    settings: &mut crate::settings::Settings,
    selection: Option<ModelSelection>,
) {
    let Some(selection) = selection else {
        return;
    };
    let provider = crate::settings::normalize_provider_id(&selection.provider);
    let fallback = crate::settings::default_strong_model_for_provider(&provider);
    let model =
        crate::settings::normalize_model_for_provider(&provider, &selection.model, &fallback);

    settings.provider = provider.clone();
    match provider.as_str() {
        "deepseek" => {
            settings.deepseek_model = model.clone();
            settings.deepseek_strong_model = model;
        }
        "qwen" => {
            settings.qwen_model = model.clone();
            settings.qwen_strong_model = model;
        }
        "kimi" => {
            settings.kimi_model = model.clone();
            settings.kimi_strong_model = model;
        }
        _ => {
            settings.glm_model = model.clone();
            settings.glm_strong_model = model;
        }
    }
}

fn provider_key_configured(settings: &crate::settings::Settings, provider: &str) -> bool {
    match provider {
        "deepseek" => !settings.deepseek_api_key.trim().is_empty(),
        "qwen" => !settings.qwen_api_key.trim().is_empty(),
        "kimi" => !settings.kimi_api_key.trim().is_empty(),
        _ => !settings.glm_api_key.trim().is_empty(),
    }
}

fn provider_label(provider: &str) -> &'static str {
    match provider {
        "deepseek" => "DeepSeek",
        "qwen" => "通义千问",
        "kimi" => "Kimi",
        _ => "GLM",
    }
}

/// 构建任务开始时的初始路由遥测：候选池按计划/跳过标注。
fn build_route_telemetry(
    selected_provider: &str,
    selected_model: &str,
    plans: &[ProviderPlan],
) -> ModelRouteTelemetry {
    let attempts = plans
        .iter()
        .map(|plan| {
            let (provider, model) = plan.provider.identity();
            ModelRouteAttempt {
                provider,
                model,
                status: if plan.skip_reason.is_some() {
                    "skipped".to_string()
                } else {
                    "planned".to_string()
                },
                reason: plan.skip_reason.clone(),
            }
        })
        .collect();
    ModelRouteTelemetry {
        selected_provider: selected_provider.to_string(),
        selected_model: selected_model.to_string(),
        final_provider: None,
        final_model: None,
        fallback_count: 0,
        attempts,
        note: Some("当前选择优先，其他已配置 provider 作为回退候选".to_string()),
    }
}

fn emit_model_route(app: &AppHandle, route: &ModelRouteTelemetry) {
    let _ = app.emit(
        "agent:event",
        AgentEvent::ModelRoute {
            route: route.clone(),
        },
    );
}

fn update_route_attempt(
    route: &mut ModelRouteTelemetry,
    provider: &str,
    model: &str,
    status: &str,
    reason: Option<String>,
) {
    if let Some(attempt) = route
        .attempts
        .iter_mut()
        .find(|attempt| attempt.provider == provider && attempt.model == model)
    {
        attempt.status = status.to_string();
        attempt.reason = reason;
    }
}

/// 每轮开始时把上一次成功的 provider/model 移到链首，避免每轮都重复打失败的主模型。
fn ordered_provider_chain(
    plans: &[ProviderPlan],
    preferred_provider: Option<&str>,
    preferred_model: Option<&str>,
) -> Vec<ProviderPlan> {
    let (Some(preferred_provider), Some(preferred_model)) = (preferred_provider, preferred_model)
    else {
        return plans.to_vec();
    };
    if let Some(index) = plans.iter().position(|plan| {
        let (provider, model) = plan.provider.identity();
        provider == preferred_provider && model == preferred_model
    }) {
        let mut ordered = Vec::with_capacity(plans.len());
        ordered.push(plans[index].clone());
        ordered.extend(
            plans
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != index)
                .map(|(_, plan)| plan.clone()),
        );
        ordered
    } else {
        plans.to_vec()
    }
}

/// 依次尝试 provider 候选链，第一个成功的返回；全部失败返回最后一个错误（带 failover 提示）。
/// 每次状态变化（跳过/尝试/回退/失败/选中）都更新并发出 model_route 事件。
async fn step_with_failover(
    plans: &[ProviderPlan],
    route: &mut ModelRouteTelemetry,
    history: &[MessageView],
    app: &AppHandle,
) -> Result<StepOutput, String> {
    let mut last_err = String::new();
    let mut attempted = 0usize;
    for (i, plan) in plans.iter().enumerate() {
        let (provider_id, model) = plan.provider.identity();
        if let Some(reason) = &plan.skip_reason {
            update_route_attempt(route, &provider_id, &model, "skipped", Some(reason.clone()));
            emit_model_route(app, route);
            continue;
        }
        attempted += 1;
        update_route_attempt(route, &provider_id, &model, "attempting", None);
        emit_model_route(app, route);

        match plan.provider.step(history, app).await {
            Ok(out) => {
                update_route_attempt(route, &provider_id, &model, "selected", None);
                route.final_provider = Some(provider_id);
                route.final_model = Some(model);
                emit_model_route(app, route);
                return Ok(out);
            }
            Err(e) => {
                last_err = e;
                let remaining_usable = plans
                    .iter()
                    .skip(i + 1)
                    .filter(|p| p.skip_reason.is_none())
                    .count();
                if remaining_usable > 0 {
                    route.fallback_count += 1;
                    update_route_attempt(
                        route,
                        &provider_id,
                        &model,
                        "fallback",
                        Some(last_err.clone()),
                    );
                    // 通知前端：主模型失败，正在切换备用模型（比赛"自动切换"亮点）。
                    let next = plans
                        .iter()
                        .skip(i + 1)
                        .find(|p| p.skip_reason.is_none())
                        .map(|p| p.provider.display_name())
                        .unwrap_or_default();
                    let _ = app.emit(
                        "agent:event",
                        AgentEvent::AssistantTrace {
                            delta: &format!("\n[模型切换] 当前模型不可用，正在切换到 {}…\n", next),
                        },
                    );
                } else {
                    update_route_attempt(
                        route,
                        &provider_id,
                        &model,
                        "failed",
                        Some(last_err.clone()),
                    );
                }
                emit_model_route(app, route);
            }
        }
    }
    if attempted == 0 {
        return Err("没有已配置的模型 provider".to_string());
    }
    Err(format!(
        "所有模型均不可用（已尝试 {} 个）：{}",
        attempted, last_err
    ))
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct GeminiProvider {
    api_key: String,
    model: String,
    base_url: String,
    selected_tools: Vec<String>,
}

#[allow(dead_code)]
impl GeminiProvider {
    async fn step(&self, history: &[MessageView], app: &AppHandle) -> Result<StepOutput, String> {
        if self.api_key.trim().is_empty() {
            return Err("尚未配置 Gemini API Key".to_string());
        }
        if self.model.trim().is_empty() {
            return Err("Gemini 必须指定模型 ID".to_string());
        }

        let contents = history_to_gemini_contents(history);
        let body = json!({
            "systemInstruction": { "parts": [{ "text": SYSTEM_PROMPT }] },
            "contents": contents,
            "tools": tools::gemini_function_declarations_for(&self.selected_tools),
        });

        let base = self.base_url.trim_end_matches('/');
        let url = format!(
            "{base}/v1beta/models/{}:streamGenerateContent?alt=sse",
            self.model
        );

        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("网络请求失败: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.map_err(|e| format!("读响应失败: {e}"))?;
            return Err(format!("Gemini API {status}: {text}"));
        }
        let mut text_buf = String::new();
        let mut calls: Vec<ToolCall> = Vec::new();
        let mut usage: Option<ProviderTokenUsage> = None;
        let mut saw_event = false;
        let stream_result = read_sse_events(resp, |data| {
            saw_event = true;
            let parsed: Value =
                serde_json::from_str(data).map_err(|e| format!("解析流式响应失败: {e}"))?;
            if let Some(next_usage) =
                parse_gemini_usage(parsed.get("usageMetadata"), "Gemini", &self.model)
            {
                usage = Some(next_usage);
            }
            let parts = parsed["candidates"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|c| c["content"]["parts"].as_array())
                .cloned()
                .unwrap_or_default();
            for (i, p) in parts.iter().enumerate() {
                if let Some(t) = p["text"].as_str() {
                    if !t.is_empty() {
                        text_buf.push_str(t);
                        let _ = app.emit("agent:event", AgentEvent::AssistantTrace { delta: t });
                    }
                } else if let Some(fc) = p.get("functionCall") {
                    let name = fc["name"].as_str().unwrap_or("").to_string();
                    let args = fc.get("args").cloned().unwrap_or(json!({}));
                    let call_id = format!("g-{i}");
                    if let Some(existing) = calls.iter_mut().find(|existing| existing.id == call_id)
                    {
                        existing.name = name;
                        existing.args = args;
                    } else {
                        calls.push(ToolCall {
                            id: call_id,
                            name,
                            args,
                        });
                    }
                }
            }
            Ok(())
        })
        .await;

        if let Err(error) = stream_result {
            if !saw_event {
                let fallback_url = format!("{base}/v1beta/models/{}:generateContent", self.model);
                let fallback_resp = client
                    .post(&fallback_url)
                    .header("x-goog-api-key", &self.api_key)
                    .header("content-type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| format!("网络请求失败: {e}"))?;
                let fallback_status = fallback_resp.status();
                let fallback_text = fallback_resp
                    .text()
                    .await
                    .map_err(|e| format!("读响应失败: {e}"))?;
                if !fallback_status.is_success() {
                    return Err(format!("Gemini API {fallback_status}: {fallback_text}"));
                }
                let fallback_parsed: Value = serde_json::from_str(&fallback_text)
                    .map_err(|e| format!("解析响应失败: {e}"))?;
                return parse_gemini_step_output(&fallback_parsed, &fallback_text, &self.model);
            }
            return Err(error);
        }

        if !calls.is_empty() {
            let final_text = if text_buf.trim().is_empty() {
                None
            } else {
                Some(text_buf)
            };
            Ok(StepOutput::ToolCalls {
                text: final_text,
                calls,
                usage,
            })
        } else if !text_buf.trim().is_empty() {
            Ok(StepOutput::Text {
                text: text_buf,
                usage,
            })
        } else {
            Err("Gemini 返回为空".to_string())
        }
    }
}

#[allow(dead_code)]
fn history_to_gemini_contents(history: &[MessageView]) -> Vec<Value> {
    let mut out = Vec::new();
    for msg in history {
        match msg {
            MessageView::User { content } => out.push(json!({
                "role": "user",
                "parts": [{ "text": content }],
            })),
            MessageView::Assistant { text, tool_calls } => {
                let mut parts: Vec<Value> = Vec::new();
                if let Some(t) = text.as_ref().filter(|s| !s.trim().is_empty()) {
                    parts.push(json!({ "text": t }));
                }
                for c in tool_calls {
                    parts.push(json!({
                        "functionCall": { "name": c.name, "args": c.args }
                    }));
                }
                if parts.is_empty() {
                    continue;
                }
                out.push(json!({ "role": "model", "parts": parts }));
            }
            MessageView::Tool {
                name, ok, content, ..
            } => {
                let response_payload = if *ok {
                    json!({ "content": content })
                } else {
                    json!({ "error": content })
                };
                out.push(json!({
                    "role": "user",
                    "parts": [{
                        "functionResponse": {
                            "name": name,
                            "response": response_payload,
                        }
                    }],
                }));
            }
        }
    }
    out
}

// ---------------- Zhipu GLM (OpenAI-compatible) ----------------

#[derive(Clone, Debug)]
struct GlmProvider {
    provider_id: String,
    label: String,
    api_key: String,
    model: String,
    base_url: String,
    selected_tools: Vec<String>,
}

impl GlmProvider {
    async fn step(&self, history: &[MessageView], app: &AppHandle) -> Result<StepOutput, String> {
        if self.api_key.trim().is_empty() {
            return Err(format!("尚未配置 {} API Key", self.label));
        }
        if self.model.trim().is_empty() {
            return Err(format!("{} 必须指定模型 ID", self.label));
        }

        let mut messages = vec![json!({ "role": "system", "content": SYSTEM_PROMPT })];
        messages.extend(history_to_openai_messages(history));

        let body = json!({
            "model": self.model,
            "messages": messages.clone(),
            "tools": tools::openai_tools_for(&self.selected_tools),
            "tool_choice": "auto",
            "stream": true,
            "stream_options": { "include_usage": true },
        });

        let base = self.base_url.trim_end_matches('/');
        let url = format!("{base}/chat/completions");

        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("网络请求失败: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.map_err(|e| format!("读响应失败: {e}"))?;
            return Err(format!("{} API {status}: {text}", self.label));
        }
        let mut text_buf = String::new();
        let mut calls: Vec<PendingOpenAiToolCall> = Vec::new();
        let mut usage: Option<ProviderTokenUsage> = None;
        let mut saw_event = false;
        let stream_result = read_sse_events(resp, |data| {
            if data == "[DONE]" {
                saw_event = true;
                return Ok(());
            }
            saw_event = true;
            let parsed: Value =
                serde_json::from_str(data).map_err(|e| format!("解析流式响应失败: {e}"))?;
            if let Some(next_usage) =
                parse_openai_usage(parsed.get("usage"), &self.label, &self.model)
            {
                usage = Some(next_usage);
            }
            let Some(choice) = parsed["choices"]
                .as_array()
                .and_then(|a| a.first())
            else {
                // OpenAI-compatible providers may send a final usage-only
                // chunk with an empty choices array.
                return Ok(());
            };
            let delta = &choice["delta"];
            if let Some(reasoning) = delta["reasoning_content"].as_str() {
                if !reasoning.is_empty() {
                    let _ = app.emit(
                        "agent:event",
                        AgentEvent::AssistantTrace { delta: reasoning },
                    );
                }
            }
            if let Some(content) = delta["content"].as_str() {
                if !content.is_empty() {
                    text_buf.push_str(content);
                    let _ = app.emit("agent:event", AgentEvent::AssistantTrace { delta: content });
                }
            }
            if let Some(tool_calls) = delta["tool_calls"].as_array() {
                for (pos, tc) in tool_calls.iter().enumerate() {
                    let index = tc["index"].as_u64().map(|v| v as usize).unwrap_or(pos);
                    while calls.len() <= index {
                        calls.push(PendingOpenAiToolCall::default());
                    }
                    let pending = &mut calls[index];
                    if let Some(id) = tc["id"].as_str() {
                        pending.id = id.to_string();
                    }
                    if let Some(name) = tc["function"]["name"].as_str() {
                        pending.name = name.to_string();
                    }
                    if let Some(arguments) = tc["function"]["arguments"].as_str() {
                        pending.arguments.push_str(arguments);
                    }
                }
            }
            Ok(())
        })
        .await;

        if let Err(error) = stream_result {
            if !saw_event {
                let fallback_body = json!({
                    "model": self.model,
                    "messages": messages,
                    "tools": tools::openai_tools_for(&self.selected_tools),
                    "tool_choice": "auto",
                });
                let fallback_resp = client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", self.api_key))
                    .header("content-type", "application/json")
                    .json(&fallback_body)
                    .send()
                    .await
                    .map_err(|e| format!("网络请求失败: {e}"))?;
                let fallback_status = fallback_resp.status();
                let fallback_text = fallback_resp
                    .text()
                    .await
                    .map_err(|e| format!("读响应失败: {e}"))?;
                if !fallback_status.is_success() {
                    return Err(format!(
                        "{} API {fallback_status}: {fallback_text}",
                        self.label
                    ));
                }
                let fallback_parsed: Value = serde_json::from_str(&fallback_text)
                    .map_err(|e| format!("解析响应失败: {e}"))?;
                return parse_glm_step_output(
                    &fallback_parsed,
                    &fallback_text,
                    &self.label,
                    &self.model,
                );
            }
            return Err(error);
        }

        let calls: Vec<ToolCall> = calls
            .into_iter()
            .enumerate()
            .map(|(i, pending)| ToolCall {
                id: if pending.id.is_empty() {
                    format!("glm-{i}")
                } else {
                    pending.id
                },
                name: pending.name,
                args: serde_json::from_str(&pending.arguments).unwrap_or(json!({})),
            })
            .filter(|call| !call.name.is_empty())
            .collect();

        if !calls.is_empty() {
            let final_text = if text_buf.trim().is_empty() {
                None
            } else {
                Some(text_buf)
            };
            Ok(StepOutput::ToolCalls {
                text: final_text,
                calls,
                usage,
            })
        } else if !text_buf.trim().is_empty() {
            Ok(StepOutput::Text {
                text: text_buf,
                usage,
            })
        } else {
            Err(format!("{} 返回为空", self.label))
        }
    }
}

fn history_to_openai_messages(history: &[MessageView]) -> Vec<Value> {
    let mut out = Vec::new();
    for msg in history {
        match msg {
            MessageView::User { content } => out.push(json!({
                "role": "user",
                "content": content,
            })),
            MessageView::Assistant { text, tool_calls } => {
                let mut m = json!({
                    "role": "assistant",
                    "content": text.as_deref().unwrap_or(""),
                });
                if !tool_calls.is_empty() {
                    let tcs: Vec<Value> = tool_calls
                        .iter()
                        .map(|c| {
                            json!({
                                "id": c.id,
                                "type": "function",
                                "function": {
                                    "name": c.name,
                                    "arguments": c.args.to_string(),
                                }
                            })
                        })
                        .collect();
                    m["tool_calls"] = Value::Array(tcs);
                    // OpenAI convention: content may be null when tool_calls present
                    if text.is_none() {
                        m["content"] = Value::Null;
                    }
                }
                out.push(m);
            }
            MessageView::Tool { id, content, .. } => out.push(json!({
                "role": "tool",
                "tool_call_id": id,
                "content": content,
            })),
        }
    }
    out
}

// ---------------- Streamed step events to the frontend ----------------

#[derive(Default)]
struct PendingOpenAiToolCall {
    id: String,
    name: String,
    arguments: String,
}

#[allow(dead_code)]
fn parse_gemini_step_output(
    parsed: &Value,
    raw_text: &str,
    model: &str,
) -> Result<StepOutput, String> {
    let parts = parsed["candidates"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|c| c["content"]["parts"].as_array())
        .cloned()
        .unwrap_or_default();

    let mut text_buf = String::new();
    let mut calls: Vec<ToolCall> = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        if let Some(t) = p["text"].as_str() {
            text_buf.push_str(t);
        } else if let Some(fc) = p.get("functionCall") {
            let name = fc["name"].as_str().unwrap_or("").to_string();
            let args = fc.get("args").cloned().unwrap_or(json!({}));
            calls.push(ToolCall {
                id: format!("g-{i}"),
                name,
                args,
            });
        }
    }

    if !calls.is_empty() {
        let final_text = if text_buf.trim().is_empty() {
            None
        } else {
            Some(text_buf)
        };
        Ok(StepOutput::ToolCalls {
            text: final_text,
            calls,
            usage: parse_gemini_usage(parsed.get("usageMetadata"), "Gemini", model),
        })
    } else if !text_buf.trim().is_empty() {
        Ok(StepOutput::Text {
            text: text_buf,
            usage: parse_gemini_usage(parsed.get("usageMetadata"), "Gemini", model),
        })
    } else {
        Err(format!("Gemini 返回为空: {raw_text}"))
    }
}

fn parse_glm_step_output(
    parsed: &Value,
    raw_text: &str,
    provider: &str,
    model: &str,
) -> Result<StepOutput, String> {
    let msg = parsed["choices"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"))
        .ok_or_else(|| format!("响应缺少 choices[0].message: {raw_text}"))?;

    let content = msg["content"].as_str().unwrap_or("").trim().to_string();
    let mut calls: Vec<ToolCall> = Vec::new();
    if let Some(tcs) = msg["tool_calls"].as_array() {
        for tc in tcs {
            let id = tc["id"].as_str().unwrap_or("").to_string();
            let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
            let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
            let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
            calls.push(ToolCall { id, name, args });
        }
    }

    if !calls.is_empty() {
        let final_text = if content.is_empty() {
            None
        } else {
            Some(content)
        };
        Ok(StepOutput::ToolCalls {
            text: final_text,
            calls,
            usage: parse_openai_usage(parsed.get("usage"), provider, model),
        })
    } else if !content.is_empty() {
        Ok(StepOutput::Text {
            text: content,
            usage: parse_openai_usage(parsed.get("usage"), provider, model),
        })
    } else {
        Err(format!("GLM 返回为空: {raw_text}"))
    }
}

fn json_u64(value: Option<&Value>) -> Option<u64> {
    value
        .and_then(Value::as_u64)
        .or_else(|| value.and_then(Value::as_i64).filter(|v| *v >= 0).map(|v| v as u64))
}

fn parse_openai_usage(
    value: Option<&Value>,
    provider: &str,
    model: &str,
) -> Option<ProviderTokenUsage> {
    let usage = value?.as_object()?;
    let prompt_tokens = json_u64(usage.get("prompt_tokens"));
    let output_tokens = json_u64(usage.get("completion_tokens"));
    let details = usage.get("prompt_tokens_details");
    let cache_read_tokens = json_u64(details.and_then(|v| v.get("cached_tokens")))
        .or_else(|| json_u64(usage.get("prompt_cache_hit_tokens")));
    let input_tokens = json_u64(details.and_then(|v| v.get("prompt_cache_miss_tokens")))
        .or_else(|| {
            prompt_tokens.map(|total| total.saturating_sub(cache_read_tokens.unwrap_or(0)))
        })
        .or(prompt_tokens);
    let cache_write_tokens = json_u64(details.and_then(|v| v.get("cache_write_tokens")))
        .or_else(|| json_u64(usage.get("prompt_cache_creation_tokens")));
    let reasoning_tokens =
        json_u64(usage.get("completion_tokens_details").and_then(|v| v.get("reasoning_tokens")));

    // 所有字段都缺失时才返回 None；部分字段存在时保留可解析的部分。
    if input_tokens.is_none()
        && output_tokens.is_none()
        && cache_read_tokens.is_none()
        && cache_write_tokens.is_none()
        && reasoning_tokens.is_none()
    {
        return None;
    }

    Some(ProviderTokenUsage {
        provider: provider.to_string(),
        model: model.to_string(),
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
        reasoning_tokens,
    })
}

fn parse_gemini_usage(
    value: Option<&Value>,
    provider: &str,
    model: &str,
) -> Option<ProviderTokenUsage> {
    let usage = value?.as_object()?;
    let input_tokens = json_u64(usage.get("promptTokenCount"));
    let output_tokens = json_u64(usage.get("candidatesTokenCount"));
    let cache_read_tokens = json_u64(usage.get("cachedContentTokenCount"));
    let reasoning_tokens = json_u64(usage.get("thoughtsTokenCount"));

    // 所有字段都缺失时才返回 None。
    if input_tokens.is_none()
        && output_tokens.is_none()
        && cache_read_tokens.is_none()
        && reasoning_tokens.is_none()
    {
        return None;
    }

    Some(ProviderTokenUsage {
        provider: provider.to_string(),
        model: model.to_string(),
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens: None,
        reasoning_tokens,
    })
}

async fn read_sse_events<F>(mut resp: reqwest::Response, mut on_data: F) -> Result<(), String>
where
    F: FnMut(&str) -> Result<(), String>,
{
    let mut buffer = String::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("读取流失败: {e}"))? {
        buffer.push_str(&String::from_utf8_lossy(&chunk).replace("\r\n", "\n"));
        while let Some(idx) = buffer.find("\n\n") {
            let raw_event = buffer[..idx].to_string();
            buffer.drain(..idx + 2);
            if let Some(data) = extract_sse_data(&raw_event) {
                on_data(&data)?;
            }
        }
    }

    if !buffer.trim().is_empty() {
        if let Some(data) = extract_sse_data(&buffer) {
            on_data(&data)?;
        }
    }
    Ok(())
}

fn extract_sse_data(raw_event: &str) -> Option<String> {
    let mut data_lines: Vec<String> = Vec::new();
    for line in raw_event.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start().to_string());
        }
    }

    if data_lines.is_empty() {
        None
    } else {
        Some(data_lines.join("\n"))
    }
}

#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum AgentEvent<'a> {
    AssistantTrace {
        delta: &'a str,
    },
    Usage {
        usage: &'a ProviderTokenUsage,
    },
    ModelRoute {
        route: ModelRouteTelemetry,
    },
    Assistant {
        text: Option<&'a str>,
        tool_calls: &'a [ToolCall],
    },
    ToolResult {
        result: &'a ToolResult,
    },
    Done {
        text: &'a str,
    },
    Error {
        message: &'a str,
    },
}

fn redact(s: &str, settings: &crate::settings::Settings) -> String {
    let mut out = s.to_string();
    for key in [
        settings.anthropic_api_key.trim(),
        settings.gemini_api_key.trim(),
        settings.glm_api_key.trim(),
        settings.deepseek_api_key.trim(),
        settings.qwen_api_key.trim(),
        settings.kimi_api_key.trim(),
    ] {
        if key.len() >= 8 {
            out = out.replace(key, "***REDACTED***");
        }
    }
    if let Some(idx) = out.find("key=") {
        let tail = &out[idx + 4..];
        let end = tail
            .find(|c: char| c == '&' || c == ' ' || c == '"' || c == ')')
            .unwrap_or(tail.len());
        let span = &tail[..end];
        if !span.is_empty() {
            out = out.replace(&format!("key={span}"), "key=***REDACTED***");
        }
    }
    out
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => serde_json::to_string(v).unwrap_or_else(|_| "\"\"".to_string()),
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", parts.join(","))
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .into_iter()
                .map(|key| format!("{}:{}", key, canonical_json(&map[key])))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
    }
}

fn tool_call_batch_signature(calls: &[ToolCall]) -> String {
    calls
        .iter()
        .map(|call| format!("{}({})", call.name, canonical_json(&call.args)))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn summarize_tool_results(results: &[ToolResult]) -> String {
    if results.is_empty() {
        return "已完成。".to_string();
    }

    let ok_parts: Vec<&str> = results
        .iter()
        .filter(|r| r.ok)
        .map(|r| r.content.as_str())
        .collect();
    let err_parts: Vec<&str> = results
        .iter()
        .filter(|r| !r.ok)
        .map(|r| r.content.as_str())
        .collect();

    if err_parts.is_empty() {
        if ok_parts.len() == 1 {
            format!("已完成：{}", ok_parts[0])
        } else {
            format!("已完成：{}", ok_parts.join("；"))
        }
    } else if ok_parts.is_empty() {
        format!("操作失败：{}", err_parts.join("；"))
    } else {
        format!(
            "已完成部分操作：{}；失败：{}",
            ok_parts.join("；"),
            err_parts.join("；")
        )
    }
}

fn is_mutating_tool(name: &str) -> bool {
    matches!(
        name,
        "draw_line"
            | "draw_circle"
            | "draw_regular_polygon"
            | "draw_equilateral_triangle_about_circle"
            | "draw_rectangle_by_center"
            | "draw_elevator_shaft_protection"
            | "draw_text"
            | "move"
            | "move_handle"
            | "rotate_handle"
            | "copy_handle"
            | "mirror_handle"
            | "offset_handle"
            | "trim_by_handle"
            | "extend_by_handle"
            | "erase_last"
            | "erase_handle"
            | "zoom_extents"
            | "run_lisp"
    )
}

fn kind_reference_terms(kind: &str) -> Vec<String> {
    match kind {
        "LINE" => vec!["条线".to_string(), "条直线".to_string()],
        "CIRCLE" => vec!["个圆".to_string()],
        "ARC" => vec!["段圆弧".to_string()],
        "LWPOLYLINE" => vec!["条多段线".to_string()],
        other => vec![format!("个{}对象", other)],
    }
}

fn source_display_name(source: Option<&str>) -> &'static str {
    match source {
        Some("generated") => "创建",
        Some("selection") => "导入",
        _ => "纳入",
    }
}

fn ordinal_cn(n: usize) -> Option<&'static str> {
    match n {
        1 => Some("一"),
        2 => Some("二"),
        3 => Some("三"),
        4 => Some("四"),
        5 => Some("五"),
        6 => Some("六"),
        7 => Some("七"),
        8 => Some("八"),
        9 => Some("九"),
        10 => Some("十"),
        11 => Some("十一"),
        12 => Some("十二"),
        _ => None,
    }
}

fn push_unique_string(into: &mut Vec<String>, value: String) {
    if !into.iter().any(|existing| existing == &value) {
        into.push(value);
    }
}

fn numbered_aliases(prefix: &str, n: usize, suffix: &str) -> Vec<String> {
    let mut aliases = vec![format!("{prefix}{n}{suffix}")];
    if let Some(cn) = ordinal_cn(n) {
        aliases.push(format!("{prefix}{cn}{suffix}"));
    }
    aliases
}

fn build_object_aliases(
    object_index: usize,
    kind: &str,
    kind_index: usize,
    source: Option<&str>,
    source_index: usize,
) -> Vec<String> {
    let mut aliases = Vec::new();

    for alias in numbered_aliases("第", object_index, "个对象") {
        push_unique_string(&mut aliases, alias);
    }
    if object_index == 1 {
        push_unique_string(&mut aliases, "最新对象".to_string());
    }

    for term in kind_reference_terms(kind) {
        for alias in numbered_aliases("第", kind_index, &term) {
            push_unique_string(&mut aliases, alias);
        }
        if kind_index == 1 {
            push_unique_string(&mut aliases, format!("最新{term}"));
            push_unique_string(&mut aliases, format!("上一{term}"));
        }
    }

    let source_name = source_display_name(source);
    for alias in numbered_aliases("第", source_index, &format!("个{source_name}对象")) {
        push_unique_string(&mut aliases, alias);
    }
    if source_index == 1 {
        push_unique_string(&mut aliases, format!("最新{source_name}的对象"));
        push_unique_string(&mut aliases, format!("刚{source_name}的对象"));
    }

    aliases
}

fn build_context_aliases(
    object_index: usize,
    kind: &str,
    kind_index: usize,
    source: Option<&str>,
    source_index: usize,
) -> Vec<String> {
    let mut aliases = Vec::new();

    if object_index == 1 {
        push_unique_string(&mut aliases, "最新对象".to_string());
    }
    if let Some(alias) = numbered_aliases("第", object_index, "个对象")
        .into_iter()
        .next()
    {
        push_unique_string(&mut aliases, alias);
    }

    if let Some(term) = kind_reference_terms(kind).into_iter().next() {
        if kind_index == 1 {
            push_unique_string(&mut aliases, format!("最新{term}"));
        }
        if let Some(alias) = numbered_aliases("第", kind_index, &term).into_iter().next() {
            push_unique_string(&mut aliases, alias);
        }
    }

    let source_name = source_display_name(source);
    if source_index == 1 {
        push_unique_string(&mut aliases, format!("最新{source_name}的对象"));
    }
    if let Some(alias) = numbered_aliases("第", source_index, &format!("个{source_name}对象"))
        .into_iter()
        .next()
    {
        push_unique_string(&mut aliases, alias);
    }

    aliases
}

#[derive(Clone)]
struct ResolvedObjectReference {
    start: usize,
    end: usize,
    phrase: String,
    handle: String,
    kind: String,
    label: String,
}

fn resolve_user_object_references(
    user_input: &str,
    session_objects: &[SessionObject],
) -> Vec<ResolvedObjectReference> {
    let mut kind_counts: std::collections::BTreeMap<&str, usize> =
        std::collections::BTreeMap::new();
    let mut source_counts: std::collections::BTreeMap<&str, usize> =
        std::collections::BTreeMap::new();
    let mut candidates: Vec<ResolvedObjectReference> = Vec::new();

    for (idx, obj) in session_objects.iter().enumerate() {
        let kind_index = kind_counts
            .entry(obj.kind.as_str())
            .and_modify(|count| *count += 1)
            .or_insert(1);
        let source_key = obj.source.as_deref().unwrap_or("session");
        let source_index = source_counts
            .entry(source_key)
            .and_modify(|count| *count += 1)
            .or_insert(1);

        for alias in build_object_aliases(
            idx + 1,
            &obj.kind,
            *kind_index,
            obj.source.as_deref(),
            *source_index,
        ) {
            if let Some(start) = user_input.find(&alias) {
                candidates.push(ResolvedObjectReference {
                    start,
                    end: start + alias.len(),
                    phrase: alias,
                    handle: obj.handle.clone(),
                    kind: obj.kind.clone(),
                    label: obj.label.clone(),
                });
            }
        }
    }

    candidates.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then_with(|| b.phrase.len().cmp(&a.phrase.len()))
    });

    let mut resolved: Vec<ResolvedObjectReference> = Vec::new();
    for candidate in candidates {
        let overlaps = resolved
            .iter()
            .any(|existing| candidate.start < existing.end && existing.start < candidate.end);
        let duplicate_phrase = resolved.iter().any(|existing| {
            existing.phrase == candidate.phrase && existing.handle == candidate.handle
        });
        if !overlaps && !duplicate_phrase {
            resolved.push(candidate);
        }
    }

    resolved
}

fn format_resolved_object_reference_context(
    user_input: &str,
    session_objects: &[SessionObject],
) -> Option<String> {
    let resolved = resolve_user_object_references(user_input, session_objects);
    if resolved.is_empty() {
        return None;
    }

    let lines: Vec<String> = resolved
        .iter()
        .map(|item| {
            format!(
                "- “{}” => handle={} type={} label={}",
                item.phrase, item.handle, item.kind, item.label
            )
        })
        .collect();

    Some(format!(
        "系统引用解析（已显式完成；后续优先按这些 handle 理解用户指代）：\n{}",
        lines.join("\n")
    ))
}

fn session_object_context(session_objects: &[SessionObject]) -> Option<String> {
    if session_objects.is_empty() {
        return None;
    }

    let mut kind_counts: std::collections::BTreeMap<&str, usize> =
        std::collections::BTreeMap::new();
    let mut source_counts: std::collections::BTreeMap<&str, usize> =
        std::collections::BTreeMap::new();
    let lines: Vec<String> = session_objects
        .iter()
        .take(SESSION_OBJECT_CONTEXT_LIMIT)
        .enumerate()
        .map(|(idx, obj)| {
            let kind_count = kind_counts
                .entry(obj.kind.as_str())
                .and_modify(|count| *count += 1)
                .or_insert(1);
            let source_key = obj.source.as_deref().unwrap_or("session");
            let source_count = source_counts
                .entry(source_key)
                .and_modify(|count| *count += 1)
                .or_insert(1);
            let aliases = build_context_aliases(
                idx + 1,
                &obj.kind,
                *kind_count,
                obj.source.as_deref(),
                *source_count,
            );

            format!(
                "{}. handle={} type={} refs={} label={}",
                idx + 1,
                obj.handle,
                obj.kind,
                aliases.join(" / "),
                obj.label
            )
        })
        .collect();

    let mut header =
        "当前会话可引用对象表（最近在前；generated=创建，selection=导入）。".to_string();
    if session_objects.len() > SESSION_OBJECT_CONTEXT_LIMIT {
        header.push_str(&format!(
            " 仅展开最近 {} 个，其余 {} 个未展开。",
            SESSION_OBJECT_CONTEXT_LIMIT,
            session_objects.len() - SESSION_OBJECT_CONTEXT_LIMIT
        ));
    }

    Some(format!("{header}\n{}", lines.join("\n")))
}

#[cfg(windows)]
fn begin_undo_group() -> Result<(), String> {
    crate::cad::cad_begin_undo_group()
}

#[cfg(not(windows))]
fn begin_undo_group() -> Result<(), String> {
    Err("UNDO GROUP 仅在 Windows 上可用".to_string())
}

#[cfg(windows)]
fn end_undo_group() -> Result<(), String> {
    crate::cad::cad_end_undo_group()
}

#[cfg(not(windows))]
fn end_undo_group() -> Result<(), String> {
    Err("UNDO GROUP 仅在 Windows 上可用".to_string())
}

// ---------------- Tauri command: run_agent ----------------

#[tauri::command]
pub async fn run_agent(
    app: AppHandle,
    user_input: String,
    history: Vec<MessageView>,
    session_objects: Vec<SessionObject>,
    model_selection: Option<ModelSelection>,
) -> Result<(), String> {
    let mut settings = crate::settings::load(&app)?;
    apply_model_selection(&mut settings, model_selection);
    let user_text = user_input.trim().to_string();
    if let Some(response) = scoped_response_for_unrelated_request(&user_text) {
        let _ = app.emit(
            "agent:event",
            AgentEvent::Assistant {
                text: Some(response),
                tool_calls: &[],
            },
        );
        let _ = app.emit("agent:event", AgentEvent::Done { text: response });
        return Ok(());
    }

    let tooling = tools::select_tooling_context(&user_text, &session_objects, settings.work_mode);
    let use_safety_context = settings.work_mode == crate::settings::WorkMode::SafetyDemoMode
        || tools::is_safety_request(&user_text);

    // 确定性任务分级：出图/规划/复核走强模型，纯问答走便宜模型。
    // 安全防护请求（需严格按知识卡出图/校核）强制走强模型，保证质量。
    let tier = if use_safety_context {
        TaskTier::Strong
    } else {
        classify_task(&user_text)
    };

    // 构建 provider 链：主模型 + 备用模型（failover）。
    // 兜底顺序：
    //   1) 主 provider 的指定档位模型（strong/cheap）；
    //   2) 同 provider 降级到另一档位（strong 失败降 cheap，或 cheap 失败升 strong）；
    //   3) 另一个 provider（若已配 key）。
    // 每步失败后自动切下一个，避免单点断连导致整个请求失败。
    let provider_plans =
        build_provider_chain(&settings, &tooling.tool_names, tier, settings.auto_failover);
    // 初始模型路由遥测：把候选池、跳过原因先发给前端，后续每步更新状态。
    let selected_provider = match settings.provider.as_str() {
        "deepseek" | "qwen" | "kimi" | "glm" => settings.provider.clone(),
        _ => "glm".to_string(),
    };
    let selected_model = provider_plans
        .first()
        .map(|plan| plan.provider.identity().1)
        .unwrap_or_default();
    let mut route = build_route_telemetry(&selected_provider, &selected_model, &provider_plans);
    emit_model_route(&app, &route);
    if !provider_key_configured(&settings, &settings.provider) {
        if let Some(first) = provider_plans.first() {
            let _ = app.emit(
                "agent:event",
                AgentEvent::AssistantTrace {
                    delta: &format!(
                        "\n[模型切换] {} 未配置 API Key，正在使用 {}…\n",
                        provider_label(&settings.provider),
                        first.provider.display_name()
                    ),
                },
            );
        }
    }

    let mut msgs = history;
    if let Some(context) = session_object_context(&session_objects) {
        msgs.push(MessageView::User { content: context });
    }
    if !tooling.guidance.trim().is_empty() {
        msgs.push(MessageView::User {
            content: format!("系统工具分层策略：\n{}", tooling.guidance),
        });
    }
    if use_safety_context {
        // 闭环第 3 步：按用户输入关键词检索知识卡（search_scenes 多卡命中），
        // 让模型基于受控规则出图/追问，而非自由发挥。
        // 兜底：若关键词检索未命中，回退到电梯井口这张默认卡。
        let scenes = crate::knowledge::search_scenes(&user_text);
        let scene = scenes
            .first()
            .map(|s| s.as_str())
            .unwrap_or("elevator_shaft_protection");
        if let Some(card) = crate::knowledge::render_scene_context(scene) {
            msgs.push(MessageView::User {
                content: format!("系统提醒（标准图册知识卡，出图/追问须遵守）：\n{card}"),
            });
        }
        if let Some(prompt) = tools::safety_clarification_prompt(&user_text) {
            msgs.push(MessageView::User {
                content: format!("系统提醒（安全防护缺参追问）：\n{}", prompt),
            });
        }
    }
    // 制图规范知识卡：任何绘图请求都注入，保证标注/图线/字体符合 GB/T 50001。
    // 放在安全知识卡之后，作为出图的通用约束。
    if use_safety_context || classify_task(&user_text) == TaskTier::Strong {
        if let Some(card) = crate::knowledge::render_scene_context("cad_drafting_standard") {
            msgs.push(MessageView::User {
                content: format!("系统提醒（CAD 制图规范，尺寸标注/图线/字体须遵守）：\n{card}"),
            });
        }
    }
    if !user_text.is_empty() {
        if let Some(reference_context) =
            format_resolved_object_reference_context(&user_text, &session_objects)
        {
            msgs.push(MessageView::User {
                content: reference_context,
            });
        }
        msgs.push(MessageView::User { content: user_text });
    }

    let mut undo_group_open = false;
    let mut last_executed_batch: Option<ExecutedBatch> = None;
    for _turn in 0..MAX_TURNS {
        // failover：主模型失败时依次尝试备用模型，全部失败才报错。
        // 上一轮成功的 provider 移到链首，避免每轮重复打失败的主模型。
        let turn_plans = ordered_provider_chain(
            &provider_plans,
            route.final_provider.as_deref(),
            route.final_model.as_deref(),
        );
        let step = step_with_failover(&turn_plans, &mut route, &msgs, &app).await;
        let step = match step {
            Ok(s) => s,
            Err(e) => {
                if undo_group_open {
                    let _ = end_undo_group();
                }
                let msg = redact(&e, &settings);
                let _ = app.emit("agent:event", AgentEvent::Error { message: &msg });
                return Err(msg);
            }
        };
        let usage = match &step {
            StepOutput::Text { usage, .. } | StepOutput::ToolCalls { usage, .. } => usage.as_ref(),
        };
        if let Some(usage) = usage {
            let _ = app.emit("agent:event", AgentEvent::Usage { usage });
        }
        match step {
            StepOutput::Text { text, .. } => {
                if undo_group_open {
                    if let Err(e) = end_undo_group() {
                        let msg = redact(&e, &settings);
                        let _ = app.emit("agent:event", AgentEvent::Error { message: &msg });
                        return Err(msg);
                    }
                }
                let _ = app.emit(
                    "agent:event",
                    AgentEvent::Assistant {
                        text: Some(&text),
                        tool_calls: &[],
                    },
                );
                let _ = app.emit("agent:event", AgentEvent::Done { text: &text });
                return Ok(());
            }
            StepOutput::ToolCalls { text, calls, .. } => {
                let signature = tool_call_batch_signature(&calls);
                if let Some(previous) = &last_executed_batch {
                    if previous.signature == signature {
                        if undo_group_open {
                            if let Err(e) = end_undo_group() {
                                let msg = redact(&e, &settings);
                                let _ =
                                    app.emit("agent:event", AgentEvent::Error { message: &msg });
                                return Err(msg);
                            }
                        }
                        let summary = format!(
                            "检测到模型重复生成同一批工具调用，已停止重复执行。{}",
                            previous.summary
                        );
                        let _ = app.emit(
                            "agent:event",
                            AgentEvent::Assistant {
                                text: Some(&summary),
                                tool_calls: &[],
                            },
                        );
                        let _ = app.emit("agent:event", AgentEvent::Done { text: &summary });
                        return Ok(());
                    }
                }
                if !calls.is_empty() && !undo_group_open {
                    if let Err(e) = begin_undo_group() {
                        let msg = redact(&e, &settings);
                        let _ = app.emit("agent:event", AgentEvent::Error { message: &msg });
                        return Err(msg);
                    }
                    undo_group_open = true;
                }
                let _ = app.emit(
                    "agent:event",
                    AgentEvent::Assistant {
                        text: text.as_deref(),
                        tool_calls: &calls,
                    },
                );
                msgs.push(MessageView::Assistant {
                    text: None,
                    tool_calls: calls.clone(),
                });
                let mut batch_results: Vec<ToolResult> = Vec::new();
                for call in &calls {
                    let result = tools::dispatch_with_mode(call, settings.work_mode);
                    let _ = app.emit("agent:event", AgentEvent::ToolResult { result: &result });
                    msgs.push(MessageView::Tool {
                        id: result.id.clone(),
                        name: result.name.clone(),
                        ok: result.ok,
                        content: result.content.clone(),
                    });
                    batch_results.push(result);
                }
                let has_mutating_tools = calls.iter().any(|call| is_mutating_tool(&call.name));
                if has_mutating_tools {
                    if undo_group_open {
                        if let Err(e) = end_undo_group() {
                            let msg = redact(&e, &settings);
                            let _ = app.emit("agent:event", AgentEvent::Error { message: &msg });
                            return Err(msg);
                        }
                    }
                    let done_text = "";
                    let _ = app.emit("agent:event", AgentEvent::Done { text: done_text });
                    return Ok(());
                }
                if !calls.is_empty() {
                    msgs.push(MessageView::User {
                        content: "系统提醒：上一步工具结果已返回。若工具已成功完成任务，请不要重复执行相同绘图工具，直接给出简短最终结果。".to_string(),
                    });
                }
                if !batch_results.is_empty() {
                    last_executed_batch = Some(ExecutedBatch {
                        signature,
                        summary: summarize_tool_results(&batch_results),
                    });
                }
            }
        }
    }

    if undo_group_open {
        let _ = end_undo_group();
    }
    let msg = format!("超出最大轮次 {MAX_TURNS}，循环已中止");
    let _ = app.emit("agent:event", AgentEvent::Error { message: &msg });
    Err(msg)
}

#[tauri::command]
pub fn confirm_tool_call(app: AppHandle, call: ToolCall) -> Result<ToolResult, String> {
    let settings = crate::settings::load(&app)?;
    let mut undo_group_open = false;
    if is_mutating_tool(&call.name) {
        begin_undo_group()?;
        undo_group_open = true;
    }

    let result = tools::dispatch_confirmed_with_mode(&call, settings.work_mode);

    if undo_group_open {
        if let Err(e) = end_undo_group() {
            return Err(e);
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_strong_for_drawing_and_review() {
        assert_eq!(classify_task("画一条线"), TaskTier::Strong);
        assert_eq!(classify_task("生成电梯井口防护门图"), TaskTier::Strong);
        assert_eq!(classify_task("校核一下这个防护"), TaskTier::Strong);
        assert_eq!(classify_task("画一个双跑楼梯"), TaskTier::Strong);
    }

    #[test]
    fn classify_cheap_for_qa() {
        assert_eq!(
            classify_task("电梯井口防护要满足什么规范？"),
            TaskTier::Cheap
        );
        assert_eq!(classify_task("你好"), TaskTier::Cheap);
        assert_eq!(classify_task("立杆间距最大多少"), TaskTier::Cheap);
    }

    #[test]
    fn domain_rule_table_is_wellformed() {
        for rule in DOMAIN_RULES {
            assert!(!rule.label.is_empty());
            assert!(
                !rule.patterns.is_empty(),
                "规则 {} 的 patterns 为空",
                rule.label
            );
            assert!(
                rule.patterns.iter().all(|p| !p.trim().is_empty()),
                "规则 {} 存在空白 pattern",
                rule.label
            );
        }
    }

    #[test]
    fn scope_guard_rejects_unrelated_questions() {
        assert!(scoped_response_for_unrelated_request("1+1等于几？").is_some());
        assert!(scoped_response_for_unrelated_request("写一首诗").is_some());
        assert!(scoped_response_for_unrelated_request("今天天气怎么样").is_some());
        assert!(scoped_response_for_unrelated_request("给我讲个笑话").is_some());
        assert!(scoped_response_for_unrelated_request("介绍一下量子力学").is_some());
        assert!(scoped_response_for_unrelated_request("帮我写个周报").is_some());
    }

    #[test]
    fn scope_guard_allows_vague_cad_for_clarification() {
        // CAD 相关但表达模糊的问题必须放行，由模型追问澄清。
        assert!(scoped_response_for_unrelated_request("帮我处理一下这个图").is_none());
        assert!(scoped_response_for_unrelated_request("上面那个对象再改改").is_none());
        assert!(scoped_response_for_unrelated_request("这个还要调整一下").is_none());
        assert!(scoped_response_for_unrelated_request("算一下这面墙的面积").is_none());
    }

    #[test]
    fn scope_guard_allows_cad_safety_and_app_questions() {
        assert!(scoped_response_for_unrelated_request("画一条 7000mm 的直线").is_none());
        assert!(scoped_response_for_unrelated_request("电梯井口防护要满足什么规范？").is_none());
        assert!(scoped_response_for_unrelated_request("GLM Key 怎么配置？").is_none());
        assert!(scoped_response_for_unrelated_request("怎么配置 DeepSeek API Key").is_none());
        assert!(scoped_response_for_unrelated_request("怎么切换深色模式").is_none());
        assert!(scoped_response_for_unrelated_request("立杆间距最大多少").is_none());
    }

    #[test]
    fn scope_guard_keeps_domain_signals_above_reject_patterns() {
        // 拒绝词只在没有任何允许类信号时才生效：带领域词的请求不受影响。
        assert!(scoped_response_for_unrelated_request("计算一下这条线的长度").is_none());
        assert!(scoped_response_for_unrelated_request("这张图纸的历史版本怎么查").is_none());
        assert!(scoped_response_for_unrelated_request("检查一下这个圆的半径").is_none());
    }

    #[test]
    fn openai_usage_splits_cache_hits_from_input_tokens() {
        let parsed = json!({
            "prompt_tokens": 1200,
            "completion_tokens": 340,
            "prompt_tokens_details": {
                "cached_tokens": 700
            },
            "completion_tokens_details": {
                "reasoning_tokens": 90
            }
        });

        let usage = parse_openai_usage(Some(&parsed), "DeepSeek", "deepseek-v4-flash")
            .expect("usage should parse");

        assert_eq!(usage.input_tokens, Some(500));
        assert_eq!(usage.output_tokens, Some(340));
        assert_eq!(usage.cache_read_tokens, Some(700));
        assert_eq!(usage.reasoning_tokens, Some(90));
    }

    #[test]
    fn gemini_usage_maps_prompt_and_thought_tokens() {
        let parsed = json!({
            "promptTokenCount": 800,
            "candidatesTokenCount": 160,
            "cachedContentTokenCount": 200,
            "thoughtsTokenCount": 40
        });

        let usage =
            parse_gemini_usage(Some(&parsed), "Gemini", "gemini-2.5-pro").expect("usage should parse");

        assert_eq!(usage.input_tokens, Some(800));
        assert_eq!(usage.output_tokens, Some(160));
        assert_eq!(usage.cache_read_tokens, Some(200));
        assert_eq!(usage.reasoning_tokens, Some(40));
    }

    #[test]
    fn provider_chain_glm_primary_filters_unconfigured() {
        let settings = crate::settings::Settings {
            provider: "glm".to_string(),
            glm_api_key: "glm-key-12345678".to_string(),
            glm_model: "glm-4-flash".to_string(),
            glm_strong_model: "glm-4.5".to_string(),
            // Gemini 未配 key
            gemini_api_key: String::new(),
            ..Default::default()
        };
        let chain = build_provider_chain(&settings, &[], TaskTier::Strong, true);
        // 已配置的只有 GLM 的 strong + 降级 cheap；其余未配 key 的候选保留并标记 skipped。
        let configured: Vec<_> = chain.iter().filter(|p| p.skip_reason.is_none()).collect();
        assert_eq!(configured.len(), 2);
        assert!(matches!(configured[0].provider, Provider::Glm(_)));
        assert!(matches!(configured[1].provider, Provider::Glm(_)));
        assert!(chain.iter().any(|p| p.skip_reason.is_some()));
    }

    #[test]
    fn provider_chain_strong_fallback_downgrades_to_cheap() {
        let settings = crate::settings::Settings {
            provider: "glm".to_string(),
            glm_api_key: "glm-key-12345678".to_string(),
            glm_model: "glm-4-flash".to_string(),
            glm_strong_model: "glm-4.5".to_string(),
            ..Default::default()
        };
        let chain = build_provider_chain(&settings, &[], TaskTier::Strong, true);
        // 主模型是 strong（glm-4.5），备用是 cheap（glm-4-flash）。
        let Provider::Glm(primary) = &chain[0].provider;
        assert_eq!(primary.model, "glm-4.5");
        let Provider::Glm(fallback) = &chain[1].provider;
        assert_eq!(fallback.model, "glm-4-flash");
    }

    #[test]
    fn explicit_session_model_overrides_tier_slots() {
        let mut settings = crate::settings::Settings {
            provider: "glm".to_string(),
            glm_api_key: "glm-key-12345678".to_string(),
            glm_model: "glm-4.5-air".to_string(),
            glm_strong_model: "glm-4.5".to_string(),
            ..Default::default()
        };

        apply_model_selection(
            &mut settings,
            Some(ModelSelection {
                provider: "glm".to_string(),
                model: "glm-4.5-flash".to_string(),
            }),
        );
        let chain = build_provider_chain(&settings, &[], TaskTier::Cheap, true);

        let configured: Vec<_> = chain.iter().filter(|p| p.skip_reason.is_none()).collect();
        assert_eq!(configured.len(), 1);
        let Provider::Glm(primary) = &configured[0].provider;
        assert_eq!(primary.model, "glm-4.5-flash");
    }

    #[test]
    fn provider_chain_respects_failover_toggle() {
        let settings = crate::settings::Settings {
            provider: "glm".to_string(),
            glm_api_key: "glm-key-12345678".to_string(),
            deepseek_api_key: "deepseek-key-12345678".to_string(),
            glm_model: "glm-4.5-air".to_string(),
            glm_strong_model: "glm-4.5".to_string(),
            deepseek_model: "deepseek-v4-flash".to_string(),
            deepseek_strong_model: "deepseek-v4-pro".to_string(),
            ..Default::default()
        };

        let chain = build_provider_chain(&settings, &[], TaskTier::Strong, false);

        assert_eq!(chain.len(), 1);
        let Provider::Glm(primary) = &chain[0].provider;
        assert_eq!(primary.model, "glm-4.5");
    }

    #[test]
    fn provider_chain_no_key_marks_all_skipped() {
        let settings = crate::settings::Settings::default(); // 全空 key
        let chain = build_provider_chain(&settings, &[], TaskTier::Cheap, true);
        assert!(!chain.is_empty());
        assert!(chain.iter().all(|p| p.skip_reason.is_some()));
    }

    #[test]
    fn route_telemetry_marks_unconfigured_as_skipped() {
        let settings = crate::settings::Settings {
            provider: "glm".to_string(),
            glm_api_key: "glm-key-12345678".to_string(),
            glm_model: "glm-4-flash".to_string(),
            glm_strong_model: "glm-4.5".to_string(),
            ..Default::default()
        };
        let chain = build_provider_chain(&settings, &[], TaskTier::Strong, true);
        let route = build_route_telemetry("glm", "glm-4.5", &chain);

        assert_eq!(route.selected_provider, "glm");
        assert_eq!(route.selected_model, "glm-4.5");
        assert_eq!(route.fallback_count, 0);
        let glm_planned = route
            .attempts
            .iter()
            .filter(|a| a.status == "planned")
            .count();
        assert_eq!(glm_planned, 2);
        assert!(route.attempts.iter().any(|a| a.status == "skipped"));
        assert!(route
            .attempts
            .iter()
            .filter(|a| a.status == "skipped")
            .all(|a| a.reason.as_deref().unwrap_or("").contains("API Key")));
    }

    #[test]
    fn openai_usage_allows_partial_fields() {
        // provider 只回输出 token 时也应解析成功，缺失字段保持 None（前端显示「未返回」）。
        let parsed = json!({ "completion_tokens": 120 });
        let usage = parse_openai_usage(Some(&parsed), "GLM", "glm-4.5")
            .expect("partial usage should parse");
        assert_eq!(usage.input_tokens, None);
        assert_eq!(usage.output_tokens, Some(120));
        assert_eq!(usage.cache_read_tokens, None);
    }

    #[test]
    fn openai_usage_all_missing_returns_none() {
        let parsed = json!({ "unknown_field": 1 });
        assert!(parse_openai_usage(Some(&parsed), "GLM", "glm-4.5").is_none());
    }
}
