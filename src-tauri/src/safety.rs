use serde::Serialize;

pub const GUARD_DOOR_HEIGHT_MM: f64 = 1500.0;
pub const TOE_BOARD_HEIGHT_MM: f64 = 200.0;
pub const DOOR_WIDTH_NARROW_M: f64 = 1.5;
pub const DOOR_WIDTH_WIDE_M: f64 = 2.1;
pub const DOOR_WIDTH_THRESHOLD_MM: f64 = 1800.0;

#[derive(Serialize, Debug, Clone)]
pub struct ValidationCheck {
    pub id: &'static str,
    pub label: &'static str,
    pub passed: bool,
}

#[derive(Serialize, Debug, Clone)]
pub struct MaterialTable {
    pub guard_door: String,
    pub toe_board_height: f64,
    pub warning_sign: bool,
    pub material_table_included: bool,
}

#[derive(Serialize, Debug, Clone)]
pub struct ElevatorShaftValidation {
    pub ok: bool,
    pub issues: Vec<&'static str>,
    pub checks: Vec<ValidationCheck>,
    pub material_table: MaterialTable,
}

pub fn guard_door_width_spec_m(opening_width_mm: f64) -> f64 {
    if opening_width_mm <= DOOR_WIDTH_THRESHOLD_MM {
        DOOR_WIDTH_NARROW_M
    } else {
        DOOR_WIDTH_WIDE_M
    }
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

pub fn is_elevator_shaft_request(user_input: &str) -> bool {
    let text = user_input.to_lowercase();
    contains_any(
        &text,
        &[
            "电梯井",
            "电梯口",
            "电梯洞",
            "井道",
            "电梯洞口",
            "电梯门洞",
            "elevator shaft",
        ],
    ) || (text.contains("井口")
        && contains_any(&text, &["防护", "防护门", "踢脚板", "挡脚板", "坠落"]))
}

fn has_unqualified_dimension_marker(text: &str, marker: &str, excluded_prefixes: &[&str]) -> bool {
    text.match_indices(marker).any(|(idx, _)| {
        let before = &text[..idx];
        !excluded_prefixes
            .iter()
            .any(|prefix| before.trim_end().ends_with(prefix))
    })
}

fn has_opening_width(text: &str) -> bool {
    contains_any(
        text,
        &[
            "opening_width",
            "shaft_width",
            "井口宽",
            "井口净宽",
            "洞口宽",
            "门洞宽",
            "开口宽",
        ],
    ) || (text.contains("width") && !contains_any(text, &["guard_width", "door_width"]))
        || has_unqualified_dimension_marker(text, "宽", &["防护门", "门", "栏杆", "警示牌"])
}

fn has_opening_height(text: &str) -> bool {
    contains_any(
        text,
        &[
            "opening_height",
            "shaft_height",
            "井口高",
            "井口高度",
            "洞口高",
            "门洞高",
            "开口高",
            "进深",
        ],
    ) || (text.contains("height")
        && !contains_any(text, &["guard_height", "door_height", "toe_board_height"]))
        || has_unqualified_dimension_marker(
            text,
            "高",
            &[
                "防护门",
                "门",
                "栏杆",
                "上杆",
                "下杆",
                "踢脚板",
                "挡脚板",
                "警示牌",
            ],
        )
}

pub fn missing_elevator_shaft_params(user_input: &str) -> Vec<&'static str> {
    let text = user_input.to_lowercase();
    let wants_draw = contains_any(&text, &["画", "绘制", "生成", "做", "出图", "创建", "加"]);

    if !wants_draw {
        return Vec::new();
    }

    let mut missing = Vec::new();
    if !has_opening_width(&text) {
        missing.push("井口宽度");
    }
    if !has_opening_height(&text) {
        missing.push("井口高度/进深");
    }
    missing
}

pub fn elevator_shaft_clarification_prompt(user_input: &str) -> Option<String> {
    let missing = missing_elevator_shaft_params(user_input);
    if missing.is_empty() {
        return None;
    }

    let mut lines = vec![
        "用户请求绘制电梯井口防护门，但缺少以下关键尺寸，请先向用户追问确认，不要自行编造："
            .to_string(),
    ];
    for (i, field) in missing.iter().enumerate() {
        lines.push(format!("{}. {}", i + 1, field));
    }
    lines.push(
        "防护门高度（1.5m）、踢脚板（200mm）按规范定值，无需追问；警示牌和材料表未说明时默认包含。"
            .to_string(),
    );
    Some(lines.join("\n"))
}

