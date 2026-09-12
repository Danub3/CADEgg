//! RAG 知识卡层 —— 把受控的安全防护标准图册知识卡转成模型可读上下文。
//!
//! 技术路线第 1 层（初计划）：不把整篇图册塞给模型，而是拆成结构化片段（知识卡），
//! 在出图/追问闭环前按场景检索注入。运行时 agent **只查知识卡，不检索原始 PDF**。
//!
//! ## 数据分层（可迭代、可溯源）
//! - `data/atlas/*.json`      —— 知识卡（面向 agent 的"结论"，字段见 schema）
//! - `data/sources/*.json`    —— 规范原文摘录（面向"溯源"，逐条带出处与页码）
//! - `data/schema/*.json`     —— 知识卡 JSON Schema（约束字段，保证可迭代）
//!
//! ## 检索方式（支持多卡快速命中）
//! 1. `by_scene(scene)`：按卡片 `scene` 字段精确匹配（当前调用方仍走此路径）；
//! 2. `search(query)`：按 `keywords` 数组做关键词打分匹配，卡片多了能快速命中相关卡。
//!    - 运行时扫描 `data/atlas/` 目录（新增场景只需丢一张卡）；
//!    - 磁盘不存在时回退到 `include_str!` 内置卡片（打包后可离线运行）。
//!
//! ## 迭代约定
//! - 新增规范/更新规范：改 `data/sources/` 摘录 + 改 `data/atlas/` 卡片 + 递增 `version`；
//! - 每条关键结论必须挂 `citations`（source_id + excerpt_id + page），否则无法溯源；
//! - 卡片要「指向性」：`scene` 唯一 + `keywords` 覆盖用户常见说法，便于命中。

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// 内置兜底卡片：把 `data/atlas/` 下已收录的卡片编译进二进制。
/// 相对路径以本文件（`src-tauri/src/knowledge.rs`）为基准：`../` → `src-tauri/`。
const ELEVATOR_SHAFT_CARD: &str = include_str!("../../data/atlas/elevator_shaft_protection.json");
const ELEVATOR_SHAFT_SAFETY_NET_CARD: &str =
    include_str!("../../data/atlas/elevator_shaft_safety_net.json");
const EDGE_GUARDRAIL_CARD: &str = include_str!("../../data/atlas/edge_guardrail.json");
const OPENING_COVER_CARD: &str = include_str!("../../data/atlas/opening_cover.json");
const DRAFTING_STANDARD_CARD: &str = include_str!("../../data/atlas/cad_drafting_standard.json");

/// 内置来源摘录：证据门控在离线打包后仍可逐条核验 citation，而不是只信知识卡文本。
const JGJ_80_2016_SOURCE: &str = include_str!("../../data/sources/jgj-80-2016.json");
const MOHURD_2019_90_SOURCE: &str = include_str!("../../data/sources/mohurd-2019-90-guidance.json");
const GB_T_50001_SOURCE: &str = include_str!("../../data/sources/gb-t-50001.json");

