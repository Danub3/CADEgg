# CADEgg

**Construction-Safety Drawing Agent · 建筑施工安全绘图智能体**

[English](#english) · [中文](#中文)

* * *

## English

### What is CADEgg?

CADEgg is a Windows desktop AutoCAD assistant that turns a single natural-language instruction into a validated safety-protection drawing. It is built with Tauri, React, and Rust, drives AutoCAD through an internal .NET bridge with COM fallback, and focuses on one complete closed loop: input site conditions → ask for missing parameters → retrieve the standard atlas → generate the protection drawing → run rule-based validation → output the drawing and a traceable log.

### Features

| Feature | Description |
|---|---|
| **Safety Scene Registry** | A `src-tauri/src/scenes.rs` registry drives safety routing by scene: elevator shaft protection (deterministic draw + validate), edge guardrail, opening cover, stair guard, and safety passage shed are registered; unmatched requests fall back to a generic safety context instead of misusing elevator tools |
| **Elevator Shaft Protection Door** | Draws shaft opening, upper-flip protection door, hinge markers, toe board, warning sign, dimensions, and material table; deterministic validation enforces door height >= 1500 mm, door bottom gap <= 50 mm, and toe board presence, with recommended items reported as warnings |
| **Rule-based Validation** | Deterministic checks (door height, toe board, warning sign, material table, dimensions) returned as structured JSON |
| **Missing-parameter Clarification** | Asks for shaft opening width/height instead of inventing site dimensions; protection-door height and toe board use standard defaults |
| **Standard Atlas & Rules** | Versioned knowledge cards under `data/atlas/` (elevator shaft protection, edge guardrail, CAD drafting standard), scanned at runtime; new cards are picked up by dropping a JSON file |
| **Session Object Tracking** | Tracks created handles in the frontend object table for later reference |
| **Model Routing** | Practical providers are GLM and Gemini; Claude is hidden in the UI for future support |
| **Competition Mode** | `competition_mode` hides and blocks `run_lisp`, reducing arbitrary command execution risk |
| **Workflow Docs** | Demo walkthrough under `workflows/elevator_shaft_protection_demo.md` |

### Requirements

- Windows
- AutoCAD with COM automation available
- Node.js and npm
- Rust toolchain with Cargo
- API key for GLM or Gemini

### Build from Source

```powershell
npm install
npm run build
npm run tauri dev
```

If PowerShell blocks `npm.ps1`, run `npm.cmd` instead or adjust the local execution policy.

### Verification

```powershell
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

Some Rust tests require a local AutoCAD installation and are marked ignored.

* * *

## 中文

### 什么是 CADEgg？

CADEgg 是一款 Windows 桌面 AutoCAD 助手，能把一句自然语言指令变成经过校核的安全防护图纸。它基于 Tauri、React 和 Rust 构建，通过内置 .NET 桥接（COM 回退）驱动 AutoCAD，聚焦一个完整闭环：输入现场条件 → 追问缺参 → 检索标准图册 → 生成防护图 → 规则校核 → 输出图纸和可追溯履历。

### 功能特性

| 功能 | 说明 |
|---|---|
| **安全场景注册表** | `src-tauri/src/scenes.rs` 驱动安全场景路由：电梯井口防护（确定性出图+校核）已就绪；普通临边栏杆、洞口盖板、楼梯口防护、安全通道棚已注册登记；未命中的安全请求进入通用安全上下文，不会误用电梯井口工具 |
| **电梯井口防护门** | 按参数绘制井口轮廓、上翻式防护门、翻转轴、踢脚板、警示牌、尺寸标注和材料表；确定性校核强制门高 ≥ 1500mm、门底间隙 ≤ 50mm、设置挡脚板，推荐项以 warnings 提醒 |
| **规则校核** | 确定性检查（防护门高度、踢脚板、警示牌、材料表、尺寸）以结构化 JSON 返回 |
| **缺参追问** | 追问井口宽高等现场尺寸；防护门高和踢脚板按规范默认值处理 |
| **标准图册与规则** | `data/atlas/` 下版本化维护知识卡（电梯井口防护、普通临边栏杆、CAD 制图标准），运行时扫描目录，新增卡片只需放入 JSON |
| **会话对象追踪** | 在前端对象表记录创建的 handle，供后续引用 |
| **模型路由** | 实际可用提供方为 GLM 和 Gemini；Claude 入口隐藏，留待后续支持 |
| **比赛模式** | `competition_mode` 隐藏并阻断 `run_lisp`，降低任意命令执行风险 |
| **工作流文档** | 演示流程见 `workflows/elevator_shaft_protection_demo.md` |

### 环境要求

- Windows
- 已启用 COM 自动化的 AutoCAD
- Node.js 与 npm
- Rust 工具链与 Cargo
- GLM 或 Gemini 的 API Key

### 从源码构建

```powershell
npm install
npm run build
npm run tauri dev
```

如果 PowerShell 拦截 `npm.ps1`，请改用 `npm.cmd`，或调整本地执行策略。

### 验证

```powershell
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

部分 Rust 测试需要本地 AutoCAD 环境，已被标记为 ignored。

* * *

## License

This project is licensed under the [PolyForm Noncommercial License 1.0.0](https://polyformproject.org/licenses/noncommercial/1.0.0) — 非商业使用，可自由修改与学习，禁止商业用途。© [Danub3](https://github.com/Danub3)
