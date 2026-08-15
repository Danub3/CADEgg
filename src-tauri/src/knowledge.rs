//! RAG 知识卡层 —— 把受控的安全防护标准图册知识卡转成模型可读上下文。
//!
//! 技术路线第 1 层（初计划）：不把整篇图册塞给模型，而是拆成结构化片段（知识卡），
//! 在出图/追问闭环前按场景检索注入。
//!
//! ## 数据分层（可迭代、可溯源）
//! - `data/atlas/*.json`      —— 知识卡（面向 agent 的"结论"，字段见 schema）
//! - `data/sources/*.json`    —— 规范原文摘录（面向"溯源"，逐条带出处与页码）
//! - `data/schema/*.json`     —— 知识卡 JSON Schema（约束字段，保证可迭代）
//!
//! ## 检索顺序
//! 1. 运行时扫描 `data/atlas/` 目录，按卡片的 `scene` 字段匹配（新增场景只需丢一张卡）；
//! 2. 磁盘不存在时回退到 `include_str!` 内置卡片（打包后可离线运行）。
//!
//! ## 迭代约定
//! - 新增规范/更新规范：改 `data/sources/` 摘录 + 改 `data/atlas/` 卡片 + 递增 `version`；
//! - 每条关键结论必须挂 `citations`（source_id + excerpt_id + page），否则无法溯源。

use std::fs;
use std::path::Path;

/// 内置兜底卡片：把 `data/atlas/` 下已收录的卡片编译进二进制。
/// 相对路径以本文件（`src-tauri/src/knowledge.rs`）为基准：`../` → `src-tauri/`。
const ELEVATOR_SHAFT_CARD: &str = include_str!("../../data/atlas/elevator_shaft_protection.json");

/// 运行时知识卡目录（相对工作目录，dev 模式下即仓库根 `D:\CADEgg`）。
const ATLAS_DIR_CANDIDATES: [&str; 2] = ["data/atlas", "src-tauri/../data/atlas"];

/// 内置兜底卡片表：scene -> 卡片 JSON。
/// 新增场景时，除了在 `data/atlas/` 放文件，也在这里补一行 `("scene", include_str!(...))`。
fn builtin_cards() -> Vec<(&'static str, &'static str)> {
    vec![("elevator_shaft_protection", ELEVATOR_SHAFT_CARD)]
}

/// 解析卡片里的 scene 字段（用于磁盘扫描时匹配）。
fn scene_of(raw: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|v| v.get("scene").and_then(|s| s.as_str().map(|x| x.to_string())))
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

/// 按场景名检索知识卡原始 JSON 文本。
///
/// - 磁盘优先：扫描 `data/atlas/`，按卡片 `scene` 字段匹配（改动即时生效，新增场景即插即用）；
/// - 否则回退到编译期内置卡片表。
pub fn load_scene_card(scene: &str) -> Option<String> {
    for text in scan_disk_cards() {
        if scene_of(&text).as_deref() == Some(scene) {
            return Some(text);
        }
    }
    builtin_cards()
        .into_iter()
        .find(|(s, _)| *s == scene)
        .map(|(_, text)| text.to_string())
}

/// 列出当前所有可用场景名（磁盘 + 内置去重）。
pub fn list_scenes() -> Vec<String> {
    let mut scenes: Vec<String> = Vec::new();
    for text in scan_disk_cards() {
        if let Some(s) = scene_of(&text) {
            if !scenes.contains(&s) {
                scenes.push(s);
            }
        }
    }
    for (s, _) in builtin_cards() {
        let s = s.to_string();
        if !scenes.contains(&s) {
            scenes.push(s);
        }
    }
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
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;

    let name = v.get("name").and_then(|s| s.as_str()).unwrap_or(scene);
    let mut lines: Vec<String> = vec![format!("【标准图册知识卡：{name}】")];

    for it in str_items(&v, "applicable_conditions") {
        if lines.last().map(|l| l.as_str()) != Some("适用条件：") {
            lines.push("适用条件：".to_string());
        }
        lines.push(format!("  - {it}"));
    }
    for it in str_items(&v, "required_components") {
        lines.push("必配构件：".to_string());
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
        assert!(v["citations"].as_array().map(|a| !a.is_empty()).unwrap_or(false));
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
        // 溯源必须指向住建部图册 2.7.4 第 126/127 页。
        assert!(ctx.contains("2.7.4"));
        assert!(ctx.contains("126"));
        assert!(ctx.contains("127"));
    }

    #[test]
    fn unknown_scene_returns_none() {
        assert!(load_scene_card("no_such_scene").is_none());
    }

    #[test]
    fn list_scenes_contains_elevator_shaft() {
        assert!(list_scenes().contains(&"elevator_shaft_protection".to_string()));
    }
}
