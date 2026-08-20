use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::settings::WorkMode;

/// Tool invocation requested by the LLM. `id` echoes Claude's tool_use_id when
/// available; for Gemini we synthesize one since functionCall has no id.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: Value,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolResult {
    pub id: String,
    pub name: String,
    pub ok: bool,
    pub content: String,
    #[serde(default)]
    pub confirmation_required: bool,
    #[serde(default)]
    pub object_updates: Vec<ObjectUpdate>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SessionObject {
    pub handle: String,
    pub kind: String,
    pub label: String,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ObjectUpdate {
    Upsert { object: SessionObject },
    Remove { handle: String },
    RemoveLast,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ToolLayer {
    Selection,
    Query,
    BasicDraw,
    SemanticGeometry,
    Annotation,
    Modify,
    Escape,
}

impl ToolLayer {
    fn label(self) -> &'static str {
        match self {
            ToolLayer::Selection => "选择/会话",
            ToolLayer::Query => "查询",
            ToolLayer::BasicDraw => "基础绘制",
            ToolLayer::SemanticGeometry => "语义几何",
            ToolLayer::Annotation => "标注文字",
            ToolLayer::Modify => "修改变换",
            ToolLayer::Escape => "逃生舱",
        }
    }
}

struct ToolSpec {
    name: &'static str,
    layer: ToolLayer,
    description: &'static str,
    parameters: fn() -> Value,
}

impl ToolSpec {
    fn schema(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "parameters": (self.parameters)(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct ToolingContext {
    pub tool_names: Vec<String>,
    pub guidance: String,
}

fn params_draw_line() -> Value {
    json!({
        "type": "object",
        "properties": {
            "x1": {"type": "number", "description": "起点 X"},
            "y1": {"type": "number", "description": "起点 Y"},
            "x2": {"type": "number", "description": "终点 X"},
            "y2": {"type": "number", "description": "终点 Y"}
        },
        "required": ["x1", "y1", "x2", "y2"]
    })
}

fn params_draw_circle() -> Value {
    json!({
        "type": "object",
        "properties": {
            "cx": {"type": "number", "description": "圆心 X"},
            "cy": {"type": "number", "description": "圆心 Y"},
            "r":  {"type": "number", "description": "半径，须大于 0"}
        },
        "required": ["cx", "cy", "r"]
    })
}

fn params_draw_regular_polygon() -> Value {
    json!({
        "type": "object",
        "properties": {
            "cx": {"type": "number", "description": "中心 X"},
            "cy": {"type": "number", "description": "中心 Y"},
            "sides": {"type": "integer", "description": "边数，至少为 3"},
            "radius": {"type": "number", "description": "半径，须大于 0"},
            "radius_mode": {
                "type": "string",
                "enum": ["circumradius", "inradius"],
                "description": "radius 的含义"
            },
            "rotation_deg": {
                "type": "number",
                "description": "首个顶点角度，单位度；可选，默认 0"
            }
        },
        "required": ["cx", "cy", "sides", "radius", "radius_mode"]
    })
}

fn params_draw_equilateral_triangle_about_circle() -> Value {
    json!({
        "type": "object",
        "properties": {
            "cx": {"type": "number", "description": "圆心/三角形中心 X"},
            "cy": {"type": "number", "description": "圆心/三角形中心 Y"},
            "r": {"type": "number", "description": "给定圆半径，须大于 0"},
            "relation": {
                "type": "string",
                "enum": ["incircle", "circumcircle"],
                "description": "incircle 表示该圆内切于三角形；circumcircle 表示该圆外接于三角形"
            },
            "apex_up": {
                "type": "boolean",
                "description": "是否让尖角朝上；可选，默认 true"
            }
        },
        "required": ["cx", "cy", "r", "relation"]
    })
}

fn params_draw_rectangle_by_center() -> Value {
    json!({
        "type": "object",
        "properties": {
            "cx": {"type": "number", "description": "中心 X"},
            "cy": {"type": "number", "description": "中心 Y"},
            "width": {"type": "number", "description": "宽度，须大于 0"},
            "height": {"type": "number", "description": "高度，须大于 0"},
            "rotation_deg": {"type": "number", "description": "旋转角度，单位度；可选，默认 0"}
        },
        "required": ["cx", "cy", "width", "height"]
    })
}

fn params_draw_double_flight_stair() -> Value {
    json!({
        "type": "object",
        "properties": {
            "x": {"type": "number", "description": "楼梯左下角或第一跑起点 X"},
            "y": {"type": "number", "description": "楼梯左下角或第一跑起点 Y"},
            "flight_width": {"type": "number", "description": "单跑楼梯净宽，默认可用 1200"},
            "step_depth": {"type": "number", "description": "踏步进深，默认可用 280"},
            "steps_per_flight": {"type": "integer", "description": "每跑踏步数，默认可用 10"},
            "landing_depth": {"type": "number", "description": "休息平台深度，默认通常等于 flight_width"},
            "turn": {
                "type": "string",
                "enum": ["left", "right"],
                "description": "第二跑相对第一跑的位置。right 表示向右转，left 表示向左转"
            },
            "include_arrow": {"type": "boolean", "description": "是否绘制上下方向箭头，默认 true"},
            "include_label": {"type": "boolean", "description": "是否绘制 UP 标注，默认 true"}
        },
        "required": ["x", "y", "flight_width", "step_depth", "steps_per_flight", "landing_depth", "turn"]
    })
}

fn params_draw_text() -> Value {
    json!({
        "type": "object",
        "properties": {
            "x": {"type": "number", "description": "插入点 X"},
            "y": {"type": "number", "description": "插入点 Y"},
            "text": {"type": "string", "description": "文字内容"},
            "height": {"type": "number", "description": "字高，须大于 0"},
            "rotation_deg": {"type": "number", "description": "旋转角度，单位度；可选，默认 0"}
        },
        "required": ["x", "y", "text", "height"]
    })
}

fn params_move() -> Value {
    json!({
        "type": "object",
        "properties": {
            "dx": {"type": "number"},
            "dy": {"type": "number"},
            "target": {"type": "string", "enum": ["last", "previous"], "description": "选择哪批对象"}
        },
        "required": ["dx", "dy", "target"]
    })
}

fn params_move_handle() -> Value {
    json!({
        "type": "object",
        "properties": {
            "handle": {"type": "string", "description": "对象 Handle，如 2A3B"},
            "dx": {"type": "number"},
            "dy": {"type": "number"}
        },
        "required": ["handle", "dx", "dy"]
    })
}

fn params_rotate_handle() -> Value {
    json!({
        "type": "object",
        "properties": {
            "handle": {"type": "string", "description": "对象 Handle，如 2A3B"},
            "cx": {"type": "number", "description": "旋转基点 X"},
            "cy": {"type": "number", "description": "旋转基点 Y"},
            "angle_deg": {"type": "number", "description": "旋转角度，单位度"}
        },
        "required": ["handle", "cx", "cy", "angle_deg"]
    })
}

fn params_copy_handle() -> Value {
    json!({
        "type": "object",
        "properties": {
            "handle": {"type": "string", "description": "对象 Handle，如 2A3B"},
            "dx": {"type": "number", "description": "复制位移 X"},
            "dy": {"type": "number", "description": "复制位移 Y"}
        },
        "required": ["handle", "dx", "dy"]
    })
}

fn params_mirror_handle() -> Value {
    json!({
        "type": "object",
        "properties": {
            "handle": {"type": "string", "description": "对象 Handle，如 2A3B"},
            "x1": {"type": "number", "description": "镜像轴第一个点 X"},
            "y1": {"type": "number", "description": "镜像轴第一个点 Y"},
            "x2": {"type": "number", "description": "镜像轴第二个点 X"},
            "y2": {"type": "number", "description": "镜像轴第二个点 Y"}
        },
        "required": ["handle", "x1", "y1", "x2", "y2"]
    })
}

fn params_offset_handle() -> Value {
    json!({
        "type": "object",
        "properties": {
            "handle": {"type": "string", "description": "对象 Handle，如 2A3B"},
            "distance": {"type": "number", "description": "偏移距离，须大于 0"},
            "side_x": {"type": "number", "description": "用于确定偏移方向的一侧点 X"},
            "side_y": {"type": "number", "description": "用于确定偏移方向的一侧点 Y"}
        },
        "required": ["handle", "distance", "side_x", "side_y"]
    })
}

fn params_trim_by_handle() -> Value {
    json!({
        "type": "object",
        "properties": {
            "boundary_handle": {"type": "string", "description": "作为修剪边界的对象 Handle"},
            "target_handle": {"type": "string", "description": "需要被修剪的对象 Handle"},
            "pick_x": {"type": "number", "description": "靠近要修掉那一端的拾取点 X"},
            "pick_y": {"type": "number", "description": "靠近要修掉那一端的拾取点 Y"}
        },
        "required": ["boundary_handle", "target_handle", "pick_x", "pick_y"]
    })
}

fn params_extend_by_handle() -> Value {
    json!({
        "type": "object",
        "properties": {
            "boundary_handle": {"type": "string", "description": "作为延伸边界的对象 Handle"},
            "target_handle": {"type": "string", "description": "需要被延伸的对象 Handle"},
            "pick_x": {"type": "number", "description": "靠近要延伸那一端的拾取点 X"},
            "pick_y": {"type": "number", "description": "靠近要延伸那一端的拾取点 Y"}
        },
        "required": ["boundary_handle", "target_handle", "pick_x", "pick_y"]
    })
}

fn params_erase_handle() -> Value {
    json!({
        "type": "object",
        "properties": {
            "handle": {"type": "string", "description": "对象 Handle，如 2A3B"}
        },
        "required": ["handle"]
    })
}

fn params_empty() -> Value {
    json!({ "type": "object", "properties": {} })
}

fn params_inspect_handle() -> Value {
    json!({
        "type": "object",
        "properties": {
            "handle": {"type": "string", "description": "对象 Handle，如 2A3B"}
        },
        "required": ["handle"]
    })
}

fn params_run_lisp() -> Value {
    json!({
        "type": "object",
        "properties": {
            "code": {"type": "string", "description": "AutoLISP 表达式，自动补外层括号"}
        },
        "required": ["code"]
    })
}

fn params_draw_elevator_shaft_protection() -> Value {
    json!({
        "type": "object",
        "properties": {
            "x": {"type": "number", "description": "电梯井口中心 X"},
            "y": {"type": "number", "description": "电梯井口中心 Y"},
            "opening_width": {"type": "number", "description": "井口宽度，毫米，须大于 0（现场实测）"},
            "opening_height": {"type": "number", "description": "井口高度/进深，毫米，须大于 0（现场实测）"},
            "guard_height": {"type": "number", "description": "防护门高度，毫米，不小于 1500（1.5m），缺省按最低合规值 1500"},
            "door_bottom_gap": {"type": "number", "description": "防护门底端距地面高度，毫米，不大于 50，缺省按最大合规值 50"},
            "toe_board_height": {"type": "number", "description": "踢脚板高度，毫米，指导图册推荐 200，缺省按 200"},
            "include_warning_sign": {"type": "boolean", "description": "是否绘制警示牌「当心坠落 严禁抛物」，默认 true"},
            "include_material_table": {"type": "boolean", "description": "是否绘制材料表，默认 true"},
            "scale": {"type": "number", "description": "图面缩放比例，默认 1.0"}
        },
        "required": [
            "x",
            "y",
            "opening_width",
            "opening_height"
        ]
    })
}

fn params_validate_elevator_shaft_protection() -> Value {
    json!({
        "type": "object",
        "properties": {
            "opening_width": {"type": "number", "description": "井口宽度，毫米，须大于 0"},
            "opening_height": {"type": "number", "description": "井口高度/进深，毫米，须大于 0"},
            "guard_height": {"type": "number", "description": "防护门高度，毫米，须不小于 1500"},
            "door_bottom_gap": {"type": "number", "description": "防护门底端距地面高度，毫米，须不大于 50"},
            "toe_board_height": {"type": "number", "description": "踢脚板高度，毫米，推荐 200"},
            "include_warning_sign": {"type": "boolean", "description": "是否包含警示牌"},
            "include_material_table": {"type": "boolean", "description": "是否包含材料表"}
        },
        "required": [
            "opening_width",
            "opening_height",
            "guard_height",
            "door_bottom_gap",
            "toe_board_height",
            "include_warning_sign",
            "include_material_table"
        ]
    })
}

fn all_tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "draw_line",
            layer: ToolLayer::BasicDraw,
            description: "在 AutoCAD 模型空间画一条直线。坐标单位与当前图纸一致（通常为毫米）。",
            parameters: params_draw_line,
        },
        ToolSpec {
            name: "draw_circle",
            layer: ToolLayer::BasicDraw,
            description: "画一个圆。",
            parameters: params_draw_circle,
        },
        ToolSpec {
            name: "draw_regular_polygon",
            layer: ToolLayer::BasicDraw,
            description: "以给定圆心绘制正多边形。radius_mode='circumradius' 时 radius 是外接圆半径；radius_mode='inradius' 时 radius 是内切圆半径。rotation_deg 是首个顶点相对 +X 轴的角度，90 度可让正三角形尖朝上。",
            parameters: params_draw_regular_polygon,
        },
        ToolSpec {
            name: "draw_equilateral_triangle_about_circle",
            layer: ToolLayer::SemanticGeometry,
            description: "高层几何工具。给定一个圆，直接绘制与该圆相关的等边三角形：relation='incircle' 表示该圆内切于三角形；relation='circumcircle' 表示该圆外接于三角形。apex_up=true 时尖角朝上。",
            parameters: params_draw_equilateral_triangle_about_circle,
        },
        ToolSpec {
            name: "draw_rectangle_by_center",
            layer: ToolLayer::SemanticGeometry,
            description: "按中心点、宽高和可选旋转角绘制矩形。",
            parameters: params_draw_rectangle_by_center,
        },
        ToolSpec {
            name: "draw_double_flight_stair",
            layer: ToolLayer::SemanticGeometry,
            description: "绘制建筑平面中的 U 型双跑楼梯：两段平行梯跑、休息平台、踏步线、可选上行箭头和 UP 标注。适合“双跑楼梯、折返楼梯、带平台楼梯”等请求；未给尺寸时可用 flight_width=1200, step_depth=280, steps_per_flight=10, landing_depth=1200。",
            parameters: params_draw_double_flight_stair,
        },
        ToolSpec {
            name: "draw_elevator_shaft_protection",
            layer: ToolLayer::SemanticGeometry,
            description: "绘制室内电梯井口防护门标准布置：井口轮廓、上翻式防护门扇、翻转轴、踢脚板、警示牌、尺寸标注和可选材料表。适合安全防护 demo 的主绘图工具。",
            parameters: params_draw_elevator_shaft_protection,
        },
        ToolSpec {
            name: "validate_elevator_shaft_protection",
            layer: ToolLayer::Query,
            description: "按确定性规则校核室内电梯井口防护门参数，返回 JSON：ok、issues、checks、material_table。",
            parameters: params_validate_elevator_shaft_protection,
        },
        ToolSpec {
            name: "draw_text",
            layer: ToolLayer::Annotation,
            description: "在指定点绘制单行文字。",
            parameters: params_draw_text,
        },
        ToolSpec {
            name: "move",
            layer: ToolLayer::Modify,
            description: "把对象按 (dx, dy) 平移。target='last' 平移上一次绘制的对象，target='previous' 平移上次选择集。",
            parameters: params_move,
        },
        ToolSpec {
            name: "move_handle",
            layer: ToolLayer::Modify,
            description: "按对象 handle 精确平移，优先用于继续操作之前由 agent 创建过的对象。",
            parameters: params_move_handle,
        },
        ToolSpec {
            name: "rotate_handle",
            layer: ToolLayer::Modify,
            description: "按对象 handle 围绕指定基点旋转。",
            parameters: params_rotate_handle,
        },
        ToolSpec {
            name: "copy_handle",
            layer: ToolLayer::Modify,
            description: "按对象 handle 复制一个对象，并按 (dx, dy) 位移副本。",
            parameters: params_copy_handle,
        },
        ToolSpec {
            name: "mirror_handle",
            layer: ToolLayer::Modify,
            description: "按对象 handle 生成镜像副本。镜像轴由两点 (x1,y1) 和 (x2,y2) 定义，原对象保留。",
            parameters: params_mirror_handle,
        },
        ToolSpec {
            name: "offset_handle",
            layer: ToolLayer::Modify,
            description: "按对象 handle 生成偏移副本。distance 是偏移距离，(side_x,side_y) 用来指明偏移到哪一侧。",
            parameters: params_offset_handle,
        },
        ToolSpec {
            name: "trim_by_handle",
            layer: ToolLayer::Modify,
            description: "按 handle 精确修剪对象。boundary_handle 是边界对象，target_handle 是被修剪对象，(pick_x,pick_y) 用来指明修掉哪一端。",
            parameters: params_trim_by_handle,
        },
        ToolSpec {
            name: "extend_by_handle",
            layer: ToolLayer::Modify,
            description: "按 handle 精确延伸对象。boundary_handle 是边界对象，target_handle 是被延伸对象，(pick_x,pick_y) 用来指明延伸哪一端。",
            parameters: params_extend_by_handle,
        },
        ToolSpec {
            name: "erase_last",
            layer: ToolLayer::Modify,
            description: "删除最后绘制的一个对象。无参数。",
            parameters: params_empty,
        },
        ToolSpec {
            name: "erase_handle",
            layer: ToolLayer::Modify,
            description: "按对象 handle 精确删除一个对象，优先用于继续操作之前由 agent 创建过的对象。",
            parameters: params_erase_handle,
        },
        ToolSpec {
            name: "zoom_extents",
            layer: ToolLayer::Modify,
            description: "把视图缩放到所有对象的范围。无参数。",
            parameters: params_empty,
        },
        ToolSpec {
            name: "inspect_handle",
            layer: ToolLayer::Query,
            description: "按对象 handle 查询对象类型和主要几何信息。",
            parameters: params_inspect_handle,
        },
        ToolSpec {
            name: "list_selection",
            layer: ToolLayer::Selection,
            description: "查询用户当前在 AutoCAD 中预先圈选的对象（PickfirstSelectionSet），返回数量与类型分布。无参数。",
            parameters: params_empty,
        },
        ToolSpec {
            name: "modelspace_snapshot",
            layer: ToolLayer::Query,
            description: "图面快照：枚举模型空间所有对象，返回对象总数、类型分布、整体包围盒和每个对象的 handle/类型/图层/颜色/几何信息。用于自动化审查与验收，无需人工目视即可了解 CAD 图面内容。无参数。",
            parameters: params_empty,
        },
        ToolSpec {
            name: "import_selection",
            layer: ToolLayer::Selection,
            description: "把用户当前在 AutoCAD 中预先圈选的对象导入会话对象表，便于后续用 handle 或“刚才选中的那条线”继续引用。无参数。",
            parameters: params_empty,
        },
        ToolSpec {
            name: "run_lisp",
            layer: ToolLayer::Escape,
            description: "逃生舱：当结构化工具不足以表达需求时，直接执行一段 AutoLISP 代码。注意：执行结果不会回传，只确认已下发。请仅在必要时使用。",
            parameters: params_run_lisp,
        },
    ]
}

fn ordered_unique(mut names: Vec<&'static str>) -> Vec<String> {
    let mut out = Vec::new();
    for name in names.drain(..) {
        if !out.iter().any(|existing: &String| existing == name) {
            out.push(name.to_string());
        }
    }
    out
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

/// 判断是否为已登记的施工安全场景请求。
#[cfg_attr(not(test), allow(dead_code))]
pub fn is_safety_request(user_input: &str) -> bool {
    crate::scenes::match_safety_scene(user_input).is_some()
        || crate::safety::is_elevator_shaft_request(user_input)
}

pub fn safety_context_scene(user_input: &str) -> Option<&'static str> {
    if crate::safety::is_elevator_shaft_request(user_input) {
        return Some("elevator_shaft_protection");
    }
    crate::scenes::match_safety_scene(user_input).map(|scene| scene.scene)
}

/// 电梯井口防护门的必填关键参数。返回缺参列表，用于追问闭环。
/// 仅对「要画防护图」但缺关键尺寸的请求返回缺项；若用户已在同一句里给出尺寸则视为已提供。
pub fn safety_missing_params(user_input: &str) -> Vec<&'static str> {
    if crate::safety::is_elevator_shaft_request(user_input) {
        crate::safety::missing_elevator_shaft_params(user_input)
    } else {
        Vec::new()
    }
}

