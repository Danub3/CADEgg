//! P5 全模型客观基准：对已配置 provider 的模型跑紧凑固定测试集，产出可解释评分与报告。
//!
//! 预算护栏（避免无意义反复试验与 token 爆炸）：
//! - 每个候选模型固定 6 次小请求（p5 非领域拒绝走本地守卫，0 成本）；
//! - 全量运行硬上限 MAX_BENCHMARK_REQUESTS 次请求（命令参数只能调小）；
//! - 单请求 60s 超时；支持 cancel_model_benchmark 随时中断；
//! - 绝不执行 AutoCAD 绘图：只验证模型输出的工具调用 JSON 与意图。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

use crate::llm::{
    parse_openai_usage, redact, scoped_response_for_unrelated_request, ProviderTokenUsage,
};
use crate::session_export::{memory_bundle_dir, SessionStorageLocation};
use crate::settings::Settings;

/// 全量运行硬上限：请求次数（全部模型 ≈ 38×6=228，留余量）。
pub const MAX_BENCHMARK_REQUESTS: u32 = 256;
/// 每个模型固定请求数（p1/p2/p3/p4/p6_turn1/p6_turn2）。
const REQUESTS_PER_MODEL: usize = 6;
/// 单请求超时。
const REQUEST_TIMEOUT_SECS: u64 = 60;
/// 429 限流重试上限（含首次尝试）。
const MAX_ATTEMPTS_PER_CASE: usize = 3;

/// 按供应商的请求间隔，避免触发 RPM 限流（Kimi 实测 org RPM=3，需 ≥20s/次）。
fn provider_pause_ms(provider: &str) -> u64 {
    if provider == "kimi" {
        21_000
    } else {
        2_500
    }
}

/// 判断是否为限流类错误。
fn is_rate_limit_error(message: &str) -> bool {
    message.contains("429") || message.to_ascii_lowercase().contains("rate_limit")
}

/// 从错误信息解析「try again after X seconds」，夹在 [5, 90] 秒。
fn parse_retry_after_secs(message: &str) -> Option<u64> {
    let lower = message.to_ascii_lowercase();
    let marker = "after ";
    let pos = lower.find(marker)?;
    let tail = &lower[pos + marker.len()..];
    let num: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
    num.parse::<u64>().ok().map(|s| s.clamp(5, 90))
}

/// 限流错误的重试等待：有 Retry-After 用 Retry-After，否则用较长退避（≥30s）。
fn rate_limit_wait_ms(message: &str, pause_ms: u64) -> u64 {
    parse_retry_after_secs(message)
        .map(|secs| (secs * 1000).max(pause_ms))
        .unwrap_or_else(|| pause_ms.max(30_000))
}

static RUNNING: AtomicBool = AtomicBool::new(false);
static CANCELLED: AtomicBool = AtomicBool::new(false);
static REQUEST_COUNT: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 把 epoch 毫秒转成 UTC `YYYYMMDDTHHMMSSZ` 文件名时间戳（不引入 chrono）。
fn utc_file_stamp(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    let ss = rem % 60;
    // Howard Hinnant civil_from_days 逆算法
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    if m <= 2 {
        y += 1;
    }
    format!("{y:04}{m:02}{d:02}T{hh:02}{mm:02}{ss:02}Z")
}

