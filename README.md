# CADEgg

**Construction-Safety Drawing Agent for AutoCAD · 面向施工安全防护图纸的 AutoCAD 智能绘图 Agent**

[English](#english) · [中文](#中文)

CADEgg connects a desktop chat interface to a real AutoCAD drawing session. It turns natural-language construction-safety requests into editable DWG entities, then returns tool traces, object handles, validation results, and session memory records.

Current focus: deterministic, traceable safety-protection drawings rather than generic CAD chat.

---

## English

### Current Status

| Area | Status | Notes |
|---|---|---|
| Safety drawing loop | Production demo ready | Three deterministic safety scenes can draw, validate, and inspect the resulting model space. |
| AutoCAD integration | Local Bridge first, COM fallback | The C# AutoCAD Bridge listens on `127.0.0.1:50471`; version `0.3.8.0` marshals drawing work onto AutoCAD's UI dispatcher. |
| Model providers | Domestic providers only | The active user-facing providers are GLM, DeepSeek, Qwen, and Kimi through OpenAI-compatible chat/tool APIs. Gemini and Claude are not part of the current route. |
| Knowledge base | Local, versioned JSON cards | `data/atlas/` stores agent-facing conclusions; `data/sources/` stores source excerpts and citations. |
| Verification baseline | Passing | Last recorded baseline: `npm.cmd run build` passed, Rust unit suite `92 passed / 0 failed / 10 ignored`, AutoCAD smoke suite `4 passed / 0 failed` when run serially. |

### What CADEgg Does

CADEgg implements a closed loop for construction-safety drawings:

1. Read a Chinese natural-language request from the Tauri/React desktop UI.
2. Detect the safety scene and decide whether required site dimensions are missing.
3. Inject the matching standard atlas card and CAD drafting card into the model context.
4. Ask a configured model to choose structured CAD tools, not to invent drawing output.
5. Execute drawing and editing tools in AutoCAD through the local Bridge.
6. Return created handles, object summaries, rule checks, token telemetry, and route diagnostics.
7. Save the session as Markdown, summary, JSONL events, and optional global memory.

### Feature Matrix

| Module | Capability | Implementation |
|---|---|---|
| Scene routing | Registry-based routing for construction-safety requests, with registered fallback behavior for unfinished scenes. | `src-tauri/src/scenes.rs`, `tools::select_tooling_context` |
| Full-loop scenes | Elevator shaft protection door, elevator shaft safety net, and edge guardrail can all draw, validate, and run `modelspace_snapshot`. | `draw_*`, `validate_*`, knowledge cards |
| Registered scenes | Opening cover, stair guard, and safety passage shed are registered with boundaries, keywords, and required parameters, but are not yet deterministic draw/validate scenes. | Scene registry only |
| Missing parameters | Drawing requests that lack critical site dimensions are routed to clarification instead of fabricated values. | `safety_missing_params`, scene-specific prompts |
| Rule validation | Mandatory rules become `issues`; recommended rules become `warnings`; outputs are structured JSON for the UI and exported logs. | `src-tauri/src/safety.rs` |
| CAD tools | Basic drawing, semantic geometry, safety components, text, editing by handle, selection import, inspection, and model-space snapshots. | Single catalog in `src-tauri/src/tools.rs` |
| Session objects | Created or imported CAD handles are tracked so later instructions can say "that object" and resolve to handles. | Frontend object table plus `sync_session_objects` |
| Route diagnostics | Each request records planned, skipped, attempted, selected, failed, and fallback model candidates. | `ModelRouteTelemetry` |
| Session memory | Exports full Markdown, compact summary, event JSONL, index, and `global-memory.md`; global memory is only carried when explicitly enabled. | `src-tauri/src/session_export.rs` |
| Model benchmark | Runs a fixed, side-effect-free model benchmark and writes `benchmark-results.json/md`. | `src-tauri/src/benchmark.rs` |
| Execution safety | `run_lisp` is hidden/blocked in competition mode and confirmation-gated when exposed. Destructive editing tools also require confirmation. | `WorkMode`, `requires_confirmation` |

### Safety Scenes

| Scene | Current Level | Key Rules |
|---|---|---|
| Elevator shaft protection door | Full loop | Guard door height `>= 1500mm`, door bottom gap `<= 50mm`, toe board required. Supports `flip_up` and `three_piece` door types plus lifecycle checks for temporary removal/restoration. |
| Elevator shaft safety net | Full loop | Safety net every 2 floors and not more than `10m`; upper isolation protection above the construction level; positive shaft dimensions. `net_to_wall_gap <= 25mm` remains a recommended item until its source is fully confirmed. |
| Edge guardrail | Full loop | Top rail `>= 1200mm`, post spacing `<= 2000mm`, toe board `>= 180mm`, lower/mid rail between top rail and toe board; dense mesh net is recommended. |
| Opening cover | Registered | Parameters and boundaries are recorded; deterministic drawing and validation are still pending. |
| Stair guard | Registered | Parameters and boundaries are recorded; deterministic drawing and validation are still pending. |
| Safety passage shed | Registered | Parameters and boundaries are recorded; deterministic drawing and validation are still pending. |

### Model Routing

CADEgg currently routes only to these user-facing providers:

| Provider | Default Cheap Model | Default Strong Model | Base URL |
|---|---|---|---|
| GLM | `glm-4.5-air` | `glm-4.5` | `https://open.bigmodel.cn/api/paas/v4` |
| DeepSeek | `deepseek-v4-flash` | `deepseek-v4-pro` | `https://api.deepseek.com` |
| Qwen | `qwen3.7-flash` | `qwen3.8-max` | `https://dashscope.aliyuncs.com/compatible-mode/v1` |
| Kimi | `kimi-k2.5` | `kimi-k2.6` | `https://api.moonshot.cn/v1` |

Routing behavior:

- Cheap/strong tiering is deterministic: drawing, editing, review, and safety-scene tasks use the strong slot; simple Q&A uses the cheap slot.
- A session-level model pick overrides the cheap/strong slots for that session.
- If automatic failover is enabled, CADEgg tries the selected provider first, then the same provider's alternate slot, then other configured providers.
- Providers without an API key are kept in route telemetry as skipped candidates, but are never called.
- API keys stay in the local Tauri AppData `settings.json` and are redacted in frontend views and errors.

Legacy note: older settings structs still contain Claude/Gemini fields for compatibility with existing `settings.json` files and earlier experiments. The current frontend provider type and backend provider chain expose GLM, DeepSeek, Qwen, and Kimi only.

### Model Benchmark

The benchmark harness evaluates model behavior for CADEgg-specific work rather than general chat quality. It does not execute AutoCAD drawing commands.

| Dimension | Weight |
|---|---:|
| Tool-call reliability | 25% |
| CAD / safety-rule accuracy | 25% |
| Stability | 15% |
| Speed | 15% |
| Cost placeholder | 10% |
| Long-context continuation | 10% |

The recorded full-list benchmark on 2026-08-18 covered 38 listed models. Ratings in `src/constants.ts` are the single source for the UI and help panel.

### Architecture

```text
React 19 + TypeScript + Vite
        |
        | Tauri commands / events
        v
Rust backend
  - settings and provider routing
  - domain guard and task tiering
  - scene registry and knowledge-card rendering
  - tool catalog and confirmation policy
  - session export, memory, and benchmark harness
        |
        | JSON over localhost
        v
C# AutoCAD Bridge 0.3.8.0
        |
        v
AutoCAD model space entities
```

### Repository Layout

| Path | Purpose |
|---|---|
| `src/` | React desktop UI, model picker, session memory, benchmark panel, exported Markdown builders. |
| `src-tauri/src/` | Rust backend, model routing, safety rules, CAD tools, knowledge rendering, session export, benchmark harness. |
| `src-tauri/autocad_bridge/` | C# AutoCAD Bridge source. |
| `data/atlas/` | Versioned knowledge cards consumed by the agent. |
| `data/sources/` | Source excerpts used to support knowledge-card citations. |
| `rules/` | Rule data used by deterministic validators. |
| `workflows/` | Demo workflow documentation. |
| `启动CADEgg.cmd` | Windows launcher with AutoCAD preflight, stale build detection, and existing-window restore. |

### Requirements

- Windows
- AutoCAD with .NET plugin support and COM automation available
- Node.js and npm
- Rust toolchain with Cargo
- API key for at least one current provider: GLM, DeepSeek, Qwen, or Kimi

AutoCAD 2027 was used for the latest recorded smoke validation. Other AutoCAD versions may work if their .NET/COM automation interfaces are compatible.

### Build and Run

```powershell
npm install
npm.cmd run build
npm.cmd run tauri -- dev
```

For the local debug executable:

```powershell
npm.cmd run tauri -- build --debug --no-bundle
.\启动CADEgg.cmd
```

The launcher can also be used in dev mode:

```powershell
.\启动CADEgg.cmd --dev
```

If PowerShell blocks `npm.ps1`, use `npm.cmd` as shown above or adjust the local execution policy.

### Verification

```powershell
npm.cmd run build
cargo test --manifest-path src-tauri\Cargo.toml
```

Real AutoCAD smoke tests are ignored by default. Run them serially because they share one AutoCAD document:

```powershell
cargo test --manifest-path src-tauri\Cargo.toml smoke_test_round_trip -- --ignored --nocapture --test-threads=1
```

### Extending a Safety Scene

1. Add or update source excerpts in `data/sources/`.
2. Add a versioned knowledge card in `data/atlas/`.
3. Register keywords, required parameters, rule tiers, and readiness flags in `scenes.rs`.
4. Add the built-in card fallback in `knowledge.rs`.
5. Implement deterministic validation in `safety.rs`.
6. Implement CAD drawing in `cad.rs` with parameter checks before AutoCAD calls.
7. Add tool schemas and dispatch entries in `tools.rs`.
8. Add frontend summary parsing and display in `types.ts` / `App.tsx`, then run tests and serial AutoCAD smoke checks.

---

## 中文

### 项目定位

CADEgg 是一个面向施工安全防护图纸的 AutoCAD 智能绘图 Agent。它不是把大模型回复当成最终图纸，而是让模型负责理解意图和选择工具，让 Rust 后端、知识卡、规则函数和 AutoCAD Bridge 共同完成“自然语言 -> 标准图册约束 -> 真实 DWG 图元 -> 规则校核 -> 可追溯记录”的闭环。

当前重点是施工安全防护场景，尤其是电梯井口防护、井内安全平网、普通临边防护栏杆等规则密集且需要可复核交付的图纸任务。

### 当前状态

| 模块 | 状态 | 说明 |
|---|---|---|
| 场景闭环 | 已可演示 | 电梯井口防护门、电梯井内安全平网、普通临边防护栏杆三类场景已具备确定性出图、校核和图面快照。 |
| CAD 连接 | Bridge 优先 | C# AutoCAD Bridge 监听 `127.0.0.1:50471`，当前版本 `0.3.8.0`，绘图事务调度到 AutoCAD UI Dispatcher，COM 作为回退/辅助通道。 |
| 模型路线 | 国产模型 | 当前用户可见、后端实际路由的供应商为智谱 GLM、DeepSeek、通义千问、Kimi；Gemini 和 Claude 已不属于当前可用路线。 |
| 知识库 | 本地 JSON | `data/atlas/` 保存面向 Agent 的知识卡，`data/sources/` 保存规范摘录和引用出处；运行时可扫描磁盘并带内置兜底。 |
| 验证基线 | 已记录 | 最近记录：`npm.cmd run build` 通过；Rust 单元测试 `92 passed / 0 failed / 10 ignored`；真实 AutoCAD 串行 smoke `4 passed / 0 failed`。 |

### 核心能力

| 模块 | 能力 | 代码位置 |
|---|---|---|
| 场景 | 用注册表识别施工安全场景，避免把临边、井口、井内平网等相近场景互相误用。 | `src-tauri/src/scenes.rs` |
| 出图 | 三个闭环场景可生成真实 AutoCAD 图元，并包含尺寸、标注、材料表和构造示意。 | `src-tauri/src/cad.rs` |
| 校核 | 强制项进入 `issues`，推荐项进入 `warnings`，结果以结构化 JSON 返回前端和导出记录。 | `src-tauri/src/safety.rs` |
| 缺参 | 缺少现场实测关键尺寸时先追问，不自行编造井口宽高、井道尺寸或临边长度。 | `safety_missing_params` |
| 工具 | 统一工具目录覆盖基础绘制、语义几何、安全构件、文字标注、按 handle 编辑、选择导入和图面快照。 | `src-tauri/src/tools.rs` |
| 对象 | 记录创建或导入的 CAD handle，支持后续用“刚才那个对象”“选中的线”等表达继续编辑。 | `sessionObjects.ts` |
| 快照 | `modelspace_snapshot` 枚举模型空间对象总数、类型分布、包围盒、图层、颜色和主要几何。 | `cad_modelspace_snapshot` |
| 路由 | 每次请求记录模型候选、跳过原因、失败原因、回退次数和最终命中模型。 | `ModelRouteTelemetry` |
| 记忆 | 自动导出会话 Markdown、摘要、事件 JSONL、索引和 `global-memory.md`，全局记忆需手动启用携带。 | `session_export.rs` |
| 基准 | 用固定小测试集评估模型工具调用、CAD/规范准确性、稳定性、速度、成本占位和长上下文。 | `benchmark.rs` |
| 安全 | 比赛模式禁用 `run_lisp`；删除、修剪、延伸和 LISP 等高风险工具需要用户确认。 | `WorkMode` |

### 安全场景

| 场景 | 完成度 | 关键规则与边界 |
|---|---|---|
| 电梯井口防护门 | 完整闭环 | 防护门高度不小于 `1500mm`，门底间隙不大于 `50mm`，必须设置踢脚板；支持上翻式 `flip_up` 和三件套式 `three_piece`；支持临时拆除、恢复、责任人、替代防护和验收状态等施工生命周期校核。 |
| 电梯井内安全平网 | 完整闭环 | 依据 JGJ 80-2016 4.2.3，每隔 2 层且不大于 `10m` 加设一道安全平网，施工层上部设置隔离防护；井道尺寸必须为正；“网体与井壁空隙 `<= 25mm`”当前作为推荐项，等待进一步出处核实。 |
| 普通临边防护栏杆 | 完整闭环 | 上杆高度不小于 `1200mm`，立杆间距不大于 `2000mm`，挡脚板高度不小于 `180mm`，下杆位于上杆和挡脚板之间；密目安全网作为推荐做法。 |
| 楼板/屋面洞口防护 | 已登记 | 已有场景关键词、必填参数和边界描述；确定性出图和校核尚未开放，不会借用电梯井口工具替代。 |
| 楼梯口/梯段边防护 | 已登记 | 已有场景关键词、必填参数和边界描述；确定性出图和校核尚未开放。 |
| 安全通道/防护棚 | 已登记 | 已有场景关键词、必填参数和边界描述；确定性出图和校核尚未开放。 |

### 模型路由

当前 CADEgg 已收敛到国产模型供应商。README 旧版里“GLM + Gemini，Claude 隐藏待支持”的说法已经过时。

| 供应商 | 便宜档默认模型 | 强模型默认模型 | 默认 Base URL |
|---|---|---|---|
| 智谱 GLM | `glm-4.5-air` | `glm-4.5` | `https://open.bigmodel.cn/api/paas/v4` |
| DeepSeek | `deepseek-v4-flash` | `deepseek-v4-pro` | `https://api.deepseek.com` |
| 通义千问 | `qwen3.7-flash` | `qwen3.8-max` | `https://dashscope.aliyuncs.com/compatible-mode/v1` |
| Kimi | `kimi-k2.5` | `kimi-k2.6` | `https://api.moonshot.cn/v1` |

路由策略：

- 本地规则先判断任务分级：绘图、编辑、复核、施工安全场景走强模型；普通问答和解释走便宜档。
- 会话输入区可以显式选择本轮模型；显式选择会覆盖该会话的便宜/强模型槽位。
- 开启自动轮转后，先尝试当前模型，再尝试同供应商备用模型，最后尝试其他已配置 API Key 的供应商。
- 没有配置 Key 的供应商会显示为 skipped，不进入真实调用。
- 路由过程通过 `agent:event` 返回前端，用户能看到 planned、attempting、selected、fallback、skipped、failed 等状态。
- 卡死也会触发轮转：单轮流式读取超过 90 秒没有任何有效输出（只有 keep-alive 心跳同样算）即判定模型无响应，直接切换到下一个候选模型；等待期间前端显示「模型已静默 Ns」。
- 任务运行中可以随时停止：输入区、思考面板的「停止」按钮或 Esc 都会置取消标记，流式连接在数百毫秒内断开，已产生的消息和已绘制图形保留；取消不会触发轮转，也不会消耗其他模型额度。
- API Key 只保存在本机 Tauri AppData 的 `settings.json`，界面只显示脱敏预览，错误信息也会做脱敏处理。

### 工作模式与工具门控

| 模式 | 行为 |
|---|---|
| 通用 CAD 模式（默认，`competition_mode`） | 直线、圆、矩形、正多边形、楼梯等结构化工具全开；命中注册安全场景时仍走场景专用出图/校核工具。为保证可复现，禁用任意 LISP。 |
| 安全场景模式（`safety_demo_mode`） | 施工安全话题只走已注册场景；未注册场景（如脚手架）只给知识卡与追问，不给绘图工具。非安全请求（画直线、画圆、双跑楼梯等）照常出图。 |

关键点：安全约束按「场景/话题」判定，不再按模式全局短路工具集。历史版本在安全场景模式下会把所有请求都降级成 `draw_text/zoom_extents/inspect_handle/modelspace_snapshot`，导致新会话示例里的直线、圆、楼梯都拿不到绘图工具（模型只能回答「没有 draw_circle」）。

兼容说明：`settings.rs` 中仍保留 Claude/Gemini 的旧字段和迁移兼容逻辑，`tools.rs` 里也还有旧 schema helper；这些不是当前用户可见供应商，也不进入当前 provider 白名单和 failover 链路。

### 模型列表与基准

`src/constants.ts` 是前端模型列表和帮助文档评级的单一事实源。当前列表覆盖：

| 供应商 | 模型范围 |
|---|---|
| 智谱 GLM | GLM-5.2、GLM-5.1、GLM-5、GLM-5-Turbo、GLM-4.7、GLM-4.7-FlashX、GLM-4.6、GLM-4.5 系列、GLM-4-Flash 系列 |
| DeepSeek | DeepSeek V4 Pro、DeepSeek V4 Flash |
| 通义千问 | Qwen 3.8/3.7/3.6/3.5、Qwen3 Coder、Qwen3 Max |
| Kimi | Kimi K3、Kimi K2.7 Code、Kimi K2.6、Kimi K2.5 |

基准测试按 CADEgg 的真实任务设计，不执行 AutoCAD 副作用，只检查模型输出意图和工具调用 JSON。评分权重为：工具调用可靠性 25%、CAD/规范准确性 25%、稳定性 15%、速度 15%、成本占位 10%、长上下文 10%。2026-08-18 已完成列表全量 38 模型实测并回写星级。

### 技术架构

```text
React 19 + TypeScript + Vite 前端
        |
        | Tauri command / event
        v
Rust 后端
  - 模型设置、任务分级、自动轮转
  - 领域守卫、场景注册表、知识卡注入
  - 工具目录、确认策略、规则校核
  - 会话导出、记忆包、模型基准
        |
        | localhost JSON
        v
C# AutoCAD Bridge 0.3.8.0
        |
        v
AutoCAD 模型空间真实图元
```

### 目录结构

| 路径 | 内容 |
|---|---|
| `src/` | React 前端、模型选择器、会话对象表、记忆面板、模型基准面板、Markdown 导出构造。 |
| `src-tauri/src/` | Rust 后端、模型路由、安全规则、CAD 工具、知识卡渲染、会话导出、基准测试。 |
| `src-tauri/autocad_bridge/` | AutoCAD 侧 C# Bridge 插件源码。 |
| `data/atlas/` | 面向 Agent 的标准图册知识卡。 |
| `data/sources/` | 规范摘录、出处和页码等溯源信息。 |
| `rules/` | 确定性校核规则数据。 |
| `workflows/` | 演示流程文档。 |
| `启动CADEgg.cmd` | Windows 启动器，包含 AutoCAD 预检、过期构建检测和已有窗口恢复。 |

### 环境要求

- Windows
- AutoCAD，需支持 .NET 插件和 COM 自动化
- Node.js 与 npm
- Rust 工具链与 Cargo
- 至少一个国产模型供应商 API Key：智谱 GLM、DeepSeek、通义千问或 Kimi

最近一次实机 smoke 使用 AutoCAD 2027。其他 AutoCAD 版本如 .NET/COM 接口兼容，理论上可以运行，但需要以实机验证为准。

### 构建与运行

```powershell
npm install
npm.cmd run build
npm.cmd run tauri -- dev
```

构建并运行 debug exe：

```powershell
npm.cmd run tauri -- build --debug --no-bundle
.\启动CADEgg.cmd
```

启动器也支持开发模式：

```powershell
.\启动CADEgg.cmd --dev
```

如果 PowerShell 拦截 `npm.ps1`，请使用 `npm.cmd`，或调整本机执行策略。

### 验证

```powershell
npm.cmd run build
cargo test --manifest-path src-tauri\Cargo.toml
```

真实 AutoCAD smoke 测试默认被标记为 ignored。运行时必须串行，避免多个测试共享同一个 DWG 文档互相污染：

```powershell
cargo test --manifest-path src-tauri\Cargo.toml smoke_test_round_trip -- --ignored --nocapture --test-threads=1
```

### 新增安全场景模板

1. 在 `data/sources/` 补规范摘录和出处。
2. 在 `data/atlas/` 新增或更新知识卡。
3. 在 `scenes.rs` 登记关键词、必填参数、规则效力分级和就绪状态。
4. 在 `knowledge.rs` 补内置知识卡兜底。
5. 在 `safety.rs` 实现确定性校核和缺参追问。
6. 在 `cad.rs` 实现绘图函数，先做参数校验，再调用 AutoCAD。
7. 在 `tools.rs` 补工具 schema、工具目录和 dispatch。
8. 在 `types.ts` / `App.tsx` 补前端 summary 解析与展示，然后跑单元测试和串行实机 smoke。

---

## License

This project is licensed under the [PolyForm Noncommercial License 1.0.0](https://polyformproject.org/licenses/noncommercial/1.0.0). Non-commercial use, modification, and learning are allowed; commercial use is prohibited.

© [Danub3](https://github.com/Danub3)