/// 运行时知识卡目录（相对工作目录，dev 模式下即仓库根 `D:\CADEgg`）。
const ATLAS_DIR_CANDIDATES: [&str; 2] = ["data/atlas", "src-tauri/../data/atlas"];
const SOURCE_DIR_CANDIDATES: [&str; 2] = ["data/sources", "src-tauri/../data/sources"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStatus {
    Verified,
    Refused,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EvidenceIssue {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excerpt_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VerifiedCitation {
    pub source_id: String,
    pub source_version: String,
    pub source_title: String,
    pub excerpt_id: String,
    pub section: String,
    pub page: i64,
    pub raw_text: String,
    pub mandatory: bool,
}

/// 面向前端、日志和后续规则层的稳定证据对象。
/// `verified` 只表示引用能与当前本地 source 数据逐项对应，不代表工程方案已获批准。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EvidenceBundle {
    pub schema: &'static str,
    pub scene: String,
    pub card_version: Option<String>,
    pub status: EvidenceStatus,
    pub citations: Vec<VerifiedCitation>,
    pub issues: Vec<EvidenceIssue>,
}

impl EvidenceBundle {
    pub fn is_verified(&self) -> bool {
        self.status == EvidenceStatus::Verified
    }

    pub fn refusal_message(&self) -> String {
        let details = if self.issues.is_empty() {
            "未获得可核验的本地证据".to_string()
        } else {
            self.issues
                .iter()
                .map(|issue| issue.message.as_str())
                .collect::<Vec<_>>()
                .join("；")
        };
        format!(
            "当前不能依据本地证据对场景 `{}` 给出工程结论或执行出图：{}。请补充并核验对应规范原文/知识卡，或转交专业人员通过现行规范和官方渠道复核。",
            self.scene, details
        )
    }
}

/// 内置兜底卡片表：scene -> 卡片 JSON。
/// 新增场景时，除了在 `data/atlas/` 放文件，也在这里补一行 `("scene", include_str!(...))`。
fn builtin_cards() -> Vec<(&'static str, &'static str)> {
    vec![
        ("elevator_shaft_protection", ELEVATOR_SHAFT_CARD),
        ("elevator_shaft_safety_net", ELEVATOR_SHAFT_SAFETY_NET_CARD),
        ("edge_guardrail", EDGE_GUARDRAIL_CARD),
        ("opening_cover", OPENING_COVER_CARD),
        ("cad_drafting_standard", DRAFTING_STANDARD_CARD),
    ]
}

/// 解析卡片里的 scene 字段（用于磁盘扫描时匹配）。
fn scene_of(raw: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|v| {
            v.get("scene")
                .and_then(|s| s.as_str().map(|x| x.to_string()))
        })
}

/// 扫描磁盘 `data/atlas/` 目录，返回所有知识卡 JSON 文本。
fn scan_disk_cards() -> Vec<String> {
    let mut out = Vec::new();
    for dir in ATLAS_DIR_CANDIDATES {
        let path = Path::new(dir);
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("json") {
                    if let Ok(text) = fs::read_to_string(&p) {
                        if !text.trim().is_empty() {
                            out.push(text);
                        }
                    }
                }
            }
        }
    }
    out
}

/// 磁盘 + 内置去重后的全部卡片原始文本。
fn all_cards() -> Vec<String> {
    let mut cards = Vec::new();
    let mut seen = BTreeSet::new();

    for text in scan_disk_cards() {
        if let Some(scene) = scene_of(&text) {
            if seen.insert(scene) {
                cards.push(text);
            }
        }
    }
    for (scene, text) in builtin_cards() {
        if seen.insert(scene.to_string()) {
            cards.push(text.to_string());
        }
    }
    cards
}

fn builtin_sources() -> [&'static str; 3] {
    [JGJ_80_2016_SOURCE, MOHURD_2019_90_SOURCE, GB_T_50001_SOURCE]
}

fn source_id_of(value: &serde_json::Value) -> Option<&str> {
    value.get("source_id").and_then(|item| item.as_str())
}

fn scan_disk_sources() -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for dir in SOURCE_DIR_CANDIDATES {
        if let Ok(entries) = fs::read_dir(Path::new(dir)) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
                    if let Ok(text) = fs::read_to_string(path) {
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                            out.push(value);
                        }
                    }
                }
            }
        }
    }
    out
}

fn all_sources() -> BTreeMap<String, serde_json::Value> {
    let mut sources = BTreeMap::new();
    for value in scan_disk_sources().into_iter().chain(
        builtin_sources()
            .into_iter()
            .filter_map(|raw| serde_json::from_str::<serde_json::Value>(raw).ok()),
    ) {
        if let Some(source_id) = source_id_of(&value).map(str::to_string) {
            sources.entry(source_id).or_insert(value);
        }
    }
    sources
}

fn source_version(source: &serde_json::Value) -> String {
    for key in ["version", "code", "document_no"] {
        if let Some(value) = source.get(key).and_then(|item| item.as_str()) {
            if !value.trim().is_empty() {
                return value.trim().to_string();
            }
        }
    }
    source
        .get("publish_year")
        .and_then(|item| item.as_i64())
        .map(|year| year.to_string())
        .unwrap_or_default()
}

fn issue(
    code: &str,
    message: String,
    source_id: Option<&str>,
    excerpt_id: Option<&str>,
) -> EvidenceIssue {
    EvidenceIssue {
        code: code.to_string(),
        message,
        source_id: source_id.map(str::to_string),
        excerpt_id: excerpt_id.map(str::to_string),
    }
}