// ---------------- 数据模型（camelCase 供前端） ----------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkCandidate {
    pub provider: String,
    pub provider_label: String,
    pub model: String,
    pub skip_reason: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkCaseResult {
    pub id: String,
    pub label: String,
    pub score: f64,
    pub note: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkModelResult {
    pub provider: String,
    pub provider_label: String,
    pub model: String,
    pub requests: usize,
    pub succeeded: usize,
    pub avg_duration_ms: u64,
    pub avg_output_tokens: Option<f64>,
    pub score: f64,
    pub rating: f64,
    pub cases: Vec<BenchmarkCaseResult>,
    pub errors: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkSummary {
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub cancelled: bool,
    pub candidates_total: usize,
    pub models_tested: usize,
    pub max_requests: u32,
    pub models: Vec<BenchmarkModelResult>,
    pub results_json_path: String,
    pub results_md_path: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkEvent {
    pub kind: String,
    pub current: usize,
    pub total: usize,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub message: String,
}

// ---------------- 候选模型 ----------------

/// 前端传入的待测模型清单项。
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkModelSpec {
    pub provider: String,
    pub model: String,
}

/// 按前端传入的完整模型清单构建候选（去重；未配 key 的供应商标记跳过）。
pub(crate) fn benchmark_candidates_from_specs(
    settings: &Settings,
    specs: &[BenchmarkModelSpec],
) -> Vec<BenchmarkCandidate> {
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for spec in specs {
        let model = spec.model.trim().to_string();
        if model.is_empty() || !seen.insert(format!("{}|{}", spec.provider, model)) {
            continue;
        }
        let (label, key, _base_url) = provider_creds(settings, &spec.provider);
        let skip_reason = if key.trim().is_empty() {
            Some(format!("{} API Key 未配置", label))
        } else {
            None
        };
        out.push(BenchmarkCandidate {
            provider: spec.provider.clone(),
            provider_label: label,
            model,
            skip_reason,
        });
    }
    out
}

fn provider_creds(settings: &Settings, provider: &str) -> (String, String, String) {
    match provider {
        "deepseek" => (
            "DeepSeek".to_string(),
            settings.deepseek_api_key.clone(),
            settings.deepseek_base_url.clone(),
        ),
        "qwen" => (
            "通义千问".to_string(),
            settings.qwen_api_key.clone(),
            settings.qwen_base_url.clone(),
        ),
        "kimi" => (
            "Kimi".to_string(),
            settings.kimi_api_key.clone(),
            settings.kimi_base_url.clone(),
        ),
        _ => (
            "GLM".to_string(),
            settings.glm_api_key.clone(),
            settings.glm_base_url.clone(),
        ),
    }
}

fn provider_models(settings: &Settings, provider: &str) -> (String, String) {
    match provider {
        "deepseek" => (
            settings.deepseek_model.clone(),
            settings.deepseek_strong_model.clone(),
        ),
        "qwen" => (settings.qwen_model.clone(), settings.qwen_strong_model.clone()),
        "kimi" => (settings.kimi_model.clone(), settings.kimi_strong_model.clone()),
        _ => (settings.glm_model.clone(), settings.glm_strong_model.clone()),
    }
}

pub(crate) fn benchmark_candidates_from_settings(settings: &Settings) -> Vec<BenchmarkCandidate> {
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for provider in ["glm", "deepseek", "qwen", "kimi"] {
        let (label, key, _base_url) = provider_creds(settings, provider);
        if key.trim().is_empty() {
            out.push(BenchmarkCandidate {
                provider: provider.to_string(),
                provider_label: label.clone(),
                model: "（未配置）".to_string(),
                skip_reason: Some(format!("{} API Key 未配置", label)),
            });
            continue;
        }
        let (cheap, strong) = provider_models(settings, provider);
        for model in [cheap, strong] {
            let model = model.trim().to_string();
            if model.is_empty() {
                continue;
            }
            if seen.insert(format!("{}|{}", provider, model)) {
                out.push(BenchmarkCandidate {
                    provider: provider.to_string(),
                    provider_label: label.clone(),
                    model,
                    skip_reason: None,
                });
            }
        }
    }
    out
}

fn model_is_known_free(model: &str) -> bool {
    matches!(
        model.to_ascii_lowercase().as_str(),
        "glm-4.7-flash" | "glm-4.5-flash" | "glm-4-flash-250414"
    )
}

// ---------------- 评分（纯函数，可单测） ----------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ScoreInputs {
    pub tool_rel: f64,
    pub accuracy: f64,
    pub stability: f64,
    pub speed: f64,
    pub cost: f64,
    pub long_context: f64,
}

/// 可解释权重（record 1 初稿）：
/// 工具调用可靠性 25%、CAD/规范准确性 25%、稳定性 15%、速度 15%、成本 10%、长上下文 10%。
pub(crate) fn compute_model_score(inputs: &ScoreInputs) -> f64 {
    (0.25 * inputs.tool_rel
        + 0.25 * inputs.accuracy
        + 0.15 * inputs.stability
        + 0.15 * inputs.speed
        + 0.10 * inputs.cost
        + 0.10 * inputs.long_context)
        .clamp(0.0, 1.0)
}

/// 0..1 分数映射到 1..5 星级（0.5 步进，与 UI 半分评级一致）。
pub(crate) fn score_to_rating(score: f64) -> f64 {
    let raw = 1.0 + score.clamp(0.0, 1.0) * 4.0;
    (raw * 2.0).round() / 2.0
}

fn speed_score(avg_ms: u64) -> f64 {
    match avg_ms {
        0 => 0.0,
        1..=4000 => 1.0,
        4001..=8000 => 0.8,
        8001..=15000 => 0.6,
        15001..=30000 => 0.4,
        _ => 0.2,
    }
}

fn cost_score(model: &str) -> f64 {
    // 占位策略：已知免费模型 1.0，其余 0.5（未核实价格，不主观加分）。
    if model_is_known_free(model) {
        1.0
    } else {
        0.5
    }
}

// ---------------- 单次请求 ----------------

pub(crate) struct ChatResponse {
    pub text: String,
    pub tool_calls: Vec<(String, Value)>,
    pub tool_args_valid: bool,
    pub usage: Option<ProviderTokenUsage>,
}

async fn chat_once(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    model: &str,
    messages: &[Value],
    with_tools: bool,
) -> Result<ChatResponse, String> {
    let base = base_url.trim_end_matches('/');
    let url = format!("{base}/chat/completions");
    // 部分模型（如 Kimi k2/k3）只允许 temperature=1：先试 0，被拒后去掉该字段重试。
    for temperature in [Some(0.0_f64), None] {
        let mut body = json!({
            "model": model,
            "messages": messages,
        });
        if let Some(t) = temperature {
            body["temperature"] = json!(t);
        }
        if with_tools {
            body["tools"] = crate::tools::openai_tools_for(&[
                "draw_line".to_string(),
                "draw_elevator_shaft_protection".to_string(),
                "draw_text".to_string(),
                "move".to_string(),
            ]);
            body["tool_choice"] = json!("auto");
        }

        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("网络请求失败: {e}"))?;
        let status = resp.status();
        let raw = resp.text().await.map_err(|e| format!("读响应失败: {e}"))?;
        if !status.is_success() {
            let err = format!("API {status}: {}", raw.chars().take(300).collect::<String>());
            if temperature.is_some() && err.to_ascii_lowercase().contains("temperature") {
                continue; // 去掉 temperature 字段重试一次
            }
            return Err(err);
        }
        return parse_chat_response(&raw, model);
    }
    Err("temperature 参数不被支持且无法重试".to_string())
}

fn parse_chat_response(raw: &str, model: &str) -> Result<ChatResponse, String> {

    let parsed: Value =
        serde_json::from_str(&raw).map_err(|e| format!("解析响应失败: {e}"))?;
    let usage = parse_openai_usage(parsed.get("usage"), "", model);
    let choice = parsed["choices"]
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| format!("响应缺少 choices: {}", raw.chars().take(200).collect::<String>()))?;
    let text = choice["message"]["content"].as_str().unwrap_or("").to_string();
    let mut tool_calls: Vec<(String, Value)> = Vec::new();
    let mut tool_args_valid = true;
    if let Some(list) = choice["message"]["tool_calls"].as_array() {
        for tc in list {
            let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
            let args = match tc["function"]["arguments"].as_str() {
                Some(s) => match serde_json::from_str::<Value>(s) {
                    Ok(v) => v,
                    Err(_) => {
                        tool_args_valid = false;
                        Value::String(s.to_string())
                    }
                },
                None => {
                    tool_args_valid = false;
                    Value::Null
                }
            };
            tool_calls.push((name, args));
        }
    }
    Ok(ChatResponse {
        text,
        tool_calls,
        tool_args_valid,
        usage,
    })
}

