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
    pub mandatory_rules: &'static [&'static str],
    pub recommended_rules: &'static [&'static str],
    pub prohibited_rules: &'static [&'static str],
    pub cad_components: &'static [&'static str],
    pub sources: &'static [&'static str],
    pub draw_tool: Option<&'static str>,
    pub validate_tool: Option<&'static str>,
    pub auto_draw: bool,
    pub requires_approval: bool,
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
        auto_draw: true,
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
        mandatory_rules: &[],
        recommended_rules: &[
            "top_rail_height = 1.2m",
            "post_spacing <= 2.0m",
            "toe_board_height >= 180mm",
        ],
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
        auto_draw: false,
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
        auto_draw: false,
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
        auto_draw: false,
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
        auto_draw: false,
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
        assert!(scene.auto_draw);
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
}