fn valid_card_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.parse::<u64>().is_ok())
}

fn assess_card_evidence(
    requested_scene: &str,
    raw: &str,
    sources: &BTreeMap<String, serde_json::Value>,
) -> EvidenceBundle {
    let mut bundle = EvidenceBundle {
        schema: "cadegg-evidence/v1",
        scene: requested_scene.to_string(),
        card_version: None,
        status: EvidenceStatus::Refused,
        citations: Vec::new(),
        issues: Vec::new(),
    };
    let card = match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(value) => value,
        Err(error) => {
            bundle.issues.push(issue(
                "invalid_card_json",
                format!("知识卡 JSON 无法解析：{error}"),
                None,
                None,
            ));
            return bundle;
        }
    };

    match card.get("scene").and_then(|value| value.as_str()) {
        Some(scene) if scene == requested_scene => {}
        Some(scene) => bundle.issues.push(issue(
            "scene_mismatch",
            format!("知识卡场景 `{scene}` 与请求场景 `{requested_scene}` 不一致"),
            None,
            None,
        )),
        None => bundle.issues.push(issue(
            "missing_scene",
            "知识卡缺少 scene".to_string(),
            None,
            None,
        )),
    }

    match card.get("version").and_then(|value| value.as_str()) {
        Some(version) if valid_card_version(version) => {
            bundle.card_version = Some(version.to_string())
        }
        Some(version) => bundle.issues.push(issue(
            "invalid_card_version",
            format!("知识卡版本 `{version}` 不是 x.y.z 格式"),
            None,
            None,
        )),
        None => bundle.issues.push(issue(
            "missing_card_version",
            "知识卡缺少版本号".to_string(),
            None,
            None,
        )),
    }

    let declared_source_versions = card
        .get("source_versions")
        .and_then(|value| value.as_object());
    if declared_source_versions.is_none() {
        bundle.issues.push(issue(
            "missing_source_versions",
            "知识卡缺少 source_versions，无法检测来源版本漂移".to_string(),
            None,
            None,
        ));
    }

    let Some(citations) = card.get("citations").and_then(|value| value.as_array()) else {
        bundle.issues.push(issue(
            "missing_citations",
            "知识卡没有 citations 数组".to_string(),
            None,
            None,
        ));
        return bundle;
    };
    if citations.is_empty() {
        bundle.issues.push(issue(
            "empty_citations",
            "知识卡 citations 为空".to_string(),
            None,
            None,
        ));
        return bundle;
    }

    let mut seen = BTreeSet::new();
    for citation in citations {
        let source_id = citation.get("source_id").and_then(|value| value.as_str());
        let excerpt_id = citation.get("excerpt_id").and_then(|value| value.as_str());
        let page = citation.get("page").and_then(|value| value.as_i64());
        let section = citation.get("section").and_then(|value| value.as_str());
        let key = format!("{}:{}", source_id.unwrap_or(""), excerpt_id.unwrap_or(""));
        if !seen.insert(key) {
            bundle.issues.push(issue(
                "duplicate_citation",
                "知识卡包含重复引用".to_string(),
                source_id,
                excerpt_id,
            ));
            continue;
        }

        let (Some(source_id), Some(excerpt_id), Some(page), Some(section)) =
            (source_id, excerpt_id, page, section)
        else {
            bundle.issues.push(issue(
                "incomplete_citation",
                "引用必须包含 source_id、excerpt_id、section 和 page".to_string(),
                source_id,
                excerpt_id,
            ));
            continue;
        };
        let Some(source) = sources.get(source_id) else {
            bundle.issues.push(issue(
                "source_not_found",
                format!("本地 sources 中不存在 `{source_id}`"),
                Some(source_id),
                Some(excerpt_id),
            ));
            continue;
        };
        let version = source_version(source);
        if version.is_empty() {
            bundle.issues.push(issue(
                "source_version_missing",
                format!("来源 `{source_id}` 缺少 version/code/document_no/publish_year"),
                Some(source_id),
                Some(excerpt_id),
            ));
            continue;
        }
        let version_matches = match declared_source_versions
            .and_then(|versions| versions.get(source_id))
            .and_then(|value| value.as_str())
        {
            Some(expected) if expected == version => true,
            Some(expected) => {
                bundle.issues.push(issue(
                    "source_version_mismatch",
                    format!("知识卡锁定的来源版本 `{expected}` 与本地来源版本 `{version}` 不一致"),
                    Some(source_id),
                    Some(excerpt_id),
                ));
                false
            }
            None => {
                bundle.issues.push(issue(
                    "source_version_not_pinned",
                    format!("知识卡未锁定来源 `{source_id}` 的版本"),
                    Some(source_id),
                    Some(excerpt_id),
                ));
                false
            }
        };
        let excerpt = source
            .get("excerpts")
            .and_then(|value| value.as_array())
            .and_then(|items| {
                items.iter().find(|item| {
                    item.get("id").and_then(|value| value.as_str()) == Some(excerpt_id)
                })
            });
        let Some(excerpt) = excerpt else {
            bundle.issues.push(issue(
                "excerpt_not_found",
                format!("来源 `{source_id}` 中不存在摘录 `{excerpt_id}`"),
                Some(source_id),
                Some(excerpt_id),
            ));
            continue;
        };

        let mut citation_valid = version_matches;
        let source_page = excerpt.get("page").and_then(|value| value.as_i64());
        if source_page != Some(page) {
            citation_valid = false;
            bundle.issues.push(issue(
                "page_mismatch",
                format!("引用页码 {page} 与来源摘录页码 {:?} 不一致", source_page),
                Some(source_id),
                Some(excerpt_id),
            ));
        }
        let source_section = excerpt
            .get("section")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if source_section != section {
            citation_valid = false;
            bundle.issues.push(issue(
                "section_mismatch",
                format!("引用条款 `{section}` 与来源条款 `{source_section}` 不一致"),
                Some(source_id),
                Some(excerpt_id),
            ));
        }
        let raw_text = excerpt
            .get("raw_text")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim();
        if raw_text.is_empty() {
            citation_valid = false;
            bundle.issues.push(issue(
                "raw_text_missing",
                "来源摘录缺少原文".to_string(),
                Some(source_id),
                Some(excerpt_id),
            ));
        }
        if citation_valid {
            bundle.citations.push(VerifiedCitation {
                source_id: source_id.to_string(),
                source_version: version,
                source_title: source
                    .get("title")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_string(),
                excerpt_id: excerpt_id.to_string(),
                section: section.to_string(),
                page,
                raw_text: raw_text.to_string(),
                mandatory: excerpt
                    .get("mandatory")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false),
            });
        }
    }

    if bundle.issues.is_empty() && !bundle.citations.is_empty() {
        bundle.status = EvidenceStatus::Verified;
    }
    bundle
}

