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

// ── 电梯井内安全平网（JGJ 80-2016 4.2.3）──

pub const SAFETY_NET_MAX_SPACING_MM: f64 = 10000.0;
pub const SAFETY_NET_MAX_WALL_GAP_MM: f64 = 25.0;

#[derive(Serialize, Debug, Clone)]
pub struct SafetyNetSummary {
    pub shaft_width: f64,
    pub shaft_depth: f64,
    pub floor_height: f64,
    /// 实际平网垂直间距 = 2 层层高
    pub net_spacing: f64,
    pub net_to_wall_gap: f64,
    pub upper_isolation: bool,
}

#[derive(Serialize, Debug, Clone)]
pub struct SafetyNetMaterialTable {
    pub safety_net: String,
    pub net_to_wall_gap: f64,
    pub fixing: bool,
    pub upper_isolation: bool,
}

#[derive(Serialize, Debug, Clone)]
pub struct ElevatorShaftSafetyNetValidation {
    pub ok: bool,
    pub issues: Vec<&'static str>,
    pub warnings: Vec<&'static str>,
    pub checks: Vec<ValidationCheck>,
    pub material_table: SafetyNetMaterialTable,
    pub net_summary: SafetyNetSummary,
}

pub fn validate_elevator_shaft_safety_net(
    shaft_width: f64,
    shaft_depth: f64,
    floor_height: f64,
    net_to_wall_gap: f64,
    include_upper_isolation: bool,
) -> ElevatorShaftSafetyNetValidation {
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

    let net_spacing = floor_height * 2.0;

    add_check(
        "shaft_dimensions_valid",
        "井道长度和宽度已提供且为正数",
        ValidationSeverity::Mandatory,
        shaft_width > 0.0 && shaft_depth > 0.0,
    );
    add_check(
        "floor_height_valid",
        "层高已提供且为正数",
        ValidationSeverity::Mandatory,
        floor_height > 0.0,
    );
    add_check(
        "net_spacing_valid",
        "平网垂直间距不大于 10m（每隔 2 层一道）",
        ValidationSeverity::Mandatory,
        floor_height > 0.0 && net_spacing <= SAFETY_NET_MAX_SPACING_MM + 0.5,
    );
    add_check(
        "net_to_wall_gap_nonnegative",
        "平网不大于井道截面（网体与井壁空隙不为负）",
        ValidationSeverity::Mandatory,
        net_to_wall_gap >= 0.0,
    );
    add_check(
        "net_to_wall_gap_recommended",
        "网体与井壁空隙不大于 25mm（指导图册做法）",
        ValidationSeverity::Recommended,
        net_to_wall_gap <= SAFETY_NET_MAX_WALL_GAP_MM + 0.5,
    );
    add_check(
        "upper_isolation_present",
        "施工层上部已设置隔离防护设施",
        ValidationSeverity::Mandatory,
        include_upper_isolation,
    );

    ElevatorShaftSafetyNetValidation {
        ok: issues.is_empty(),
        issues,
        warnings,
        checks,
        material_table: SafetyNetMaterialTable {
            safety_net: format!("安全平网 {}x{}", fmt_dim(shaft_width), fmt_dim(shaft_depth)),
            net_to_wall_gap,
            fixing: true,
            upper_isolation: include_upper_isolation,
        },
        net_summary: SafetyNetSummary {
            shaft_width,
            shaft_depth,
            floor_height,
            net_spacing,
            net_to_wall_gap,
            upper_isolation: include_upper_isolation,
        },
    }
}

fn fmt_dim(v: f64) -> String {
    let s = format!("{v}");
    let s = s.strip_suffix(".0").unwrap_or(&s);
    format!("{s}mm")
}

pub fn missing_safety_net_params(user_input: &str) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if !user_input.contains("层高") {
        missing.push("层高/楼层间距");
    }
    let has_cross = user_input.contains('×')
        || user_input.contains('*')
        || user_input.to_lowercase().contains('x');
    if !has_cross {
        missing.push("井道长×宽");
    }
    missing
}

pub fn safety_net_clarification_prompt(user_input: &str) -> Option<String> {
    let missing = missing_safety_net_params(user_input);
    if missing.is_empty() {
        return None;
    }
    Some(format!(
        "请补充电梯井内安全平网布置信息：{}（单位 mm）。平网按每隔 2 层且不大于 10m 布置一道，施工层上部应设置隔离防护，网体与井壁空隙宜不大于 25mm（JGJ 80-2016 4.2.3）。",
        missing.join("、")
    ))
}

// ── 普通临边防护栏杆（JGJ 80-2016 4.3.1）──

pub const TOP_RAIL_HEIGHT_MM: f64 = 1200.0;
pub const POST_SPACING_MAX_MM: f64 = 2000.0;
pub const TOE_BOARD_MIN_MM: f64 = 180.0;

