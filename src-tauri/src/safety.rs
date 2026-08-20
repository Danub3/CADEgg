use serde::Serialize;

pub const GUARD_DOOR_HEIGHT_MM: f64 = 1500.0;
pub const TOE_BOARD_HEIGHT_MM: f64 = 200.0;
pub const DOOR_BOTTOM_GAP_MAX_MM: f64 = 50.0;
pub const DOOR_WIDTH_NARROW_M: f64 = 1.5;
pub const DOOR_WIDTH_WIDE_M: f64 = 2.1;
pub const DOOR_WIDTH_THRESHOLD_MM: f64 = 1800.0;

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ValidationSeverity {
    Mandatory,
    Recommended,
    #[allow(dead_code)]
    Unverified,
}

#[derive(Serialize, Debug, Clone)]
pub struct ValidationCheck {
    pub id: &'static str,
    pub label: &'static str,
    pub passed: bool,
    pub severity: ValidationSeverity,
}

#[derive(Serialize, Debug, Clone)]
pub struct MaterialTable {
    pub guard_door: String,
    pub toe_board_height: f64,
    pub door_bottom_gap: f64,
    pub warning_sign: bool,
    pub material_table_included: bool,
}

/// 施工生命周期状态：装饰阶段临时拆除防护门是真实工况，
/// 拆除期间必须有替代防护、责任人和恢复时间记录。
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    InUse,
    TemporarilyRemoved,
    Restored,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct LifecycleInfo {
    pub state: LifecycleState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removal_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replacement_protection: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responsible_person: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restore_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acceptance_status: Option<String>,
}

