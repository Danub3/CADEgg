#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SafetySceneCategory {
    FallProtection,
    AccessProtection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SafetySceneSpec {
    pub scene: &'static str,
    pub name: &'static str,
    pub category: SafetySceneCategory,
    pub keywords: &'static [&'static str],
    pub required_params: &'static [&'static str],
    /// 强制项：规范底线，违反应判 ok=false。只有知识卡核实过的数值才能放这里。
    pub mandatory_rules: &'static [&'static str],
    /// 推荐项：公司标准化图册/构造图集等推荐做法，只进 warnings。
    pub recommended_rules: &'static [&'static str],
    pub prohibited_rules: &'static [&'static str],
    pub cad_components: &'static [&'static str],
    pub sources: &'static [&'static str],
    pub draw_tool: Option<&'static str>,
    pub validate_tool: Option<&'static str>,
    /// 知识卡是否就绪（data/atlas 有对应卡且 sources 已核实）。
    pub knowledge_card_ready: bool,
    /// 确定性出图工具是否就绪。
    pub deterministic_draw_ready: bool,
    /// 确定性校核工具是否就绪。
    pub deterministic_validate_ready: bool,
    pub requires_approval: bool,
}

impl SafetySceneSpec {
    /// 完整闭环是否可用：出图 + 校核工具同时就绪。
    pub fn is_full_loop_ready(&self) -> bool {
        self.deterministic_draw_ready && self.deterministic_validate_ready
    }
}

const SCENES: &[SafetySceneSpec] = &[
    SafetySceneSpec {
        scene: "elevator_shaft_protection",
        name: "室内电梯井口防护",
        category: SafetySceneCategory::FallProtection,
        keywords: &[
            "电梯井",
            "电梯口",
            "电梯洞",
            "电梯洞口",
            "井口防护",
            "井口防护门",
            "elevator shaft",
        ],
        required_params: &["opening_width", "opening_height"],
        mandatory_rules: &[
            "guard_height >= 1500mm",
            "door_bottom_gap <= 50mm",
            "toe_board_required",
        ],
        recommended_rules: &["toe_board_height = 200mm", "warning_sign", "material_table"],
        prohibited_rules: &["do_not_use_edge_guardrail_as_elevator_shaft_door"],
        cad_components: &[
            "opening_outline",
            "guard_door",
            "toe_board",
            "warning_sign",
            "material_table",
        ],
        sources: &["jgj-80-2016 4.2.2", "mohurd-2019-90 2.7.4"],
        draw_tool: Some("draw_elevator_shaft_protection"),
        validate_tool: Some("validate_elevator_shaft_protection"),
        knowledge_card_ready: true,
        deterministic_draw_ready: true,
        deterministic_validate_ready: true,
        requires_approval: false,
    },
    SafetySceneSpec {
        scene: "edge_guardrail",
        name: "普通临边防护栏杆",
        category: SafetySceneCategory::FallProtection,
        keywords: &[
            "临边",
            "楼层边",
            "阳台边",
            "屋面边",
            "基坑边",
            "防护栏杆",
            "护栏",
        ],
        required_params: &["edge_length", "edge_type"],
        // 修正 2026-08-20 效力分级：4.3.1 的下杆位置、1.2m/2m/180mm 属规范底线（强制），
        // 密目网是标准化图册做法（推荐），此前整组被误放在 recommended_rules。
        mandatory_rules: &[
            "top_rail_height >= 1200mm",
            "mid_rail_between_top_rail_and_toe_board",
            "post_spacing <= 2000mm",
            "toe_board_height >= 180mm",
        ],
        recommended_rules: &["dense_mesh_net", "warning_sign"],
        prohibited_rules: &["do_not_route_to_elevator_shaft_protection"],
        cad_components: &[
            "top_rail",
            "mid_rail",
            "posts",
            "toe_board",
            "dense_mesh_net",
        ],
        sources: &["jgj-80-2016 4.3.1", "mohurd-2019-90 2.7.2"],
        draw_tool: None,
        validate_tool: None,
        knowledge_card_ready: true,
        deterministic_draw_ready: false,
        deterministic_validate_ready: false,
        requires_approval: false,
    },
    SafetySceneSpec {
        scene: "opening_cover",
        name: "楼板/屋面洞口防护",
        category: SafetySceneCategory::FallProtection,
        keywords: &[
            "洞口防护",
            "楼板洞口",
            "屋面洞口",
            "管井",
            "设备井",
            "采光井",
            "天窗",
            "盖板",
        ],
        required_params: &["opening_short_side", "opening_long_side"],
        mandatory_rules: &[],
        recommended_rules: &["classify_by_short_side", "cover_or_guardrail_by_size"],
        prohibited_rules: &["do_not_route_to_elevator_shaft_protection"],
        cad_components: &[
            "opening_outline",
            "cover_plate",
            "guardrail_or_net",
            "fixing_note",
        ],
        sources: &["jgj-80-2016 4.2.1", "mohurd-2019-90 2.7.1"],
        draw_tool: None,
        validate_tool: None,
        knowledge_card_ready: false,
        deterministic_draw_ready: false,
        deterministic_validate_ready: false,
        requires_approval: false,
    },
    SafetySceneSpec {
        scene: "stair_guard",
        name: "楼梯口/梯段边防护",
        category: SafetySceneCategory::FallProtection,
        keywords: &["楼梯口", "梯段边", "楼梯平台", "楼梯临边"],
        required_params: &["stair_width", "landing_size"],
        mandatory_rules: &[],
        recommended_rules: &["guardrail", "toe_board", "temporary_closure"],
        prohibited_rules: &["do_not_route_to_elevator_shaft_protection"],
        cad_components: &["stair_outline", "guardrail", "toe_board", "warning_note"],
        sources: &["jgj-80-2016 4.3.1"],
        draw_tool: None,
        validate_tool: None,
        knowledge_card_ready: false,
        deterministic_draw_ready: false,
        deterministic_validate_ready: false,
        requires_approval: false,
    },
    SafetySceneSpec {
        scene: "safety_passage_shed",
        name: "安全通道/防护棚",
        category: SafetySceneCategory::AccessProtection,
        keywords: &["安全通道", "防护棚", "通道棚", "安全防护棚"],
        required_params: &["passage_width", "passage_height", "shed_length"],
        mandatory_rules: &[],
        recommended_rules: &["clear_width", "double_layer_when_required", "warning_signs"],
        prohibited_rules: &["do_not_route_to_elevator_shaft_protection"],
        cad_components: &["posts", "roof_layers", "side_protection", "warning_signs"],
        sources: &["mohurd-2019-90"],
        draw_tool: None,
        validate_tool: None,
        knowledge_card_ready: false,
        deterministic_draw_ready: false,
        deterministic_validate_ready: false,
        requires_approval: false,
    },
];

