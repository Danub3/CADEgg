# 电梯井口临边防护 Demo 工作流

## 目标

把「用户一句话」推进成「AutoCAD 生成标准防护布置图 + 确定性校核 + 可追溯结果」的最小闭环。

## 闭环流程

```text
用户输入施工条件
  -> 缺参追问（井口宽、井口高、栏杆高、立杆间距、踢脚板高、警示牌、材料表）
  -> 检索标准图册知识卡（data/atlas/elevator_shaft_protection.json）
  -> 生成结构化绘图计划
  -> 调用 draw_elevator_shaft_protection 出图
  -> 调用 validate_elevator_shaft_protection 校核
  -> 输出图纸 + 校核 JSON + 修改履历
```

## 演示固定指令（可直接粘贴）

```text
画一个电梯井口临边防护，井口宽 2000，高 1800，防护栏杆 1200，立杆间距 2000，踢脚板 180，包含警示牌和材料表。
```

预期：

1. Agent 调用 `draw_elevator_shaft_protection`
2. Agent 调用 `validate_elevator_shaft_protection`
3. AutoCAD 生成标准布置图
4. 前端对象表同步
5. 工具结果出现 JSON 校核，前端校核面板展示通过/未通过

## 缺参追问（用户只说「画一个电梯井口防护」时）

Agent 应优先追问，不自行编造关键尺寸：

- 井口宽度
- 井口高度/进深
- 防护栏杆高度
- 立杆间距
- 踢脚板高度
- 是否包含警示牌
- 是否输出材料表

## 工作模式

- `safety_demo_mode`：只暴露电梯井口临边防护闭环工具，屏蔽 `run_lisp` 与 Claude 入口。
- `competition_mode`：隐藏并阻断 `run_lisp`，隐藏 Claude。

## 验收要点

- 洞口轮廓清楚
- 防护栏围合完整
- 上横杆 / 中横杆 / 立杆 / 踢脚板按参数生成
- 警示牌、材料表、文字标注存在且不遮挡图形
- `zoom_extents` 后图面完整可见
- 返回 handle 可被前端会话状态稳定记录