/// 生成缺参追问提示文案，供系统提示注入或前端直接展示。
pub fn safety_clarification_prompt(user_input: &str) -> Option<String> {
    if safety_missing_params(user_input).is_empty() {
        return None;
    }
    crate::safety::elevator_shaft_clarification_prompt(user_input)
}

fn scene_category_label(category: crate::scenes::SafetySceneCategory) -> &'static str {
    match category {
        crate::scenes::SafetySceneCategory::FallProtection => "高处坠落防护",
        crate::scenes::SafetySceneCategory::AccessProtection => "通行与防护棚",
    }
}

fn safety_scene_tooling_context(
    scene: &crate::scenes::SafetySceneSpec,
    user_input: &str,
    has_session: bool,
) -> ToolingContext {
    let mut selected = vec![
        "draw_text",
        "zoom_extents",
        "inspect_handle",
        "modelspace_snapshot",
    ];
    if let Some(draw_tool) = scene.draw_tool {
        selected.insert(0, draw_tool);
    }
    if let Some(validate_tool) = scene.validate_tool {
        selected.insert(if scene.draw_tool.is_some() { 1 } else { 0 }, validate_tool);
    }
    if contains_any(
        &user_input.to_lowercase(),
        &["选中", "选择集", "圈选", "预选"],
    ) {
        selected.push("list_selection");
        selected.push("import_selection");
    }

    let required_params = if scene.required_params.is_empty() {
        "无固定必填参数".to_string()
    } else {
        scene.required_params.join(", ")
    };
    let mut lines = vec![format!(
        "安全场景注册表命中：{} ({})；分类={}；必填参数={}。",
        scene.name,
        scene.scene,
        scene_category_label(scene.category),
        required_params
    )];

    if scene.auto_draw {
        let draw_tool = scene.draw_tool.unwrap_or("未配置");
        let validate_tool = scene.validate_tool.unwrap_or("未配置");
        lines.push(format!(
            "本场景已开放确定性出图/校核工具：draw={}，validate={}；优先完成 draw -> validate -> modelspace_snapshot 闭环。",
            draw_tool, validate_tool
        ));
        lines.push(
            "缺少现场实测的必填参数时先追问；不要编造尺寸，不要调用 run_lisp 绕过场景工具。"
                .to_string(),
        );
        if let Some(prompt) = safety_clarification_prompt(user_input) {
            lines.push(prompt);
        }
    } else {
        lines.push(
            "该场景已注册但尚未开放确定性 CAD 出图/校核工具；不得调用电梯井口工具替代。"
                .to_string(),
        );
        lines.push(
            "当前只能基于知识卡和注册规则输出做法边界、参数清单、追问项或人工审核提示。"
                .to_string(),
        );
    }

    if !scene.mandatory_rules.is_empty() {
        lines.push(format!("强制规则：{}。", scene.mandatory_rules.join(", ")));
    }
    if !scene.recommended_rules.is_empty() {
        lines.push(format!(
            "推荐做法：{}。",
            scene.recommended_rules.join(", ")
        ));
    }
    if !scene.prohibited_rules.is_empty() {
        lines.push(format!("禁止误用：{}。", scene.prohibited_rules.join(", ")));
    }
    if has_session {
        lines.push(
            "若继续操作已有对象，可先 inspect_handle 查询，但不要使用任意 LISP 或低层编辑工具。"
                .to_string(),
        );
    }

    ToolingContext {
        tool_names: ordered_unique(selected),
        guidance: lines.join("\n"),
    }
}