#[derive(Serialize, Debug, Clone)]
pub struct EdgeGuardrailSummary {
    pub edge_length: f64,
    pub top_rail_height: f64,
    pub post_spacing: f64,
    pub toe_board_height: f64,
    pub dense_mesh_net: bool,
    pub post_count: usize,
}

#[derive(Serialize, Debug, Clone)]
pub struct EdgeGuardrailMaterialTable {
    pub posts: usize,
    pub top_rail: f64,
    pub mid_rail: bool,
    pub toe_board_height: f64,
    pub dense_mesh_net: bool,
}

#[derive(Serialize, Debug, Clone)]
pub struct EdgeGuardrailValidation {
    pub ok: bool,
    pub issues: Vec<&'static str>,
    pub warnings: Vec<&'static str>,
    pub checks: Vec<ValidationCheck>,
    pub material_table: EdgeGuardrailMaterialTable,
    pub guardrail_summary: EdgeGuardrailSummary,
}

/// 立杆数 = 两端立杆 + 按间距布置的中间立杆（不足一段按一段计）。
pub fn edge_guardrail_post_count(edge_length: f64, post_spacing: f64) -> usize {
    if edge_length <= 0.0 || post_spacing <= 0.0 {
        return 0;
    }
    (edge_length / post_spacing).ceil() as usize + 1
}

pub fn validate_edge_guardrail(
    edge_length: f64,
    top_rail_height: f64,
    post_spacing: f64,
    toe_board_height: f64,
    include_dense_mesh_net: bool,
) -> EdgeGuardrailValidation {
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
        "edge_length_valid",
        "临边长度已提供且为正数",
        ValidationSeverity::Mandatory,
        edge_length > 0.0,
    );
    add_check(
        "top_rail_height_valid",
        "上杆距地面高度不小于 1.2m",
        ValidationSeverity::Mandatory,
        top_rail_height >= TOP_RAIL_HEIGHT_MM - 0.5,
    );
    add_check(
        "post_spacing_valid",
        "立杆间距不大于 2m",
        ValidationSeverity::Mandatory,
        post_spacing > 0.0 && post_spacing <= POST_SPACING_MAX_MM + 0.5,
    );
    add_check(
        "toe_board_height_valid",
        "挡脚板高度不小于 180mm",
        ValidationSeverity::Mandatory,
        toe_board_height >= TOE_BOARD_MIN_MM - 0.5,
    );
    add_check(
        "dense_mesh_net_recommended",
        "内侧满挂密目安全网（标准化图册做法）",
        ValidationSeverity::Recommended,
        include_dense_mesh_net,
    );

    EdgeGuardrailValidation {
        ok: issues.is_empty(),
        issues,
        warnings,
        checks,
        material_table: EdgeGuardrailMaterialTable {
            posts: edge_guardrail_post_count(edge_length, post_spacing),
            top_rail: top_rail_height,
            mid_rail: true,
            toe_board_height,
            dense_mesh_net: include_dense_mesh_net,
        },
        guardrail_summary: EdgeGuardrailSummary {
            edge_length,
            top_rail_height,
            post_spacing,
            toe_board_height,
            dense_mesh_net: include_dense_mesh_net,
            post_count: edge_guardrail_post_count(edge_length, post_spacing),
        },
    }
}

/// 临边栏杆缺参追问：临边长度、作业侧、转角/端部收口。
pub fn missing_edge_guardrail_params(user_input: &str) -> Vec<&'static str> {
    let mut missing = Vec::new();
    let has_number = user_input.chars().any(|c| c.is_ascii_digit());
    if !has_number {
        missing.push("临边长度");
    }
    if !(user_input.contains("作业侧")
        || user_input.contains("外侧")
        || user_input.contains("内侧")
        || user_input.contains("临空侧"))
    {
        missing.push("作业侧");
    }
    if !(user_input.contains("转角")
        || user_input.contains("端部")
        || user_input.contains("收口")
        || user_input.contains("拐角"))
    {
        missing.push("转角/端部收口");
    }
    missing
}

