//! RAG 知识卡层 —— 把受控的安全防护标准图册知识卡转成模型可读上下文。
//!
//! 技术路线第 1 层（初计划）：不把整篇图册塞给模型，而是拆成结构化片段（知识卡），
//! 在出图/追问闭环前按场景检索注入。当前只有一个受控场景「电梯井口临边防护」。
//!
//! 加载顺序：磁盘文件优先（便于迭代编辑） → `include_str!` 内置兜底（打包后可离线运行）。

use std::fs;

/// 内置兜底：把 `data/atlas/elevator_shaft_protection.json` 直接编译进二进制。
/// 相对路径以本文件（`src-tauri/src/knowledge.rs`）为基准：`../` → `src-tauri/`。
const ELEVATOR_SHAFT_CARD: &str = include_str!("../../data/atlas/elevator_shaft_protection.json");

/// 磁盘上的知识卡路径（相对工作目录，dev 模式下即仓库根 `D:\CADEgg`）。
const ATLAS_RELATIVE_PATHS: [&str; 2] = [
    "data/atlas/elevator_shaft_protection.json",
    "src-tauri/../data/atlas/elevator_shaft_protection.json",
];

/// 按场景名检索知识卡原始 JSON 文本。
///
/// - 磁盘优先：若 `data/atlas/` 存在且可读，读磁盘版本（改动即时生效）；
/// - 否则回退到编译期内置的 `include_str!` 内容。
pub fn load_scene_card(scene: &str) -> Option<String> {
    if scene == "elevator_shaft_protection" {
        for rel in ATLAS_RELATIVE_PATHS {
            if let Ok(text) = fs::read_to_string(rel) {
                if !text.trim().is_empty() {
                    return Some(text);
                }
            }
        }
        return Some(ELEVATOR_SHAFT_CARD.to_string());
    }
    None
}

/// 把知识卡原始 JSON 渲染成一段可注入系统提示的「标准图册上下文」。
///
/// 提取对出图/追问最关键的字段：适用条件、必配构件、尺寸规则、禁忌项、材料表规则。
/// 渲染失败时返回 `None`（不影响主流程）。
pub fn render_scene_context(scene: &str) -> Option<String> {
    let raw = load_scene_card(scene)?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;

    let name = v.get("name").and_then(|s| s.as_str()).unwrap_or(scene);
    let mut lines: Vec<String> = vec![format!("【标准图册知识卡：{name}】")];

    if let Some(items) = v.get("applicable_conditions").and_then(|a| a.as_array()) {
        lines.push("适用条件：".to_string());
        for it in items {
            if let Some(s) = it.as_str() {
                lines.push(format!("  - {s}"));
            }
        }
    }
    if let Some(items) = v.get("required_components").and_then(|a| a.as_array()) {
        lines.push("必配构件：".to_string());
        for it in items {
            if let Some(s) = it.as_str() {
                lines.push(format!("  - {s}"));
            }
        }
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
    if let Some(items) = v.get("drawing_conventions").and_then(|a| a.as_array()) {
        lines.push("绘图约定：".to_string());
        for it in items {
            if let Some(s) = it.as_str() {
                lines.push(format!("  - {s}"));
            }
        }
    }
    if let Some(items) = v.get("forbidden_items").and_then(|a| a.as_array()) {
        lines.push("禁忌项（不得违反）：".to_string());
        for it in items {
            if let Some(s) = it.as_str() {
                lines.push(format!("  - {s}"));
            }
        }
    }
    if let Some(items) = v.get("material_table_rules").and_then(|a| a.as_array()) {
        lines.push("材料表规则：".to_string());
        for it in items {
            if let Some(s) = it.as_str() {
                lines.push(format!("  - {s}"));
            }
        }
    }
    if let Some(src) = v.get("source").and_then(|s| s.as_str()) {
        if !src.is_empty() {
            lines.push(format!("来源：{src}"));
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
    }

    #[test]
    fn render_scene_context_includes_key_fields() {
        let ctx = render_scene_context("elevator_shaft_protection").unwrap();
        assert!(ctx.contains("标准图册知识卡"));
        assert!(ctx.contains("必配构件"));
        assert!(ctx.contains("尺寸规则"));
        assert!(ctx.contains("禁忌项"));
        assert!(ctx.contains("材料表规则"));
        // 关键规范值必须出现在上下文中，保证模型拿到确定性底线。
        assert!(ctx.contains("1200"));
        assert!(ctx.contains("2000"));
        assert!(ctx.contains("180"));
        // 绘图约定（坐标默认原点）也必须注入，避免模型卡在追要 x/y。
        assert!(ctx.contains("绘图约定"));
        assert!(ctx.contains("x=0, y=0"));
    }

    #[test]
    fn unknown_scene_returns_none() {
        assert!(load_scene_card("no_such_scene").is_none());
    }
}