fn generic_safety_tooling_context(user_input: &str, has_session: bool) -> ToolingContext {
    let mut selected = vec![
        "draw_text",
        "zoom_extents",
        "inspect_handle",
        "modelspace_snapshot",
    ];
    if contains_any(
        &user_input.to_lowercase(),
        &["选中", "选择集", "圈选", "预选"],
    ) {
        selected.push("list_selection");
        selected.push("import_selection");
    }

    let mut lines = vec![
        "安全模式：当前请求未命中已注册的确定性施工安全场景。".to_string(),
        "只提供文字说明、图面查询和结构化审查入口；不得自行套用电梯井口防护门工具，也不要调用 run_lisp 绕过场景边界。"
            .to_string(),
        "请先明确作业部位、风险类型、施工阶段、现场尺寸和拟采用的防护构件；涉及专项方案或专业审查时标记为人工审核。"
            .to_string(),
    ];
    if has_session {
        lines.push(
            "若继续操作已有对象，可先 inspect_handle 查询，但不要使用任意 LISP 或低层编辑工具。"
                .to_string(),
        );
    }

    ToolingContext {
        tool_names: ordered_unique(selected),
        guidance: lines.join("\n"),
    }
}

pub fn select_tooling_context(
    user_input: &str,
    session_objects: &[SessionObject],
    work_mode: WorkMode,
) -> ToolingContext {
    let text = user_input.to_lowercase();
    let has_session = !session_objects.is_empty();

    let matched_scene = crate::scenes::match_safety_scene(user_input).or_else(|| {
        if crate::safety::is_elevator_shaft_request(user_input) {
            crate::scenes::scene_by_id("elevator_shaft_protection")
        } else {
            None
        }
    });
    if let Some(scene) = matched_scene {
        return safety_scene_tooling_context(scene, user_input, has_session);
    }
    if work_mode == WorkMode::SafetyDemoMode {
        return generic_safety_tooling_context(user_input, has_session);
    }

    let is_stair_request =
        contains_any(&text, &["楼梯", "双跑", "折返", "休息平台", "踏步", "梯段"]);
    let mentions_object_ref = contains_any(
        &text,
        &[
            "handle", "对象", "刚才", "上一", "最新", "第", "选中", "那个", "这条", "这个",
        ],
    );

    let mut selected: Vec<&'static str> = if is_stair_request {
        vec!["draw_double_flight_stair"]
    } else {
        vec![
            "draw_line",
            "draw_circle",
            "draw_regular_polygon",
            "draw_equilateral_triangle_about_circle",
        ]
    };

    if contains_any(&text, &["矩形", "长方形", "方框", "框"]) {
        selected.push("draw_rectangle_by_center");
    }
    if is_stair_request {
        selected.push("draw_double_flight_stair");
    }
    if contains_any(&text, &["文字", "文本", "注释", "标注", "编号", "说明"]) {
        selected.push("draw_text");
    }
    if contains_any(&text, &["移动", "平移", "挪"]) {
        selected.push("move");
        selected.push("move_handle");
    }
    if contains_any(&text, &["旋转", "转动"]) {
        selected.push("rotate_handle");
    }
    if contains_any(&text, &["复制", "拷贝", "阵列"]) {
        selected.push("copy_handle");
    }
    if contains_any(&text, &["镜像", "对称"]) {
        selected.push("mirror_handle");
    }
    if contains_any(&text, &["偏移", "offset", "等距"]) {
        selected.push("offset_handle");
    }
    if contains_any(&text, &["修剪", "裁剪", "trim"]) {
        selected.push("trim_by_handle");
    }
    if contains_any(&text, &["延伸", "延长", "extend"]) {
        selected.push("extend_by_handle");
    }
    if contains_any(&text, &["删除", "擦除", "去掉"]) {
        selected.push("erase_last");
        selected.push("erase_handle");
    }
    if contains_any(
        &text,
        &[
            "查看", "查询", "读取", "识别", "信息", "长度", "半径", "角度", "属性",
        ],
    ) {
        selected.push("inspect_handle");
        selected.push("modelspace_snapshot");
    }
    if contains_any(&text, &["选中", "选择集", "圈选", "预选"]) {
        selected.push("list_selection");
        selected.push("import_selection");
    }
    if has_session || mentions_object_ref {
        selected.push("inspect_handle");
        selected.push("move_handle");
        selected.push("rotate_handle");
        selected.push("copy_handle");
        selected.push("mirror_handle");
        selected.push("offset_handle");
        selected.push("trim_by_handle");
        selected.push("extend_by_handle");
        selected.push("erase_handle");
    }
    if contains_any(
        &text,
        &["相切", "倒角", "圆角", "样条", "螺旋", "图片", "识图"],
    ) && work_mode != WorkMode::CompetitionMode
    {
        selected.push("run_lisp");
    }

    let tool_names = ordered_unique(selected);
    let specs = all_tool_specs();
    let selected_specs: Vec<&ToolSpec> = tool_names
        .iter()
        .filter_map(|name| specs.iter().find(|spec| spec.name == name))
        .collect();

    let mut groups: std::collections::BTreeMap<ToolLayer, Vec<&str>> =
        std::collections::BTreeMap::new();
    for spec in selected_specs {
        groups.entry(spec.layer).or_default().push(spec.name);
    }

    let mut group_lines = Vec::new();
    for (layer, names) in groups {
        group_lines.push(format!("{}={}", layer.label(), names.join(",")));
    }

    let mut lines = vec![format!(
        "本轮工具层：{}。优先语义几何，其次基础绘制，最后 run_lisp；可确定的多步任务尽量一次性给出完整 tool_calls。",
        group_lines.join("；")
    )];
    if work_mode == WorkMode::CompetitionMode {
        lines.push(
            "比赛模式：不要调用 run_lisp；需要复杂图形时优先选择结构化或语义工具。".to_string(),
        );
    }
    if has_session || mentions_object_ref {
        lines.push("若继续操作已有对象，优先按 handle 工具执行，不要退回到 last。".to_string());
    }

    ToolingContext {
        tool_names,
        guidance: lines.join("\n"),
    }
}