pub fn validate_elevator_shaft_protection(
    opening_width: f64,
    opening_height: f64,
    guard_height: f64,
    toe_board_height: f64,
    include_warning_sign: bool,
    include_material_table: bool,
) -> ElevatorShaftValidation {
    let mut checks = Vec::new();
    let mut issues = Vec::new();

    let mut add_check = |id: &'static str, label: &'static str, passed: bool| {
        checks.push(ValidationCheck { id, label, passed });
        if !passed {
            issues.push(label);
        }
    };

    add_check(
        "opening_size_valid",
        "井口宽度和高度已提供且为正数",
        opening_width > 0.0 && opening_height > 0.0,
    );
    add_check(
        "guard_door_height_valid",
        "防护门高度为 1500mm（1.5m）",
        (guard_height - GUARD_DOOR_HEIGHT_MM).abs() < 0.5,
    );
    add_check(
        "toe_board_present",
        "防护门底部踢脚板高度为 200mm",
        (toe_board_height - TOE_BOARD_HEIGHT_MM).abs() < 0.5,
    );
    add_check(
        "warning_sign_present",
        "警示牌「当心坠落 严禁抛物」已配置",
        include_warning_sign,
    );
    add_check(
        "material_table_present",
        "材料表已配置",
        include_material_table,
    );
    add_check(
        "dimension_complete",
        "井口尺寸、防护门高、踢脚板高度标注齐全",
        opening_width > 0.0 && opening_height > 0.0 && guard_height > 0.0 && toe_board_height > 0.0,
    );

    let door_spec_m = guard_door_width_spec_m(opening_width);
    ElevatorShaftValidation {
        ok: issues.is_empty(),
        issues,
        checks,
        material_table: MaterialTable {
            guard_door: format!("{}m 上翻式防护门", door_spec_m),
            toe_board_height,
            warning_sign: include_warning_sign,
            material_table_included: include_material_table,
        },
    }
}

pub fn validation_to_pretty_json(validation: &ElevatorShaftValidation) -> Result<String, String> {
    serde_json::to_string_pretty(validation).map_err(|e| format!("序列化校核结果失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevator_shaft_detection_does_not_catch_generic_edge_guardrail() {
        assert!(is_elevator_shaft_request(
            "画一个电梯井口防护，井口宽 2000，高 1800"
        ));
        assert!(is_elevator_shaft_request("井口防护门怎么画"));
        assert!(!is_elevator_shaft_request("画一个楼层临边防护栏杆"));
        assert!(!is_elevator_shaft_request("基坑边安全防护怎么做"));
    }

    #[test]
    fn elevator_request_accepts_slang() {
        assert!(is_elevator_shaft_request("画一个电梯洞"));
        assert!(is_elevator_shaft_request("电梯洞口的防护"));
        assert!(!is_elevator_shaft_request("画一个普通的洞"));
    }

    #[test]
    fn missing_params_ignore_guard_door_dimensions() {
        let missing = missing_elevator_shaft_params("画电梯井口防护，井口宽 2000，防护门高 1500");
        assert!(!missing.contains(&"井口宽度"));
        assert!(missing.contains(&"井口高度/进深"));
    }

    #[test]
    fn missing_params_accept_common_opening_dimension_phrasing() {
        assert!(missing_elevator_shaft_params("画电梯井口防护，井口宽 2000，高 1800").is_empty());
        assert!(missing_elevator_shaft_params(
            "画电梯井防护门 opening_width=2000 opening_height=1800"
        )
        .is_empty());
    }

    #[test]
    fn validation_payload_matches_standard_values() {
        let validation =
            validate_elevator_shaft_protection(2000.0, 1800.0, 1500.0, 200.0, true, true);
        assert!(validation.ok);
        assert_eq!(validation.material_table.guard_door, "2.1m 上翻式防护门");

        let validation =
            validate_elevator_shaft_protection(1500.0, 1800.0, 1200.0, 180.0, true, true);
        assert!(!validation.ok);
        assert!(validation
            .issues
            .iter()
            .any(|issue| issue.contains("防护门高度")));
    }
}