/// 核验场景知识卡的每条引用，并返回稳定的证据对象。
pub fn resolve_scene_evidence(scene: &str) -> EvidenceBundle {
    match load_scene_card(scene) {
        Some(raw) => assess_card_evidence(scene, &raw, &all_sources()),
        None => EvidenceBundle {
            schema: "cadegg-evidence/v1",
            scene: scene.to_string(),
            card_version: None,
            status: EvidenceStatus::Refused,
            citations: Vec::new(),
            issues: vec![issue(
                "knowledge_card_not_found",
                format!("本地 atlas 中不存在场景 `{scene}` 的知识卡"),
                None,
                None,
            )],
        },
    }
}

/// 只在证据全部通过时渲染模型上下文，并把同一证据对象一并注入，供回答和日志引用。
pub fn render_verified_scene_context(scene: &str) -> Result<String, EvidenceBundle> {
    // 核验和渲染共用同一份卡片快照，避免两次磁盘读取之间发生版本漂移。
    let Some(raw) = load_scene_card(scene) else {
        return Err(resolve_scene_evidence(scene));
    };
    let evidence = assess_card_evidence(scene, &raw, &all_sources());
    if !evidence.is_verified() {
        return Err(evidence);
    }
    let Some(card) = render_card_context(&raw, scene) else {
        return Err(EvidenceBundle {
            status: EvidenceStatus::Refused,
            issues: vec![issue(
                "card_render_failed",
                format!("场景 `{scene}` 的知识卡无法渲染"),
                None,
                None,
            )],
            ..evidence
        });
    };
    let evidence_json = serde_json::to_string_pretty(&evidence).unwrap_or_else(|_| "{}".into());
    Ok(format!("{card}\n证据对象：\n{evidence_json}"))
}