/// 从工具 JSON 参数解析生命周期信息；非对象或无 state 时按 in_use 处理。
pub fn parse_lifecycle(v: Option<&serde_json::Value>) -> Option<LifecycleInfo> {
    let obj = v?.as_object()?;
    let state = match obj.get("state").and_then(|s| s.as_str()) {
        Some("temporarily_removed") => LifecycleState::TemporarilyRemoved,
        Some("restored") => LifecycleState::Restored,
        _ => LifecycleState::InUse,
    };
    let text_of = |key: &str| -> Option<String> {
        obj.get(key)
            .and_then(|s| s.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    Some(LifecycleInfo {
        state,
        removal_reason: text_of("removal_reason"),
        replacement_protection: text_of("replacement_protection"),
        responsible_person: text_of("responsible_person"),
        restore_time: text_of("restore_time"),
        acceptance_status: text_of("acceptance_status"),
    })
}

#[derive(Serialize, Debug, Clone)]
pub struct ElevatorShaftValidation {
    pub ok: bool,
    pub issues: Vec<&'static str>,
    pub warnings: Vec<&'static str>,
    pub checks: Vec<ValidationCheck>,
    pub material_table: MaterialTable,
    pub lifecycle: Option<LifecycleInfo>,
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
        "防护门高度未说明时按最低合规值 1500mm，门底间隙未说明时按最大合规值 50mm，踢脚板未说明时按图册推荐值 200mm；警示牌和材料表未说明时默认包含。"
            .to_string(),
    );
    Some(lines.join("\n"))
}

pub fn validate_elevator_shaft_protection(
    opening_width: f64,
    opening_height: f64,
    guard_height: f64,
    toe_board_height: f64,
    door_bottom_gap: f64,
    include_warning_sign: bool,
    include_material_table: bool,
    lifecycle: Option<LifecycleInfo>,
) -> ElevatorShaftValidation {
    let mut checks = Vec::new();
    let mut issues = Vec::new();
    let mut warnings = Vec::new();

    let mut add_check =
        |id: &'static str, label: &'static str, severity: ValidationSeverity, passed: bool| {
            checks.push(ValidationCheck {
                id,
                label,
                passed,
                severity,
            });
            if !passed {
                match severity {
                    ValidationSeverity::Mandatory => issues.push(label),
                    ValidationSeverity::Recommended | ValidationSeverity::Unverified => {
                        warnings.push(label)
                    }
                }
            }
        };

    add_check(
        "opening_size_valid",
        "井口宽度和高度已提供且为正数",
        ValidationSeverity::Mandatory,
        opening_width > 0.0 && opening_height > 0.0,
    );
    add_check(
        "guard_door_height_valid",
        "防护门高度不小于 1500mm（1.5m）",
        ValidationSeverity::Mandatory,
        guard_height >= GUARD_DOOR_HEIGHT_MM - 0.5,
    );
    add_check(
        "door_bottom_gap_valid",
        "防护门底端距地面高度不大于 50mm",
        ValidationSeverity::Mandatory,
        door_bottom_gap >= 0.0 && door_bottom_gap <= DOOR_BOTTOM_GAP_MAX_MM + 0.5,
    );
    add_check(
        "toe_board_present",
        "已设置挡脚板（踢脚板）",
        ValidationSeverity::Mandatory,
        toe_board_height > 0.0,
    );
    add_check(
        "toe_board_height_recommended",
        "踢脚板高度为 200mm（指导图册推荐做法）",
        ValidationSeverity::Recommended,
        (toe_board_height - TOE_BOARD_HEIGHT_MM).abs() < 0.5,
    );
    add_check(
        "warning_sign_present",
        "警示牌「当心坠落 严禁抛物」已配置（指导图册做法）",
        ValidationSeverity::Recommended,
        include_warning_sign,
    );
    add_check(
        "material_table_present",
        "材料表已配置（便于验收复核）",
        ValidationSeverity::Recommended,
        include_material_table,
    );
    add_check(
        "dimension_complete",
        "井口尺寸、防护门高、门底间隙、踢脚板高度标注齐全",
        ValidationSeverity::Mandatory,
        opening_width > 0.0
            && opening_height > 0.0
            && guard_height > 0.0
            && door_bottom_gap >= 0.0
            && toe_board_height > 0.0,
    );

    // 施工生命周期校核：临时拆除是装饰阶段真实工况，拆除期间必须补齐管理记录。
    if let Some(lifecycle) = &lifecycle {
        match lifecycle.state {
            LifecycleState::TemporarilyRemoved => {
                add_check(
                    "lifecycle_removal_reason_required",
                    "已记录临时拆除原因",
                    ValidationSeverity::Mandatory,
                    lifecycle.removal_reason.is_some(),
                );
                add_check(
                    "lifecycle_replacement_protection_required",
                    "拆除期间已记录替代防护措施",
                    ValidationSeverity::Mandatory,
                    lifecycle.replacement_protection.is_some(),
                );
                add_check(
                    "lifecycle_responsible_person_required",
                    "已记录拆除责任人",
                    ValidationSeverity::Mandatory,
                    lifecycle.responsible_person.is_some(),
                );
                add_check(
                    "lifecycle_restore_time_required",
                    "已记录恢复时间",
                    ValidationSeverity::Mandatory,
                    lifecycle.restore_time.is_some(),
                );
            }
            LifecycleState::Restored => {
                add_check(
                    "lifecycle_restore_accepted",
                    "恢复后已验收（验收状态为已验收）",
                    ValidationSeverity::Recommended,
                    lifecycle.acceptance_status.as_deref() == Some("accepted"),
                );
            }
            LifecycleState::InUse => {}
        }
    }

    let door_spec_m = guard_door_width_spec_m(opening_width);
    ElevatorShaftValidation {
        ok: issues.is_empty(),
        issues,
        warnings,
        checks,
        material_table: MaterialTable {
            guard_door: format!("{}m 上翻式防护门", door_spec_m),
            toe_board_height,
            door_bottom_gap,
            warning_sign: include_warning_sign,
            material_table_included: include_material_table,
        },
        lifecycle,
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
        let validation = validate_elevator_shaft_protection(
            2000.0, 1800.0, 1500.0, 200.0, 50.0, true, true, None,
        );
        assert!(validation.ok);
        assert_eq!(validation.material_table.guard_door, "2.1m 上翻式防护门");
        assert!(validation.warnings.is_empty());

        let validation = validate_elevator_shaft_protection(
            1500.0, 1800.0, 1200.0, 200.0, 50.0, true, true, None,
        );
        assert!(!validation.ok);
        assert!(validation
            .issues
            .iter()
            .any(|issue| issue.contains("防护门高度")));
    }

    #[test]
    fn validation_accepts_guard_height_above_minimum_and_checks_bottom_gap() {
        let validation = validate_elevator_shaft_protection(
            2000.0, 1800.0, 1600.0, 200.0, 50.0, true, true, None,
        );
        assert!(validation.ok);

        let validation = validate_elevator_shaft_protection(
            2000.0, 1800.0, 1500.0, 200.0, 75.0, true, true, None,
        );
        assert!(!validation.ok);
        assert!(validation
            .issues
            .iter()
            .any(|issue| issue.contains("防护门底端距地面")));
    }

    #[test]
    fn validation_distinguishes_recommended_items_from_mandatory_failures() {
        let validation = validate_elevator_shaft_protection(
            2000.0, 1800.0, 1500.0, 180.0, 50.0, false, false, None,
        );
        assert!(validation.ok);
        assert!(validation.issues.is_empty());
        assert!(validation
            .warnings
            .iter()
            .any(|warning| warning.contains("踢脚板高度为 200mm")));
        assert!(validation
            .checks
            .iter()
            .any(|check| check.id == "toe_board_height_recommended"
                && check.severity == ValidationSeverity::Recommended
                && !check.passed));
    }

    fn removed_lifecycle() -> LifecycleInfo {
        LifecycleInfo {
            state: LifecycleState::TemporarilyRemoved,
            removal_reason: None,
            replacement_protection: None,
            responsible_person: None,
            restore_time: None,
            acceptance_status: None,
        }
    }

    #[test]
    fn lifecycle_temporary_removal_requires_management_records() {
        let validation = validate_elevator_shaft_protection(
            2000.0,
            1800.0,
            1500.0,
            200.0,
            50.0,
            true,
            true,
            Some(removed_lifecycle()),
        );
        assert!(!validation.ok);
        for expected in ["拆除原因", "替代防护", "责任人", "恢复时间"] {
            assert!(
                validation.issues.iter().any(|i| i.contains(expected)),
                "缺少生命周期强制项: {expected}"
            );
        }
        assert!(validation
            .checks
            .iter()
            .filter(|c| c.id.starts_with("lifecycle_"))
            .all(|c| c.severity == ValidationSeverity::Mandatory && !c.passed));
    }

    #[test]
    fn lifecycle_temporary_removal_with_records_passes() {
        let mut lifecycle = removed_lifecycle();
        lifecycle.removal_reason = Some("装饰阶段抹灰施工".to_string());
        lifecycle.replacement_protection = Some("作业层设临时防护栏杆并挂牌".to_string());
        lifecycle.responsible_person = Some("张三".to_string());
        lifecycle.restore_time = Some("2026-08-25 18:00 前恢复".to_string());
        let validation = validate_elevator_shaft_protection(
            2000.0,
            1800.0,
            1500.0,
            200.0,
            50.0,
            true,
            true,
            Some(lifecycle),
        );
        assert!(validation.ok);
        assert!(validation
            .issues
            .iter()
            .all(|i| !i.contains("拆除原因") && !i.contains("替代防护")));
    }

    #[test]
    fn lifecycle_restored_acceptance_is_recommended() {
        let restored_pending = LifecycleInfo {
            state: LifecycleState::Restored,
            removal_reason: None,
            replacement_protection: None,
            responsible_person: None,
            restore_time: None,
            acceptance_status: Some("pending".to_string()),
        };
        let validation = validate_elevator_shaft_protection(
            2000.0,
            1800.0,
            1500.0,
            200.0,
            50.0,
            true,
            true,
            Some(restored_pending),
        );
        assert!(validation.ok, "恢复验收未完成只是推荐提醒，不应判失败");
        assert!(validation.warnings.iter().any(|w| w.contains("已验收")));

        let restored_accepted = LifecycleInfo {
            state: LifecycleState::Restored,
            removal_reason: None,
            replacement_protection: None,
            responsible_person: None,
            restore_time: None,
            acceptance_status: Some("accepted".to_string()),
        };
        let validation = validate_elevator_shaft_protection(
            2000.0,
            1800.0,
            1500.0,
            200.0,
            50.0,
            true,
            true,
            Some(restored_accepted),
        );
        assert!(validation.ok);
        assert!(!validation.warnings.iter().any(|w| w.contains("已验收")));
    }

    #[test]
    fn lifecycle_parse_falls_back_to_in_use() {
        assert!(parse_lifecycle(None).is_none());
        let v = serde_json::json!({ "state": "in_use" });
        let l = parse_lifecycle(Some(&v)).unwrap();
        assert_eq!(l.state, LifecycleState::InUse);
        let v = serde_json::json!({ "state": "temporarily_removed", "responsible_person": "李四", "restore_time": "8-26" });
        let l = parse_lifecycle(Some(&v)).unwrap();
        assert_eq!(l.state, LifecycleState::TemporarilyRemoved);
        assert_eq!(l.responsible_person.as_deref(), Some("李四"));
        let v = serde_json::json!({ "no_state": true });
        let l = parse_lifecycle(Some(&v)).unwrap();
        assert_eq!(l.state, LifecycleState::InUse);
    }
}