/// Canonical tool catalog. Single source of truth — adapters derive their schemas from here.
pub fn catalog() -> Vec<Value> {
    all_tool_specs()
        .into_iter()
        .map(|spec| spec.schema())
        .collect()
}

fn catalog_for(tool_names: &[String]) -> Vec<Value> {
    let specs = all_tool_specs();
    tool_names
        .iter()
        .filter_map(|name| specs.iter().find(|spec| spec.name == name))
        .map(|spec| spec.schema())
        .collect()
}

/// Gemini's `tools` field expects `[{ functionDeclarations: [ ... ] }]`.
pub fn gemini_function_declarations_for(tool_names: &[String]) -> Value {
    json!([{ "functionDeclarations": catalog_for(tool_names) }])
}

#[allow(dead_code)]
pub fn gemini_function_declarations() -> Value {
    json!([{ "functionDeclarations": catalog() }])
}

/// Claude's `tools` field expects an array with `input_schema` instead of `parameters`.
/// Reserved for future use; current ClaudeProvider returns a stub error.
#[allow(dead_code)]
pub fn claude_tools_for(tool_names: &[String]) -> Value {
    let mapped: Vec<Value> = catalog_for(tool_names)
        .into_iter()
        .map(|mut t| {
            if let Some(obj) = t.as_object_mut() {
                if let Some(p) = obj.remove("parameters") {
                    obj.insert("input_schema".to_string(), p);
                }
            }
            t
        })
        .collect();
    Value::Array(mapped)
}