pub fn unmatched_safety_refusal() -> String {
    "当前安全请求未命中已核验的本地场景证据，因此不能给出工程结论或执行出图。请补充具体作业部位、施工阶段、现场尺寸和防护目标，或由专业人员依据现行规范及官方渠道复核。".to_string()
}

#[tauri::command]
pub fn get_scene_evidence(scene: String) -> EvidenceBundle {
    resolve_scene_evidence(scene.trim())
}

/// 按场景名精确检索知识卡原始 JSON 文本。
pub fn load_scene_card(scene: &str) -> Option<String> {
    all_cards()
        .into_iter()
        .find(|c| scene_of(c).as_deref() == Some(scene))
}

/// 提取卡片的关键词数组（keywords 字段，可能缺省）。
fn keywords_of(raw: &str) -> Vec<String> {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|v| v.get("keywords").cloned())
        .and_then(|v| v.as_array().cloned())
        .map(|arr| {
            arr.iter()
                .filter_map(|it| it.as_str().map(|s| s.to_lowercase()))
                .collect()
        })
        .unwrap_or_default()
}

/// 关键词检索：按 `keywords` 命中数打分，返回命中卡片的 scene 列表（按相关度降序）。
///
/// 用法：卡片多了之后，根据用户输入命中最相关的一张（或多张）卡，再渲染上下文。
/// 无命中时返回空。
pub fn search_scenes(query: &str) -> Vec<String> {
    let q = query.to_lowercase();
    let mut scored: Vec<(usize, String)> = all_cards()
        .into_iter()
        .filter_map(|c| {
            let scene = scene_of(&c)?;
            let score = keywords_of(&c)
                .iter()
                .filter(|kw| q.contains(kw.as_str()))
                .count();
            if score > 0 {
                Some((score, scene))
            } else {
                None
            }
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, s)| s).collect()
}

/// 列出当前所有可用场景名。
#[allow(dead_code)]
pub fn list_scenes() -> Vec<String> {
    let mut scenes: Vec<String> = all_cards().iter().filter_map(|c| scene_of(c)).collect();
    scenes.dedup();
    scenes
}

fn str_items<'a>(v: &'a serde_json::Value, key: &str) -> Vec<&'a str> {
    v.get(key)
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|it| it.as_str())
                .collect::<Vec<&'a str>>()
        })
        .unwrap_or_default()
}

/// 把知识卡原始 JSON 渲染成一段可注入系统提示的「标准图册上下文」。
///
/// 提取对出图/追问最关键的字段：适用条件、必配构件、尺寸规则、绘图约定、禁忌项、材料表规则、溯源。
/// 渲染失败时返回 `None`（不影响主流程）。
pub fn render_scene_context(scene: &str) -> Option<String> {
    let raw = load_scene_card(scene)?;
    render_card_context(&raw, scene)
}