pub fn edge_guardrail_clarification_prompt(user_input: &str) -> Option<String> {
    let missing = missing_edge_guardrail_params(user_input);
    if missing.is_empty() {
        return None;
    }
    Some(format!(
        "请补充临边防护栏杆布置信息：{}。上杆距地面 1.2m，下杆居中设置，立杆间距不大于 2m，挡脚板不小于 180mm，内侧满挂密目安全网（JGJ 80-2016 4.3.1）。",
        missing.join("、")
    ))
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

    #[test]
    fn safety_net_validation_passes_standard_values() {
        let v = validate_elevator_shaft_safety_net(2200.0, 1800.0, 3000.0, 20.0, true);
        assert!(v.ok);
        assert!(v.issues.is_empty());
        assert!(v.warnings.is_empty());
        assert_eq!(v.net_summary.net_spacing, 6000.0);
    }

    #[test]
    fn safety_net_spacing_over_10m_fails() {
        // 层高 5500 → 2 层 = 11m > 10m，违反 4.2.3
        let v = validate_elevator_shaft_safety_net(2200.0, 1800.0, 5500.0, 20.0, true);
        assert!(!v.ok);
        assert!(v.issues.iter().any(|i| i.contains("10m")));
    }

    #[test]
    fn safety_net_requires_upper_isolation() {
        let v = validate_elevator_shaft_safety_net(2200.0, 1800.0, 3000.0, 20.0, false);
        assert!(!v.ok);
        assert!(v.issues.iter().any(|i| i.contains("隔离防护")));
    }

    #[test]
    fn safety_net_wall_gap_25mm_is_recommended_only() {
        let v = validate_elevator_shaft_safety_net(2200.0, 1800.0, 3000.0, 40.0, true);
        assert!(v.ok, "空隙 40mm 超过图册推荐值，但不应判失败");
        assert!(v.warnings.iter().any(|w| w.contains("25mm")));
    }

    #[test]
    fn safety_net_negative_gap_fails() {
        let v = validate_elevator_shaft_safety_net(2200.0, 1800.0, 3000.0, -10.0, true);
        assert!(!v.ok);
        assert!(v.issues.iter().any(|i| i.contains("空隙不为负")));
    }

    #[test]
    fn edge_guardrail_validation_standard_values_pass() {
        let v = validate_edge_guardrail(3000.0, 1200.0, 1500.0, 180.0, true);
        assert!(v.ok);
        assert!(v.warnings.is_empty());
        assert_eq!(v.guardrail_summary.post_count, 3); // ceil(3000/1500)+1
    }

    #[test]
    fn edge_guardrail_post_count_covers_ends() {
        assert_eq!(edge_guardrail_post_count(5000.0, 2000.0), 4); // 0/2000/4000/5000
        assert_eq!(edge_guardrail_post_count(3000.0, 1000.0), 4); // 0/1000/2000/3000
        assert_eq!(edge_guardrail_post_count(0.0, 2000.0), 0);
    }

    #[test]
    fn edge_guardrail_mandatory_failures() {
        // 上杆 1.1m < 1.2m
        let v = validate_edge_guardrail(3000.0, 1100.0, 1500.0, 180.0, true);
        assert!(!v.ok);
        assert!(v.issues.iter().any(|i| i.contains("1.2m")));
        // 立杆间距 2.5m > 2m
        let v = validate_edge_guardrail(3000.0, 1200.0, 2500.0, 180.0, true);
        assert!(!v.ok);
        assert!(v.issues.iter().any(|i| i.contains("2m")));
        // 挡脚板 150mm < 180mm
        let v = validate_edge_guardrail(3000.0, 1200.0, 1500.0, 150.0, true);
        assert!(!v.ok);
        assert!(v.issues.iter().any(|i| i.contains("180mm")));
    }

    #[test]
    fn edge_guardrail_mesh_net_is_recommended_only() {
        let v = validate_edge_guardrail(3000.0, 1200.0, 1500.0, 180.0, false);
        assert!(v.ok, "缺密目网只是推荐项提醒，不应判失败");
        assert!(v.warnings.iter().any(|w| w.contains("密目")));
    }

    #[test]
    fn edge_guardrail_missing_params_detection() {
        assert_eq!(
            missing_edge_guardrail_params("画一个楼层临边防护栏杆"),
            vec!["临边长度", "作业侧", "转角/端部收口"]
        );
        assert_eq!(
            missing_edge_guardrail_params("画一个 3m 长楼层临边防护栏杆，临空侧作业，端部收口处理"),
            Vec::<&str>::new()
        );
        assert_eq!(
            missing_edge_guardrail_params("画一个 3m 长楼层临边防护栏杆"),
            vec!["作业侧", "转角/端部收口"]
        );
        assert!(edge_guardrail_clarification_prompt(
            "画一个 3m 长楼层临边防护栏杆，临空侧作业，端部收口处理"
        )
        .is_none());
    }

    #[test]
    fn safety_net_missing_params_detection() {
        assert_eq!(
            missing_safety_net_params("画电梯井内安全平网"),
            vec!["层高/楼层间距", "井道长×宽"]
        );
        assert_eq!(
            missing_safety_net_params("画电梯井内安全平网，层高 3000"),
            vec!["井道长×宽"]
        );
        assert!(
            missing_safety_net_params("画电梯井内安全平网，井道 2000×1800，层高 3000").is_empty()
        );
        assert!(
            safety_net_clarification_prompt("画电梯井内安全平网，井道 2000×1800，层高 3000")
                .is_none()
        );
    }
}