#[allow(dead_code)]
pub fn claude_tools() -> Value {
    let mapped: Vec<Value> = catalog()
        .into_iter()
        .map(|mut t| {
            if let Some(obj) = t.as_object_mut() {
                if let Some(p) = obj.remove("parameters") {
                    obj.insert("input_schema".to_string(), p);
                }
            }
            t
        })
        .collect();
    Value::Array(mapped)
}

/// OpenAI-compatible tools format (used by Zhipu GLM).
/// `[{ type: "function", function: { name, description, parameters } }]`
pub fn openai_tools_for(tool_names: &[String]) -> Value {
    let mapped: Vec<Value> = catalog_for(tool_names)
        .into_iter()
        .map(|t| json!({ "type": "function", "function": t }))
        .collect();
    Value::Array(mapped)
}

#[allow(dead_code)]
pub fn openai_tools() -> Value {
    let mapped: Vec<Value> = catalog()
        .into_iter()
        .map(|t| json!({ "type": "function", "function": t }))
        .collect();
    Value::Array(mapped)
}

/// Helper: pull a number from a JSON object, accepting either f64 or an integer.
fn num(args: &Value, key: &str) -> Result<f64, String> {
    args.get(key)
        .and_then(|v| v.as_f64())
        .ok_or_else(|| format!("参数 '{key}' 缺失或不是数字"))
}

fn s<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("参数 '{key}' 缺失或不是字符串"))
}

fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{:.1}", n)
    } else {
        format!("{n}")
    }
}

fn extract_handle(content: &str) -> Option<String> {
    content
        .split("handle=")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .map(|s| {
            s.trim_matches(|c: char| c == ',' || c == '，' || c == '。')
                .to_string()
        })
        .filter(|s| !s.is_empty())
}

fn extract_created_session_object(content: &str) -> Option<SessionObject> {
    let created = content.split("新对象 handle=").nth(1)?;
    let handle = created
        .split_whitespace()
        .next()?
        .trim_matches(|c: char| c == ',' || c == '，' || c == '。')
        .to_string();
    let kind = created
        .split(" type=")
        .nth(1)?
        .split_whitespace()
        .next()?
        .trim_matches(|c: char| c == ',' || c == '，' || c == '。')
        .to_string();
    let label = created
        .split(" label=")
        .nth(1)?
        .trim()
        .trim_matches(|c: char| c == '。')
        .to_string();

    if handle.is_empty() || kind.is_empty() || label.is_empty() {
        return None;
    }

    Some(SessionObject {
        handle,
        kind,
        label,
        source: Some("generated".to_string()),
    })
}