// ---------------- 各用例评分器（纯函数，可单测） ----------------

pub(crate) fn score_p1_draw(resp: &ChatResponse) -> (f64, String) {
    if resp.tool_calls.is_empty() {
        (0.0, "未调用绘图工具".to_string())
    } else if !resp.tool_args_valid {
        (0.5, "调用了工具但参数 JSON 不合法".to_string())
    } else if resp.tool_calls[0].0 == "draw_line" {
        (1.0, "正确选择 draw_line 且参数合法".to_string())
    } else {
        (0.5, format!("工具选择偏差：{}", resp.tool_calls[0].0))
    }
}

pub(crate) fn score_p2_clarify(resp: &ChatResponse) -> (f64, String) {
    let called_draw = resp
        .tool_calls
        .iter()
        .any(|(n, _)| n == "draw_elevator_shaft_protection");
    let asks = resp.text.contains('？')
        || resp.text.contains('?')
        || resp.text.contains("尺寸")
        || resp.text.contains("请提供")
        || resp.text.contains("多少");
    if !called_draw {
        (1.0, "缺参时未直接出图".to_string())
    } else if asks {
        (0.5, "出图但同时追问缺参".to_string())
    } else {
        (0.0, "缺参时直接用默认值出图".to_string())
    }
}

pub(crate) fn score_p3_safety(resp: &ChatResponse) -> (f64, String) {
    if resp.text.contains("200") {
        (1.0, "回答含 200mm 正确值".to_string())
    } else if resp.text.contains("180") || resp.text.contains("150") {
        (0.5, "给出接近但不正确的数值".to_string())
    } else {
        (0.0, "未给出正确踢脚板数值".to_string())
    }
}

pub(crate) fn score_p4_context(resp: &ChatResponse) -> (f64, String) {
    if resp.text.contains("7000") {
        (1.0, "正确读出 A1 长度".to_string())
    } else {
        (0.0, "未能从对象表读出长度".to_string())
    }
}

pub(crate) fn score_p6_turn2(resp: &ChatResponse) -> (f64, String) {
    let mentions_new = resp.text.contains("5000")
        || resp
            .tool_calls
            .iter()
            .any(|(_, a)| a.to_string().contains("5000"));
    if mentions_new {
        (1.0, "正确接续上下文并更新为 5000mm".to_string())
    } else if resp.text.contains('线') || resp.text.contains("3000") {
        (0.5, "仍引用旧值或丢失新值".to_string())
    } else {
        (0.0, "上下文丢失".to_string())
    }
}

/// 非领域拒绝：零成本本地校验（不调用模型）。
pub(crate) fn run_scope_guard_cases() -> (f64, String) {
    let samples = ["1+1等于几？", "写一首诗", "今天天气怎么样"];
    let mut rejected = 0;
    for s in samples {
        if scoped_response_for_unrelated_request(s).is_some() {
            rejected += 1;
        }
    }
    let allowed = scoped_response_for_unrelated_request("画一条直线").is_none();
    let score = if rejected == samples.len() && allowed {
        1.0
    } else {
        0.5
    };
    (
        score,
        format!("本地守卫拦截 {}/{} 个非领域样例", rejected, samples.len()),
    )
}

// ---------------- 单模型套件 ----------------

fn case_average(cases: &[BenchmarkCaseResult], ids: &[&str]) -> f64 {
    let mut sum = 0.0;
    let mut count = 0;
    for id in ids {
        if let Some(case) = cases.iter().find(|c| c.id == *id) {
            sum += case.score;
            count += 1;
        }
    }
    if count == 0 {
        0.0
    } else {
        sum / count as f64
    }
}

struct RunMetrics {
    requests: usize,
    succeeded: usize,
    durations: Vec<u64>,
    outputs: Vec<u64>,
    errors: Vec<String>,
}