pub fn all_safety_scenes() -> &'static [SafetySceneSpec] {
    SCENES
}

pub fn scene_by_id(scene_id: &str) -> Option<&'static SafetySceneSpec> {
    all_safety_scenes()
        .iter()
        .find(|scene| scene.scene == scene_id)
}

fn score_scene(text: &str, scene: &SafetySceneSpec) -> usize {
    scene
        .keywords
        .iter()
        .filter(|keyword| text.contains(&keyword.to_lowercase()))
        .count()
}

pub fn match_safety_scene(user_input: &str) -> Option<&'static SafetySceneSpec> {
    let text = user_input.to_lowercase();
    all_safety_scenes()
        .iter()
        .enumerate()
        .filter_map(|(idx, scene)| {
            let score = score_scene(&text, scene);
            if score > 0 {
                Some((score, idx, scene))
            } else {
                None
            }
        })
        .max_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)))
        .map(|(_, _, scene)| scene)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_elevator_shaft_scene() {
        let scene = match_safety_scene("画一个电梯井口防护，井口宽 2000 高 1800").unwrap();
        assert_eq!(scene.scene, "elevator_shaft_protection");
        assert!(scene.is_full_loop_ready());
        assert_eq!(scene.draw_tool, Some("draw_elevator_shaft_protection"));
    }

    #[test]
    fn separates_edge_guardrail_from_elevator_shaft() {
        let scene = match_safety_scene("画一个楼层临边防护栏杆，长度 3000").unwrap();
        assert_eq!(scene.scene, "edge_guardrail");
        assert_eq!(scene.draw_tool, None);
        assert!(!scene.prohibited_rules.is_empty());
    }

    #[test]
    fn exposes_registered_scene_metadata() {
        let scenes = all_safety_scenes();
        assert!(scenes.len() >= 5);
        assert!(scenes
            .iter()
            .any(|scene| scene.scene == "elevator_shaft_protection"
                && scene
                    .mandatory_rules
                    .iter()
                    .any(|rule| *rule == "door_bottom_gap <= 50mm")));
    }

    /// 注册表一致性：标记 knowledge_card_ready 的场景必须有可加载的知识卡，
    /// 未就绪的场景必须显式标 false（不允许"没标状态"的中间态）。
    #[test]
    fn ready_scenes_have_loadable_knowledge_cards() {
        for scene in all_safety_scenes() {
            let card = crate::knowledge::load_scene_card(scene.scene);
            assert_eq!(
                card.is_some(),
                scene.knowledge_card_ready,
                "场景 {} 的知识卡状态与 data/atlas 不一致",
                scene.scene
            );
        }
    }

    /// 就绪标记与工具注册必须一致：声称出图/校核就绪却没有对应工具是配置错误。
    #[test]
    fn readiness_flags_match_registered_tools() {
        for scene in all_safety_scenes() {
            assert_eq!(
                scene.deterministic_draw_ready,
                scene.draw_tool.is_some(),
                "场景 {} 的 draw 就绪标记与 draw_tool 不一致",
                scene.scene
            );
            assert_eq!(
                scene.deterministic_validate_ready,
                scene.validate_tool.is_some(),
                "场景 {} 的 validate 就绪标记与 validate_tool 不一致",
                scene.scene
            );
        }
    }

    /// 效力分级修正回归：临边栏杆的 1.2m/2m/180mm 必须在强制项，密目网留在推荐项。
    #[test]
    fn edge_guardrail_mandatory_rules_are_classified_correctly() {
        let scene = crate::scenes::scene_by_id("edge_guardrail").unwrap();
        for rule in [
            "top_rail_height >= 1200mm",
            "post_spacing <= 2000mm",
            "toe_board_height >= 180mm",
        ] {
            assert!(scene.mandatory_rules.contains(&rule), "{} 应为强制项", rule);
        }
        assert!(!scene.mandatory_rules.contains(&"dense_mesh_net"));
        assert!(scene.recommended_rules.contains(&"dense_mesh_net"));
    }

    /// 现状快照：当前只有电梯井口场景具备完整闭环，其余场景是登记态。
    #[test]
    fn only_elevator_shaft_has_full_loop_for_now() {
        let ready: Vec<&str> = all_safety_scenes()
            .iter()
            .filter(|scene| scene.is_full_loop_ready())
            .map(|scene| scene.scene)
            .collect();
        assert_eq!(ready, vec!["elevator_shaft_protection"]);
    }
}