fn int(args: &Value, key: &str) -> Result<i32, String> {
    args.get(key)
        .and_then(|v| v.as_i64())
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| format!("参数 '{key}' 缺失或不是整数"))
}

fn object_updates_for_success(call: &ToolCall, content: &str) -> Vec<ObjectUpdate> {
    match call.name.as_str() {
        "draw_line" => extract_handle(content)
            .map(|handle| ObjectUpdate::Upsert {
                object: SessionObject {
                    handle,
                    kind: "LINE".to_string(),
                    label: format!(
                        "直线 ({},{}) → ({},{})",
                        fmt_num(call.args["x1"].as_f64().unwrap_or_default()),
                        fmt_num(call.args["y1"].as_f64().unwrap_or_default()),
                        fmt_num(call.args["x2"].as_f64().unwrap_or_default()),
                        fmt_num(call.args["y2"].as_f64().unwrap_or_default())
                    ),
                    source: Some("generated".to_string()),
                },
            })
            .into_iter()
            .collect(),
        "draw_circle" => extract_handle(content)
            .map(|handle| ObjectUpdate::Upsert {
                object: SessionObject {
                    handle,
                    kind: "CIRCLE".to_string(),
                    label: format!(
                        "圆心 ({},{}) 半径 {}",
                        fmt_num(call.args["cx"].as_f64().unwrap_or_default()),
                        fmt_num(call.args["cy"].as_f64().unwrap_or_default()),
                        fmt_num(call.args["r"].as_f64().unwrap_or_default())
                    ),
                    source: Some("generated".to_string()),
                },
            })
            .into_iter()
            .collect(),
        "draw_regular_polygon" => extract_handle(content)
            .map(|handle| ObjectUpdate::Upsert {
                object: SessionObject {
                    handle,
                    kind: "LWPOLYLINE".to_string(),
                    label: format!(
                        "正{}边形 中心 ({},{}) 半径 {} ({})",
                        call.args["sides"].as_i64().unwrap_or_default(),
                        fmt_num(call.args["cx"].as_f64().unwrap_or_default()),
                        fmt_num(call.args["cy"].as_f64().unwrap_or_default()),
                        fmt_num(call.args["radius"].as_f64().unwrap_or_default()),
                        call.args["radius_mode"].as_str().unwrap_or("unknown")
                    ),
                    source: Some("generated".to_string()),
                },
            })
            .into_iter()
            .collect(),
        "draw_equilateral_triangle_about_circle" => extract_handle(content)
            .map(|handle| ObjectUpdate::Upsert {
                object: SessionObject {
                    handle,
                    kind: "LWPOLYLINE".to_string(),
                    label: format!(
                        "等边三角形 中心 ({},{}) 圆半径 {} ({})",
                        fmt_num(call.args["cx"].as_f64().unwrap_or_default()),
                        fmt_num(call.args["cy"].as_f64().unwrap_or_default()),
                        fmt_num(call.args["r"].as_f64().unwrap_or_default()),
                        call.args["relation"].as_str().unwrap_or("unknown")
                    ),
                    source: Some("generated".to_string()),
                },
            })
            .into_iter()
            .collect(),
        "draw_rectangle_by_center" => extract_handle(content)
            .map(|handle| ObjectUpdate::Upsert {
                object: SessionObject {
                    handle,
                    kind: "LWPOLYLINE".to_string(),
                    label: format!(
                        "矩形 中心 ({},{}) 宽 {} 高 {}",
                        fmt_num(call.args["cx"].as_f64().unwrap_or_default()),
                        fmt_num(call.args["cy"].as_f64().unwrap_or_default()),
                        fmt_num(call.args["width"].as_f64().unwrap_or_default()),
                        fmt_num(call.args["height"].as_f64().unwrap_or_default())
                    ),
                    source: Some("generated".to_string()),
                },
            })
            .into_iter()
            .collect(),
        "draw_double_flight_stair" => Vec::new(),
        "draw_elevator_shaft_protection" => extract_created_session_object(content)
            .map(|object| vec![ObjectUpdate::Upsert { object }])
            .unwrap_or_default(),
        "draw_text" => extract_handle(content)
            .map(|handle| ObjectUpdate::Upsert {
                object: SessionObject {
                    handle,
                    kind: "TEXT".to_string(),
                    label: format!(
                        "文字 \"{}\" @ ({},{})",
                        call.args["text"].as_str().unwrap_or(""),
                        fmt_num(call.args["x"].as_f64().unwrap_or_default()),
                        fmt_num(call.args["y"].as_f64().unwrap_or_default())
                    ),
                    source: Some("generated".to_string()),
                },
            })
            .into_iter()
            .collect(),
        "copy_handle" | "mirror_handle" | "offset_handle" => {
            extract_created_session_object(content)
                .map(|object| vec![ObjectUpdate::Upsert { object }])
                .unwrap_or_default()
        }
        "erase_handle" => s(&call.args, "handle")
            .map(|handle| {
                vec![ObjectUpdate::Remove {
                    handle: handle.trim().to_string(),
                }]
            })
            .unwrap_or_default(),
        "erase_last" => vec![ObjectUpdate::RemoveLast],
        _ => Vec::new(),
    }
}

fn summarize_imported_objects(objects: &[SessionObject]) -> String {
    let preview: Vec<String> = objects
        .iter()
        .take(4)
        .map(|object| format!("{} {}", object.handle, object.label))
        .collect();

    if preview.is_empty() {
        "未导入任何对象".to_string()
    } else if objects.len() > preview.len() {
        format!(
            "已导入 {} 个选中对象：{} 等",
            objects.len(),
            preview.join("；")
        )
    } else {
        format!(
            "已导入 {} 个选中对象：{}",
            objects.len(),
            preview.join("；")
        )
    }
}

/// Dispatch a parsed ToolCall to the corresponding cad:: function.
/// Returns ToolResult — never panics, errors become `ok: false`.
pub fn dispatch(call: &ToolCall) -> ToolResult {
    dispatch_with_policy(call, false)
}

pub fn dispatch_confirmed(call: &ToolCall) -> ToolResult {
    dispatch_with_policy(call, true)
}

pub fn dispatch_with_mode(call: &ToolCall, work_mode: WorkMode) -> ToolResult {
    if work_mode == WorkMode::CompetitionMode && call.name == "run_lisp" {
        return ToolResult {
            id: call.id.clone(),
            name: call.name.clone(),
            ok: false,
            content: "比赛模式已禁用 run_lisp。请改用结构化 CAD 工具或安全防护专用工具。"
                .to_string(),
            confirmation_required: false,
            object_updates: Vec::new(),
        };
    }
    dispatch(call)
}

pub fn dispatch_confirmed_with_mode(call: &ToolCall, work_mode: WorkMode) -> ToolResult {
    if work_mode == WorkMode::CompetitionMode && call.name == "run_lisp" {
        return ToolResult {
            id: call.id.clone(),
            name: call.name.clone(),
            ok: false,
            content: "比赛模式已禁用 run_lisp。请改用结构化 CAD 工具或安全防护专用工具。"
                .to_string(),
            confirmation_required: false,
            object_updates: Vec::new(),
        };
    }
    dispatch_confirmed(call)
}