async fn run_case(
    client: &reqwest::Client,
    app: Option<&AppHandle>,
    cand: &BenchmarkCandidate,
    settings: &Settings,
    case_id: &str,
    case_label: &str,
    messages: &[Value],
    with_tools: bool,
    scorer: fn(&ChatResponse) -> (f64, String),
    metrics: &mut RunMetrics,
    budget: u32,
    current: usize,
    total: usize,
) -> BenchmarkCaseResult {
    if CANCELLED.load(Ordering::SeqCst) {
        return BenchmarkCaseResult {
            id: case_id.to_string(),
            label: case_label.to_string(),
            score: 0.0,
            note: "已取消".to_string(),
        };
    }
    metrics.requests += 1;
    let (_, key, base_url) = provider_creds(settings, &cand.provider);
    let pause_ms = provider_pause_ms(&cand.provider);

    // 请求循环：带供应商限流间隔与 429 自动重试（解析 Retry-After）。
    let mut attempt = 0usize;
    let mut last_elapsed: u64 = 0;
    let mut retried = false;
    let result: Result<ChatResponse, String> = loop {
        attempt += 1;
        if CANCELLED.load(Ordering::SeqCst) {
            return BenchmarkCaseResult {
                id: case_id.to_string(),
                label: case_label.to_string(),
                score: 0.0,
                note: "已取消".to_string(),
            };
        }
        if REQUEST_COUNT.load(Ordering::SeqCst) >= budget as u64 {
            return BenchmarkCaseResult {
                id: case_id.to_string(),
                label: case_label.to_string(),
                score: 0.0,
                note: "超出请求预算，跳过".to_string(),
            };
        }
        REQUEST_COUNT.fetch_add(1, Ordering::SeqCst);
        // 供应商限流间隔：同一套件内从第二次请求开始等待（Kimi 21s，其他 2.5s）。
        if attempt > 1 || metrics.requests > 1 {
            tokio::time::sleep(Duration::from_millis(pause_ms)).await;
        }
        let started = std::time::Instant::now();
        let r = chat_once(client, &base_url, &key, &cand.model, messages, with_tools).await;
        last_elapsed = started.elapsed().as_millis() as u64;
        match r {
            Ok(resp) => break Ok(resp),
            Err(e) => {
                if attempt < MAX_ATTEMPTS_PER_CASE {
                    retried = true;
                    let wait_ms = if is_rate_limit_error(&e) {
                        // 429 无 Retry-After（如智谱「当前访问量过大」）时用较长退避，
                        // 而不是 2.5s 立即重试——这类过载通常持续数分钟。
                        rate_limit_wait_ms(&e, pause_ms)
                    } else {
                        pause_ms.max(2_000)
                    };
                    if let Some(app) = app {
                        let _ = app.emit(
                            "benchmark:event",
                            BenchmarkEvent {
                                kind: "case".to_string(),
                                current,
                                total,
                                provider: Some(cand.provider.clone()),
                                model: Some(cand.model.clone()),
                                message: format!(
                                    "{} / {} · {} 请求失败，{}s 后重试（第 {} 次）",
                                cand.provider_label,
                                cand.model,
                                case_label,
                                wait_ms / 1000,
                                attempt
                            ),
                        },
                        );
                    }
                    tokio::time::sleep(Duration::from_millis(wait_ms)).await;
                    continue;
                }
                break Err(e);
            }
        }
    };
    metrics.durations.push(last_elapsed);
    if let Some(app) = app {
        let _ = app.emit(
            "benchmark:event",
            BenchmarkEvent {
                kind: "case".to_string(),
                current,
                total,
                provider: Some(cand.provider.clone()),
                model: Some(cand.model.clone()),
                message: format!("{} / {} · {} · {}ms", cand.provider_label, cand.model, case_label, last_elapsed),
            },
        );
    }
    match result {
        Ok(resp) => {
            metrics.succeeded += 1;
            if let Some(usage) = &resp.usage {
                if let Some(out) = usage.output_tokens {
                    metrics.outputs.push(out);
                }
            }
            let (score, note) = scorer(&resp);
            BenchmarkCaseResult {
                id: case_id.to_string(),
                label: case_label.to_string(),
                score,
                note: if retried {
                    format!("{note}（限流后重试成功）")
                } else {
                    note
                },
            }
        }
        Err(e) => {
            let reason = redact(&e, settings);
            metrics.errors.push(format!("{}: {}", case_id, reason));
            BenchmarkCaseResult {
                id: case_id.to_string(),
                label: case_label.to_string(),
                score: 0.0,
                note: format!("请求失败：{}", reason.chars().take(120).collect::<String>()),
            }
        }
    }
}