/// 由卡片原始文本渲染上下文（内部共用）。
fn render_card_context(raw: &str, scene: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;

    let name = v.get("name").and_then(|s| s.as_str()).unwrap_or(scene);
    let mut lines: Vec<String> = vec![format!("【标准图册知识卡：{name}】")];

    for it in str_items(&v, "applicable_conditions") {
        if lines.last().map(|l| l.as_str()) != Some("适用条件：") {
            lines.push("适用条件：".to_string());
        }
        lines.push(format!("  - {it}"));
    }
    for it in str_items(&v, "required_components") {
        if lines.last().map(|l| l.as_str()) != Some("必配构件：") {
            lines.push("必配构件：".to_string());
        }
        lines.push(format!("  - {it}"));
    }
    if let Some(items) = v.get("dimension_rules").and_then(|a| a.as_array()) {
        lines.push("尺寸规则：".to_string());
        for it in items {
            let rule = it.get("rule").and_then(|s| s.as_str()).unwrap_or("");
            if !rule.is_empty() {
                lines.push(format!("  - {rule}"));
            }
        }
    }
    for it in str_items(&v, "line_rules") {
        if lines.last().map(|l| l.as_str()) != Some("图线规则：") {
            lines.push("图线规则：".to_string());
        }
        lines.push(format!("  - {it}"));
    }
    for it in str_items(&v, "font_rules") {
        if lines.last().map(|l| l.as_str()) != Some("字体规则：") {
            lines.push("字体规则：".to_string());
        }
        lines.push(format!("  - {it}"));
    }
    for it in str_items(&v, "drawing_conventions") {
        if lines.last().map(|l| l.as_str()) != Some("绘图约定：") {
            lines.push("绘图约定：".to_string());
        }
        lines.push(format!("  - {it}"));
    }
    for it in str_items(&v, "forbidden_items") {
        if lines.last().map(|l| l.as_str()) != Some("禁忌项（不得违反）：") {
            lines.push("禁忌项（不得违反）：".to_string());
        }
        lines.push(format!("  - {it}"));
    }
    for it in str_items(&v, "material_table_rules") {
        if lines.last().map(|l| l.as_str()) != Some("材料表规则：") {
            lines.push("材料表规则：".to_string());
        }
        lines.push(format!("  - {it}"));
    }
    if let Some(citations) = v.get("citations").and_then(|a| a.as_array()) {
        lines.push("规范溯源：".to_string());
        for c in citations {
            let sec = c.get("section").and_then(|s| s.as_str()).unwrap_or("");
            let page = c.get("page").and_then(|p| p.as_i64());
            let src = c.get("source_id").and_then(|s| s.as_str()).unwrap_or("");
            match page {
                Some(pg) => lines.push(format!("  - {src} {sec}（第{pg}页）")),
                None => lines.push(format!("  - {src} {sec}")),
            }
        }
    }

    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_card_is_valid_json_and_has_scene() {
        let v: serde_json::Value = serde_json::from_str(ELEVATOR_SHAFT_CARD).unwrap();
        assert_eq!(v["scene"].as_str(), Some("elevator_shaft_protection"));
        // 必须带溯源引用（这是"经得起推敲"的底线）。
        assert!(v["citations"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false));
    }

    #[test]
    fn render_scene_context_includes_key_fields() {
        let ctx = render_scene_context("elevator_shaft_protection").unwrap();
        assert!(ctx.contains("标准图册知识卡"));
        assert!(ctx.contains("必配构件"));
        assert!(ctx.contains("尺寸规则"));
        assert!(ctx.contains("禁忌项"));
        assert!(ctx.contains("规范溯源"));
        // 电梯井口防护门的确定性底线（1.5m 高、200mm 踢脚板、2.1m 规格）。
        assert!(ctx.contains("1.5m"));
        assert!(ctx.contains("200mm"));
        assert!(ctx.contains("2.1m"));
        // 绘图约定（坐标默认原点）也必须注入，避免模型卡在追要 x/y。
        assert!(ctx.contains("绘图约定"));
        assert!(ctx.contains("x=0, y=0"));
        // 溯源必须指向 JGJ 80-2016 4.2.2 与住建部图册 2.7.4。
        assert!(ctx.contains("jgj-80-2016"));
        assert!(ctx.contains("4.2.2"));
        assert!(ctx.contains("2.7.4"));
    }

    #[test]
    fn drafting_standard_card_renders() {
        let ctx = render_scene_context("cad_drafting_standard").unwrap();
        assert!(ctx.contains("尺寸规则"));
        assert!(ctx.contains("图线规则"));
        assert!(ctx.contains("字体规则"));
        assert!(ctx.contains("gb-t-50001"));
    }

    #[test]
    fn search_matches_elevator_keywords() {
        let scenes = search_scenes("画一个电梯井口防护");
        assert!(scenes.contains(&"elevator_shaft_protection".to_string()));
        assert_eq!(
            scenes
                .iter()
                .filter(|scene| *scene == "elevator_shaft_protection")
                .count(),
            1
        );
    }

    #[test]
    fn search_matches_edge_guardrail_keywords() {
        let scenes = search_scenes("画一个楼层临边防护栏杆");
        assert!(scenes.contains(&"edge_guardrail".to_string()));
        assert!(!scenes.contains(&"elevator_shaft_protection".to_string()));

        let ctx = render_scene_context("edge_guardrail").unwrap();
        assert!(ctx.contains("普通临边防护栏杆"));
        assert!(ctx.contains("1.2m"));
        assert!(ctx.contains("2m"));
        assert!(ctx.contains("不得调用电梯井口防护门工具替代"));
    }

    #[test]
    fn search_matches_drafting_keywords() {
        let scenes = search_scenes("尺寸标注要符合规范");
        assert!(scenes.contains(&"cad_drafting_standard".to_string()));
    }

    #[test]
    fn unknown_scene_returns_none() {
        assert!(load_scene_card("no_such_scene").is_none());
    }

    #[test]
    fn verified_evidence_resolves_to_source_excerpt_and_raw_text() {
        let evidence = resolve_scene_evidence("elevator_shaft_protection");
        assert_eq!(evidence.status, EvidenceStatus::Verified, "{evidence:?}");
        assert_eq!(evidence.card_version.as_deref(), Some("1.2.0"));
        assert!(evidence.issues.is_empty());
        assert!(evidence.citations.iter().any(|citation| {
            citation.source_id == "jgj-80-2016"
                && citation.excerpt_id == "jgj-80-2016-4.2.2"
                && citation.page == 14
                && citation.source_version == "JGJ 80-2016"
                && citation.raw_text.contains("高度不应小于1.5m")
                && citation.mandatory
        }));

        let context = render_verified_scene_context("elevator_shaft_protection").unwrap();
        assert!(context.contains("cadegg-evidence/v1"));
        assert!(context.contains("\"status\": \"verified\""));
        assert!(context.contains("高度不应小于1.5m"));
    }

    #[test]
    fn every_available_card_has_resolvable_evidence() {
        for scene in list_scenes() {
            let evidence = resolve_scene_evidence(&scene);
            assert!(
                evidence.is_verified(),
                "场景 {scene} 的本地证据不完整：{:?}",
                evidence.issues
            );
        }
    }

    #[test]
    fn opening_cover_card_returns_auditable_evidence() {
        let evidence = resolve_scene_evidence("opening_cover");
        assert_eq!(evidence.status, EvidenceStatus::Verified);
        assert_eq!(evidence.card_version.as_deref(), Some("1.0.0"));
        assert_eq!(evidence.citations.len(), 2);
        assert!(evidence.issues.is_empty());
    }

    #[test]
    fn citation_page_mismatch_is_refused_instead_of_silently_rendered() {
        let raw = r#"{
          "scene": "test_scene",
          "version": "1.0.0",
          "citations": [{
            "source_id": "jgj-80-2016",
            "excerpt_id": "jgj-80-2016-4.2.2",
            "section": "4.2.2 洞口作业（电梯井口防护）",
            "page": 999
          }]
        }"#;
        let evidence = assess_card_evidence("test_scene", raw, &all_sources());
        assert_eq!(evidence.status, EvidenceStatus::Refused);
        assert!(evidence
            .issues
            .iter()
            .any(|item| item.code == "page_mismatch"));
        assert!(evidence.citations.is_empty());
    }

    #[test]
    fn source_version_drift_is_refused() {
        let raw = r#"{
          "scene": "test_scene",
          "version": "1.0.0",
          "source_versions": {"jgj-80-2016": "JGJ 80-2099"},
          "citations": [{
            "source_id": "jgj-80-2016",
            "excerpt_id": "jgj-80-2016-4.2.2",
            "section": "4.2.2 洞口作业（电梯井口防护）",
            "page": 14
          }]
        }"#;
        let evidence = assess_card_evidence("test_scene", raw, &all_sources());
        assert_eq!(evidence.status, EvidenceStatus::Refused);
        assert!(evidence
            .issues
            .iter()
            .any(|item| item.code == "source_version_mismatch"));
        assert!(evidence.citations.is_empty());
    }

    #[test]
    fn missing_source_or_excerpt_is_refused() {
        let raw = r#"{
          "scene": "test_scene",
          "version": "1.0.0",
          "citations": [{
            "source_id": "missing-source",
            "excerpt_id": "missing-excerpt",
            "section": "1.0",
            "page": 1
          }]
        }"#;
        let evidence = assess_card_evidence("test_scene", raw, &BTreeMap::new());
        assert_eq!(evidence.status, EvidenceStatus::Refused);
        assert!(evidence
            .issues
            .iter()
            .any(|item| item.code == "source_not_found"));
    }

    #[test]
    fn list_scenes_contains_all() {
        let scenes = list_scenes();
        assert!(scenes.contains(&"elevator_shaft_protection".to_string()));
        assert!(scenes.contains(&"cad_drafting_standard".to_string()));
    }
}