pub fn requires_confirmation(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "erase_last" | "erase_handle" | "trim_by_handle" | "extend_by_handle" | "run_lisp"
    )
}

fn dispatch_with_policy(call: &ToolCall, confirmed: bool) -> ToolResult {
    if requires_confirmation(&call.name) && !confirmed {
        return ToolResult {
            id: call.id.clone(),
            name: call.name.clone(),
            ok: false,
            content: format!(
                "{} 需要人工确认后才会执行。请核对对象和参数，无误后点击确认。",
                call.name
            ),
            confirmation_required: true,
            object_updates: Vec::new(),
        };
    }

    #[cfg(windows)]
    if call.name == "import_selection" {
        return match crate::cad::cad_import_selection() {
            Ok(objects) => ToolResult {
                id: call.id.clone(),
                name: call.name.clone(),
                ok: true,
                content: summarize_imported_objects(&objects),
                confirmation_required: false,
                object_updates: objects
                    .into_iter()
                    .map(|object| ObjectUpdate::Upsert { object })
                    .collect(),
            },
            Err(e) => ToolResult {
                id: call.id.clone(),
                name: call.name.clone(),
                ok: false,
                content: e,
                confirmation_required: false,
                object_updates: Vec::new(),
            },
        };
    }

    let result: Result<String, String> = (|| match call.name.as_str() {
        #[cfg(windows)]
        "draw_line" => crate::cad::cad_draw_line(
            num(&call.args, "x1")?,
            num(&call.args, "y1")?,
            num(&call.args, "x2")?,
            num(&call.args, "y2")?,
        ),
        #[cfg(windows)]
        "draw_circle" => crate::cad::cad_draw_circle(
            num(&call.args, "cx")?,
            num(&call.args, "cy")?,
            num(&call.args, "r")?,
        ),
        #[cfg(windows)]
        "draw_regular_polygon" => crate::cad::cad_draw_regular_polygon(
            num(&call.args, "cx")?,
            num(&call.args, "cy")?,
            int(&call.args, "sides")?,
            num(&call.args, "radius")?,
            s(&call.args, "radius_mode")?,
            call.args
                .get("rotation_deg")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
        ),
        #[cfg(windows)]
        "draw_equilateral_triangle_about_circle" => {
            crate::cad::cad_draw_equilateral_triangle_about_circle(
                num(&call.args, "cx")?,
                num(&call.args, "cy")?,
                num(&call.args, "r")?,
                s(&call.args, "relation")?,
                call.args
                    .get("apex_up")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
            )
        }
        #[cfg(windows)]
        "draw_rectangle_by_center" => crate::cad::cad_draw_rectangle_by_center(
            num(&call.args, "cx")?,
            num(&call.args, "cy")?,
            num(&call.args, "width")?,
            num(&call.args, "height")?,
            call.args
                .get("rotation_deg")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
        ),
        #[cfg(windows)]
        "draw_double_flight_stair" => crate::cad::cad_draw_double_flight_stair(
            num(&call.args, "x")?,
            num(&call.args, "y")?,
            num(&call.args, "flight_width")?,
            num(&call.args, "step_depth")?,
            int(&call.args, "steps_per_flight")?,
            num(&call.args, "landing_depth")?,
            s(&call.args, "turn")?,
            call.args
                .get("include_arrow")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            call.args
                .get("include_label")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
        ),
        #[cfg(windows)]
        "draw_elevator_shaft_protection" => crate::cad::cad_draw_elevator_shaft_protection(
            num(&call.args, "x")?,
            num(&call.args, "y")?,
            num(&call.args, "opening_width")?,
            num(&call.args, "opening_height")?,
            call.args
                .get("guard_height")
                .and_then(|v| v.as_f64())
                .unwrap_or(crate::safety::GUARD_DOOR_HEIGHT_MM),
            call.args
                .get("toe_board_height")
                .and_then(|v| v.as_f64())
                .unwrap_or(crate::safety::TOE_BOARD_HEIGHT_MM),
            call.args
                .get("door_bottom_gap")
                .and_then(|v| v.as_f64())
                .unwrap_or(crate::safety::DOOR_BOTTOM_GAP_MAX_MM),
            call.args
                .get("include_warning_sign")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            call.args
                .get("include_material_table")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            call.args
                .get("scale")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0),
        ),
        #[cfg(windows)]
        "validate_elevator_shaft_protection" => crate::cad::cad_validate_elevator_shaft_protection(
            num(&call.args, "opening_width")?,
            num(&call.args, "opening_height")?,
            call.args
                .get("guard_height")
                .and_then(|v| v.as_f64())
                .unwrap_or(crate::safety::GUARD_DOOR_HEIGHT_MM),
            call.args
                .get("toe_board_height")
                .and_then(|v| v.as_f64())
                .unwrap_or(crate::safety::TOE_BOARD_HEIGHT_MM),
            call.args
                .get("door_bottom_gap")
                .and_then(|v| v.as_f64())
                .unwrap_or(crate::safety::DOOR_BOTTOM_GAP_MAX_MM),
            call.args
                .get("include_warning_sign")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            call.args
                .get("include_material_table")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        ),
        #[cfg(windows)]
        "draw_text" => crate::cad::cad_draw_text(
            num(&call.args, "x")?,
            num(&call.args, "y")?,
            s(&call.args, "text")?,
            num(&call.args, "height")?,
            call.args
                .get("rotation_deg")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
        ),
        #[cfg(windows)]
        "move" => crate::cad::cad_move(
            num(&call.args, "dx")?,
            num(&call.args, "dy")?,
            s(&call.args, "target")?,
        ),
        #[cfg(windows)]
        "move_handle" => crate::cad::cad_move_handle(
            s(&call.args, "handle")?,
            num(&call.args, "dx")?,
            num(&call.args, "dy")?,
        ),
        #[cfg(windows)]
        "rotate_handle" => crate::cad::cad_rotate_handle(
            s(&call.args, "handle")?,
            num(&call.args, "cx")?,
            num(&call.args, "cy")?,
            num(&call.args, "angle_deg")?,
        ),
        #[cfg(windows)]
        "copy_handle" => crate::cad::cad_copy_handle(
            s(&call.args, "handle")?,
            num(&call.args, "dx")?,
            num(&call.args, "dy")?,
        ),
        #[cfg(windows)]
        "mirror_handle" => crate::cad::cad_mirror_handle(
            s(&call.args, "handle")?,
            num(&call.args, "x1")?,
            num(&call.args, "y1")?,
            num(&call.args, "x2")?,
            num(&call.args, "y2")?,
        ),
        #[cfg(windows)]
        "offset_handle" => crate::cad::cad_offset_handle(
            s(&call.args, "handle")?,
            num(&call.args, "distance")?,
            num(&call.args, "side_x")?,
            num(&call.args, "side_y")?,
        ),
        #[cfg(windows)]
        "trim_by_handle" => crate::cad::cad_trim_by_handle(
            s(&call.args, "boundary_handle")?,
            s(&call.args, "target_handle")?,
            num(&call.args, "pick_x")?,
            num(&call.args, "pick_y")?,
        ),
        #[cfg(windows)]
        "extend_by_handle" => crate::cad::cad_extend_by_handle(
            s(&call.args, "boundary_handle")?,
            s(&call.args, "target_handle")?,
            num(&call.args, "pick_x")?,
            num(&call.args, "pick_y")?,
        ),
        #[cfg(windows)]
        "erase_last" => crate::cad::cad_erase_last(),
        #[cfg(windows)]
        "erase_handle" => crate::cad::cad_erase_handle(s(&call.args, "handle")?),
        #[cfg(windows)]
        "zoom_extents" => crate::cad::cad_zoom_extents(),
        #[cfg(windows)]
        "inspect_handle" => crate::cad::cad_inspect_handle(s(&call.args, "handle")?),
        #[cfg(windows)]
        "list_selection" => crate::cad::cad_list_selection(),
        #[cfg(windows)]
        "modelspace_snapshot" => crate::cad::cad_modelspace_snapshot(),
        #[cfg(windows)]
        "import_selection" => Err("import_selection 应由专门分支处理".to_string()),
        #[cfg(windows)]
        "run_lisp" => crate::cad::cad_run_lisp(s(&call.args, "code")?),
        #[cfg(not(windows))]
        _ => Err("CAD 工具仅在 Windows 上可用".to_string()),
        #[cfg(windows)]
        other => Err(format!("未知工具: {other}")),
    })();
    match result {
        Ok(content) => ToolResult {
            id: call.id.clone(),
            name: call.name.clone(),
            ok: true,
            confirmation_required: false,
            object_updates: object_updates_for_success(call, &content),
            content,
        },
        Err(e) => ToolResult {
            id: call.id.clone(),
            name: call.name.clone(),
            ok: false,
            content: e,
            confirmation_required: false,
            object_updates: Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn run_lisp_is_blocked_by_default() {
        let result = dispatch(&ToolCall {
            id: "t1".to_string(),
            name: "run_lisp".to_string(),
            args: json!({ "code": "(alert \"unsafe\")" }),
        });

        assert!(!result.ok);
        assert!(result.confirmation_required);
        assert!(result.object_updates.is_empty());
        assert!(result.content.contains("人工确认"));
    }

    #[test]
    fn stair_requests_select_double_flight_stair_tool() {
        let tooling = select_tooling_context(
            "画一个双跑楼梯，宽 1200，每跑 10 级",
            &[],
            WorkMode::CompetitionMode,
        );

        assert!(tooling
            .tool_names
            .iter()
            .any(|name| name == "draw_double_flight_stair"));
    }

    #[test]
    fn safety_requests_select_elevator_shaft_tools_without_lisp() {
        let tooling = select_tooling_context(
            "画一个电梯井口防护门，井口宽 2000 高 1800",
            &[],
            WorkMode::CompetitionMode,
        );

        assert!(tooling
            .tool_names
            .iter()
            .any(|name| name == "draw_elevator_shaft_protection"));
        assert!(tooling
            .tool_names
            .iter()
            .any(|name| name == "validate_elevator_shaft_protection"));
        assert!(!tooling.tool_names.iter().any(|name| name == "run_lisp"));
    }

    #[test]
    fn generic_edge_guardrail_does_not_select_elevator_shaft_tool() {
        let tooling = select_tooling_context(
            "画一个楼层临边防护栏杆，长度 3000",
            &[],
            WorkMode::CompetitionMode,
        );

        assert!(!tooling
            .tool_names
            .iter()
            .any(|name| name == "draw_elevator_shaft_protection"));
        assert!(!tooling
            .tool_names
            .iter()
            .any(|name| name == "validate_elevator_shaft_protection"));
        assert!(tooling.guidance.contains("edge_guardrail"));
        assert!(tooling.guidance.contains("尚未开放确定性 CAD 出图"));
    }

    #[test]
    fn safety_demo_mode_does_not_force_edge_guardrail_to_elevator_tool() {
        let tooling = select_tooling_context(
            "画一个屋面临边防护栏杆，长度 5000",
            &[],
            WorkMode::SafetyDemoMode,
        );

        assert!(is_safety_request("画一个屋面临边防护栏杆"));
        assert_eq!(
            safety_context_scene("画一个屋面临边防护栏杆"),
            Some("edge_guardrail")
        );
        assert!(!tooling
            .tool_names
            .iter()
            .any(|name| name == "draw_elevator_shaft_protection"));
        assert!(tooling
            .tool_names
            .iter()
            .any(|name| name == "modelspace_snapshot"));
    }

    #[test]
    fn safety_demo_mode_keeps_generic_safety_requests_in_safe_context() {
        let tooling = select_tooling_context(
            "施工安全规范怎么梳理",
            &[],
            WorkMode::SafetyDemoMode,
        );

        assert!(!tooling
            .tool_names
            .iter()
            .any(|name| name == "draw_elevator_shaft_protection"));
        assert!(tooling
            .tool_names
            .iter()
            .any(|name| name == "modelspace_snapshot"));
        assert!(tooling.guidance.contains("未命中已注册"));
    }

    #[test]
    fn competition_mode_does_not_expose_lisp_for_escape_keywords() {
        let tooling = select_tooling_context(
            "画一个带圆角和样条曲线的图形",
            &[],
            WorkMode::CompetitionMode,
        );

        assert!(!tooling.tool_names.iter().any(|name| name == "run_lisp"));
    }

    #[test]
    fn safety_missing_params_detects_incomplete_request() {
        let missing = safety_missing_params("画一个电梯井口防护");
        assert!(missing.iter().any(|m| *m == "井口宽度"));
        assert!(missing.iter().any(|m| *m == "井口高度/进深"));
        // 防护门高、踢脚板是规范定值，不应再作为缺参追问。
        assert!(!missing.iter().any(|m| *m == "防护门高度"));

        let missing = safety_missing_params("画一个电梯井口防护，井口宽 2000，防护门高 1500");
        assert!(!missing.iter().any(|m| *m == "井口宽度"));
        assert!(missing.iter().any(|m| *m == "井口高度/进深"));

        // 已给出井口宽高时不应报告缺参（防护门高/踢脚板无需追问）
        let missing = safety_missing_params("画一个电梯井口防护，井口宽 2000，高 1800");
        assert!(missing.is_empty());
    }

    #[test]
    fn safety_clarification_prompt_only_for_draw_requests() {
        assert!(safety_clarification_prompt("画一个电梯井口防护").is_some());
        assert!(safety_clarification_prompt("电梯井口防护有哪些安全要求").is_none());
    }
}