async fn run_model_suite(
    client: &reqwest::Client,
    app: Option<&AppHandle>,
    cand: &BenchmarkCandidate,
    settings: &Settings,
    budget: u32,
    current: usize,
    total: usize,
) -> BenchmarkModelResult {
    let mut metrics = RunMetrics {
        requests: 0,
        succeeded: 0,
        durations: Vec::new(),
        outputs: Vec::new(),
        errors: Vec::new(),
    };
    let mut cases: Vec<BenchmarkCaseResult> = Vec::new();

    // p1 简单绘图意图：固定参数直线。
    cases.push(
        run_case(
            client, app, cand, settings, "p1", "简单绘图意图", &[json!({"role": "user", "content": "画一条从原点出发、长 1000mm 的水平直线"})],
            true, score_p1_draw, &mut metrics, budget, current, total,
        )
        .await,
    );
    // p2 缺参澄清：不提供尺寸，看模型是否追问而不是直接出图。
    cases.push(
        run_case(
            client, app, cand, settings, "p2", "缺参澄清", &[json!({"role": "user", "content": "画一个电梯井口防护门"})],
            true, score_p2_clarify, &mut metrics, budget, current, total,
        )
        .await,
    );
    // p3 规范问答：踢脚板 200mm（知识卡底线）。
    cases.push(
        run_case(
            client, app, cand, settings, "p3", "规范问答", &[json!({"role": "user", "content": "电梯井口防护门的踢脚板高度应为多少？"})],
            false, score_p3_safety, &mut metrics, budget, current, total,
        )
        .await,
    );
    // p4 上下文读取：从给定对象表读取数值。
    cases.push(
        run_case(
            client, app, cand, settings, "p4", "上下文读取", &[json!({"role": "user", "content": "会话对象表：A1 = 直线，长度 7000mm；B2 = 圆，半径 300mm。请问 A1 的长度是多少？"})],
            false, score_p4_context, &mut metrics, budget, current, total,
        )
        .await,
    );
    // p5 非领域拒绝：零成本本地守卫（不发请求）。
    let (scope_score, scope_note) = run_scope_guard_cases();
    cases.push(BenchmarkCaseResult {
        id: "p5".to_string(),
        label: "非领域拒绝（本地）".to_string(),
        score: scope_score,
        note: scope_note,
    });
    // p6 长上下文接续：两轮。
    cases.push(
        run_case(
            client, app, cand, settings, "p6_turn1", "多轮接续-首轮", &[json!({"role": "user", "content": "画一条长 3000mm 的竖直线"})],
            true, score_p1_draw, &mut metrics, budget, current, total,
        )
        .await,
    );
    cases.push(
        run_case(
            client, app, cand, settings, "p6_turn2", "多轮接续-次轮", &[
                json!({"role": "user", "content": "画一条长 3000mm 的竖直线"}),
                json!({"role": "assistant", "content": "已调用 draw_line 绘制 3000mm 竖直线。"}),
                json!({"role": "user", "content": "把刚才那条线改成 5000mm"}),
            ],
            true, score_p6_turn2, &mut metrics, budget, current, total,
        )
        .await,
    );

    let tool_rel = case_average(&cases, &["p1", "p2", "p6_turn1"]);
    let accuracy = case_average(&cases, &["p3", "p4"]);
    let long_context = case_average(&cases, &["p6_turn2"]);
    let stability = if metrics.requests == 0 {
        0.0
    } else {
        metrics.succeeded as f64 / metrics.requests as f64
    };
    let avg_duration_ms = if metrics.durations.is_empty() {
        0
    } else {
        (metrics.durations.iter().map(|d| *d as u128).sum::<u128>() / metrics.durations.len() as u128)
            as u64
    };
    let avg_output_tokens = if metrics.outputs.is_empty() {
        None
    } else {
        Some(metrics.outputs.iter().map(|o| *o as f64).sum::<f64>() / metrics.outputs.len() as f64)
    };
    let inputs = ScoreInputs {
        tool_rel,
        accuracy,
        stability,
        speed: speed_score(avg_duration_ms),
        cost: cost_score(&cand.model),
        long_context,
    };
    let score = compute_model_score(&inputs);

    BenchmarkModelResult {
        provider: cand.provider.clone(),
        provider_label: cand.provider_label.clone(),
        model: cand.model.clone(),
        requests: metrics.requests,
        succeeded: metrics.succeeded,
        avg_duration_ms,
        avg_output_tokens,
        score,
        rating: score_to_rating(score),
        cases,
        errors: metrics.errors,
    }
}

/// 无 UI 依赖的单模型补测入口：不持有 AppHandle、不 emit 事件，
/// 供 CLI / 测试环境对个别模型（如持续 429 的 glm-4.7-flash）错峰补测。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) async fn run_headless_model_retest(
    provider: &str,
    model: &str,
) -> Result<BenchmarkModelResult, String> {
    let settings = crate::settings::load_from_default_path().map_err(|e| format!("读取设置失败: {e}"))?;
    let (label, key, _base_url) = provider_creds(&settings, provider);
    if key.trim().is_empty() {
        return Err(format!("{label} API Key 未配置"));
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;
    let cand = BenchmarkCandidate {
        provider: provider.to_string(),
        provider_label: label,
        model: model.to_string(),
        skip_reason: None,
    };
    Ok(run_model_suite(&client, None, &cand, &settings, MAX_BENCHMARK_REQUESTS, 1, 1).await)
}

// ---------------- 结果保存 ----------------

fn save_benchmark_results(
    app: &AppHandle,
    location: SessionStorageLocation,
    summary: &BenchmarkSummary,
) -> Result<(String, String), String> {
    let dir = memory_bundle_dir(app, location)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建记忆目录失败: {e}"))?;
    let json_path = dir.join("benchmark-results.json");
    let md_path = dir.join("benchmark-results.md");
    let json_text = serde_json::to_string_pretty(summary)
        .map_err(|e| format!("序列化基准结果失败: {e}"))?;
    std::fs::write(&json_path, &json_text).map_err(|e| format!("写入 benchmark-results.json 失败: {e}"))?;
    // 按时间戳归档，避免每次运行互相覆盖历史结果
    let stamp = utc_file_stamp(summary.started_at_ms);
    let archive_json = dir.join(format!("benchmark-results-{stamp}.json"));
    std::fs::write(&archive_json, &json_text)
        .map_err(|e| format!("写入基准归档 {stamp}.json 失败: {e}"))?;

    let started = summary.started_at_ms;
    let mut md = String::new();
    md.push_str(&format!(
        "# CADEgg 模型基准测试结果

- 测试时间戳：{started} ms（UTC）
- 测试模型数：{}
- 已取消：{}

",
        summary.models_tested, summary.cancelled
    ));
    md.push_str("| 模型 | 评分(0-1) | 星级(1-5) | 成功/请求 | 平均耗时 | 平均输出 token |
");
    md.push_str("| --- | --- | --- | --- | --- | --- |
");
    for m in &summary.models {
        md.push_str(&format!(
            "| {} / {} | {:.3} | {:.1} | {}/{} | {}ms | {} |
",
            m.provider_label,
            m.model,
            m.score,
            m.rating,
            m.succeeded,
            m.requests,
            m.avg_duration_ms,
            m.avg_output_tokens
                .map(|v| format!("{v:.1}"))
                .unwrap_or_else(|| "未返回".to_string())
        ));
    }
    md.push_str("
## 权重（可解释）

工具调用可靠性 25% · CAD/规范准确性 25% · 稳定性 15% · 速度 15% · 成本 10% · 长上下文 10%
");
    md.push_str("
## 用例明细

");
    for m in &summary.models {
        md.push_str(&format!("
### {} / {}

", m.provider_label, m.model));
        for c in &m.cases {
            md.push_str(&format!("- {}（{}）：{:.2} — {}
", c.id, c.label, c.score, c.note));
        }
        for e in &m.errors {
            md.push_str(&format!("- 错误：{e}
"));
        }
    }
    std::fs::write(&md_path, &md).map_err(|e| format!("写入 benchmark-results.md 失败: {e}"))?;
    let archive_md = dir.join(format!("benchmark-results-{stamp}.md"));
    std::fs::write(&archive_md, &md).map_err(|e| format!("写入基准归档 {stamp}.md 失败: {e}"))?;
    Ok((
        json_path.display().to_string(),
        md_path.display().to_string(),
    ))
}

// ---------------- Tauri 命令 ----------------

#[tauri::command]
pub fn benchmark_candidates(app: AppHandle) -> Result<Vec<BenchmarkCandidate>, String> {
    let settings = crate::settings::load(&app)?;
    Ok(benchmark_candidates_from_settings(&settings))
}

#[tauri::command]
pub fn cancel_model_benchmark() {
    CANCELLED.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub fn read_benchmark_results(
    app: AppHandle,
    location: SessionStorageLocation,
) -> Result<Option<BenchmarkSummary>, String> {
    let dir = memory_bundle_dir(&app, location)?;
    let json_path = dir.join("benchmark-results.json");
    if !json_path.is_file() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&json_path).map_err(|e| format!("读取基准结果失败: {e}"))?;
    serde_json::from_str::<BenchmarkSummary>(&raw).map(Some).map_err(|e| format!("解析基准结果失败: {e}"))
}

#[tauri::command]
pub async fn run_model_benchmark(
    app: AppHandle,
    location: SessionStorageLocation,
    max_requests: Option<u32>,
    models: Option<Vec<BenchmarkModelSpec>>,
) -> Result<BenchmarkSummary, String> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("已有基准测试在运行".to_string());
    }
    let result = run_benchmark_inner(&app, location, max_requests, models).await;
    RUNNING.store(false, Ordering::SeqCst);
    result
}

async fn run_benchmark_inner(
    app: &AppHandle,
    location: SessionStorageLocation,
    max_requests: Option<u32>,
    models: Option<Vec<BenchmarkModelSpec>>,
) -> Result<BenchmarkSummary, String> {
    CANCELLED.store(false, Ordering::SeqCst);
    REQUEST_COUNT.store(0, Ordering::SeqCst);
    let settings = crate::settings::load(app)?;
    let budget = max_requests
        .unwrap_or(MAX_BENCHMARK_REQUESTS)
        .min(MAX_BENCHMARK_REQUESTS);
    let started_at_ms = now_ms();

    let candidates = match models.as_ref().filter(|list| !list.is_empty()) {
        Some(specs) => benchmark_candidates_from_specs(&settings, specs),
        None => benchmark_candidates_from_settings(&settings),
    };
    let runnable: Vec<&BenchmarkCandidate> =
        candidates.iter().filter(|c| c.skip_reason.is_none()).collect();
    let total_models = runnable.len();
    let estimate = total_models * REQUESTS_PER_MODEL;
    let _ = app.emit(
        "benchmark:event",
        BenchmarkEvent {
            kind: "started".to_string(),
            current: 0,
            total: total_models,
            provider: None,
            model: None,
            message: format!("预计 {} 个模型 × {} 次请求 ≈ {} 次（上限 {}）", total_models, REQUESTS_PER_MODEL, estimate, budget),
        },
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

    let mut models: Vec<BenchmarkModelResult> = Vec::new();
    for (index, cand) in runnable.iter().enumerate() {
        if CANCELLED.load(Ordering::SeqCst) {
            break;
        }
        let _ = app.emit(
            "benchmark:event",
            BenchmarkEvent {
                kind: "model_start".to_string(),
                current: index + 1,
                total: total_models,
                provider: Some(cand.provider.clone()),
                model: Some(cand.model.clone()),
                message: format!("开始测试 {} / {}", cand.provider_label, cand.model),
            },
        );
        let result = run_model_suite(&client, Some(app), cand, &settings, budget, index + 1, total_models).await;
        let _ = app.emit(
            "benchmark:event",
            BenchmarkEvent {
                kind: "model_done".to_string(),
                current: index + 1,
                total: total_models,
                provider: Some(cand.provider.clone()),
                model: Some(cand.model.clone()),
                message: format!(
                    "{} / {} 完成：评分 {:.3}，星级 {:.1}",
                    cand.provider_label, cand.model, result.score, result.rating
                ),
            },
        );
        models.push(result);
    }

    let cancelled = CANCELLED.load(Ordering::SeqCst);
    let mut summary = BenchmarkSummary {
        started_at_ms,
        finished_at_ms: Some(now_ms()),
        cancelled,
        candidates_total: candidates.len(),
        models_tested: models.len(),
        max_requests: budget,
        models,
        results_json_path: String::new(),
        results_md_path: String::new(),
    };
    let (json_path, md_path) = save_benchmark_results(app, location, &summary)?;
    summary.results_json_path = json_path;
    summary.results_md_path = md_path;
    let _ = app.emit(
        "benchmark:event",
        BenchmarkEvent {
            kind: if cancelled { "cancelled".to_string() } else { "finished".to_string() },
            current: summary.models_tested,
            total: total_models,
            provider: None,
            model: None,
            message: format!("基准完成：{} 个模型，结果已保存", summary.models_tested),
        },
    );
    Ok(summary)
}

// ---------------- 测试 ----------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_response(text: &str, calls: Vec<(&str, &str)>, args_valid: bool) -> ChatResponse {
        ChatResponse {
            text: text.to_string(),
            tool_calls: calls
                .into_iter()
                .map(|(n, a)| (n.to_string(), serde_json::from_str(a).unwrap_or(Value::String(a.to_string()))))
                .collect(),
            tool_args_valid: args_valid,
            usage: None,
        }
    }

    #[test]
    fn score_weights_and_rating_mapping() {
        let perfect = ScoreInputs { tool_rel: 1.0, accuracy: 1.0, stability: 1.0, speed: 1.0, cost: 1.0, long_context: 1.0 };
        assert!((compute_model_score(&perfect) - 1.0).abs() < 1e-9);
        let zero = ScoreInputs { tool_rel: 0.0, accuracy: 0.0, stability: 0.0, speed: 0.0, cost: 0.0, long_context: 0.0 };
        assert!((compute_model_score(&zero) - 0.0).abs() < 1e-9);
        assert_eq!(score_to_rating(1.0), 5.0);
        assert_eq!(score_to_rating(0.5), 3.0);
        assert_eq!(score_to_rating(0.0), 1.0);
    }

    #[test]
    fn p1_scorer_rewards_draw_line_with_valid_json() {
        let ok = fake_response("我来画线", vec![("draw_line", "{\"start\":[0,0],\"end\":[1000,0]}")], true);
        assert_eq!(score_p1_draw(&ok).0, 1.0);
        let invalid = fake_response("我来画线", vec![("draw_line", "not-json")], false);
        assert_eq!(score_p1_draw(&invalid).0, 0.5);
        let none = fake_response("我可以帮你画", vec![], true);
        assert_eq!(score_p1_draw(&none).0, 0.0);
    }

    #[test]
    fn p2_scorer_penalizes_silent_default_draw() {
        let asks = fake_response("请提供井口尺寸和高度", vec![], true);
        assert_eq!(score_p2_clarify(&asks).0, 1.0);
        let silent_draw = fake_response("好的", vec![("draw_elevator_shaft_protection", "{}")], true);
        assert_eq!(score_p2_clarify(&silent_draw).0, 0.0);
    }

    #[test]
    fn p3_scorer_accepts_200mm() {
        assert_eq!(score_p3_safety(&fake_response("踢脚板高度应为 200mm", vec![], true)).0, 1.0);
        assert_eq!(score_p3_safety(&fake_response("应该是 150mm", vec![], true)).0, 0.5);
        assert_eq!(score_p3_safety(&fake_response("不清楚", vec![], true)).0, 0.0);
    }

    #[test]
    fn p4_scorer_requires_7000() {
        assert_eq!(score_p4_context(&fake_response("A1 的长度是 7000mm", vec![], true)).0, 1.0);
        assert_eq!(score_p4_context(&fake_response("A1 是直线", vec![], true)).0, 0.0);
    }

    #[test]
    fn p6_turn2_scorer_checks_new_value() {
        assert_eq!(score_p6_turn2(&fake_response("已把该线改成 5000mm", vec![], true)).0, 1.0);
        assert_eq!(score_p6_turn2(&fake_response("该线长 3000mm", vec![], true)).0, 0.5);
        assert_eq!(score_p6_turn2(&fake_response("好的", vec![], true)).0, 0.0);
    }

    #[test]
    fn candidates_from_specs_dedupe_and_skip_unconfigured() {
        let settings = Settings {
            provider: "glm".to_string(),
            glm_api_key: "glm-key-12345678".to_string(),
            ..Default::default()
        };
        let specs = vec![
            BenchmarkModelSpec { provider: "glm".to_string(), model: "glm-5.2".to_string() },
            BenchmarkModelSpec { provider: "glm".to_string(), model: "glm-5.2".to_string() },
            BenchmarkModelSpec { provider: "deepseek".to_string(), model: "deepseek-v4-pro".to_string() },
        ];
        let candidates = benchmark_candidates_from_specs(&settings, &specs);
        assert_eq!(candidates.len(), 2, "同 provider+model 应去重");
        assert!(candidates.iter().all(|c| c.provider_label.len() > 0));
        let glm = candidates.iter().find(|c| c.provider == "glm").unwrap();
        assert!(glm.skip_reason.is_none());
        let ds = candidates.iter().find(|c| c.provider == "deepseek").unwrap();
        assert!(ds.skip_reason.as_deref().unwrap_or("").contains("未配置"));
    }

    /// 无 UI 单模型补测入口（ignored）：对持续 429 的模型错峰补测。
    /// 用法：cargo test retest_model_headless -- --ignored --nocapture
    /// 默认补测 glm-4.7-flash；改环境变量 CADEGG_RETEST_MODEL 可换模型。
    #[test]
    #[ignore = "requires live API credentials — 按需错峰补测"]
    fn retest_model_headless() {
        let model = std::env::var("CADEGG_RETEST_MODEL").unwrap_or_else(|_| "glm-4.7-flash".to_string());
        let provider = if model.starts_with("glm") {
            "glm"
        } else if model.starts_with("deepseek") {
            "deepseek"
        } else if model.starts_with("qwen") {
            "qwen"
        } else {
            "kimi"
        };
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let result = rt.block_on(super::run_headless_model_retest(provider, &model));
        match result {
            Ok(summary) => {
                println!("=== 单模型补测结果 ===");
                println!("{}", serde_json::to_string_pretty(&summary).unwrap());
                assert!(
                    summary.succeeded == summary.requests,
                    "补测未全通过：{}/{}，错误：{:?}",
                    summary.succeeded,
                    summary.requests,
                    summary.errors
                );
            }
            Err(e) => panic!("补测失败: {e}"),
        }
    }

    #[test]
    fn candidates_dedupe_and_skip_unconfigured() {
        let settings = Settings::default();
        let candidates = benchmark_candidates_from_settings(&settings);
        assert_eq!(candidates.len(), 4);
        assert!(candidates.iter().all(|c| c.skip_reason.is_some()));

        let settings = Settings {
            provider: "glm".to_string(),
            glm_api_key: "glm-key-12345678".to_string(),
            glm_model: "glm-4.5".to_string(),
            glm_strong_model: "glm-4.5".to_string(),
            ..Default::default()
        };
        let candidates = benchmark_candidates_from_settings(&settings);
        let runnable: Vec<_> = candidates.iter().filter(|c| c.skip_reason.is_none()).collect();
        assert_eq!(runnable.len(), 1, "glm 双档位相同模型应去重");
        assert_eq!(runnable[0].model, "glm-4.5");
    }

    #[test]
    fn rate_limit_wait_without_retry_after_uses_long_backoff() {
        assert_eq!(rate_limit_wait_ms("该模型当前访问量过大，请您稍后再试", 2_500), 30_000);
        assert_eq!(rate_limit_wait_ms("please try again after 12 seconds", 2_500), 12_000);
        // Retry-After 秒数小于供应商间隔时，仍遵守供应商间隔下限
        assert_eq!(rate_limit_wait_ms("try again after 5 seconds", 21_000), 21_000);
    }

    #[test]
    fn parse_retry_after_handles_kimi_429_message() {
        let msg = r#"API 429 Too Many Requests: {"error":{"message":"... please try again after 1 seconds","type":"rate_limit_reached_error"}}"#;
        assert_eq!(parse_retry_after_secs(msg), Some(5), "至少等待 5s");
        let msg2 = "please try again after 12 seconds";
        assert_eq!(parse_retry_after_secs(msg2), Some(12));
        assert_eq!(parse_retry_after_secs("没有重试信息"), None);
        assert!(is_rate_limit_error(msg));
        assert!(!is_rate_limit_error("网络请求失败: timeout"));
    }

    #[test]
    fn utc_file_stamp_formats_epoch_millis() {
        // 2026-08-19 10:00:00 UTC = 1787133600000 ms
        assert_eq!(utc_file_stamp(1_787_133_600_000), "20260819T100000Z");
        // 1970-01-01 00:00:00
        assert_eq!(utc_file_stamp(0), "19700101T000000Z");
        // 跨日边界：23:59:59.500 仍是 235959Z（毫秒被截断为秒）
        assert_eq!(utc_file_stamp(1_787_183_999_500), "20260819T235959Z");
    }

    #[test]
    fn provider_pause_respects_kimi_rpm() {
        assert_eq!(provider_pause_ms("kimi"), 21_000);
        assert_eq!(provider_pause_ms("glm"), 2_500);
    }

    #[test]
    fn scope_guard_cases_zero_cost() {
        let (score, note) = run_scope_guard_cases();
        assert_eq!(score, 1.0);
        assert!(note.contains("3/3"));
    }
}
