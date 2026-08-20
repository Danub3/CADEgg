import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type Dispatch,
  type FormEvent,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent,
  type ReactNode,
  type RefObject,
  type SetStateAction,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openPath } from "@tauri-apps/plugin-opener";

import "./App.css";
import type {
  AgentEvent,
  BenchmarkCandidate,
  BenchmarkCaseResult,
  BenchmarkEvent,
  BenchmarkModelResult,
  BenchmarkSummary,
  DemoLogEntry,
  ElevatorValidation,
  MemoryBundleInfo,
  MemoryFileInfo,
  Message,
  ModelRouteTelemetry,
  ObjectUpdate,
  Provider,
  ProviderTokenUsage,
  SessionObject,
  SettingsView,
  TokenTelemetry,
  ToolCall,
  View,
} from "./types";
import {
  DEFAULT_VIEW,
  MODEL_PROVIDERS,
  modelRating,
  providerMeta,
  type ModelOption,
  type ModelRating,
} from "./constants";
import {
  applyObjectUpdates,
  cloneSessionObjects,
  getObjectReferenceHints,
  mergeSessionObjects,
  sourceDisplayLabel,
} from "./sessionObjects";
import { buildHistoryPayload, shouldAutoSyncObjectTable } from "./messages";
import { compactToolArgs, planSummary } from "./formatting";

type GlassBorderStyle = "pixel" | "glow";

interface GlassSettings {
  transparency: number;
  blur: number;
  border: GlassBorderStyle;
}

const GLASS_STORAGE_KEY = "cadegg.glassSettings.v1";
const APP_PREFS_STORAGE_KEY = "cadegg.appPreferences.v1";
const CHAT_SESSIONS_STORAGE_KEY = "cadegg.chatSessions.v1";
const DEFAULT_GLASS_SETTINGS: GlassSettings = {
  transparency: 45,
  blur: 70,
  border: "pixel",
};

interface AppPreferences {
  language: "zh-CN" | "en-US";
  fontSize: number;
  storageLocation: "appdata" | "project";
  autoExportSessionMarkdown: boolean;
  notifications: boolean;
  autoSyncObjects: boolean;
  alwaysOnTop: boolean;
  reduceMotion: boolean;
  densePanels: boolean;
}

function clampNumber(value: number, min: number, max: number) {
  if (!Number.isFinite(value)) return min;
  return Math.min(max, Math.max(min, Math.round(value)));
}

/// 全量基准清单：模型列表里的全部模型（与 constants.ts 单一事实源保持一致）。
const ALL_BENCHMARK_SPECS = MODEL_PROVIDERS.flatMap((provider) =>
  provider.models.map((model) => ({ provider: provider.id, model: model.id }))
);
/// 全量基准请求上限（≈ 38 模型 × 6 请求 = 228，留余量）。
const BENCHMARK_MAX_REQUESTS = 256;

const DEFAULT_APP_PREFERENCES: AppPreferences = {
  language: "zh-CN",
  fontSize: 14,
  storageLocation: "appdata",
  autoExportSessionMarkdown: true,
  notifications: true,
  autoSyncObjects: true,
  alwaysOnTop: false,
  reduceMotion: false,
  densePanels: false,
};

// Lightweight i18n — native Chinese/English, not translations.
const UI: Record<string, Record<"zh-CN" | "en-US", string>> = {
  helpButton: { "zh-CN": "帮助", "en-US": "Help" },
  settingsTitle: { "zh-CN": "总设置", "en-US": "Settings" },
  welcomeTitle: { "zh-CN": "CADEgg", "en-US": "CADEgg" },
  welcomeSubtitle: {
    "zh-CN": "选择模型后，直接描述你要在 AutoCAD 里完成的操作。",
    "en-US": "Select a model, then describe what you want to do in AutoCAD.",
  },
  keyWarning: {
    "zh-CN": "需要配置 {provider} API Key",
    "en-US": "API Key required for {provider}",
  },
  keyWarningHint: {
    "zh-CN": "点这里打开设置",
    "en-US": "Click to open settings",
  },
  composerPlaceholder: {
    "zh-CN": "描述你要绘制、修改或查询的 CAD 操作...",
    "en-US": "Describe the CAD operation you want to draw, modify, or query...",
  },
  composerPlaceholderWaiting: {
    "zh-CN": "等待回复...",
    "en-US": "Waiting for reply...",
  },
  composerHint: {
    "zh-CN": "Enter 发送 · Shift+Enter 换行",
    "en-US": "Enter to send · Shift+Enter for new line",
  },
  byokConfigured: { "zh-CN": "BYOK 已配置", "en-US": "BYOK Ready" },
  byokFillKey: { "zh-CN": "填写 Key", "en-US": "Set Key" },
  autoFailover: { "zh-CN": "自动轮转", "en-US": "Auto Failover" },
  autoFailoverOn: { "zh-CN": "轮转开", "en-US": "Failover On" },
  autoFailoverOff: { "zh-CN": "轮转关", "en-US": "Failover Off" },
  autoFailoverHint: {
    "zh-CN": "关闭后只使用当前会话选择的模型。",
    "en-US": "When off, only the selected session model is used.",
  },
  languageHint: {
    "zh-CN": "界面语言切换，覆盖主界面、设置页、帮助文档和主要操作入口。",
    "en-US": "Switches the main UI, settings, help docs, and primary controls.",
  },
  quickPrompt1: {
    "zh-CN": "画电梯井口防护，井口宽 2000，高 1800",
    "en-US": "Draw elevator shaft protection, opening width 2000, height 1800",
  },
  quickPrompt2: {
    "zh-CN": "画一个双跑楼梯，宽 1200，每跑 10 步",
    "en-US": "Draw a double-flight stair, width 1200, 10 steps per flight",
  },
  quickPrompt3: {
    "zh-CN": "画一个矩形，中心在原点，宽 3000，高 2000",
    "en-US": "Draw a rectangle centered at origin, width 3000, height 2000",
  },
  thinkingLabel: {
    "zh-CN": "正在分析...",
    "en-US": "Analyzing...",
  },
  thinkingProcessTitle: { "zh-CN": "思考与生成", "en-US": "Thinking & Generation" },
  thinkingProcessHint: {
    "zh-CN": "运行期 token 保留在固定小框里，最终回复完成后整体显示。",
    "en-US": "Live tokens stay in this fixed trace box; the final answer appears when complete.",
  },
  waitingForModel: { "zh-CN": "等待模型响应", "en-US": "Waiting for model" },
  bridgeOnline: { "zh-CN": "BRIDGE 在线", "en-US": "BRIDGE Online" },
  bridgeError: { "zh-CN": "BRIDGE 异常", "en-US": "BRIDGE Error" },
  bridgeIdle: { "zh-CN": "BRIDGE 待测", "en-US": "BRIDGE Idle" },
  bridgeChecking: { "zh-CN": "BRIDGE 检测中", "en-US": "BRIDGE Checking" },
  sessionTitle: { "zh-CN": "新会话", "en-US": "New Session" },
  saveAppModel: { "zh-CN": "保存应用与模型", "en-US": "Save App & Model" },
  actionCardTitle: { "zh-CN": "生成操作", "en-US": "Generation" },
  actionCardDesc: {
    "zh-CN": "上一轮出图或导入对象会进入撤回栈。",
    "en-US": "The latest generated or imported objects can be undone.",
  },
  undoLast: { "zh-CN": "撤回上一次生成", "en-US": "Undo Last Generation" },
  undoing: { "zh-CN": "撤回中...", "en-US": "Undoing..." },
  drawResultTitle: { "zh-CN": "本次出图", "en-US": "Drawing Result" },
  drawResultPending: { "zh-CN": "等待 AutoCAD 生成结果", "en-US": "Waiting for AutoCAD output" },
  openingWidth: { "zh-CN": "井口宽度", "en-US": "Opening Width" },
  openingHeight: { "zh-CN": "井口高度", "en-US": "Opening Height" },
  guardHeight: { "zh-CN": "防护门高", "en-US": "Guard Height" },
  doorBottomGap: { "zh-CN": "门底间隙", "en-US": "Door Gap" },
  toeBoard: { "zh-CN": "踢脚板", "en-US": "Toe Board" },
  warningSign: { "zh-CN": "警示牌", "en-US": "Warning Sign" },
  materialTable: { "zh-CN": "材料表", "en-US": "Material Table" },
  included: { "zh-CN": "已配", "en-US": "Included" },
  notIncluded: { "zh-CN": "未配", "en-US": "Not Included" },
  validationTitle: { "zh-CN": "安全校核", "en-US": "Safety Check" },
  validationPending: {
    "zh-CN": "校核结果会在工具执行后出现",
    "en-US": "Validation results appear after tool execution",
  },
  validationPassed: { "zh-CN": "安全校核通过", "en-US": "Safety Check Passed" },
  validationPassedWithWarnings: {
    "zh-CN": "强制项通过，存在建议项提醒",
    "en-US": "Mandatory Checks Passed, Recommendations Pending",
  },
  validationFailed: { "zh-CN": "安全校核未通过", "en-US": "Safety Check Failed" },
  riskItems: { "zh-CN": "风险项：{items}", "en-US": "Risks: {items}" },
  warningItems: { "zh-CN": "建议项：{items}", "en-US": "Recommendations: {items}" },
  materialSummary: {
    "zh-CN": "材料表：防护门 {guardDoor} · 踢脚板 {toeBoard}mm · 警示牌 {warningSign}",
    "en-US": "Materials: guard door {guardDoor} · toe board {toeBoard}mm · warning sign {warningSign}",
  },
  appSectionTitle: { "zh-CN": "应用", "en-US": "Application" },
  appSectionDesc: {
    "zh-CN": "这些选项只影响 CADEgg 前端体验，不会改动 AutoCAD 图形。",
    "en-US": "These options only affect the CADEgg UI, not AutoCAD drawings.",
  },
  languageLabel: { "zh-CN": "界面语言", "en-US": "UI Language" },
  fontSizeLabel: { "zh-CN": "字体大小", "en-US": "Font Size" },
  fontSizeHint: {
    "zh-CN": "拖动时即时预览主界面、会话、右栏和设置文字；保存后固化偏好。",
    "en-US": "Drag to preview the app, chat, right rail, and settings text; save to keep it.",
  },
  storageLabel: { "zh-CN": "存储位置", "en-US": "Storage Location" },
  storageHint: {
    "zh-CN": "模型 Key 始终保存在系统 AppData；自动会话记忆包会按这里选择的位置保存。",
    "en-US": "Model keys always stay in system AppData; automatic session memory bundles use this location.",
  },
  storageAppData: { "zh-CN": "系统 AppData（推荐）", "en-US": "System AppData (Recommended)" },
  storageProject: {
    "zh-CN": "项目目录（cadegg-sessions）",
    "en-US": "Project directory (cadegg-sessions)",
  },
  autoExportSessionLabel: {
    "zh-CN": "任务完成后自动保存会话记忆包",
    "en-US": "Auto-save session memory bundle after tasks",
  },
  notificationsLabel: { "zh-CN": "通知", "en-US": "Notifications" },
  autoSyncLabel: { "zh-CN": "对象自动同步", "en-US": "Auto Sync Objects" },
  alwaysOnTopLabel: { "zh-CN": "窗口置顶", "en-US": "Always on Top" },
  reduceMotionLabel: { "zh-CN": "减少动画", "en-US": "Reduce Motion" },
  densePanelsLabel: { "zh-CN": "紧凑右栏", "en-US": "Dense Right Rail" },
  glassSectionTitle: { "zh-CN": "外观玻璃", "en-US": "Glass Appearance" },
  glassSectionDesc: {
    "zh-CN": "拖动会即时应用并保存；透明度控制透出量，粗糙度控制磨砂感和透后清晰度。",
    "en-US": "Drag changes apply immediately; transparency controls see-through, roughness controls frost.",
  },
  transparencyLabel: { "zh-CN": "透明度", "en-US": "Transparency" },
  roughnessLabel: { "zh-CN": "粗糙度", "en-US": "Roughness" },
  borderStyleLabel: { "zh-CN": "边框样式", "en-US": "Border Style" },
  pixelBorder: { "zh-CN": "像素墨线", "en-US": "Pixel Ink" },
  glowBorder: { "zh-CN": "柔和发光", "en-US": "Soft Glow" },
  resetGlass: { "zh-CN": "重置玻璃参数", "en-US": "Reset Glass" },
  recoverWindow: { "zh-CN": "恢复窗口位置", "en-US": "Recover Window" },
  windowRecovered: { "zh-CN": "窗口已重新居中", "en-US": "Window recentered" },
  modelSectionTitle: { "zh-CN": "模型密钥与轮转", "en-US": "Model Keys & Failover" },
  modelSectionDesc: {
    "zh-CN": "会话里选择当前模型；这里管理各供应商 BYOK、Base URL 和自动轮转候选。",
    "en-US": "Select the current model in chat; manage BYOK, Base URL, and failover candidates here.",
  },
  modelCardDesc: {
    "zh-CN": "轻量模型用于普通问答；强模型用于规划、出图、校核。请求失败时后端会自动切换。",
    "en-US": "Cheap models handle Q&A; strong models handle planning, drawing, and validation.",
  },
  modelCostNote: {
    "zh-CN": "模型列表只显式标注免费模型；未标注免费的一律按会消耗 Token/额度处理。",
    "en-US": "Only free models are explicitly marked; unmarked models should be treated as token/credit consuming.",
  },
  failoverPoolTitle: { "zh-CN": "当前轮转池", "en-US": "Current Failover Pool" },
  failoverPoolAvailable: {
    "zh-CN": "可用供应商：{providers}",
    "en-US": "Available providers: {providers}",
  },
  failoverPoolEmpty: {
    "zh-CN": "暂无可用供应商：至少配置一个 API Key 后，轮转池才会生效。",
    "en-US": "No providers are available: configure at least one API key to enable failover.",
  },
  providerConfigured: { "zh-CN": "已加入轮转池", "en-US": "In failover pool" },
  providerMissingKey: { "zh-CN": "未配置 Key，轮转时会跳过", "en-US": "No key configured; skipped by failover" },
  baseUrlLabel: { "zh-CN": "API Base URL", "en-US": "API Base URL" },
  baseUrlHint: {
    "zh-CN": "兼容 OpenAI /chat/completions 的官方或中转地址。",
    "en-US": "Official or relay endpoint compatible with OpenAI /chat/completions.",
  },
  cheapModelLabel: { "zh-CN": "轻量模型", "en-US": "Cheap Model" },
  strongModelLabel: { "zh-CN": "强模型", "en-US": "Strong Model" },
  settingsKeyNote: {
    "zh-CN": "API Key 仅保存在本机 AppData/settings.json，界面不会明文回显。请勿分享或上传 settings.json，泄露等于泄露密钥。",
    "en-US": "API keys are stored only in local AppData/settings.json and are never shown in full. Never share or upload settings.json — it contains your secrets.",
  },
  developerToolsTitle: { "zh-CN": "开发者工具", "en-US": "Developer Tools" },
  developerToolsDesc: {
    "zh-CN": "这些入口用于本机 AutoCAD 自动化排查，默认不放在右侧用户工作栏。",
    "en-US": "These controls are for local AutoCAD automation diagnostics.",
  },
  cadDebugTitle: { "zh-CN": "CAD 调试", "en-US": "CAD Diagnostics" },
  cadDebugDesc: {
    "zh-CN": "连接测试和画线测试只用于验证本机 AutoCAD 自动化链路。",
    "en-US": "Connection and line tests verify the local AutoCAD automation path.",
  },
  connectionTest: { "zh-CN": "连接测试", "en-US": "Connection Test" },
  drawLineTest: { "zh-CN": "画线测试", "en-US": "Draw Line Test" },
  sessionObjectsTitle: { "zh-CN": "会话对象 · {count} 个可引用", "en-US": "Session Objects · {count}" },
  importSelected: { "zh-CN": "导入选中", "en-US": "Import Selection" },
  importing: { "zh-CN": "导入中", "en-US": "Importing" },
  syncObjects: { "zh-CN": "同步", "en-US": "Sync" },
  syncing: { "zh-CN": "同步中", "en-US": "Syncing" },
  noSessionObjects: {
    "zh-CN": "在 AutoCAD 中选中对象后可导入当前会话。",
    "en-US": "Select objects in AutoCAD, then import them into this session.",
  },
  newConversation: { "zh-CN": "新建会话", "en-US": "New Chat" },
  conversations: { "zh-CN": "会话", "en-US": "Chats" },
  defaultModel: { "zh-CN": "默认模型", "en-US": "Default Model" },
  deleteSession: { "zh-CN": "删除会话", "en-US": "Delete Chat" },
  exportSession: { "zh-CN": "导出会话", "en-US": "Export Session" },
  exportSessionDone: { "zh-CN": "会话 Markdown 已导出", "en-US": "Session Markdown exported" },
  exportSessionEmpty: { "zh-CN": "当前会话还没有可导出的内容", "en-US": "Current session has nothing to export" },
  exportSessionFailed: { "zh-CN": "会话导出/保存失败：{error}", "en-US": "Session export/save failed: {error}" },
  taskDuration: { "zh-CN": "任务耗时", "en-US": "Task Duration" },
  taskRunning: { "zh-CN": "运行中", "en-US": "Running" },
  taskPaused: { "zh-CN": "已暂停", "en-US": "Paused" },
  noTaskDuration: { "zh-CN": "等待任务", "en-US": "Waiting" },
  firstTokenLatency: { "zh-CN": "首响应", "en-US": "First Response" },
  avgTokenGap: { "zh-CN": "平均片段间隔", "en-US": "Avg Chunk Gap" },
  streamChunks: { "zh-CN": "流式片段", "en-US": "Stream Chunks" },
  inputTokens: { "zh-CN": "输入 Token", "en-US": "Input Tokens" },
  outputTokens: { "zh-CN": "输出 Token", "en-US": "Output Tokens" },
  cacheReadTokens: { "zh-CN": "缓存命中", "en-US": "Cache Read" },
  cacheWriteTokens: { "zh-CN": "缓存写入", "en-US": "Cache Write" },
  reasoningTokens: { "zh-CN": "推理 Token", "en-US": "Reasoning Tokens" },
  providerCalls: { "zh-CN": "模型调用", "en-US": "Provider Calls" },
  estimatedContextTokens: { "zh-CN": "上下文估算", "en-US": "Context Estimate" },
  totalModelDuration: { "zh-CN": "总耗时", "en-US": "Total Duration" },
  outputThroughput: { "zh-CN": "输出速率", "en-US": "Output Rate" },
  modelRouteTitle: { "zh-CN": "模型路由", "en-US": "Model Route" },
  routeSelected: { "zh-CN": "选择", "en-US": "Selected" },
  routeFinal: { "zh-CN": "命中", "en-US": "Final" },
  routeFallbackCount: { "zh-CN": "回退 {count} 次", "en-US": "{count} fallback(s)" },
  routeNoFallback: { "zh-CN": "无回退", "en-US": "No fallback" },
  routeWaiting: { "zh-CN": "等待路由", "en-US": "Waiting route" },
  routeNone: { "zh-CN": "尚无路由", "en-US": "No route yet" },
  routeProcessing: { "zh-CN": "处理中", "en-US": "Processing" },
  memoryTitle: { "zh-CN": "会话记忆", "en-US": "Session Memory" },
  memoryLoading: { "zh-CN": "读取中…", "en-US": "Loading…" },
  memoryEmpty: {
    "zh-CN": "还没有记忆包。完成一次任务后会自动生成（取决于「自动导出」开关）。",
    "en-US": "No memory bundle yet. It is created after a task completes (when auto-export is on).",
  },
  memoryFileCount: { "zh-CN": "共 {count} 个文件", "en-US": "{count} files" },
  memoryOpenDir: { "zh-CN": "打开目录", "en-US": "Open Folder" },
  memoryRefresh: { "zh-CN": "刷新", "en-US": "Refresh" },
  memoryDirFailed: { "zh-CN": "打开目录失败：{error}", "en-US": "Failed to open folder: {error}" },
  memoryGlobalTitle: { "zh-CN": "全局记忆", "en-US": "Global Memory" },
  memoryGlobalMissing: {
    "zh-CN": "还没有 global-memory.md。完成一次任务后会自动创建，可在目录里手动编辑。",
    "en-US": "No global-memory.md yet. It is created after a task; you can edit it manually in the folder.",
  },
  memoryCarryLabel: { "zh-CN": "本次发送携带全局记忆", "en-US": "Carry global memory with next send" },
  memoryCarryHint: {
    "zh-CN": "默认关闭，不自动注入；开启后仅下一条消息携带，发送后自动复位。",
    "en-US": "Off by default; when on, only the next message carries it, then it resets.",
  },
  memoryTokensFull: { "zh-CN": "全文约 {tokens} tokens", "en-US": "Full text ≈ {tokens} tokens" },
  memoryBudget: { "zh-CN": "携带预算 {budget} tokens，超出截断", "en-US": "Carry budget {budget} tokens, truncated if exceeded" },
  memoryBudgetSetting: { "zh-CN": "记忆携带预算（tokens）", "en-US": "Memory carry budget (tokens)" },
  memoryBudgetSettingHint: { "zh-CN": "范围 200–8000，超出部分截断", "en-US": "Range 200–8000, truncated beyond" },
  memoryPreviewShow: { "zh-CN": "展开预览", "en-US": "Expand" },
  memoryPreviewHide: { "zh-CN": "收起预览", "en-US": "Collapse" },
  memoryCarriedBadge: { "zh-CN": "将携带全局记忆 · 约 {tokens} tokens", "en-US": "Carrying global memory · ≈{tokens} tokens" },
  memoryTruncated: { "zh-CN": "已截断", "en-US": "truncated" },
  benchmarkTitle: { "zh-CN": "模型基准", "en-US": "Model Benchmark" },
  benchmarkRunnable: { "zh-CN": "可测 {count} 个模型", "en-US": "{count} runnable models" },
  benchmarkEstimate: {
    "zh-CN": "每模型 6 次小请求，预计 ≤ {count} 次（硬上限 64，可随时取消）",
    "en-US": "6 small requests per model, est. ≤ {count} (hard cap 64, cancellable)",
  },
  benchmarkSkipped: { "zh-CN": "跳过：{text}", "en-US": "Skipped: {text}" },
  benchmarkStart: { "zh-CN": "开始基准", "en-US": "Run Benchmark" },
  benchmarkCancel: { "zh-CN": "取消", "en-US": "Cancel" },
  benchmarkSaved: { "zh-CN": "结果已保存到记忆目录 benchmark-results.md", "en-US": "Saved to benchmark-results.md in the memory folder" },
  benchmarkOpenDir: { "zh-CN": "打开结果目录", "en-US": "Open Results Folder" },
  benchmarkFailed: { "zh-CN": "基准失败：{error}", "en-US": "Benchmark failed: {error}" },
  benchmarkLastRun: { "zh-CN": "最近测试：{date}", "en-US": "Last run: {date}" },
  benchmarkNever: { "zh-CN": "还没有基准结果", "en-US": "No benchmark results yet" },
  benchmarkWeights: {
    "zh-CN": "权重：工具 25% · 准确性 25% · 稳定性 15% · 速度 15% · 成本 10% · 长上下文 10%",
    "en-US": "Weights: tools 25% · accuracy 25% · stability 15% · speed 15% · cost 10% · long context 10%",
  },
  benchmarkCancelledNote: { "zh-CN": "已取消（部分结果保留）", "en-US": "Cancelled (partial results kept)" },
  benchmarkScopeLabel: { "zh-CN": "测试范围", "en-US": "Scope" },
  benchmarkScopeConfigured: { "zh-CN": "配置模型", "en-US": "Configured" },
  benchmarkScopeAll: { "zh-CN": "列表全部模型", "en-US": "All Listed" },
  benchmarkScopeAllHint: {
    "zh-CN": "将测试列表全部 {count} 个模型（约 {requests} 次请求），预计 30-40 分钟；Kimi 每请求间隔 21 秒，请耐心等待。",
    "en-US": "Tests all {count} listed models (≈{requests} requests), est. 30-40 min; Kimi waits 21s per request.",
  },
  benchmarkScopeFailed: { "zh-CN": "重跑失败模型", "en-US": "Retry Failed" },
  benchmarkScopeFailedHint: {
    "zh-CN": "重测上次 {count} 个未完全成功的模型（约 {requests} 次请求，含限流间隔）。",
    "en-US": "Re-tests {count} models that did not fully succeed (≈{requests} requests, incl. pacing).",
  },
  benchmarkNoFailed: { "zh-CN": "上次没有失败的模型", "en-US": "No failed models in the last run" },
  settingsNav: { "zh-CN": "设置", "en-US": "Settings" },
  minimizeWindow: { "zh-CN": "最小化", "en-US": "Minimize" },
  maximizeWindow: { "zh-CN": "最大化", "en-US": "Maximize" },
  closeWindow: { "zh-CN": "关闭", "en-US": "Close" },
  send: { "zh-CN": "发送", "en-US": "Send" },
  providerSelectAria: { "zh-CN": "模型供应商", "en-US": "Model Provider" },
  sessionModelAria: { "zh-CN": "当前会话模型", "en-US": "Current Session Model" },
  executionPlan: { "zh-CN": "执行计划", "en-US": "Execution Plan" },
  stepCount: { "zh-CN": "{count} 步", "en-US": "{count} steps" },
  confirmExecute: { "zh-CN": "确认执行", "en-US": "Confirm" },
  confirmedHint: { "zh-CN": "已确认，结果见下方。", "en-US": "Confirmed. Result appears below." },
  keyWillOverwrite: { "zh-CN": "正在修改，保存后覆盖原 key", "en-US": "Editing; saving will overwrite the key" },
  keySavedHint: {
    "zh-CN": "已保存。点右侧按钮修改，原 key 不会被读回前端。",
    "en-US": "Saved. Use the button to replace it; the original key is never read back.",
  },
  keyEmptyHint: {
    "zh-CN": "本地保存到 AppData，不上传任何服务器。",
    "en-US": "Stored locally in AppData and never uploaded.",
  },
  cancel: { "zh-CN": "取消", "en-US": "Cancel" },
  keyNotSet: { "zh-CN": "（未设置）", "en-US": "(Not Set)" },
  modify: { "zh-CN": "修改", "en-US": "Modify" },
  set: { "zh-CN": "设置", "en-US": "Set" },
  saved: { "zh-CN": "已保存", "en-US": "Saved" },
  back: { "zh-CN": "返回", "en-US": "Back" },
};

function t(key: string, lang: "zh-CN" | "en-US", vars?: Record<string, string>): string {
  let text = UI[key]?.[lang] ?? UI[key]?.["zh-CN"] ?? key;
  if (vars) {
    for (const [k, v] of Object.entries(vars)) {
      text = text.replace(`{${k}}`, v);
    }
  }
  return text;
}

const PROVIDER_UI: Record<Provider, Record<"zh-CN" | "en-US", { label: string; short: string }>> = {
  glm: {
    "zh-CN": { label: "智谱 GLM", short: "GLM" },
    "en-US": { label: "Zhipu GLM", short: "GLM" },
  },
  deepseek: {
    "zh-CN": { label: "DeepSeek", short: "DeepSeek" },
    "en-US": { label: "DeepSeek", short: "DeepSeek" },
  },
  qwen: {
    "zh-CN": { label: "通义千问", short: "Qwen" },
    "en-US": { label: "Qwen", short: "Qwen" },
  },
  kimi: {
    "zh-CN": { label: "Kimi", short: "Kimi" },
    "en-US": { label: "Kimi", short: "Kimi" },
  },
};

const MODEL_UI: Record<string, Record<"zh-CN" | "en-US", string>> = {
  "glm-5.2": { "zh-CN": "GLM-5.2", "en-US": "GLM-5.2" },
  "glm-5.1": { "zh-CN": "GLM-5.1", "en-US": "GLM-5.1" },
  "glm-5": { "zh-CN": "GLM-5", "en-US": "GLM-5" },
  "glm-5-turbo": { "zh-CN": "GLM-5-Turbo", "en-US": "GLM-5-Turbo" },
  "glm-4.7": { "zh-CN": "GLM-4.7", "en-US": "GLM-4.7" },
  "glm-4.7-flashx": { "zh-CN": "GLM-4.7-FlashX", "en-US": "GLM-4.7-FlashX" },
  "glm-4.6": { "zh-CN": "GLM-4.6", "en-US": "GLM-4.6" },
  "glm-4.5": { "zh-CN": "GLM-4.5", "en-US": "GLM-4.5" },
  "glm-4.5-air": { "zh-CN": "GLM-4.5-Air", "en-US": "GLM-4.5-Air" },
  "glm-4.5-airx": { "zh-CN": "GLM-4.5-AirX", "en-US": "GLM-4.5-AirX" },
  "glm-4.5-flash": { "zh-CN": "GLM-4.5-Flash（免费）", "en-US": "GLM-4.5-Flash (Free)" },
  "glm-4-flash-250414": { "zh-CN": "GLM-4-Flash-250414（免费）", "en-US": "GLM-4-Flash-250414 (Free)" },
  "glm-4-flashx-250414": { "zh-CN": "GLM-4-FlashX-250414", "en-US": "GLM-4-FlashX-250414" },
  "deepseek-v4-pro": { "zh-CN": "DeepSeek V4 Pro", "en-US": "DeepSeek V4 Pro" },
  "deepseek-v4-flash": { "zh-CN": "DeepSeek V4 Flash", "en-US": "DeepSeek V4 Flash" },
  "qwen3.8-max": { "zh-CN": "通义千问 3.8 Max", "en-US": "Qwen 3.8 Max" },
  "qwen3.7-max": { "zh-CN": "通义千问 3.7 Max", "en-US": "Qwen 3.7 Max" },
  "qwen3.7-plus": { "zh-CN": "通义千问 3.7 Plus", "en-US": "Qwen 3.7 Plus" },
  "qwen3.7-flash": { "zh-CN": "通义千问 3.7 Flash", "en-US": "Qwen 3.7 Flash" },
  "qwen3.6-plus": { "zh-CN": "通义千问 3.6 Plus", "en-US": "Qwen 3.6 Plus" },
  "qwen3.6-flash": { "zh-CN": "通义千问 3.6 Flash", "en-US": "Qwen 3.6 Flash" },
  "qwen3.5-plus": { "zh-CN": "通义千问 3.5 Plus", "en-US": "Qwen 3.5 Plus" },
  "qwen3.5-flash": { "zh-CN": "通义千问 3.5 Flash", "en-US": "Qwen 3.5 Flash" },
  "qwen3-coder-plus": { "zh-CN": "通义千问 Coder Plus", "en-US": "Qwen Coder Plus" },
  "qwen3-coder-flash": { "zh-CN": "通义千问 Coder Flash", "en-US": "Qwen Coder Flash" },
  "qwen3-max": { "zh-CN": "通义千问 3 Max", "en-US": "Qwen 3 Max" },
  "kimi-k3": { "zh-CN": "Kimi K3", "en-US": "Kimi K3" },
  "kimi-k2.7-code": { "zh-CN": "Kimi K2.7 Code", "en-US": "Kimi K2.7 Code" },
  "kimi-k2.6": { "zh-CN": "Kimi K2.6", "en-US": "Kimi K2.6" },
  "kimi-k2.5": { "zh-CN": "Kimi K2.5", "en-US": "Kimi K2.5" },
};

function providerDisplay(provider: Provider, lang: "zh-CN" | "en-US", kind: "label" | "short") {
  return PROVIDER_UI[provider]?.[lang]?.[kind] ?? providerMeta(provider)[kind === "short" ? "shortLabel" : "label"];
}

function modelDisplay(model: ModelOption, lang: "zh-CN" | "en-US") {
  return MODEL_UI[model.id]?.[lang] ?? model.label;
}

function modelTierDisplay(model: ModelOption, lang: "zh-CN" | "en-US") {
  if (model.tier === "production") return lang === "zh-CN" ? "强模型" : "Strong";
  if (model.tier === "limited") return lang === "zh-CN" ? "轻量/兼容" : "Light/Compat";
  return lang === "zh-CN" ? "旧版/停用" : "Legacy";
}

function ratingStarStates(rating: ModelRating) {
  return Array.from({ length: 5 }, (_, index) => {
    const value = index + 1;
    if (rating >= value) return "full";
    if (rating >= value - 0.5) return "half";
    return "empty";
  });
}

function modelRatingCircleStates(rating: ModelRating) {
  return ratingStarStates(rating);
}

function modelRatingById(provider: Provider, modelId: string): ModelRating | null {
  const model = providerMeta(provider).models.find((item) => item.id === modelId);
  return model ? modelRating(model) : null;
}

interface ChatSession {
  id: string;
  title: string;
  createdAt: number;
  updatedAt: number;
  provider: Provider;
  model: string;
  messages: Message[];
  sessionObjects: SessionObject[];
  demoLog: DemoLogEntry[];
  lastValidation: ElevatorValidation | null;
  lastDrawParams: Record<string, unknown> | null;
  lastTaskDurationMs: number | null;
  lastTokenTelemetry: TokenTelemetry | null;
}

interface StoredChatSessions {
  activeSessionId: string;
  sessions: ChatSession[];
}

interface SessionMemorySaveResult {
  latestMarkdownPath: string;
  summaryMarkdownPath: string;
  eventsPath: string;
  indexPath: string;
  globalMemoryPath: string;
}

function clamp(value: number, min: number, max: number) {
  if (Number.isNaN(value)) return min;
  return Math.min(max, Math.max(min, value));
}

function makeId(prefix: string) {
  return `${prefix}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

function normalizeProvider(value: unknown): Provider {
  return MODEL_PROVIDERS.some((provider) => provider.id === value) ? (value as Provider) : "glm";
}

function normalizeSettingsView(value: Partial<SettingsView>): SettingsView {
  const next = {
    ...DEFAULT_VIEW,
    ...value,
    provider: normalizeProvider(value.provider),
  };
  for (const provider of MODEL_PROVIDERS) {
    const cheapFallback = String(DEFAULT_VIEW[provider.cheapModelField] ?? provider.models[0]?.id ?? "");
    const strongFallback = String(DEFAULT_VIEW[provider.strongModelField] ?? provider.models[0]?.id ?? "");
    (next as Record<string, unknown>)[provider.cheapModelField] = normalizeModelForProvider(
      provider.id,
      next[provider.cheapModelField],
      cheapFallback
    );
    (next as Record<string, unknown>)[provider.strongModelField] = normalizeModelForProvider(
      provider.id,
      next[provider.strongModelField],
      strongFallback
    );
  }
  return next;
}

function normalizeModelForProvider(provider: Provider, value: unknown, fallback?: unknown) {
  const meta = providerMeta(provider);
  const candidate = typeof value === "string" ? value.trim() : "";
  if (candidate && meta.models.some((item) => item.id === candidate)) return candidate;
  const fallbackCandidate = typeof fallback === "string" ? fallback.trim() : "";
  if (fallbackCandidate && meta.models.some((item) => item.id === fallbackCandidate)) {
    return fallbackCandidate;
  }
  return meta.models[0]?.id ?? candidate;
}

function currentModelFor(settings: SettingsView, provider: Provider = settings.provider) {
  const meta = providerMeta(provider);
  return normalizeModelForProvider(
    provider,
    settings[meta.strongModelField],
    DEFAULT_VIEW[meta.strongModelField]
  );
}

const DEFAULT_SESSION_TITLE = "新会话";

function displaySessionTitle(title: string, language: "zh-CN" | "en-US") {
  const normalized = title.trim();
  if (!normalized || normalized === DEFAULT_SESSION_TITLE || normalized === "New Session") {
    return t("sessionTitle", language);
  }
  return title;
}

function providerKeyIsSet(settings: SettingsView, provider: Provider) {
  const meta = providerMeta(provider);
  return Boolean(settings[meta.keySetField]);
}

function failoverPoolText(settings: SettingsView, language: "zh-CN" | "en-US") {
  const providers = MODEL_PROVIDERS
    .filter((provider) => providerKeyIsSet(settings, provider.id))
    .map((provider) => providerDisplay(provider.id, language, "short"));

  if (providers.length === 0) return t("failoverPoolEmpty", language);
  return t("failoverPoolAvailable", language, { providers: providers.join(" → ") });
}

function createChatSession(settings: SettingsView): ChatSession {
  const now = Date.now();
  return {
    id: makeId("session"),
    title: DEFAULT_SESSION_TITLE,
    createdAt: now,
    updatedAt: now,
    provider: settings.provider,
    model: currentModelFor(settings),
    messages: [],
    sessionObjects: [],
    demoLog: [],
    lastValidation: null,
    lastDrawParams: null,
    lastTaskDurationMs: null,
    lastTokenTelemetry: null,
  };
}

function sessionTitleFromMessages(messages: Message[]) {
  const lastUserMessage = [...messages].reverse().find((message) => message.role === "user");
  if (!lastUserMessage || !("content" in lastUserMessage)) return DEFAULT_SESSION_TITLE;
  return lastUserMessage.content.replace(/\s+/g, " ").trim().slice(0, 28) || DEFAULT_SESSION_TITLE;
}

function normalizeChatSession(value: Partial<ChatSession>, fallback: ChatSession): ChatSession {
  const provider = normalizeProvider(value.provider ?? fallback.provider);
  const fallbackModel =
    fallback.provider === provider ? fallback.model : currentModelFor(DEFAULT_VIEW, provider);
  return {
    id: typeof value.id === "string" && value.id ? value.id : fallback.id,
    title: typeof value.title === "string" && value.title ? value.title : fallback.title,
    createdAt: Number(value.createdAt || fallback.createdAt),
    updatedAt: Number(value.updatedAt || fallback.updatedAt),
    provider,
    model: normalizeModelForProvider(provider, value.model, fallbackModel),
    messages: Array.isArray(value.messages) ? (value.messages as Message[]) : [],
    sessionObjects: Array.isArray(value.sessionObjects)
      ? (value.sessionObjects as SessionObject[])
      : [],
    demoLog: Array.isArray(value.demoLog) ? (value.demoLog as DemoLogEntry[]) : [],
    lastValidation: value.lastValidation ?? null,
    lastDrawParams: value.lastDrawParams ?? null,
    lastTaskDurationMs:
      typeof value.lastTaskDurationMs === "number" ? value.lastTaskDurationMs : null,
    lastTokenTelemetry:
      value.lastTokenTelemetry && typeof value.lastTokenTelemetry === "object"
        ? (value.lastTokenTelemetry as TokenTelemetry)
        : null,
  };
}

function formatMarkdownTime(value: number | string | null | undefined) {
  if (value == null || value === "") return "n/a";
  const normalizedValue = typeof value === "string" && /^\d+$/.test(value) ? Number(value) : value;
  if (typeof normalizedValue === "number" && normalizedValue <= 0) return "n/a";
  const date = new Date(normalizedValue);
  if (Number.isNaN(date.getTime())) return String(value);
  return date.toLocaleString();
}

function readRecordField(record: Record<string, unknown>, snakeKey: string, camelKey: string) {
  if (Object.prototype.hasOwnProperty.call(record, snakeKey)) return record[snakeKey];
  if (Object.prototype.hasOwnProperty.call(record, camelKey)) return record[camelKey];
  return undefined;
}

function coerceText(value: unknown, fallback = "") {
  if (typeof value === "string") return value;
  if (typeof value === "number" && Number.isFinite(value)) return String(value);
  return fallback;
}

function coerceNumber(value: unknown, fallback = 0) {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string" && value.trim()) {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return fallback;
}

function normalizeMemoryFileInfo(value: MemoryFileInfo | Record<string, unknown>): MemoryFileInfo {
  const record = value as Record<string, unknown>;
  return {
    name: coerceText(readRecordField(record, "name", "name")),
    size_bytes: coerceNumber(readRecordField(record, "size_bytes", "sizeBytes")),
    updated_at_ms: coerceNumber(readRecordField(record, "updated_at_ms", "updatedAtMs")),
  };
}

function normalizeMemoryBundleInfo(value: MemoryBundleInfo | null | undefined): MemoryBundleInfo | null {
  if (!value) return null;
  const record = value as unknown as Record<string, unknown>;
  const filesValue = readRecordField(record, "files", "files");
  return {
    dir: coerceText(readRecordField(record, "dir", "dir")),
    files: Array.isArray(filesValue)
      ? filesValue.map((file) => normalizeMemoryFileInfo(file as Record<string, unknown>))
      : [],
    global_memory: coerceText(readRecordField(record, "global_memory", "globalMemory")),
    global_memory_exists: Boolean(readRecordField(record, "global_memory_exists", "globalMemoryExists")),
  };
}

function normalizeBenchmarkCandidate(
  value: BenchmarkCandidate | Record<string, unknown>
): BenchmarkCandidate {
  const record = value as Record<string, unknown>;
  const skipReason = coerceText(readRecordField(record, "skip_reason", "skipReason"));
  return {
    provider: coerceText(readRecordField(record, "provider", "provider")),
    provider_label: coerceText(readRecordField(record, "provider_label", "providerLabel")),
    model: coerceText(readRecordField(record, "model", "model")),
    ...(skipReason ? { skip_reason: skipReason } : {}),
  };
}

function normalizeBenchmarkCaseResult(
  value: BenchmarkCaseResult | Record<string, unknown>
): BenchmarkCaseResult {
  const record = value as Record<string, unknown>;
  return {
    id: coerceText(readRecordField(record, "id", "id")),
    label: coerceText(readRecordField(record, "label", "label")),
    score: coerceNumber(readRecordField(record, "score", "score")),
    note: coerceText(readRecordField(record, "note", "note")),
  };
}

function normalizeBenchmarkModelResult(
  value: BenchmarkModelResult | Record<string, unknown>
): BenchmarkModelResult {
  const record = value as Record<string, unknown>;
  const casesValue = readRecordField(record, "cases", "cases");
  const errorsValue = readRecordField(record, "errors", "errors");
  const avgOutputTokens = readRecordField(record, "avg_output_tokens", "avgOutputTokens");
  return {
    provider: coerceText(readRecordField(record, "provider", "provider")),
    provider_label: coerceText(readRecordField(record, "provider_label", "providerLabel")),
    model: coerceText(readRecordField(record, "model", "model")),
    requests: Math.max(0, Math.floor(coerceNumber(readRecordField(record, "requests", "requests")))),
    succeeded: Math.max(0, Math.floor(coerceNumber(readRecordField(record, "succeeded", "succeeded")))),
    avg_duration_ms: Math.max(
      0,
      Math.floor(coerceNumber(readRecordField(record, "avg_duration_ms", "avgDurationMs")))
    ),
    avg_output_tokens:
      avgOutputTokens == null ? undefined : coerceNumber(avgOutputTokens, Number.NaN),
    score: coerceNumber(readRecordField(record, "score", "score")),
    rating: coerceNumber(readRecordField(record, "rating", "rating")),
    cases: Array.isArray(casesValue)
      ? casesValue.map((item) => normalizeBenchmarkCaseResult(item as Record<string, unknown>))
      : [],
    errors: Array.isArray(errorsValue)
      ? errorsValue.map((item) => coerceText(item))
      : [],
  };
}

function normalizeBenchmarkSummary(
  value: BenchmarkSummary | null | undefined
): BenchmarkSummary | null {
  if (!value) return null;
  const record = value as unknown as Record<string, unknown>;
  const modelsValue = readRecordField(record, "models", "models");
  const startedAtMs = coerceNumber(readRecordField(record, "started_at_ms", "startedAtMs"));
  const finishedAtMs = readRecordField(record, "finished_at_ms", "finishedAtMs");
  const resultsJsonPath = coerceText(readRecordField(record, "results_json_path", "resultsJsonPath"));
  const resultsMdPath = coerceText(readRecordField(record, "results_md_path", "resultsMdPath"));
  return {
    started_at_ms: startedAtMs,
    ...(finishedAtMs == null ? {} : { finished_at_ms: coerceNumber(finishedAtMs, startedAtMs) }),
    cancelled: Boolean(readRecordField(record, "cancelled", "cancelled")),
    candidates_total: Math.max(
      0,
      Math.floor(coerceNumber(readRecordField(record, "candidates_total", "candidatesTotal")))
    ),
    models_tested: Math.max(
      0,
      Math.floor(coerceNumber(readRecordField(record, "models_tested", "modelsTested")))
    ),
    max_requests: Math.max(
      0,
      Math.floor(coerceNumber(readRecordField(record, "max_requests", "maxRequests")))
    ),
    models: Array.isArray(modelsValue)
      ? modelsValue.map((item) => normalizeBenchmarkModelResult(item as Record<string, unknown>))
      : [],
    results_json_path: resultsJsonPath,
    results_md_path: resultsMdPath,
  };
}

function formatDuration(ms: number | null | undefined) {
  if (!ms || ms < 0) return "00:00";
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  const tenths = Math.floor((ms % 1000) / 100);
  if (minutes < 60) {
    return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}.${tenths}`;
  }
  const hours = Math.floor(minutes / 60);
  const restMinutes = minutes % 60;
  return `${hours}:${String(restMinutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
}

function formatLatency(ms: number | null | undefined) {
  if (typeof ms !== "number" || !Number.isFinite(ms) || ms < 0) return "n/a";
  if (ms < 1000) return `${Math.round(ms)}ms`;
  return `${(ms / 1000).toFixed(ms < 10_000 ? 2 : 1)}s`;
}

function formatTokenCount(value: number | null | undefined) {
  if (typeof value !== "number" || !Number.isFinite(value)) return "n/a";
  return Math.round(value).toLocaleString();
}

function formatTokensPerSecond(value: number | null | undefined, estimated = false) {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) return "n/a";
  const digits = value < 10 ? 1 : 0;
  return `${estimated ? "~" : ""}${value.toFixed(digits)} tok/s`;
}

/** UI 用 token 显示：区分「未返回」与「本地估算」。 */
function formatTokenCountUi(
  value: number | null | undefined,
  estimated: boolean | undefined,
  lang: "zh-CN" | "en-US"
) {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return lang === "zh-CN" ? "未返回" : "Not returned";
  }
  const text = Math.round(value).toLocaleString();
  if (estimated) {
    return lang === "zh-CN" ? `本地估算 ${text}` : `Est. ${text}`;
  }
  return text;
}

function estimateTextTokens(value: unknown) {
  const text = typeof value === "string" ? value : JSON.stringify(value ?? "");
  if (!text) return 0;
  return Math.ceil(text.length / 4) + 4;
}

function estimateSessionContextTokens(
  messages: Message[],
  objects: SessionObject[],
  demoLog: DemoLogEntry[]
) {
  const messageTokens = messages.reduce((sum, message) => sum + estimateTextTokens(message), 0);
  const objectTokens = objects
    .slice(0, 20)
    .reduce((sum, object) => sum + estimateTextTokens(object), 0);
  const logTokens = demoLog
    .slice(0, 8)
    .reduce((sum, entry) => sum + estimateTextTokens(entry), 0);
  return messageTokens + objectTokens + logTokens;
}

function safeMarkdownCell(value: unknown) {
  return String(value ?? "")
    .replace(/\|/g, "\\|")
    .replace(/\r?\n/g, " ");
}

function fencedJson(value: unknown) {
  return `\`\`\`json\n${JSON.stringify(value ?? null, null, 2)}\n\`\`\``;
}

function markdownFilenamePart(value: string) {
  return value
    .replace(/[<>:"/\\|?*\x00-\x1F]/g, "_")
    .replace(/\s+/g, "_")
    .slice(0, 48) || "session";
}

function markdownMessage(message: Message, index: number) {
  const lines = [`### ${index + 1}. ${message.role}`];
  if (message.role === "user") {
    lines.push("", message.content);
  } else if (message.role === "assistant") {
    if (message.text) lines.push("", message.text);
    if (message.tool_calls.length > 0) {
      lines.push("", "Tool calls:", "");
      for (const call of message.tool_calls) {
        lines.push(`- ${call.name} (${call.id})`, fencedJson(call.args));
      }
    }
  } else if (message.role === "plan") {
    if (message.text) lines.push("", message.text);
    if (message.tool_calls.length > 0) {
      lines.push("", "Planned tool calls:", "");
      for (const call of message.tool_calls) {
        lines.push(`- ${call.name} (${call.id})`, fencedJson(call.args));
      }
    }
  } else {
    lines.push("", `Tool: ${message.name}`, `Status: ${message.ok ? "ok" : "failed"}`);
    if (message.content) lines.push("", message.content);
    if (message.object_updates.length > 0) {
      lines.push("", "Object updates:", fencedJson(message.object_updates));
    }
  }
  return lines.join("\n");
}

function buildSessionMarkdown(session: ChatSession, language: "zh-CN" | "en-US") {
  const title = displaySessionTitle(session.title, language);
  const lines = [
    `# CADEgg Session - ${title}`,
    "",
    "## Metadata",
    "",
    `- Session ID: ${session.id}`,
    `- Created: ${formatMarkdownTime(session.createdAt)}`,
    `- Updated: ${formatMarkdownTime(session.updatedAt)}`,
    `- Provider: ${providerDisplay(session.provider, language, "label")}`,
    `- Model: ${session.model}`,
    `- Message Count: ${session.messages.length}`,
    `- CAD Object Count: ${session.sessionObjects.length}`,
    `- Last Task Duration: ${session.lastTaskDurationMs ? formatDuration(session.lastTaskDurationMs) : "n/a"}`,
    `- First Response Latency: ${formatLatency(session.lastTokenTelemetry?.first_response_ms)}`,
    `- Average Stream Chunk Gap: ${formatLatency(session.lastTokenTelemetry?.avg_chunk_gap_ms)}`,
    `- Stream Chunk Count: ${session.lastTokenTelemetry?.chunk_count ?? "n/a"}`,
    `- Input Tokens: ${formatTokenCount(session.lastTokenTelemetry?.input_tokens)}`,
    `- Output Tokens: ${formatTokenCount(session.lastTokenTelemetry?.output_tokens)}`,
    `- Cache Read Tokens: ${formatTokenCount(session.lastTokenTelemetry?.cache_read_tokens)}`,
    `- Cache Write Tokens: ${formatTokenCount(session.lastTokenTelemetry?.cache_write_tokens)}`,
    `- Reasoning Tokens: ${formatTokenCount(session.lastTokenTelemetry?.reasoning_tokens)}`,
    `- Estimated Context Tokens: ${formatTokenCount(session.lastTokenTelemetry?.estimated_context_tokens)}`,
    "",
  ];

  const firstUserMessage = session.messages.find((message) => message.role === "user");
  if (firstUserMessage?.role === "user") {
    lines.push("## Task", "", firstUserMessage.content, "");
  }

  if (session.lastDrawParams) {
    lines.push("## Last Draw Parameters", "", fencedJson(session.lastDrawParams), "");
  }

  if (session.lastValidation) {
    lines.push(
      "## Last Validation",
      "",
      `- Result: ${session.lastValidation.ok ? "passed" : "failed"}`,
      `- Issues: ${session.lastValidation.issues.length > 0 ? session.lastValidation.issues.join("; ") : "none"}`,
      "",
      "| Check | Passed |",
      "| --- | --- |",
      ...session.lastValidation.checks.map((check) => `| ${safeMarkdownCell(check.label)} | ${check.passed ? "yes" : "no"} |`),
      "",
      "Material table:",
      fencedJson(session.lastValidation.material_table),
      "",
    );
  }

  lines.push("## CAD Objects", "");
  if (session.sessionObjects.length === 0) {
    lines.push("No CAD objects recorded for this session.", "");
  } else {
    lines.push("| Handle | Kind | Label | Source |", "| --- | --- | --- | --- |");
    for (const object of session.sessionObjects) {
      lines.push(
        `| ${safeMarkdownCell(object.handle)} | ${safeMarkdownCell(object.kind)} | ${safeMarkdownCell(object.label)} | ${safeMarkdownCell(object.source ?? "session")} |`
      );
    }
    lines.push("");
  }

  lines.push("## Task Log", "");
  if (session.demoLog.length === 0) {
    lines.push("No completed task log entries recorded.", "");
  } else {
    for (const entry of session.demoLog) {
      lines.push(
        `### ${formatMarkdownTime(entry.time)}`,
        "",
        `- User input: ${entry.user_input}`,
        `- Tool calls: ${entry.tool_calls.join(", ") || "none"}`,
        `- Duration: ${entry.duration_ms ? formatDuration(entry.duration_ms) : "n/a"}`,
        `- First response: ${formatLatency(entry.token_telemetry?.first_response_ms)}`,
        `- Avg chunk gap: ${formatLatency(entry.token_telemetry?.avg_chunk_gap_ms)}`,
        `- Stream chunks: ${entry.token_telemetry?.chunk_count ?? "n/a"}`,
        `- Input tokens: ${formatTokenCount(entry.token_telemetry?.input_tokens)}`,
        `- Output tokens: ${formatTokenCount(entry.token_telemetry?.output_tokens)}`,
        `- Cache read: ${formatTokenCount(entry.token_telemetry?.cache_read_tokens)}`,
        `- Cache write: ${formatTokenCount(entry.token_telemetry?.cache_write_tokens)}`,
        `- Reasoning tokens: ${formatTokenCount(entry.token_telemetry?.reasoning_tokens)}`,
        `- Estimated context tokens: ${formatTokenCount(entry.token_telemetry?.estimated_context_tokens)}`,
        `- Summary: ${entry.summary}`,
        "",
        "Parameters:",
        fencedJson(entry.params),
        "",
      );
      if (entry.validation) {
        lines.push("Validation:", fencedJson(entry.validation), "");
      }
    }
  }

  lines.push("## Conversation", "");
  if (session.messages.length === 0) {
    lines.push("No messages recorded.", "");
  } else {
    lines.push(...session.messages.map((message, index) => markdownMessage(message, index)), "");
  }

  return lines.join("\n").replace(/\n{4,}/g, "\n\n\n");
}

function downloadMarkdown(filename: string, content: string) {
  const blob = new Blob([content], { type: "text/markdown;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 0);
}

function sessionMarkdownFilename(title: string, language: "zh-CN" | "en-US") {
  const displayTitle = displaySessionTitle(title, language);
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  return `CADEgg-${markdownFilenamePart(displayTitle)}-${stamp}.md`;
}

function latestUserInput(session: ChatSession) {
  const entry = session.demoLog[0];
  if (entry?.user_input) return entry.user_input;
  const message = [...session.messages].reverse().find((item) => item.role === "user");
  return message?.role === "user" ? message.content : "";
}

function memoryKeywords(session: ChatSession) {
  const source = [
    session.title,
    ...session.messages.map((message) =>
      message.role === "user"
        ? message.content
        : message.role === "tool"
          ? message.name
          : message.text ?? ""
    ),
    ...session.demoLog.flatMap((entry) => [entry.user_input, entry.summary, ...entry.tool_calls]),
    ...session.sessionObjects.flatMap((object) => [object.kind, object.label, object.source ?? ""]),
  ].join(" ");
  const tags = new Set<string>([
    "CADEgg",
    "AutoCAD",
    providerMeta(session.provider).shortLabel,
    session.model,
  ]);

  const keywordRules: Array<[string, string[]]> = [
    ["电梯", ["电梯井口", "临边防护"]],
    ["井口", ["电梯井口", "井口尺寸"]],
    ["防护", ["施工安全", "防护门"]],
    ["楼梯", ["双跑楼梯", "楼梯出图"]],
    ["矩形", ["矩形", "基础绘图"]],
    ["圆", ["圆", "基础绘图"]],
    ["直线", ["直线", "基础绘图"]],
    ["校核", ["安全校核"]],
    ["材料表", ["材料表"]],
  ];
  for (const [needle, values] of keywordRules) {
    if (source.includes(needle)) values.forEach((value) => tags.add(value));
  }

  for (const object of session.sessionObjects.slice(0, 12)) {
    if (object.kind) tags.add(object.kind);
    if (object.source) tags.add(object.source);
  }
  for (const entry of session.demoLog.slice(0, 8)) {
    entry.tool_calls.forEach((tool) => tags.add(tool));
  }

  return Array.from(tags).filter(Boolean).slice(0, 24);
}

function buildSessionSummaryMarkdown(session: ChatSession, language: "zh-CN" | "en-US") {
  const isZh = language === "zh-CN";
  const title = displaySessionTitle(session.title, language);
  const recentEntries = session.demoLog.slice(0, 8);
  const objects = session.sessionObjects.slice(0, 20);
  const keywords = memoryKeywords(session);
  const lines = [
    `# CADEgg Memory Summary - ${title}`,
    "",
    "## Session",
    "",
    `- Session ID: ${session.id}`,
    `- Updated: ${formatMarkdownTime(session.updatedAt)}`,
    `- Provider: ${providerDisplay(session.provider, language, "label")}`,
    `- Model: ${session.model}`,
    `- Message Count: ${session.messages.length}`,
    `- CAD Object Count: ${session.sessionObjects.length}`,
    `- Last User Input: ${latestUserInput(session) || "n/a"}`,
    `- Last Task Duration: ${session.lastTaskDurationMs ? formatDuration(session.lastTaskDurationMs) : "n/a"}`,
    `- First Response Latency: ${formatLatency(session.lastTokenTelemetry?.first_response_ms)}`,
    `- Average Stream Chunk Gap: ${formatLatency(session.lastTokenTelemetry?.avg_chunk_gap_ms)}`,
    `- Stream Chunk Count: ${session.lastTokenTelemetry?.chunk_count ?? "n/a"}`,
    `- Input Tokens: ${formatTokenCount(session.lastTokenTelemetry?.input_tokens)}`,
    `- Output Tokens: ${formatTokenCount(session.lastTokenTelemetry?.output_tokens)}`,
    `- Cache Read Tokens: ${formatTokenCount(session.lastTokenTelemetry?.cache_read_tokens)}`,
    `- Cache Write Tokens: ${formatTokenCount(session.lastTokenTelemetry?.cache_write_tokens)}`,
    `- Reasoning Tokens: ${formatTokenCount(session.lastTokenTelemetry?.reasoning_tokens)}`,
    `- Estimated Context Tokens: ${formatTokenCount(session.lastTokenTelemetry?.estimated_context_tokens)}`,
    "",
    "## Compact Memory",
    "",
  ];

  if (recentEntries.length === 0) {
    lines.push(isZh ? "- 暂无已完成任务。" : "- No completed task yet.");
  } else {
    for (const entry of recentEntries) {
      lines.push(
        `- ${formatMarkdownTime(entry.time)} | ${entry.user_input}`,
        `  - ${isZh ? "结果" : "Result"}: ${entry.summary}`,
        `  - ${isZh ? "工具" : "Tools"}: ${entry.tool_calls.join(", ") || "none"}`,
        `  - ${isZh ? "耗时" : "Duration"}: ${entry.duration_ms ? formatDuration(entry.duration_ms) : "n/a"}`
      );
      if (entry.token_telemetry) {
        lines.push(
          `  - ${isZh ? "首响应" : "First response"}: ${formatLatency(entry.token_telemetry.first_response_ms)}`,
          `  - ${isZh ? "平均片段间隔" : "Avg chunk gap"}: ${formatLatency(entry.token_telemetry.avg_chunk_gap_ms)}`,
          `  - ${isZh ? "流式片段" : "Stream chunks"}: ${entry.token_telemetry.chunk_count}`,
          `  - ${isZh ? "输入 Token" : "Input tokens"}: ${formatTokenCount(entry.token_telemetry.input_tokens)}`,
          `  - ${isZh ? "输出 Token" : "Output tokens"}: ${formatTokenCount(entry.token_telemetry.output_tokens)}`,
          `  - ${isZh ? "缓存命中" : "Cache read"}: ${formatTokenCount(entry.token_telemetry.cache_read_tokens)}`,
          `  - ${isZh ? "缓存写入" : "Cache write"}: ${formatTokenCount(entry.token_telemetry.cache_write_tokens)}`,
          `  - ${isZh ? "推理 Token" : "Reasoning tokens"}: ${formatTokenCount(entry.token_telemetry.reasoning_tokens)}`,
          `  - ${isZh ? "上下文估算" : "Context estimate"}: ${formatTokenCount(entry.token_telemetry.estimated_context_tokens)}`
        );
      }
      if (entry.validation) {
        lines.push(
          `  - ${isZh ? "校核" : "Validation"}: ${entry.validation.ok ? "passed" : "failed"}${
            entry.validation.issues.length > 0 ? ` (${entry.validation.issues.join("; ")})` : ""
          }`
        );
      }
    }
  }

  lines.push("", "## CAD Object Snapshot", "");
  if (objects.length === 0) {
    lines.push(isZh ? "No CAD objects recorded." : "No CAD objects recorded.");
  } else {
    lines.push("| Handle | Kind | Label | Source |", "| --- | --- | --- | --- |");
    for (const object of objects) {
      lines.push(
        `| ${safeMarkdownCell(object.handle)} | ${safeMarkdownCell(object.kind)} | ${safeMarkdownCell(object.label)} | ${safeMarkdownCell(object.source ?? "session")} |`
      );
    }
    if (session.sessionObjects.length > objects.length) {
      lines.push(`| ... | ... | ${session.sessionObjects.length - objects.length} more objects omitted | ... |`);
    }
  }

  lines.push("", "## Retrieval Keywords", "", keywords.join(", ") || "n/a", "");
  lines.push(
    "## Note",
    "",
    isZh
      ? "这是本地规则摘要，不是模型生成摘要。它用于后续检索候选，不应直接等同于完整会话记忆。"
      : "This is a local rule-based summary, not a model-generated summary. It is a retrieval candidate, not the full session memory.",
    ""
  );

  return lines.join("\n").replace(/\n{4,}/g, "\n\n\n");
}

function buildSessionMemoryEvent(session: ChatSession, eventKind: "task_completed" | "tool_confirmed") {
  const latestEntry = session.demoLog[0];
  return JSON.stringify({
    schemaVersion: 1,
    eventKind,
    sessionId: session.id,
    title: session.title,
    createdAt: session.createdAt,
    updatedAt: session.updatedAt,
    provider: session.provider,
    model: session.model,
    userInput: latestEntry?.user_input || latestUserInput(session) || "",
    summary: latestEntry?.summary || "",
    toolCalls: latestEntry?.tool_calls ?? [],
    durationMs: latestEntry?.duration_ms ?? session.lastTaskDurationMs,
    tokenTelemetry: latestEntry?.token_telemetry ?? session.lastTokenTelemetry,
    validationOk: latestEntry?.validation?.ok ?? session.lastValidation?.ok ?? null,
    cadObjectCount: session.sessionObjects.length,
    cadObjects: session.sessionObjects.slice(0, 20),
    keywords: memoryKeywords(session),
  });
}

function buildSessionMemoryIndexEntry(session: ChatSession) {
  const latestEntry = session.demoLog[0];
  return {
    schemaVersion: 1,
    sessionId: session.id,
    title: displaySessionTitle(session.title, "zh-CN"),
    createdAt: session.createdAt,
    updatedAt: session.updatedAt,
    provider: session.provider,
    model: session.model,
    latestUserInput: latestEntry?.user_input || latestUserInput(session) || "",
    latestSummary: latestEntry?.summary || "",
    toolCalls: latestEntry?.tool_calls ?? [],
    tokenTelemetry: latestEntry?.token_telemetry ?? session.lastTokenTelemetry,
    validationOk: latestEntry?.validation?.ok ?? session.lastValidation?.ok ?? null,
    messageCount: session.messages.length,
    cadObjectCount: session.sessionObjects.length,
    keywords: memoryKeywords(session),
    relativePaths: {
      latest: `sessions/${session.id}/latest.md`,
      summary: `sessions/${session.id}/summary.md`,
      events: `sessions/${session.id}/events.jsonl`,
    },
  };
}

function loadChatSessions(settings: SettingsView): StoredChatSessions {
  const fallback = createChatSession(settings);
  if (typeof window === "undefined") {
    return { activeSessionId: fallback.id, sessions: [fallback] };
  }

  try {
    const raw = window.localStorage.getItem(CHAT_SESSIONS_STORAGE_KEY);
    if (!raw) return { activeSessionId: fallback.id, sessions: [fallback] };
    const parsed = JSON.parse(raw) as Partial<StoredChatSessions>;
    const sessions = Array.isArray(parsed.sessions)
      ? parsed.sessions
          .map((session) => normalizeChatSession(session, fallback))
          .sort((a, b) => b.updatedAt - a.updatedAt)
          .slice(0, 30)
      : [];
    if (sessions.length === 0) {
      return { activeSessionId: fallback.id, sessions: [fallback] };
    }
    const activeSessionId =
      typeof parsed.activeSessionId === "string" &&
      sessions.some((session) => session.id === parsed.activeSessionId)
        ? parsed.activeSessionId
        : sessions[0].id;
    return { activeSessionId, sessions };
  } catch {
    return { activeSessionId: fallback.id, sessions: [fallback] };
  }
}

function normalizeGlassSettings(value: Partial<GlassSettings> & { opacity?: number }): GlassSettings {
  return {
    transparency: clamp(
      Number(value.transparency ?? value.opacity ?? DEFAULT_GLASS_SETTINGS.transparency),
      0,
      90
    ),
    blur: clamp(Number(value.blur ?? DEFAULT_GLASS_SETTINGS.blur), 0, 100),
    border: value.border === "glow" ? "glow" : "pixel",
  };
}

function appFontCssVariables(fontSize: number): CSSProperties {
  const base = clamp(Math.round(fontSize), 11, 22);
  return {
    "--app-font-size": `${base}px`,
    "--content-font-size": `${clamp(base, 11, 22)}px`,
    "--small-font-size": `${clamp(base - 2, 10, 16)}px`,
    "--tiny-font-size": `${clamp(base - 3, 9, 14)}px`,
    "--panel-title-font-size": `${clamp(base + 1, 13, 20)}px`,
    "--modal-title-font-size": `${clamp(base + 10, 20, 30)}px`,
    "--hero-title-font-size": `${clamp(base + 38, 42, 58)}px`,
    "--brand-font-size": `${clamp(base + 8, 20, 28)}px`,
  } as CSSProperties;
}

function glassCssVariables(settings: GlassSettings, fontSize?: number): CSSProperties {
  const next = normalizeGlassSettings(settings);
  const transparency = clamp(next.transparency, 0, 90);
  // 0 = opaque, 1 = fully transparent
  const transparencyRatio = transparency / 90;
  // aggressive curve: at 90% transparency most layers approach 0
  const transparencyCurve = Math.pow(transparencyRatio, 0.85);
  // 0 = clear glass, 1 = heavy frost
  const roughness = clamp(next.blur / 100, 0, 1);
  const roughnessCurve = Math.pow(roughness, 1.4);

  // Alpha layers: each decays at a different rate so controls stay visible
  // even when the background is nearly gone.
  const glassAlpha = clamp(1 - transparencyCurve, 0, 1);
  const glassAlphaSoft = clamp(1 - transparencyCurve * 0.9, 0, 1);
  const glassAlphaMuted = clamp(1 - transparencyCurve * 0.85, 0, 1);
  const controlAlpha = clamp(1 - transparencyCurve * 0.5, 0, 1);

  const blurPx = Math.round(roughnessCurve * 42);
  const grainStep = Math.round(clamp(14 - roughnessCurve * 8, 6, 14));

  // Decorative layers (grid lines, gradient edges) also fade with transparency.
  const gridAlpha = (0.1 * clamp(1 - transparencyCurve * 0.9, 0, 1)).toFixed(3);
  const gridAlphaSubtle = (0.08 * clamp(1 - transparencyCurve * 0.9, 0, 1)).toFixed(3);
  // Gradient edges: fully transparent at 90%.
  const edgeAlpha = clamp(1 - transparencyCurve, 0, 1);
  // Text shadow strengthens as the panel becomes transparent —
  // white text on a transparent dark panel needs a dark halo to stay readable.
  const textShadowStrength = clamp(1 - glassAlpha, 0, 0.75);

  return {
    ...(typeof fontSize === "number" ? appFontCssVariables(fontSize) : {}),
    "--window-bg-alpha": glassAlpha.toFixed(2),
    "--glass-alpha": glassAlpha.toFixed(2),
    "--glass-alpha-soft": glassAlphaSoft.toFixed(2),
    "--glass-alpha-muted": glassAlphaMuted.toFixed(2),
    "--control-alpha": controlAlpha.toFixed(2),
    "--glass-blur": `${blurPx}px`,
    "--glass-roughness": roughness.toFixed(2),
    "--glass-frost-alpha": clamp(roughnessCurve * 0.45, 0, 0.45).toFixed(2),
    "--glass-grain-light": clamp(roughnessCurve * 0.35, 0, 0.35).toFixed(2),
    "--glass-grain-dark": clamp(roughnessCurve * 0.18, 0, 0.18).toFixed(2),
    "--glass-grain-step": `${grainStep}px`,
    "--glass-shine-opacity": clamp(1 - roughnessCurve * 0.55, 0.45, 1).toFixed(2),
    "--ambient-opacity": clamp(1 - transparencyCurve * 0.7, 0.05, 1).toFixed(2),
    "--glass-saturation-strong": `${Math.round(190 - roughnessCurve * 60)}%`,
    "--glass-saturation": `${Math.round(180 - roughnessCurve * 55)}%`,
    "--glass-saturation-soft": `${Math.round(165 - roughnessCurve * 45)}%`,
    "--glass-saturation-muted": `${Math.round(155 - roughnessCurve * 40)}%`,
    "--grid-alpha": gridAlpha,
    "--grid-alpha-subtle": gridAlphaSubtle,
    "--edge-alpha": edgeAlpha.toFixed(2),
    "--text-shadow-strength": textShadowStrength.toFixed(2),
  } as CSSProperties;
}

function saveGlassSettingsNow(settings: GlassSettings) {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(GLASS_STORAGE_KEY, JSON.stringify(normalizeGlassSettings(settings)));
}

function applyAppFontCssVariables(fontSize: number) {
  if (typeof document === "undefined") return;
  const app = document.querySelector<HTMLElement>(".cadegg-app");
  if (!app) return;
  const vars = appFontCssVariables(fontSize);
  Object.entries(vars).forEach(([name, value]) => {
    app.style.setProperty(name, String(value));
  });
}

function applyGlassCssVariables(settings: GlassSettings) {
  if (typeof document === "undefined") return;
  const app = document.querySelector<HTMLElement>(".cadegg-app");
  if (!app) return;
  const vars = glassCssVariables(settings);
  Object.entries(vars).forEach(([name, value]) => {
    app.style.setProperty(name, String(value));
  });
}

function normalizeAppPreferences(value: Partial<AppPreferences>): AppPreferences {
  return {
    language: value.language === "en-US" ? "en-US" : "zh-CN",
    fontSize: clamp(Number(value.fontSize ?? DEFAULT_APP_PREFERENCES.fontSize), 11, 22),
    storageLocation: value.storageLocation === "project" ? "project" : "appdata",
    autoExportSessionMarkdown:
      value.autoExportSessionMarkdown ?? DEFAULT_APP_PREFERENCES.autoExportSessionMarkdown,
    notifications: value.notifications ?? DEFAULT_APP_PREFERENCES.notifications,
    autoSyncObjects: value.autoSyncObjects ?? DEFAULT_APP_PREFERENCES.autoSyncObjects,
    alwaysOnTop: value.alwaysOnTop ?? DEFAULT_APP_PREFERENCES.alwaysOnTop,
    reduceMotion: value.reduceMotion ?? DEFAULT_APP_PREFERENCES.reduceMotion,
    densePanels: value.densePanels ?? DEFAULT_APP_PREFERENCES.densePanels,
  };
}

function loadGlassSettings(): GlassSettings {
  if (typeof window === "undefined") return DEFAULT_GLASS_SETTINGS;

  try {
    const raw = window.localStorage.getItem(GLASS_STORAGE_KEY);
    if (!raw) return DEFAULT_GLASS_SETTINGS;
    const parsed = JSON.parse(raw) as Partial<GlassSettings> & { opacity?: number };

    return normalizeGlassSettings(parsed);
  } catch {
    return DEFAULT_GLASS_SETTINGS;
  }
}

function loadAppPreferences(): AppPreferences {
  if (typeof window === "undefined") return DEFAULT_APP_PREFERENCES;

  try {
    const raw = window.localStorage.getItem(APP_PREFS_STORAGE_KEY);
    if (!raw) return DEFAULT_APP_PREFERENCES;
    const parsed = JSON.parse(raw) as Partial<AppPreferences>;

    return normalizeAppPreferences(parsed);
  } catch {
    return DEFAULT_APP_PREFERENCES;
  }
}

export default function App() {
  const appWindowRef = useRef<ReturnType<typeof getCurrentWindow> | null>(null);
  if (appWindowRef.current === null) {
    appWindowRef.current = getCurrentWindow();
  }
  const appWindow = appWindowRef.current;
  const [view, setView] = useState<View>("chat");
  const [glassSettings, setGlassSettings] = useState<GlassSettings>(() => loadGlassSettings());
  const [appPreferences, setAppPreferences] = useState<AppPreferences>(() => loadAppPreferences());
  const appPreferencesRef = useRef(appPreferences);
  const initialSessionsRef = useRef<StoredChatSessions | null>(null);
  if (initialSessionsRef.current === null) {
    initialSessionsRef.current = loadChatSessions(DEFAULT_VIEW);
  }
  const initialSessions = initialSessionsRef.current;
  const initialSession =
    initialSessions.sessions.find((session) => session.id === initialSessions.activeSessionId) ??
    initialSessions.sessions[0];
  const [settings, setSettings] = useState<SettingsView>({
    ...DEFAULT_VIEW,
  });
  const [sessions, setSessions] = useState<ChatSession[]>(initialSessions.sessions);
  const sessionsRef = useRef<ChatSession[]>(initialSessions.sessions);
  const [activeSessionId, setActiveSessionId] = useState(initialSession.id);
  const activeSessionIdRef = useRef(initialSession.id);

  const [input, setInput] = useState("");
  const [messages, setMessages] = useState<Message[]>(initialSession.messages);
  const messagesRef = useRef<Message[]>(initialSession.messages);
  const [assistantDraft, setAssistantDraft] = useState("");
  const [sessionObjects, setSessionObjects] = useState<SessionObject[]>(
    initialSession.sessionObjects
  );
  const [sending, setSending] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const [savedHint, setSavedHint] = useState(false);
  const [glmKeyDraft, setGlmKeyDraft] = useState<string | null>(null);
  const [deepseekKeyDraft, setDeepseekKeyDraft] = useState<string | null>(null);
  const [qwenKeyDraft, setQwenKeyDraft] = useState<string | null>(null);
  const [kimiKeyDraft, setKimiKeyDraft] = useState<string | null>(null);
  const [settingsFocusProvider, setSettingsFocusProvider] = useState<Provider | null>(null);

  const [testStatus, setTestStatus] = useState<{ ok: boolean; msg: string } | null>(null);
  const [undoing, setUndoing] = useState(false);
  const [syncingObjects, setSyncingObjects] = useState(false);
  const [importingSelection, setImportingSelection] = useState(false);

  const [demoLog, setDemoLog] = useState<DemoLogEntry[]>(initialSession.demoLog);
  const demoLogRef = useRef<DemoLogEntry[]>(initialSession.demoLog);
  const [lastValidation, setLastValidation] = useState<ElevatorValidation | null>(
    initialSession.lastValidation
  );
  const lastValidationRef = useRef<ElevatorValidation | null>(initialSession.lastValidation);
  const [lastDrawParams, setLastDrawParams] = useState<Record<string, unknown> | null>(
    initialSession.lastDrawParams
  );
  const lastDrawParamsRef = useRef<Record<string, unknown> | null>(
    initialSession.lastDrawParams
  );
  const [taskStartedAt, setTaskStartedAt] = useState<number | null>(null);
  const [taskElapsedMs, setTaskElapsedMs] = useState(0);
  const [lastTaskDurationMs, setLastTaskDurationMs] = useState<number | null>(
    initialSession.lastTaskDurationMs
  );
  const lastTaskDurationMsRef = useRef<number | null>(initialSession.lastTaskDurationMs);
  const [lastTokenTelemetry, setLastTokenTelemetry] = useState<TokenTelemetry | null>(
    initialSession.lastTokenTelemetry
  );
  const lastTokenTelemetryRef = useRef<TokenTelemetry | null>(initialSession.lastTokenTelemetry);

  const scrollRef = useRef<HTMLDivElement | null>(null);
  const thinkingTraceRef = useRef<HTMLDivElement | null>(null);
  const assistantDraftRef = useRef("");
  const sessionObjectsRef = useRef<SessionObject[]>(initialSession.sessionObjects);
  const pendingUndoSnapshotRef = useRef<SessionObject[] | null>(null);
  const undoSnapshotsRef = useRef<SessionObject[][]>([]);
  const runTouchedObjectTableRef = useRef(false);
  const pendingPostRunSyncRef = useRef(false);
  const pendingToolCallsRef = useRef<Record<string, ToolCall>>({});
  const completedToolIdsRef = useRef<Set<string>>(new Set());
  const preserveSelectedSessionTimestampRef = useRef<string | null>(initialSession.id);
  const taskStartedAtRef = useRef<number | null>(null);
  const modelRequestStartedAtRef = useRef<number | null>(null);
  const firstResponseAtRef = useRef<number | null>(null);
  const lastStreamChunkAtRef = useRef<number | null>(null);
  const streamChunkIntervalsRef = useRef<number[]>([]);
  const streamChunkCountRef = useRef(0);
  const providerUsageRef = useRef({
    input_tokens: 0,
    output_tokens: 0,
    cache_read_tokens: 0,
    cache_write_tokens: 0,
    reasoning_tokens: 0,
    input_tokens_known: false,
    output_tokens_known: false,
    cache_read_tokens_known: false,
    cache_write_tokens_known: false,
    reasoning_tokens_known: false,
    calls: 0,
    models: new Set<string>(),
    hasUsage: false,
  });
  const [lastModelRoute, setLastModelRoute] = useState<ModelRouteTelemetry | null>(null);
  const lastModelRouteRef = useRef<ModelRouteTelemetry | null>(null);
  const [memoryBundle, setMemoryBundle] = useState<MemoryBundleInfo | null>(null);
  const memoryBundleRef = useRef<MemoryBundleInfo | null>(null);
  const [memoryLoading, setMemoryLoading] = useState(false);
  const [carryMemory, setCarryMemory] = useState(false);
  const carryMemoryRef = useRef(false);
  const [memoryPreviewOpen, setMemoryPreviewOpen] = useState(false);
  const [memoryDirError, setMemoryDirError] = useState<string | null>(null);
  const [completedToolIds, setCompletedToolIds] = useState<Set<string>>(new Set());
  const lastUserInputRef = useRef("");
  const pendingLogRef = useRef<{
    toolCalls: string[];
    params: Record<string, unknown>;
    validation: ElevatorValidation | null;
    summary: string;
    startedAt: number | null;
  } | null>(null);

  function setSessionsNow(next: SetStateAction<ChatSession[]>) {
    const resolved =
      typeof next === "function"
        ? (next as (prev: ChatSession[]) => ChatSession[])(sessionsRef.current)
        : next;
    sessionsRef.current = resolved;
    setSessions(resolved);
  }

  function setActiveSessionIdNow(id: string) {
    activeSessionIdRef.current = id;
    setActiveSessionId(id);
  }

  function setMessagesNow(next: SetStateAction<Message[]>) {
    const resolved =
      typeof next === "function"
        ? (next as (prev: Message[]) => Message[])(messagesRef.current)
        : next;
    messagesRef.current = resolved;
    setMessages(resolved);
  }

  function setDemoLogNow(next: SetStateAction<DemoLogEntry[]>) {
    const resolved =
      typeof next === "function"
        ? (next as (prev: DemoLogEntry[]) => DemoLogEntry[])(demoLogRef.current)
        : next;
    demoLogRef.current = resolved;
    setDemoLog(resolved);
  }

  function setLastValidationNow(next: ElevatorValidation | null) {
    lastValidationRef.current = next;
    setLastValidation(next);
  }

  function setLastDrawParamsNow(next: Record<string, unknown> | null) {
    lastDrawParamsRef.current = next;
    setLastDrawParams(next);
  }

  function setLastTaskDurationMsNow(next: number | null) {
    lastTaskDurationMsRef.current = next;
    setLastTaskDurationMs(next);
  }

  function setLastTokenTelemetryNow(next: TokenTelemetry | null) {
    lastTokenTelemetryRef.current = next;
    setLastTokenTelemetry(next);
  }

  function setLastModelRouteNow(next: ModelRouteTelemetry | null) {
    lastModelRouteRef.current = next;
    setLastModelRoute(next);
  }

  function setCarryMemoryNow(next: boolean) {
    carryMemoryRef.current = next;
    setCarryMemory(next);
  }

  async function refreshMemoryBundle() {
    setMemoryLoading(true);
    try {
      const info = normalizeMemoryBundleInfo(
        await invoke<MemoryBundleInfo>("read_memory_bundle", {
          location: appPreferencesRef.current.storageLocation,
        })
      );
      memoryBundleRef.current = info;
      setMemoryBundle(info);
    } catch (e) {
      memoryBundleRef.current = null;
      setMemoryBundle(null);
      console.error("read memory bundle:", e);
    } finally {
      setMemoryLoading(false);
    }
  }

  async function openMemoryDir() {
    const dir = memoryBundleRef.current?.dir;
    if (!dir) return;
    try {
      await openPath(dir);
      setMemoryDirError(null);
    } catch (e) {
      setMemoryDirError(String(e));
    }
  }

  const [benchmarkCandidates, setBenchmarkCandidates] = useState<BenchmarkCandidate[]>([]);
  const [benchmarkScope, setBenchmarkScope] = useState<"configured" | "all" | "failed">("configured");
  const [benchmarkRunning, setBenchmarkRunning] = useState(false);
  const [benchmarkProgress, setBenchmarkProgress] = useState<BenchmarkEvent | null>(null);
  const [benchmarkSummary, setBenchmarkSummary] = useState<BenchmarkSummary | null>(null);
  const [benchmarkError, setBenchmarkError] = useState<string | null>(null);
  const benchmarkCancelledRef = useRef(false);

  async function refreshBenchmarkState() {
    try {
      const candidates = (await invoke<BenchmarkCandidate[]>("benchmark_candidates")).map((item) =>
        normalizeBenchmarkCandidate(item)
      );
      setBenchmarkCandidates(candidates);
      const summary = normalizeBenchmarkSummary(
        await invoke<BenchmarkSummary | null>("read_benchmark_results", {
          location: appPreferencesRef.current.storageLocation,
        })
      );
      setBenchmarkSummary(summary);
    } catch (e) {
      console.error("load benchmark state:", e);
    }
  }

  async function startBenchmark() {
    if (benchmarkRunning) return;
    setBenchmarkRunning(true);
    setBenchmarkError(null);
    setBenchmarkProgress(null);
    setBenchmarkSummary(null);
    try {
      const failedSpecs = (benchmarkSummary?.models ?? [])
        .filter((m) => m.succeeded < m.requests)
        .map((m) => ({ provider: m.provider, model: m.model }));
      const specs =
        benchmarkScope === "all"
          ? ALL_BENCHMARK_SPECS
          : benchmarkScope === "failed"
            ? failedSpecs
            : null;
      const summary = normalizeBenchmarkSummary(
        await invoke<BenchmarkSummary>("run_model_benchmark", {
          location: appPreferencesRef.current.storageLocation,
          maxRequests: BENCHMARK_MAX_REQUESTS,
          models: specs,
        })
      );
      setBenchmarkSummary(summary);
    } catch (e) {
      setBenchmarkError(String(e));
    } finally {
      setBenchmarkRunning(false);
      setBenchmarkProgress(null);
    }
  }

  async function cancelBenchmark() {
    benchmarkCancelledRef.current = true;
    try {
      await invoke("cancel_model_benchmark");
    } catch (e) {
      console.error("cancel benchmark:", e);
    }
  }

  async function openBenchmarkResultsDir() {
    const path = benchmarkSummary?.results_json_path;
    if (!path) return;
    const dir = path.replace(/[/\\][^/\\]*$/, "");
    try {
      await openPath(dir);
    } catch (e) {
      setBenchmarkError(String(e));
    }
  }

  function tryParseValidation(content: string): ElevatorValidation | null {
    return parseValidationPayload(content);
  }

  function updateSessionObjects(
    updater: SessionObject[] | ((prev: SessionObject[]) => SessionObject[])
  ) {
    setSessionObjects((prev) => {
      const next = typeof updater === "function" ? updater(prev) : updater;
      sessionObjectsRef.current = next;
      return next;
    });
  }

  function commitUndoSnapshotIfNeeded() {
    if (runTouchedObjectTableRef.current && pendingUndoSnapshotRef.current) {
      undoSnapshotsRef.current.push(cloneSessionObjects(pendingUndoSnapshotRef.current));
    }
    pendingUndoSnapshotRef.current = null;
    runTouchedObjectTableRef.current = false;
  }

  function startTaskTimer() {
    const now = Date.now();
    taskStartedAtRef.current = now;
    setTaskStartedAt(now);
    setTaskElapsedMs(0);
  }

  function stopTaskTimer() {
    const startedAt = taskStartedAtRef.current;
    if (!startedAt) {
      setTaskStartedAt(null);
      return null;
    }
    const duration = Math.max(0, Date.now() - startedAt);
    taskStartedAtRef.current = null;
    setTaskStartedAt(null);
    setTaskElapsedMs(duration);
    setLastTaskDurationMsNow(duration);
    return duration;
  }

  function startModelTelemetry() {
    modelRequestStartedAtRef.current = Date.now();
    firstResponseAtRef.current = null;
    lastStreamChunkAtRef.current = null;
    streamChunkIntervalsRef.current = [];
    streamChunkCountRef.current = 0;
    providerUsageRef.current = {
      input_tokens: 0,
      output_tokens: 0,
      cache_read_tokens: 0,
      cache_write_tokens: 0,
      reasoning_tokens: 0,
      input_tokens_known: false,
      output_tokens_known: false,
      cache_read_tokens_known: false,
      cache_write_tokens_known: false,
      reasoning_tokens_known: false,
      calls: 0,
      models: new Set<string>(),
      hasUsage: false,
    };
    setLastTokenTelemetryNow(null);
    setLastModelRouteNow(null);
  }

  function recordModelResponseEvent(countChunk: boolean) {
    const startedAt = modelRequestStartedAtRef.current;
    if (!startedAt) return;

    const now = Date.now();
    if (!firstResponseAtRef.current) {
      firstResponseAtRef.current = now;
    }
    if (countChunk) {
      if (lastStreamChunkAtRef.current) {
        streamChunkIntervalsRef.current.push(Math.max(0, now - lastStreamChunkAtRef.current));
      }
      lastStreamChunkAtRef.current = now;
      streamChunkCountRef.current += 1;
    }
  }

  function recordProviderUsage(usage: ProviderTokenUsage) {
    const current = providerUsageRef.current;
    current.hasUsage = true;
    current.calls += 1;
    if (typeof usage.input_tokens === "number") {
      current.input_tokens += usage.input_tokens;
      current.input_tokens_known = true;
    }
    if (typeof usage.output_tokens === "number") {
      current.output_tokens += usage.output_tokens;
      current.output_tokens_known = true;
    }
    if (typeof usage.cache_read_tokens === "number") {
      current.cache_read_tokens += usage.cache_read_tokens;
      current.cache_read_tokens_known = true;
    }
    if (typeof usage.cache_write_tokens === "number") {
      current.cache_write_tokens += usage.cache_write_tokens;
      current.cache_write_tokens_known = true;
    }
    if (typeof usage.reasoning_tokens === "number") {
      current.reasoning_tokens += usage.reasoning_tokens;
      current.reasoning_tokens_known = true;
    }
    current.models.add(`${usage.provider} / ${usage.model}`);
  }

  function finishModelTelemetry(finalText: string | null) {
    const startedAt = modelRequestStartedAtRef.current;
    if (!startedAt) return null;

    const now = Date.now();
    const durationMs = Math.max(0, now - startedAt);
    const intervals = streamChunkIntervalsRef.current;
    const firstResponseMs = firstResponseAtRef.current
      ? Math.max(0, firstResponseAtRef.current - startedAt)
      : undefined;
    const avgChunkGapMs =
      intervals.length > 0
        ? intervals.reduce((sum, value) => sum + value, 0) / intervals.length
        : undefined;
    const maxChunkGapMs = intervals.length > 0 ? Math.max(...intervals) : undefined;
    const usage = providerUsageRef.current;
    const hasUsage = usage.hasUsage;
    const estimatedContextTokens = estimateSessionContextTokens(
      messagesRef.current,
      sessionObjectsRef.current,
      demoLogRef.current
    );
    const estimatedOutputTokens = finalText ? estimateTextTokens(finalText) : undefined;
    const inputTokens = usage.input_tokens_known ? usage.input_tokens : estimatedContextTokens;
    const outputTokens = usage.output_tokens_known ? usage.output_tokens : estimatedOutputTokens;
    const telemetry: TokenTelemetry = {
      started_at: startedAt,
      total_duration_ms: durationMs,
      first_response_ms: firstResponseMs,
      avg_chunk_gap_ms: avgChunkGapMs,
      max_chunk_gap_ms: maxChunkGapMs,
      chunk_count: streamChunkCountRef.current,
      input_tokens: inputTokens,
      input_tokens_estimated:
        !usage.input_tokens_known && typeof estimatedContextTokens === "number",
      output_tokens: outputTokens,
      output_tokens_estimated:
        !usage.output_tokens_known && typeof estimatedOutputTokens === "number",
      cache_read_tokens: usage.cache_read_tokens_known ? usage.cache_read_tokens : undefined,
      cache_write_tokens: usage.cache_write_tokens_known ? usage.cache_write_tokens : undefined,
      reasoning_tokens: usage.reasoning_tokens_known ? usage.reasoning_tokens : undefined,
      provider_calls: hasUsage ? usage.calls : undefined,
      provider_models: hasUsage && usage.models.size > 0 ? Array.from(usage.models) : undefined,
      output_tokens_per_second:
        typeof outputTokens === "number" && durationMs > 0
          ? outputTokens / (durationMs / 1000)
          : undefined,
      throughput_estimated:
        !usage.output_tokens_known && typeof estimatedOutputTokens === "number",
      estimated_context_tokens: estimatedContextTokens,
    };

    modelRequestStartedAtRef.current = null;
    firstResponseAtRef.current = null;
    lastStreamChunkAtRef.current = null;
    streamChunkIntervalsRef.current = [];
    streamChunkCountRef.current = 0;
    setLastTokenTelemetryNow(telemetry);
    return telemetry;
  }

  function currentSessionSnapshot(overrides: Partial<ChatSession> = {}): ChatSession {
    const now = Date.now();
    const currentId = activeSessionIdRef.current;
    const baseSession = sessionsRef.current.find((session) => session.id === currentId);
    const snapshotMessages = overrides.messages ?? messagesRef.current;
    const provider = normalizeProvider(overrides.provider ?? baseSession?.provider ?? settings.provider);
    const model = normalizeModelForProvider(
      provider,
      overrides.model ?? baseSession?.model,
      currentModelFor(settings, provider)
    );

    return {
      id: baseSession?.id ?? currentId,
      title: overrides.title ?? sessionTitleFromMessages(snapshotMessages),
      createdAt: baseSession?.createdAt ?? now,
      updatedAt: overrides.updatedAt ?? now,
      provider,
      model,
      messages: snapshotMessages,
      sessionObjects: overrides.sessionObjects ?? sessionObjectsRef.current,
      demoLog: overrides.demoLog ?? demoLogRef.current,
      lastValidation: overrides.lastValidation ?? lastValidationRef.current,
      lastDrawParams: overrides.lastDrawParams ?? lastDrawParamsRef.current,
      lastTaskDurationMs: overrides.lastTaskDurationMs ?? lastTaskDurationMsRef.current,
      lastTokenTelemetry: overrides.lastTokenTelemetry ?? lastTokenTelemetryRef.current,
    };
  }

  function sessionHasExportableContent(session: ChatSession) {
    return (
      session.messages.length > 0 ||
      session.sessionObjects.length > 0 ||
      session.demoLog.length > 0
    );
  }

  async function autoSaveSessionMemory(
    snapshot: ChatSession,
    eventKind: "task_completed" | "tool_confirmed"
  ) {
    const preferences = appPreferencesRef.current;
    if (!preferences.autoExportSessionMarkdown || !sessionHasExportableContent(snapshot)) return;

    try {
      await invoke<SessionMemorySaveResult>("save_session_memory_bundle", {
        sessionId: snapshot.id,
        markdownContent: buildSessionMarkdown(snapshot, preferences.language),
        summaryContent: buildSessionSummaryMarkdown(snapshot, preferences.language),
        eventContent: buildSessionMemoryEvent(snapshot, eventKind),
        indexEntry: buildSessionMemoryIndexEntry(snapshot),
        location: preferences.storageLocation,
      });
      void refreshMemoryBundle();
    } catch (e) {
      console.error("auto save session memory:", e);
      setErrorMsg(t("exportSessionFailed", preferences.language, { error: String(e) }));
    }
  }

  async function refreshSettings() {
    try {
      const s = await invoke<SettingsView>("get_settings");
      setSettings(normalizeSettingsView(s));
      setGlmKeyDraft(null);
      setDeepseekKeyDraft(null);
      setQwenKeyDraft(null);
      setKimiKeyDraft(null);
    } catch (e) {
      console.error("load settings:", e);
    }
  }

  useEffect(() => {
    refreshSettings();
  }, []);

  useEffect(() => {
    void refreshMemoryBundle();
  }, []);

  useEffect(() => {
    void refreshMemoryBundle();
  }, [appPreferences.storageLocation]);

  useEffect(() => {
    void refreshBenchmarkState();
  }, []);

  useEffect(() => {
    void refreshBenchmarkState();
  }, [appPreferences.storageLocation]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn | null = null;
    listen<BenchmarkEvent>("benchmark:event", (ev) => {
      if (!cancelled) setBenchmarkProgress(ev.payload);
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!taskStartedAt) return;
    const timer = window.setInterval(() => {
      setTaskElapsedMs(Math.max(0, Date.now() - taskStartedAt));
    }, 200);
    return () => window.clearInterval(timer);
  }, [taskStartedAt]);

  useEffect(() => {
    let cancelled = false;
    setTestStatus({ ok: true, msg: "Bridge 自动检测中..." });
    invoke<string>("test_cad_connection")
      .then((msg) => {
        if (!cancelled) setTestStatus({ ok: true, msg });
      })
      .catch((e) => {
        if (!cancelled) setTestStatus({ ok: false, msg: String(e) });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      window.localStorage.setItem(GLASS_STORAGE_KEY, JSON.stringify(glassSettings));
    }, 180);
    return () => window.clearTimeout(timer);
  }, [glassSettings]);

  // Native Effect.Blur on Windows makes the window opaque, overriding CSS transparency.
  // CSS backback-filter on panels is sufficient for the glass effect.
  // Apply CSS variables from stored settings once on mount.
  useEffect(() => {
    const stored = loadGlassSettings();
    applyGlassCssVariables(stored);
  }, []);

  useEffect(() => {
    appPreferencesRef.current = appPreferences;
    const timer = window.setTimeout(() => {
      window.localStorage.setItem(APP_PREFS_STORAGE_KEY, JSON.stringify(appPreferences));
    }, 180);
    return () => window.clearTimeout(timer);
  }, [appPreferences]);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      window.localStorage.setItem(
        CHAT_SESSIONS_STORAGE_KEY,
        JSON.stringify({ activeSessionId, sessions })
      );
    }, 180);
    return () => window.clearTimeout(timer);
  }, [activeSessionId, sessions]);

  useEffect(() => {
    const now = Date.now();
    const title = sessionTitleFromMessages(messages);
    const preserveTimestampFor = preserveSelectedSessionTimestampRef.current;
    if (preserveTimestampFor === activeSessionId) {
      preserveSelectedSessionTimestampRef.current = null;
    }
    setSessionsNow((prev) =>
      prev
        .map((session) =>
          session.id === activeSessionId
            ? {
                ...session,
                title,
                updatedAt: preserveTimestampFor === activeSessionId ? session.updatedAt : now,
                messages,
                sessionObjects,
                demoLog,
                lastValidation,
                lastDrawParams,
                lastTaskDurationMs,
                lastTokenTelemetry,
              }
            : session
        )
        .sort((a, b) => b.updatedAt - a.updatedAt)
    );
  }, [
    activeSessionId,
    messages,
    sessionObjects,
    demoLog,
    lastValidation,
    lastDrawParams,
    lastTaskDurationMs,
    lastTokenTelemetry,
  ]);

  useEffect(() => {
    void appWindow.setAlwaysOnTop(appPreferences.alwaysOnTop).catch((e) => {
      console.error("set always on top:", e);
    });
  }, [appWindow, appPreferences.alwaysOnTop]);

  // Subscribe to agent streaming events from the Rust backend.
  // listen() resolves async; React StrictMode can clean up before the promise resolves.
  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn | null = null;
    listen<AgentEvent>("agent:event", (ev) => {
      const e = ev.payload;
      if (e.kind === "assistant_trace" || e.kind === "assistant_delta") {
        recordModelResponseEvent(true);
        assistantDraftRef.current += e.delta;
        setAssistantDraft(assistantDraftRef.current);
      } else if (e.kind === "usage") {
        recordProviderUsage(e.usage);
      } else if (e.kind === "model_route") {
        setLastModelRouteNow(e.route);
      } else if (e.kind === "assistant") {
        recordModelResponseEvent(Boolean(e.text || e.tool_calls.length > 0));
        for (const call of e.tool_calls) {
          pendingToolCallsRef.current[call.id] = call;
          if (pendingLogRef.current) {
            pendingLogRef.current.toolCalls.push(call.name);
            Object.assign(pendingLogRef.current.params, call.args);
          }
        }
        if (assistantDraftRef.current && (e.text || e.tool_calls.length > 0)) {
          assistantDraftRef.current = "";
          setAssistantDraft("");
        }
        setMessagesNow((prev) => [
          ...prev,
          e.tool_calls.length > 0
            ? { role: "plan", text: e.text, tool_calls: e.tool_calls }
            : { role: "assistant", text: e.text, tool_calls: e.tool_calls },
        ]);
      } else if (e.kind === "tool_result") {
        if (assistantDraftRef.current) {
          assistantDraftRef.current = "";
          setAssistantDraft("");
        }
        completedToolIdsRef.current.add(e.result.id);
        setCompletedToolIds(new Set(completedToolIdsRef.current));
        const pendingCall = e.result.confirmation_required
          ? pendingToolCallsRef.current[e.result.id]
          : undefined;
        if (e.result.name === "validate_elevator_shaft_protection") {
          const validation = tryParseValidation(e.result.content);
          if (validation) {
            setLastValidationNow(validation);
            if (pendingLogRef.current) {
              pendingLogRef.current.validation = validation;
            }
          }
        }
        if (e.result.name === "draw_elevator_shaft_protection") {
          if (e.result.ok) {
            const call = pendingToolCallsRef.current[e.result.id];
            if (call) setLastDrawParamsNow(call.args);
          }
          if (pendingLogRef.current) {
            pendingLogRef.current.summary = e.result.ok
              ? e.result.content
              : `绘图失败：${e.result.content}`;
          }
        }
        setMessagesNow((prev) => [
          ...prev,
          { role: "tool", ...e.result, pending_call: pendingCall },
        ]);
        if (e.result.ok && shouldAutoSyncObjectTable(e.result.name)) {
          runTouchedObjectTableRef.current = true;
          pendingPostRunSyncRef.current = appPreferencesRef.current.autoSyncObjects;
        }
        if (e.result.object_updates.length > 0) {
          updateSessionObjects((prev) => applyObjectUpdates(prev, e.result.object_updates));
        }
      } else if (e.kind === "done") {
        if (assistantDraftRef.current) {
          assistantDraftRef.current = "";
          setAssistantDraft("");
        }
        const taskDuration = stopTaskTimer();
        const tokenTelemetry = finishModelTelemetry(e.text || null);
        commitUndoSnapshotIfNeeded();
        const pending = pendingLogRef.current;
        pendingLogRef.current = null;
        if (pending && pending.toolCalls.length > 0) {
          if (!e.text.trim()) {
            const summary = completionSummary(pending, appPreferencesRef.current.language);
            setMessagesNow((prev) => [...prev, { role: "assistant", text: summary, tool_calls: [] }]);
          }
          const entry: DemoLogEntry = {
            time: new Date().toLocaleTimeString("zh-CN", { hour12: false }),
            user_input: lastUserInputRef.current,
            tool_calls: pending.toolCalls,
            params: pending.params,
            validation: pending.validation ?? undefined,
            summary: pending.summary || "完成",
            duration_ms: taskDuration ?? undefined,
            token_telemetry: tokenTelemetry ?? undefined,
          };
          setDemoLogNow((prev) => [entry, ...prev].slice(0, 30));
        }
        const shouldPostRunSync = pendingPostRunSyncRef.current;
        pendingPostRunSyncRef.current = false;
        if (shouldPostRunSync) {
          void (async () => {
            await syncSessionObjects(false);
            await autoSaveSessionMemory(
              currentSessionSnapshot({
                lastTaskDurationMs: taskDuration ?? lastTaskDurationMsRef.current,
                lastTokenTelemetry: tokenTelemetry ?? lastTokenTelemetryRef.current,
              }),
              "task_completed"
            );
            setSending(false);
          })();
        } else {
          void (async () => {
            await autoSaveSessionMemory(
              currentSessionSnapshot({
                lastTaskDurationMs: taskDuration ?? lastTaskDurationMsRef.current,
                lastTokenTelemetry: tokenTelemetry ?? lastTokenTelemetryRef.current,
              }),
              "task_completed"
            );
            setSending(false);
          })();
        }
      } else if (e.kind === "error") {
        if (assistantDraftRef.current) {
          assistantDraftRef.current = "";
          setAssistantDraft("");
        }
        commitUndoSnapshotIfNeeded();
        pendingPostRunSyncRef.current = false;
        setErrorMsg(e.message);
        stopTaskTimer();
        finishModelTelemetry(null);
        setSending(false);
        if (e.message.includes("API Key")) {
          const session = sessionsRef.current.find((item) => item.id === activeSessionIdRef.current);
          openSettingsForProvider(normalizeProvider(session?.provider ?? settings.provider), true);
        }
      }
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, [messages, assistantDraft, sending]);

  useEffect(() => {
    const trace = thinkingTraceRef.current;
    if (!trace) return;
    trace.scrollTop = trace.scrollHeight;
  }, [assistantDraft]);

  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (e.ctrlKey && e.shiftKey && e.key === "0") {
        setGlassSettings(DEFAULT_GLASS_SETTINGS);
        saveGlassSettingsNow(DEFAULT_GLASS_SETTINGS);
        setAppPreferences(DEFAULT_APP_PREFERENCES);
        setErrorMsg("外观与字体设置已重置");
      } else if (e.key === "Escape" && (view === "settings" || view === "help")) {
        applyAppFontCssVariables(appPreferences.fontSize);
        setSettingsFocusProvider(null);
        setView("chat");
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [view]);

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    const text = input.trim();
    if (!text || sending) return;

    const previousMessages = messagesRef.current;
    const newHistory: Message[] = [...previousMessages, { role: "user", content: text }];
    setMessagesNow(newHistory);
    setInput("");
    assistantDraftRef.current = "";
    setAssistantDraft("");
    setSending(true);
    startTaskTimer();
    setErrorMsg(null);
    completedToolIdsRef.current = new Set();
    setCompletedToolIds(new Set());

    try {
      let syncedObjects = sessionObjectsRef.current;
      if (syncedObjects.length > 0) {
        const latest = await syncSessionObjects(false);
        if (latest === null) {
          stopTaskTimer();
          setSending(false);
          return;
        }
        syncedObjects = latest;
      }

      pendingUndoSnapshotRef.current = cloneSessionObjects(syncedObjects);
      runTouchedObjectTableRef.current = false;
      pendingPostRunSyncRef.current = false;
      lastUserInputRef.current = text;
      pendingLogRef.current = {
        toolCalls: [],
        params: {},
        validation: null,
        summary: "",
        startedAt: taskStartedAtRef.current,
      };

      // 会话记忆：默认不注入；仅当用户显式打开「本次发送携带全局记忆」开关时才携带，
      // 且只注入 payload，不污染聊天显示；发送后开关自动复位。
      const memoryInjection =
        carryMemoryRef.current && memoryBundleRef.current?.global_memory.trim()
          ? buildMemoryInjection(
              memoryBundleRef.current.global_memory,
              clampNumber(settings.memory_carry_token_budget, 200, 8000)
            )
          : null;
      setCarryMemoryNow(false);
      const payloadHistory = memoryInjection?.text
        ? [{ role: "user" as const, content: memoryInjection.text }, ...buildHistoryPayload(previousMessages)]
        : buildHistoryPayload(previousMessages);

      // run_agent emits agent:event for each step; resolve only means the backend loop ended.
      startModelTelemetry();
      await invoke("run_agent", {
        userInput: text,
        history: payloadHistory,
        sessionObjects: syncedObjects,
        modelSelection: {
          provider: activeProvider,
          model: activeModel,
        },
      });
    } catch (e) {
      pendingUndoSnapshotRef.current = null;
      runTouchedObjectTableRef.current = false;
      pendingPostRunSyncRef.current = false;
      setErrorMsg(String(e));
      stopTaskTimer();
      finishModelTelemetry(null);
      setSending(false);
    }
  }

  async function saveSettings(nextSettings: SettingsView = settings) {
    try {
      await invoke("save_settings", {
        update: {
          provider: nextSettings.provider,
          work_mode: nextSettings.work_mode,
          auto_failover: nextSettings.auto_failover,
          memory_carry_token_budget: nextSettings.memory_carry_token_budget,
          glm_model: nextSettings.glm_model,
          glm_strong_model: nextSettings.glm_strong_model,
          glm_base_url: nextSettings.glm_base_url,
          deepseek_model: nextSettings.deepseek_model,
          deepseek_strong_model: nextSettings.deepseek_strong_model,
          deepseek_base_url: nextSettings.deepseek_base_url,
          qwen_model: nextSettings.qwen_model,
          qwen_strong_model: nextSettings.qwen_strong_model,
          qwen_base_url: nextSettings.qwen_base_url,
          kimi_model: nextSettings.kimi_model,
          kimi_strong_model: nextSettings.kimi_strong_model,
          kimi_base_url: nextSettings.kimi_base_url,
          glm_api_key: glmKeyDraft,
          deepseek_api_key: deepseekKeyDraft,
          qwen_api_key: qwenKeyDraft,
          kimi_api_key: kimiKeyDraft,
        },
      });
      setSavedHint(true);
      setTimeout(() => setSavedHint(false), 1500);
      await refreshSettings();
    } catch (e) {
      setErrorMsg(String(e));
    }
  }

  async function runCadAction(cmd: "test_cad_connection" | "draw_test_line") {
    setTestStatus({ ok: true, msg: cmd === "test_cad_connection" ? "Bridge 检测中..." : "执行中..." });
    try {
      const msg = await invoke<string>(cmd);
      setTestStatus({ ok: true, msg });
    } catch (e) {
      setTestStatus({ ok: false, msg: String(e) });
    }
  }

  async function syncSessionObjects(showStatus: boolean) {
    const current = sessionObjectsRef.current;
    if (current.length === 0) {
      if (showStatus) {
        setTestStatus({ ok: true, msg: "当前没有可同步的会话对象" });
      }
      return current;
    }

    setSyncingObjects(true);
    if (showStatus) {
      setTestStatus({ ok: true, msg: "对象表同步中..." });
    }
    setErrorMsg(null);

    try {
      const synced = await invoke<SessionObject[]>("sync_session_objects", {
        sessionObjects: current,
      });
      updateSessionObjects(synced);
      if (showStatus) {
        const removed = current.length - synced.length;
        setTestStatus({
          ok: true,
          msg:
            removed > 0
              ? `对象表已同步，移除了 ${removed} 个已不存在对象`
              : `对象表已同步，${synced.length} 个对象仍有效`,
        });
      }
      return synced;
    } catch (e) {
      const msg = String(e);
      if (showStatus) {
        setTestStatus({ ok: false, msg });
      }
      setErrorMsg(msg);
      return null;
    } finally {
      setSyncingObjects(false);
    }
  }

  async function handleImportSelectedObjects() {
    if (sending || undoing || syncingObjects || importingSelection) return;
    setImportingSelection(true);
    setTestStatus({ ok: true, msg: "导入选中对象中..." });
    setErrorMsg(null);

    try {
      const before = sessionObjectsRef.current;
      const imported = await invoke<SessionObject[]>("import_selected_objects");
      const added = imported.filter(
        (object) => !before.some((existing) => existing.handle === object.handle)
      ).length;
      const refreshed = imported.length - added;

      updateSessionObjects((prev) => mergeSessionObjects(prev, imported));

      setTestStatus({
        ok: true,
        msg:
          refreshed > 0
            ? `已导入 ${imported.length} 个选中对象，其中 ${added} 个新增、${refreshed} 个已更新`
            : `已导入 ${imported.length} 个选中对象`,
      });
    } catch (e) {
      const msg = String(e);
      setTestStatus({ ok: false, msg });
      setErrorMsg(msg);
    } finally {
      setImportingSelection(false);
    }
  }

  async function handleUndoLastGeneration() {
    if (sending || undoing || syncingObjects || importingSelection) return;
    setUndoing(true);
    setTestStatus({ ok: true, msg: "撤回中..." });
    setErrorMsg(null);
    try {
      const msg = await invoke<string>("undo_last_generation");
      setTestStatus({ ok: true, msg });
      setMessagesNow((prev) => [...prev, { role: "assistant", text: msg, tool_calls: [] }]);
      const snapshot = undoSnapshotsRef.current.pop();
      if (snapshot) {
        updateSessionObjects(snapshot);
      }
      await syncSessionObjects(false);
    } catch (e) {
      const msg = String(e);
      setTestStatus({ ok: false, msg });
      setErrorMsg(msg);
    } finally {
      setUndoing(false);
    }
  }

  async function handleConfirmToolCall(messageIndex: number, call: ToolCall) {
    if (sending || undoing || syncingObjects || importingSelection) return;
    setSending(true);
    startTaskTimer();
    setErrorMsg(null);
    let shouldAutoSave = false;
    try {
      const result = await invoke<{
        id: string;
        name: string;
        ok: boolean;
        content: string;
        confirmation_required: boolean;
        object_updates: ObjectUpdate[];
      }>("confirm_tool_call", { call });

      setMessagesNow((prev) =>
        prev
          .map((message, index) =>
            index === messageIndex && message.role === "tool"
              ? { ...message, confirmed: true }
              : message
          )
          .concat({ role: "tool", ...result })
      );

      if (result.ok && shouldAutoSyncObjectTable(result.name)) {
        runTouchedObjectTableRef.current = true;
      }
      if (result.object_updates.length > 0) {
        updateSessionObjects((prev) => applyObjectUpdates(prev, result.object_updates));
      }
      if (result.ok && shouldAutoSyncObjectTable(result.name) && appPreferencesRef.current.autoSyncObjects) {
        await syncSessionObjects(false);
      }
      shouldAutoSave = true;
    } catch (e) {
      setErrorMsg(String(e));
    } finally {
      const taskDuration = stopTaskTimer();
      if (shouldAutoSave) {
        await autoSaveSessionMemory(
          currentSessionSnapshot({
            lastTaskDurationMs: taskDuration ?? lastTaskDurationMsRef.current,
          }),
          "tool_confirmed"
        );
      }
      setSending(false);
    }
  }

  function applySession(session: ChatSession, preserveTimestamp = false) {
    preserveSelectedSessionTimestampRef.current = preserveTimestamp ? session.id : null;
    assistantDraftRef.current = "";
    pendingToolCallsRef.current = {};
    completedToolIdsRef.current = new Set();
    setCompletedToolIds(new Set());
    pendingLogRef.current = null;
    pendingUndoSnapshotRef.current = null;
    runTouchedObjectTableRef.current = false;
    pendingPostRunSyncRef.current = false;
    taskStartedAtRef.current = null;
    modelRequestStartedAtRef.current = null;
    firstResponseAtRef.current = null;
    lastStreamChunkAtRef.current = null;
    streamChunkIntervalsRef.current = [];
    streamChunkCountRef.current = 0;
    setTaskStartedAt(null);
    setTaskElapsedMs(0);
    setLastTaskDurationMsNow(session.lastTaskDurationMs);
    setLastTokenTelemetryNow(session.lastTokenTelemetry);
    setLastModelRouteNow(null);
    setCarryMemoryNow(false);
    sessionObjectsRef.current = session.sessionObjects;
    setActiveSessionIdNow(session.id);
    setMessagesNow(session.messages);
    setAssistantDraft("");
    setSessionObjects(session.sessionObjects);
    setDemoLogNow(session.demoLog);
    setLastValidationNow(session.lastValidation);
    setLastDrawParamsNow(session.lastDrawParams);
    setInput("");
    setErrorMsg(null);
  }

  function handleNewConversation() {
    if (sending) return;
    const session = createChatSession(settings);
    setSessionsNow((prev) => [session, ...prev]);
    applySession(session);
  }

  function handleSelectSession(id: string) {
    if (sending || id === activeSessionIdRef.current) return;
    const session = sessionsRef.current.find((item) => item.id === id);
    if (session) applySession(session, true);
  }

  function handleDeleteSession(id: string) {
    if (sending) return;
    const remaining = sessionsRef.current.filter((session) => session.id !== id);
    if (remaining.length > 0) {
      setSessionsNow(remaining);
      if (id === activeSessionIdRef.current) applySession(remaining[0], true);
      return;
    }
    const next = createChatSession(settings);
    setSessionsNow([next]);
    applySession(next);
  }

  function handleExportCurrentSession() {
    const exportSession = currentSessionSnapshot({ updatedAt: Date.now() });

    if (!sessionHasExportableContent(exportSession)) {
      setErrorMsg(t("exportSessionEmpty", appPreferences.language));
      return;
    }

    try {
      const filename = sessionMarkdownFilename(exportSession.title, appPreferences.language);
      downloadMarkdown(filename, buildSessionMarkdown(exportSession, appPreferences.language));
      setErrorMsg(t("exportSessionDone", appPreferences.language));
    } catch (e) {
      setErrorMsg(t("exportSessionFailed", appPreferences.language, { error: String(e) }));
    }
  }

  async function handleModelChange(provider: Provider, model: string) {
    const nextModel = normalizeModelForProvider(provider, model, currentModelFor(settings, provider));
    setSessionsNow((prev) =>
      prev.map((s) =>
        s.id === activeSessionIdRef.current
          ? { ...s, provider, model: nextModel, updatedAt: Date.now() }
          : s,
      ),
    );
  }

  async function handleAutoFailoverChange(enabled: boolean) {
    const nextSettings = { ...settings, auto_failover: enabled };
    setSettings(nextSettings);
    await saveSettings(nextSettings);
  }

  async function recoverWindow(message?: string) {
    try {
      await appWindow.unminimize();
      await appWindow.show();
      await appWindow.center();
      await appWindow.setFocus();
      if (message) setErrorMsg(message);
    } catch (e) {
      setErrorMsg(`窗口恢复失败：${String(e)}`);
    }
  }

  function handleWindowDrag(e: MouseEvent<HTMLElement>) {
    if (e.button !== 0) return;
    const target = e.target as HTMLElement;
    if (target.closest("button, input, textarea, select, a, [data-no-drag]")) return;
    void appWindow.startDragging().catch((err) => {
      setErrorMsg(`窗口拖拽失败：${String(err)}`);
    });
  }

  async function runWindowAction(action: "minimize" | "toggleMaximize" | "close") {
    try {
      if (action === "minimize") {
        await appWindow.minimize();
      } else if (action === "toggleMaximize") {
        await appWindow.toggleMaximize();
      } else {
        await appWindow.close();
      }
    } catch (e) {
      setErrorMsg(`窗口控制失败：${String(e)}`);
    }
  }

  function setKeyDraftForProvider(provider: Provider, draft: string | null) {
    if (provider === "deepseek") {
      setDeepseekKeyDraft(draft);
    } else if (provider === "qwen") {
      setQwenKeyDraft(draft);
    } else if (provider === "kimi") {
      setKimiKeyDraft(draft);
    } else {
      setGlmKeyDraft(draft);
    }
  }

  function openSettingsForProvider(provider?: Provider, editKey = false) {
    setSettingsFocusProvider(provider ?? null);
    if (provider && editKey) {
      setKeyDraftForProvider(provider, "");
    }
    setView("settings");
  }

  const activeSession = sessions.find((session) => session.id === activeSessionId);
  const activeProvider = normalizeProvider(activeSession?.provider ?? settings.provider);
  const activeModel = normalizeModelForProvider(
    activeProvider,
    activeSession?.model,
    currentModelFor(settings, activeProvider)
  );
  const selectedProviderMeta = providerMeta(activeProvider);
  const providerLabel = providerDisplay(activeProvider, appPreferences.language, "short");
  const bridgeChecking = Boolean(
    testStatus?.msg.includes("检测中") || testStatus?.msg.includes("执行中")
  );
  const bridgeState = testStatus === null || bridgeChecking ? "idle" : testStatus.ok ? "online" : "error";
  const bridgeLabel =
    bridgeChecking
      ? t("bridgeChecking", appPreferences.language)
      : bridgeState === "online"
        ? t("bridgeOnline", appPreferences.language)
        : bridgeState === "error"
          ? t("bridgeError", appPreferences.language)
          : t("bridgeIdle", appPreferences.language);
  const objectReferenceHints = getObjectReferenceHints(sessionObjects);
  const currentKeySet = Boolean(settings[selectedProviderMeta.keySetField]);
  const quickPromptsZh = [
    "画一条 7000mm 的直线",
    "画一个半径 3000 的圆",
    "画一个双跑楼梯，层高 3000",
    "画一个电梯井口防护门，井口宽 2000，高 1800",
  ];
  const quickPromptsEn = [
    "Draw a 7000mm line",
    "Draw a circle with radius 3000",
    "Draw a double-flight stair, floor height 3000",
    "Draw elevator shaft protection, opening width 2000, height 1800",
  ];
  const quickPrompts = appPreferences.language === "en-US" ? quickPromptsEn : quickPromptsZh;
  const sessionTitle =
    sessions.find((session) => session.id === activeSessionId)?.title ??
    sessionTitleFromMessages(messages);
  const glassStyle = glassCssVariables(glassSettings, appPreferences.fontSize);

  return (
    <div
      className={`cadegg-app ${glassSettings.border === "glow" ? "glass-glow" : "glass-pixel"} ${
        appPreferences.reduceMotion ? "reduce-motion" : ""
      } ${appPreferences.densePanels ? "dense-panels" : ""}`}
      style={glassStyle}
    >
      <div className="ambient-layer" aria-hidden="true" />

      <header className="topbar glass-dark">
        <div className="brand-lockup drag-zone" onMouseDown={handleWindowDrag}>
          <EggLogo />
          <div>
            <div className="brand-name">CADEgg</div>
            <div className="brand-caption">AutoCAD AGENT</div>
          </div>
        </div>

        <div
          className="session-title drag-zone"
          title={displaySessionTitle(sessionTitle, appPreferences.language)}
          onMouseDown={handleWindowDrag}
        >
          {displaySessionTitle(sessionTitle, appPreferences.language)}
        </div>

        <div className="topbar-actions" data-no-drag>
          <StatusPill state={bridgeState} label={bridgeLabel} />
          <button
            type="button"
            className="help-chip"
            title={t("helpButton", appPreferences.language)}
            onClick={() => setView("help")}
            data-no-drag
          >
            {t("helpButton", appPreferences.language)}
          </button>
          <div className="window-controls" data-no-drag onMouseDown={(e) => e.stopPropagation()}>
            <button
              type="button"
              onClick={() => void runWindowAction("minimize")}
              aria-label={t("minimizeWindow", appPreferences.language)}
            >
              <span />
            </button>
            <button
              type="button"
              onClick={() => void runWindowAction("toggleMaximize")}
              aria-label={t("maximizeWindow", appPreferences.language)}
            >
              <span />
            </button>
            <button
              type="button"
              onClick={() => void runWindowAction("close")}
              aria-label={t("closeWindow", appPreferences.language)}
            >
              <span />
            </button>
          </div>
        </div>
      </header>

      <div className="decor-stripe" aria-hidden="true">
        <span />
        <span />
      </div>

      <div className="app-grid">
        <aside className="sidebar glass-panel pink-glass">
          <button type="button" className="new-session-button" onClick={handleNewConversation}>
            <IconPlus />
            <span>{t("newConversation", appPreferences.language)}</span>
          </button>

          <div className="sidebar-section">
            <div className="section-label">{t("conversations", appPreferences.language)}</div>
            {sessions.map((session) => {
              const sessionRating = modelRatingById(session.provider, session.model);
              return (
                <button
                  type="button"
                  className={`session-card ${session.id === activeSessionId ? "active" : ""}`}
                  onClick={() => handleSelectSession(session.id)}
                  key={session.id}
                >
                  <strong>{displaySessionTitle(session.title, appPreferences.language)}</strong>
                  <span>
                    {providerDisplay(session.provider, appPreferences.language, "short")} ·{" "}
                    {session.model || t("defaultModel", appPreferences.language)}
                    {sessionRating && (
                      <>
                        {" · "}
                        <ModelRatingCircles rating={sessionRating} />
                      </>
                    )}
                  </span>
                  {sessions.length > 1 && (
                    <i
                      role="button"
                      tabIndex={0}
                      aria-label={t("deleteSession", appPreferences.language)}
                      onClick={(e) => {
                        e.stopPropagation();
                        handleDeleteSession(session.id);
                      }}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault();
                          e.stopPropagation();
                          handleDeleteSession(session.id);
                        }
                      }}
                    >
                      ×
                    </i>
                  )}
                </button>
              );
            })}
          </div>

          <div className="sidebar-spacer" />

          <button
            type="button"
            className="sidebar-settings"
            onClick={handleExportCurrentSession}
            disabled={sending}
          >
            <IconDownload />
            <span>{t("exportSession", appPreferences.language)}</span>
            <span>.md</span>
          </button>

          <button type="button" className="sidebar-settings" onClick={() => openSettingsForProvider()}>
            <IconGear />
            <span>{t("settingsNav", appPreferences.language)}</span>
            <span>v0.3.4</span>
          </button>
        </aside>

        <section className="chat-column">
          <div ref={scrollRef} className="message-stage">
            {messages.length === 0 ? (
              <WelcomeStage
                quickPrompts={quickPrompts}
                setInput={setInput}
                currentKeySet={currentKeySet}
                providerLabel={providerLabel}
                onOpenSettings={() => openSettingsForProvider(activeProvider, !currentKeySet)}
                language={appPreferences.language}
              />
            ) : (
              messages.map((m, i) =>
                renderMessage(m, i, handleConfirmToolCall, completedToolIds, appPreferences.language)
              )
            )}

            {sending && (
              <div className="agent-status">
                <ThinkingTrace
                  text={assistantDraft}
                  language={appPreferences.language}
                  traceRef={thinkingTraceRef}
                />
              </div>
            )}

            {errorMsg && <div className="inline-error">{errorMsg}</div>}
          </div>

          <form className="composer glass-panel amber-glass" onSubmit={handleSubmit}>
            <ModelPicker
              settings={settings}
              provider={activeProvider}
              model={activeModel}
              currentKeySet={currentKeySet}
              onModelChange={handleModelChange}
              onAutoFailoverChange={handleAutoFailoverChange}
              onOpenSettings={() => openSettingsForProvider(activeProvider, !currentKeySet)}
              language={appPreferences.language}
            />
            <div className="composer-hint">{t("composerHint", appPreferences.language)}</div>
            {carryMemory &&
              memoryBundle?.global_memory.trim() &&
              (() => {
                const injection = buildMemoryInjection(
                  memoryBundle.global_memory,
                  clampNumber(settings.memory_carry_token_budget, 200, 8000)
                );
                return (
                  <div className="memory-carry-badge">
                    {t("memoryCarriedBadge", appPreferences.language, {
                      tokens: String(injection.tokens),
                    })}
                    {injection.truncated
                      ? ` · ${t("memoryTruncated", appPreferences.language)}`
                      : ""}
                  </div>
                );
              })()}
            <textarea
              rows={2}
              placeholder={
                sending
                  ? t("composerPlaceholderWaiting", appPreferences.language)
                  : t("composerPlaceholder", appPreferences.language)
              }
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  handleSubmit(e as unknown as FormEvent);
                }
              }}
              disabled={sending}
            />
            <div className="composer-side">
              <span>{providerLabel}</span>
              <button
                type="submit"
                disabled={!input.trim() || sending}
                aria-label={t("send", appPreferences.language)}
              >
                <IconSend />
              </button>
            </div>
          </form>
        </section>

        <aside className="right-rail glass-panel lavender-glass">
          <GenerationActionsCard
            handleUndoLastGeneration={handleUndoLastGeneration}
            sending={sending}
            undoing={undoing}
            syncingObjects={syncingObjects}
            taskElapsedMs={taskElapsedMs}
            taskStartedAt={taskStartedAt}
            lastTaskDurationMs={lastTaskDurationMs}
            lastTokenTelemetry={lastTokenTelemetry}
            lastModelRoute={lastModelRoute}
            language={appPreferences.language}
          />
          <DrawResultCard lastDrawParams={lastDrawParams} language={appPreferences.language} />
          <ValidationCard lastValidation={lastValidation} language={appPreferences.language} />
          <MemoryCard
            bundle={memoryBundle}
            loading={memoryLoading}
            carry={carryMemory}
            onCarryChange={setCarryMemoryNow}
            previewOpen={memoryPreviewOpen}
            onPreviewToggle={() => setMemoryPreviewOpen((v) => !v)}
            onRefresh={() => void refreshMemoryBundle()}
            onOpenDir={() => void openMemoryDir()}
            openError={memoryDirError}
            budgetTokens={clampNumber(settings.memory_carry_token_budget, 200, 8000)}
            language={appPreferences.language}
          />
          <BenchmarkCard
            candidates={benchmarkCandidates}
            scope={benchmarkScope}
            onScopeChange={setBenchmarkScope}
            running={benchmarkRunning}
            progress={benchmarkProgress}
            summary={benchmarkSummary}
            error={benchmarkError}
            onStart={() => void startBenchmark()}
            onCancel={() => void cancelBenchmark()}
            onOpenResultsDir={() => void openBenchmarkResultsDir()}
            language={appPreferences.language}
          />
        </aside>
      </div>

      {view === "settings" && (
        <SettingsModal
          settings={settings}
          setSettings={setSettings}
          appPreferences={appPreferences}
          setAppPreferences={setAppPreferences}
          glassSettings={glassSettings}
          setGlassSettings={setGlassSettings}
          savedHint={savedHint}
          saveSettings={saveSettings}
          glmKeyDraft={glmKeyDraft}
          setGlmKeyDraft={setGlmKeyDraft}
          deepseekKeyDraft={deepseekKeyDraft}
          setDeepseekKeyDraft={setDeepseekKeyDraft}
          qwenKeyDraft={qwenKeyDraft}
          setQwenKeyDraft={setQwenKeyDraft}
          kimiKeyDraft={kimiKeyDraft}
          setKimiKeyDraft={setKimiKeyDraft}
          onRecoverWindow={recoverWindow}
          testStatus={testStatus}
          sessionObjects={sessionObjects}
          objectReferenceHints={objectReferenceHints}
          sending={sending}
          undoing={undoing}
          syncingObjects={syncingObjects}
          importingSelection={importingSelection}
          runCadAction={runCadAction}
          handleImportSelectedObjects={handleImportSelectedObjects}
          syncSessionObjects={syncSessionObjects}
          focusedProvider={settingsFocusProvider}
          onClose={() => {
            setSettingsFocusProvider(null);
            setView("chat");
          }}
        />
      )}

      {view === "help" && (
        <HelpPanel
          language={appPreferences.language}
          onClose={() => setView("chat")}
        />
      )}
    </div>
  );
}

function ModelRatingCircles({
  rating,
  className,
}: {
  rating: ModelRating;
  className?: string;
}) {
  return (
    <span className={className ? `circle-rating ${className}` : "circle-rating"} aria-label={`${rating}/5`}>
      {modelRatingCircleStates(rating).map((state, index) => (
        <span className={`rating-circle ${state}`} key={index} aria-hidden="true" />
      ))}
    </span>
  );
}

function ModelSelect({
  models,
  value,
  onChange,
  language,
  ariaLabel,
}: {
  models: ModelOption[];
  value: string;
  onChange: (model: string) => void;
  language: "zh-CN" | "en-US";
  ariaLabel: string;
}) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const selected = models.find((item) => item.id === value);
  const selectedRating = selected ? modelRating(selected) : null;

  useEffect(() => {
    if (!open) return;

    function handlePointerDown(e: PointerEvent) {
      if (!rootRef.current?.contains(e.target as Node)) {
        setOpen(false);
      }
    }

    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        setOpen(false);
      }
    }

    document.addEventListener("pointerdown", handlePointerDown);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [open]);

  return (
    <div className={`model-select ${open ? "open" : ""}`} ref={rootRef}>
      <button
        type="button"
        className="model-select-button"
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((prev) => !prev)}
      >
        <span>{selected ? modelDisplay(selected, language) : value}</span>
        {selectedRating && <ModelRatingCircles rating={selectedRating} />}
        <em>{selected ? modelTierDisplay(selected, language) : "Custom"}</em>
      </button>

      {open && (
        <div className="model-select-options" role="listbox" aria-label={ariaLabel}>
          {models.map((item) => {
            const rating = modelRating(item);
            return (
              <button
                type="button"
                className={`model-select-option ${item.id === value ? "selected" : ""}`}
                role="option"
                aria-selected={item.id === value}
                key={item.id}
                onClick={() => {
                  onChange(item.id);
                  setOpen(false);
                }}
              >
                <span>{modelDisplay(item, language)}</span>
                <ModelRatingCircles rating={rating} />
                <em>{modelTierDisplay(item, language)}</em>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

function ModelPicker({
  settings,
  provider,
  model,
  currentKeySet,
  onModelChange,
  onAutoFailoverChange,
  onOpenSettings,
  language,
}: {
  settings: SettingsView;
  provider: Provider;
  model: string;
  currentKeySet: boolean;
  onModelChange: (provider: Provider, model: string) => Promise<void>;
  onAutoFailoverChange: (enabled: boolean) => Promise<void>;
  onOpenSettings: () => void;
  language: "zh-CN" | "en-US";
}) {
  const meta = providerMeta(provider);

  return (
    <div className="model-picker" data-no-drag>
      <select
        aria-label={t("providerSelectAria", language)}
        value={provider}
        onChange={(e) => {
          const nextProvider = e.target.value as Provider;
          const nextMeta = providerMeta(nextProvider);
          const nextModel = String(settings[nextMeta.strongModelField] || nextMeta.models[0].id);
          void onModelChange(nextProvider, nextModel);
        }}
      >
        {MODEL_PROVIDERS.map((provider) => (
          <option key={provider.id} value={provider.id}>
            {providerDisplay(provider.id, language, "label")}
          </option>
        ))}
      </select>

      <ModelSelect
        ariaLabel={t("sessionModelAria", language)}
        models={meta.models}
        value={model}
        onChange={(nextModel) => void onModelChange(provider, nextModel)}
        language={language}
      />

      <button
        type="button"
        className={`key-status ${currentKeySet ? "ready" : ""}`}
        onClick={onOpenSettings}
      >
        {currentKeySet ? t("byokConfigured", language) : t("byokFillKey", language)}
      </button>
      <label
        className={`failover-toggle ${settings.auto_failover ? "active" : ""}`}
        title={t("autoFailoverHint", language)}
      >
        <input
          type="checkbox"
          checked={settings.auto_failover}
          onChange={(e) => void onAutoFailoverChange(e.target.checked)}
        />
        <b aria-hidden="true" />
        <span>{settings.auto_failover ? t("autoFailoverOn", language) : t("autoFailoverOff", language)}</span>
      </label>
    </div>
  );
}

function WelcomeStage({
  quickPrompts,
  setInput,
  currentKeySet,
  providerLabel,
  onOpenSettings,
  language,
}: {
  quickPrompts: string[];
  setInput: (value: string) => void;
  currentKeySet: boolean;
  providerLabel: string;
  onOpenSettings: () => void;
  language: "zh-CN" | "en-US";
}) {
  return (
    <div className="welcome">
      <div className="hero-logo">
        <EggLogo large />
      </div>
      <h1>{t("welcomeTitle", language)}</h1>
      <p>{t("welcomeSubtitle", language)}</p>

      {!currentKeySet && (
        <button type="button" className="key-warning" onClick={onOpenSettings}>
          <b>{t("keyWarning", language, { provider: providerLabel })}</b>
          <span>{t("keyWarningHint", language)}</span>
        </button>
      )}

      <div className="prompt-list">
        {quickPrompts.map((prompt) => (
          <button type="button" key={prompt} onClick={() => setInput(prompt)}>
            {prompt}
          </button>
        ))}
      </div>
    </div>
  );
}

const ROUTE_PROVIDER_IDS = ["glm", "deepseek", "qwen", "kimi"] as const;

function routeProviderDisplay(provider: string, lang: "zh-CN" | "en-US") {
  if ((ROUTE_PROVIDER_IDS as readonly string[]).includes(provider)) {
    return providerDisplay(provider as Provider, lang, "short");
  }
  return provider;
}

function routeStatusLabel(status: string, lang: "zh-CN" | "en-US") {
  const labels: Record<string, string> = {
    selected: lang === "zh-CN" ? "选中" : "Selected",
    fallback: lang === "zh-CN" ? "回退" : "Fallback",
    skipped: lang === "zh-CN" ? "跳过" : "Skipped",
    failed: lang === "zh-CN" ? "失败" : "Failed",
    attempting: lang === "zh-CN" ? "尝试" : "Trying",
    planned: lang === "zh-CN" ? "计划" : "Planned",
  };
  return labels[status] ?? status;
}

function routeStatusClass(status: string) {
  if (status === "selected") return "route-chip selected";
  if (status === "fallback") return "route-chip fallback";
  if (status === "skipped") return "route-chip skipped";
  if (status === "failed") return "route-chip failed";
  if (status === "attempting") return "route-chip attempting";
  return "route-chip planned";
}

function formatFileSize(bytes: number) {
  if (!Number.isFinite(bytes) || bytes < 0) return "n/a";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function formatRelativeTime(updatedAtMs: number, lang: "zh-CN" | "en-US") {
  if (!Number.isFinite(updatedAtMs) || updatedAtMs <= 0) return "n/a";
  const deltaMs = Math.max(0, Date.now() - updatedAtMs);
  const minutes = Math.floor(deltaMs / 60_000);
  if (minutes < 1) return lang === "zh-CN" ? "刚刚" : "just now";
  if (minutes < 60) return lang === "zh-CN" ? `${minutes} 分钟前` : `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return lang === "zh-CN" ? `${hours} 小时前` : `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return lang === "zh-CN" ? `${days} 天前` : `${days}d ago`;
}

/** 构建本次发送要携带的全局记忆文本：带系统提醒包装与 token 预算截断。 */
function buildMemoryInjection(content: string, budgetTokens: number) {
  const trimmed = content.trim();
  if (!trimmed) return { text: "", tokens: 0, truncated: false };
  const budgetChars = Math.max(200, budgetTokens * 4);
  const full = `系统提醒（项目记忆，本次手动携带，仅供参考；若与当前任务无关请忽略）：\n${trimmed}`;
  if (full.length <= budgetChars) {
    return { text: full, tokens: estimateTextTokens(full), truncated: false };
  }
  const head = trimmed.slice(0, Math.max(120, budgetChars - 120));
  const text = `系统提醒（项目记忆，本次手动携带，已截断到约 ${budgetTokens} tokens，仅供参考）：\n${head}`;
  return { text, tokens: estimateTextTokens(text), truncated: true };
}

function GenerationActionsCard({
  undoing,
  sending,
  syncingObjects,
  taskElapsedMs,
  taskStartedAt,
  lastTaskDurationMs,
  lastTokenTelemetry,
  lastModelRoute,
  handleUndoLastGeneration,
  language,
}: {
  undoing: boolean;
  sending: boolean;
  syncingObjects: boolean;
  taskElapsedMs: number;
  taskStartedAt: number | null;
  lastTaskDurationMs: number | null;
  lastTokenTelemetry: TokenTelemetry | null;
  lastModelRoute: ModelRouteTelemetry | null;
  handleUndoLastGeneration: () => Promise<void>;
  language: "zh-CN" | "en-US";
}) {
  const visibleDuration = taskStartedAt ? taskElapsedMs : lastTaskDurationMs;
  const routeSelectedLabel = lastModelRoute
    ? `${routeProviderDisplay(lastModelRoute.selected_provider, language)} / ${lastModelRoute.selected_model}`
    : sending
      ? t("routeWaiting", language)
      : t("routeNone", language);
  const routeFinalLabel = lastModelRoute?.final_provider
    ? `${routeProviderDisplay(lastModelRoute.final_provider, language)} / ${lastModelRoute.final_model ?? lastModelRoute.selected_model}`
    : lastModelRoute
      ? t("routeProcessing", language)
      : "n/a";
  return (
    <section className="rail-card action-card">
      <PanelHeader title={t("actionCardTitle", language)} />
      <p>{t("actionCardDesc", language)}</p>
      <div className={`task-duration ${taskStartedAt ? "running" : "paused"}`}>
        <span>{t("taskDuration", language)}</span>
        <strong>{visibleDuration ? formatDuration(visibleDuration) : t("noTaskDuration", language)}</strong>
        <em>{taskStartedAt ? t("taskRunning", language) : t("taskPaused", language)}</em>
      </div>
    <div className="token-telemetry">
      <span>{t("firstTokenLatency", language)}</span>
      <b>{formatLatency(lastTokenTelemetry?.first_response_ms)}</b>
      <span>{t("avgTokenGap", language)}</span>
      <b>{formatLatency(lastTokenTelemetry?.avg_chunk_gap_ms)}</b>
      <span>{t("totalModelDuration", language)}</span>
      <b>{formatDuration(lastTokenTelemetry?.total_duration_ms)}</b>
      <span>{t("outputThroughput", language)}</span>
      <b>{formatTokensPerSecond(lastTokenTelemetry?.output_tokens_per_second, lastTokenTelemetry?.throughput_estimated)}</b>
      <span>{t("streamChunks", language)}</span>
      <b>{lastTokenTelemetry?.chunk_count ?? "n/a"}</b>
      <span>{t("inputTokens", language)}</span>
      <b>{formatTokenCountUi(lastTokenTelemetry?.input_tokens, lastTokenTelemetry?.input_tokens_estimated, language)}</b>
      <span>{t("outputTokens", language)}</span>
      <b>{formatTokenCountUi(lastTokenTelemetry?.output_tokens, lastTokenTelemetry?.output_tokens_estimated, language)}</b>
      <span>{t("cacheReadTokens", language)}</span>
      <b>{formatTokenCountUi(lastTokenTelemetry?.cache_read_tokens, false, language)}</b>
      <span>{t("cacheWriteTokens", language)}</span>
      <b>{formatTokenCountUi(lastTokenTelemetry?.cache_write_tokens, false, language)}</b>
      <span>{t("reasoningTokens", language)}</span>
      <b>{formatTokenCountUi(lastTokenTelemetry?.reasoning_tokens, false, language)}</b>
      <span>{t("providerCalls", language)}</span>
      <b>{formatTokenCountUi(lastTokenTelemetry?.provider_calls, false, language)}</b>
      <span>{t("estimatedContextTokens", language)}</span>
      <b>{formatTokenCountUi(lastTokenTelemetry?.estimated_context_tokens, true, language)}</b>
    </div>
    {(lastModelRoute || sending) && (
      <div className="route-telemetry">
        <div className="route-telemetry-header">
          <span>{t("modelRouteTitle", language)}</span>
          <b>
            {lastModelRoute
              ? lastModelRoute.fallback_count > 0
                ? t("routeFallbackCount", language, { count: String(lastModelRoute.fallback_count) })
                : t("routeNoFallback", language)
              : ""}
          </b>
        </div>
        <div className="route-line">
          <span>{t("routeSelected", language)}</span>
          <b>{routeSelectedLabel}</b>
        </div>
        <div className="route-line">
          <span>{t("routeFinal", language)}</span>
          <b>{routeFinalLabel}</b>
        </div>
        {lastModelRoute && lastModelRoute.attempts.length > 0 && (
          <div className="route-chips">
            {lastModelRoute.attempts.map((attempt, index) => (
              <span
                key={`${attempt.provider}-${attempt.model}-${index}`}
                className={routeStatusClass(attempt.status)}
                title={attempt.reason}
              >
                {routeStatusLabel(attempt.status, language)} {routeProviderDisplay(attempt.provider, language)} / {attempt.model}
              </span>
            ))}
          </div>
        )}
        {lastModelRoute?.note && <p className="route-note">{lastModelRoute.note}</p>}
      </div>
    )}
      <button
        type="button"
        className="dark-action"
        onClick={handleUndoLastGeneration}
        disabled={sending || undoing || syncingObjects}
      >
        {undoing ? t("undoing", language) : t("undoLast", language)}
      </button>
    </section>
  );
}

function CadDebugCard({
  testStatus,
  runCadAction,
  language,
}: {
  testStatus: { ok: boolean; msg: string } | null;
  runCadAction: (cmd: "test_cad_connection" | "draw_test_line") => Promise<void>;
  language: "zh-CN" | "en-US";
}) {
  const checking = Boolean(testStatus?.msg.includes("检测中") || testStatus?.msg.includes("执行中"));
  const status = testStatus === null || checking ? "idle" : testStatus.ok ? "online" : "error";

  return (
    <section className="debug-panel">
      <PanelHeader title={t("cadDebugTitle", language)} status={status} />
      <p>{t("cadDebugDesc", language)}</p>
      <div className="button-row">
        <button type="button" onClick={() => runCadAction("test_cad_connection")}>
          {t("connectionTest", language)}
        </button>
        <button type="button" onClick={() => runCadAction("draw_test_line")}>
          {t("drawLineTest", language)}
        </button>
      </div>
      {testStatus && (
        <div className={`status-readout ${testStatus.ok ? "ok" : "bad"}`}>{testStatus.msg}</div>
      )}
    </section>
  );
}

function DrawResultCard({
  lastDrawParams,
  language,
}: {
  lastDrawParams: Record<string, unknown> | null;
  language: "zh-CN" | "en-US";
}) {
  if (!lastDrawParams) {
    return (
      <section className="rail-card muted-card">
        <PanelHeader title={t("drawResultTitle", language)} />
        <p>{t("drawResultPending", language)}</p>
      </section>
    );
  }

  return (
    <section className="rail-card">
      <PanelHeader title={t("drawResultTitle", language)} />
      <div className="metric-grid">
        <Metric label={t("openingWidth", language)} value={`${String(lastDrawParams.opening_width ?? "-")} mm`} />
        <Metric label={t("openingHeight", language)} value={`${String(lastDrawParams.opening_height ?? "-")} mm`} />
        <Metric label={t("guardHeight", language)} value={`${String(lastDrawParams.guard_height ?? "1500")} mm`} />
        <Metric label={t("doorBottomGap", language)} value={`${String(lastDrawParams.door_bottom_gap ?? "50")} mm`} />
        <Metric label={t("toeBoard", language)} value={`${String(lastDrawParams.toe_board_height ?? "200")} mm`} />
      </div>
      <div className="tag-row">
        <span>
          {t("warningSign", language)}{" "}
          {lastDrawParams.include_warning_sign === false ? t("notIncluded", language) : t("included", language)}
        </span>
        <span>
          {t("materialTable", language)}{" "}
          {lastDrawParams.include_material_table === false ? t("notIncluded", language) : t("included", language)}
        </span>
      </div>
    </section>
  );
}

function ValidationCard({
  lastValidation,
  language,
}: {
  lastValidation: ElevatorValidation | null;
  language: "zh-CN" | "en-US";
}) {
  if (!lastValidation) {
    return (
      <section className="rail-card muted-card">
        <PanelHeader title={t("validationTitle", language)} />
        <p>{t("validationPending", language)}</p>
      </section>
    );
  }
  const warnings = lastValidation.warnings ?? [];
  const statusText =
    lastValidation.ok && warnings.length > 0
      ? t("validationPassedWithWarnings", language)
      : lastValidation.ok
        ? t("validationPassed", language)
        : t("validationFailed", language);

  return (
    <section className={`rail-card validation-card ${lastValidation.ok ? "valid" : "invalid"}`}>
      <PanelHeader title={t("validationTitle", language)} status={lastValidation.ok ? "online" : "error"} />
      <strong>{statusText}</strong>
      <div className="check-list">
        {lastValidation.checks.map((check) => (
          <div key={check.id}>
            <span className={check.passed ? "pass" : "fail"}>
              {check.passed ? <IconCheck /> : <IconCross />}
            </span>
            <span>{check.label}</span>
          </div>
        ))}
      </div>
      {lastValidation.issues.length > 0 && (
        <div className="risk-box">
          {t("riskItems", language, { items: lastValidation.issues.join(language === "zh-CN" ? "；" : "; ") })}
        </div>
      )}
      {warnings.length > 0 && (
        <div className="risk-box">
          {t("warningItems", language, { items: warnings.join(language === "zh-CN" ? "；" : "; ") })}
        </div>
      )}
      <p>
        {t("materialSummary", language, {
          guardDoor: lastValidation.material_table.guard_door,
          toeBoard: String(lastValidation.material_table.toe_board_height),
          warningSign: lastValidation.material_table.warning_sign
            ? t("included", language)
            : t("notIncluded", language),
        })}
      </p>
    </section>
  );
}

function MemoryCard({
  bundle,
  loading,
  carry,
  onCarryChange,
  previewOpen,
  onPreviewToggle,
  onRefresh,
  onOpenDir,
  openError,
  budgetTokens,
  language,
}: {
  bundle: MemoryBundleInfo | null;
  loading: boolean;
  carry: boolean;
  onCarryChange: (checked: boolean) => void;
  previewOpen: boolean;
  onPreviewToggle: () => void;
  onRefresh: () => void;
  onOpenDir: () => void;
  openError: string | null;
  budgetTokens: number;
  language: "zh-CN" | "en-US";
}) {
  const fileList = bundle?.files ?? [];
  const shownFiles = fileList.slice(0, 8);
  const globalMemory = bundle?.global_memory ?? "";
  const globalExists = bundle?.global_memory_exists ?? false;
  const fullTokens = globalMemory.trim() ? estimateTextTokens(globalMemory.trim()) : 0;
  return (
    <section className="rail-card memory-card">
      <PanelHeader title={t("memoryTitle", language)} status={bundle ? "online" : "idle"} />
      {loading && <p>{t("memoryLoading", language)}</p>}
      {!loading && fileList.length === 0 && <p>{t("memoryEmpty", language)}</p>}
      {!loading && fileList.length > 0 && (
        <>
          <p className="memory-count">
            {t("memoryFileCount", language, { count: String(fileList.length) })}
          </p>
          <div className="memory-file-list">
            {shownFiles.map((file: MemoryFileInfo) => (
              <div key={file.name} className="memory-file-row" title={file.name}>
                <span className="memory-file-name">{file.name}</span>
                <span className="memory-file-meta">
                  {formatFileSize(file.size_bytes)} · {formatRelativeTime(file.updated_at_ms, language)}
                </span>
              </div>
            ))}
          </div>
        </>
      )}
      <div className="button-row">
        <button type="button" onClick={onOpenDir} disabled={!bundle}>
          {t("memoryOpenDir", language)}
        </button>
        <button type="button" onClick={onRefresh} disabled={loading}>
          {t("memoryRefresh", language)}
        </button>
      </div>
      {bundle && (
        <p className="memory-dir" title={bundle.dir}>
          {bundle.dir}
        </p>
      )}
      {openError && (
        <p className="inline-error">{t("memoryDirFailed", language, { error: openError })}</p>
      )}
      {bundle && (
        <div className="memory-global">
          <div className="memory-global-head">
            <strong>{t("memoryGlobalTitle", language)}</strong>
            {globalExists && (
              <button type="button" className="link-button" onClick={onPreviewToggle}>
                {previewOpen ? t("memoryPreviewHide", language) : t("memoryPreviewShow", language)}
              </button>
            )}
          </div>
          {!globalExists ? (
            <p>{t("memoryGlobalMissing", language)}</p>
          ) : (
            <>
              <SwitchField
                label={t("memoryCarryLabel", language)}
                checked={carry}
                onChange={onCarryChange}
              />
              <p className="memory-est">
                {t("memoryTokensFull", language, { tokens: String(fullTokens) })} ·{" "}
                {t("memoryBudget", language, { budget: String(budgetTokens) })}
              </p>
              {previewOpen && <pre className="memory-preview">{globalMemory}</pre>}
              <p className="memory-hint">{t("memoryCarryHint", language)}</p>
            </>
          )}
        </div>
      )}
    </section>
  );
}

function BenchmarkCard({
  candidates,
  scope,
  onScopeChange,
  running,
  progress,
  summary,
  error,
  onStart,
  onCancel,
  onOpenResultsDir,
  language,
}: {
  candidates: BenchmarkCandidate[];
  scope: "configured" | "all" | "failed";
  onScopeChange: (scope: "configured" | "all" | "failed") => void;
  running: boolean;
  progress: BenchmarkEvent | null;
  summary: BenchmarkSummary | null;
  error: string | null;
  onStart: () => void;
  onCancel: () => void;
  onOpenResultsDir: () => void;
  language: "zh-CN" | "en-US";
}) {
  const runnable = candidates.filter((c) => !c.skip_reason);
  const skipped = candidates.filter((c) => c.skip_reason);
  const allCount = ALL_BENCHMARK_SPECS.length;
  const failedCount = (summary?.models ?? []).filter((m) => m.succeeded < m.requests).length;
  const estimate =
    scope === "all" ? allCount * 6 : scope === "failed" ? failedCount * 6 : runnable.length * 6;
  const canStart =
    running ||
    (scope === "all" ? allCount === 0 : scope === "failed" ? failedCount === 0 : runnable.length === 0);
  return (
    <section className="rail-card benchmark-card">
      <PanelHeader
        title={t("benchmarkTitle", language)}
        status={running || summary ? "online" : "idle"}
      />
      <div className="benchmark-scope">
        <span>{t("benchmarkScopeLabel", language)}</span>
        <div className="benchmark-scope-buttons">
          <button
            type="button"
            className={scope === "configured" ? "active" : ""}
            onClick={() => onScopeChange("configured")}
            disabled={running}
          >
            {t("benchmarkScopeConfigured", language)}
          </button>
          <button
            type="button"
            className={scope === "all" ? "active" : ""}
            onClick={() => onScopeChange("all")}
            disabled={running}
          >
            {t("benchmarkScopeAll", language)}
          </button>
          <button
            type="button"
            className={scope === "failed" ? "active" : ""}
            onClick={() => onScopeChange("failed")}
            disabled={running || failedCount === 0}
          >
            {t("benchmarkScopeFailed", language)}
          </button>
        </div>
      </div>
      <p className="benchmark-meta">
        {scope === "all"
          ? t("benchmarkScopeAllHint", language, {
              count: String(allCount),
              requests: String(estimate),
            })
          : scope === "failed"
            ? failedCount > 0
              ? t("benchmarkScopeFailedHint", language, {
                  count: String(failedCount),
                  requests: String(estimate),
                })
              : t("benchmarkNoFailed", language)
            : `${t("benchmarkRunnable", language, { count: String(runnable.length) })} · ${t(
                "benchmarkEstimate",
                language,
                { count: String(estimate) }
              )}`}
      </p>
      {scope === "configured" && skipped.length > 0 && (
        <p className="benchmark-skipped">
          {t("benchmarkSkipped", language, {
            text: skipped
              .map((s) => `${s.provider_label}（${s.skip_reason}）`)
              .join("；"),
          })}
        </p>
      )}
      <div className="button-row">
        <button type="button" onClick={onStart} disabled={canStart}>
          {t("benchmarkStart", language)}
        </button>
        <button type="button" onClick={onCancel} disabled={!running}>
          {t("benchmarkCancel", language)}
        </button>
      </div>
      {running && progress && (
        <div className="benchmark-progress">
          <div className="benchmark-progress-head">
            <span>
              {progress.current}/{progress.total}
            </span>
            <span>{progress.model ? `${progress.provider} / ${progress.model}` : ""}</span>
          </div>
          <p>{progress.message}</p>
        </div>
      )}
      {error && (
        <p className="inline-error">{t("benchmarkFailed", language, { error })}</p>
      )}
      {summary && !running && (
        <div className="benchmark-results">
          <p className="benchmark-meta">
            {t("benchmarkLastRun", language, {
              date: formatMarkdownTime(summary.started_at_ms),
            })}
            {summary.cancelled ? ` · ${t("benchmarkCancelledNote", language)}` : ""}
          </p>
          <div className="benchmark-rows">
            {summary.models.map((m: BenchmarkModelResult) => (
              <div key={`${m.provider}-${m.model}`} className="benchmark-row">
                <span className="benchmark-model">
                  {m.provider_label} / {m.model}
                </span>
                <span className="benchmark-score" title={`score=${m.score.toFixed(3)}`}>
                  {m.rating.toFixed(1)}★ · {m.score.toFixed(2)} · {m.succeeded}/{m.requests} ·{" "}
                  {formatDuration(m.avg_duration_ms)}
                </span>
              </div>
            ))}
          </div>
          <p className="benchmark-weights">{t("benchmarkWeights", language)}</p>
          <div className="button-row">
            <button type="button" onClick={onOpenResultsDir}>
              {t("benchmarkOpenDir", language)}
            </button>
          </div>
        </div>
      )}
      {!summary && !running && (
        <p className="benchmark-meta">{t("benchmarkNever", language)}</p>
      )}
    </section>
  );
}

function SessionObjectsCard({
  sessionObjects,
  objectReferenceHints,
  sending,
  undoing,
  syncingObjects,
  importingSelection,
  handleImportSelectedObjects,
  syncSessionObjects,
  language,
}: {
  sessionObjects: SessionObject[];
  objectReferenceHints: Map<string, string[]>;
  sending: boolean;
  undoing: boolean;
  syncingObjects: boolean;
  importingSelection: boolean;
  handleImportSelectedObjects: () => Promise<void>;
  syncSessionObjects: (showStatus: boolean) => Promise<SessionObject[] | null>;
  language: "zh-CN" | "en-US";
}) {
  return (
    <section className="rail-card object-card">
      <PanelHeader
        title={t("sessionObjectsTitle", language, { count: String(sessionObjects.length) })}
      />
      <div className="button-row">
        <button
          type="button"
          onClick={handleImportSelectedObjects}
          disabled={sending || undoing || syncingObjects || importingSelection}
        >
          {importingSelection ? t("importing", language) : t("importSelected", language)}
        </button>
        <button
          type="button"
          onClick={() => void syncSessionObjects(true)}
          disabled={sending || undoing || syncingObjects || importingSelection || sessionObjects.length === 0}
        >
          {syncingObjects ? t("syncing", language) : t("syncObjects", language)}
        </button>
      </div>

      {sessionObjects.length > 0 ? (
        <div className="object-list">
          {sessionObjects.map((object) => (
            <article key={object.handle} className="object-item">
              <div>
                <code>{object.handle}</code>
                <b>{object.kind}</b>
                <span>{sourceDisplayLabel(object.source)}</span>
              </div>
              <p>{object.label}</p>
              <div className="hint-row">
                {(objectReferenceHints.get(object.handle) ?? []).slice(0, 3).map((hint) => (
                  <span key={hint}>{hint}</span>
                ))}
              </div>
            </article>
          ))}
        </div>
      ) : (
        <p>{t("noSessionObjects", language)}</p>
      )}
    </section>
  );
}

function SettingsModal({
  settings,
  setSettings,
  appPreferences,
  setAppPreferences,
  glassSettings,
  setGlassSettings,
  savedHint,
  saveSettings,
  glmKeyDraft,
  setGlmKeyDraft,
  deepseekKeyDraft,
  setDeepseekKeyDraft,
  qwenKeyDraft,
  setQwenKeyDraft,
  kimiKeyDraft,
  setKimiKeyDraft,
  onRecoverWindow,
  testStatus,
  sessionObjects,
  objectReferenceHints,
  sending,
  undoing,
  syncingObjects,
  importingSelection,
  runCadAction,
  handleImportSelectedObjects,
  syncSessionObjects,
  focusedProvider,
  onClose,
}: {
  settings: SettingsView;
  setSettings: Dispatch<SetStateAction<SettingsView>>;
  appPreferences: AppPreferences;
  setAppPreferences: Dispatch<SetStateAction<AppPreferences>>;
  glassSettings: GlassSettings;
  setGlassSettings: Dispatch<SetStateAction<GlassSettings>>;
  savedHint: boolean;
  saveSettings: (nextSettings?: SettingsView) => Promise<void>;
  glmKeyDraft: string | null;
  setGlmKeyDraft: (v: string | null) => void;
  deepseekKeyDraft: string | null;
  setDeepseekKeyDraft: (v: string | null) => void;
  qwenKeyDraft: string | null;
  setQwenKeyDraft: (v: string | null) => void;
  kimiKeyDraft: string | null;
  setKimiKeyDraft: (v: string | null) => void;
  onRecoverWindow: (message?: string) => Promise<void>;
  testStatus: { ok: boolean; msg: string } | null;
  sessionObjects: SessionObject[];
  objectReferenceHints: Map<string, string[]>;
  sending: boolean;
  undoing: boolean;
  syncingObjects: boolean;
  importingSelection: boolean;
  runCadAction: (cmd: "test_cad_connection" | "draw_test_line") => Promise<void>;
  handleImportSelectedObjects: () => Promise<void>;
  syncSessionObjects: (showStatus: boolean) => Promise<SessionObject[] | null>;
  focusedProvider: Provider | null;
  onClose: () => void;
}) {
  const [draftAppPreferences, setDraftAppPreferences] = useState<AppPreferences>(() =>
    normalizeAppPreferences(appPreferences)
  );
  const [draftGlassSettings, setDraftGlassSettings] = useState<GlassSettings>(() =>
    normalizeGlassSettings(glassSettings)
  );
  const [isPreviewingGlass, setIsPreviewingGlass] = useState(false);
  const previewGlassSettingsRef = useRef(draftGlassSettings);
  const providerCardRefs = useRef<Partial<Record<Provider, HTMLElement | null>>>({});

  useEffect(() => {
    setDraftAppPreferences(normalizeAppPreferences(appPreferences));
  }, [appPreferences]);

  useEffect(() => {
    const next = normalizeGlassSettings(glassSettings);
    previewGlassSettingsRef.current = next;
    setDraftGlassSettings(next);
  }, [glassSettings]);

  useEffect(() => {
    if (!focusedProvider) return;
    const timer = window.setTimeout(() => {
      providerCardRefs.current[focusedProvider]?.scrollIntoView({
        block: "center",
        behavior: "smooth",
      });
    }, 80);
    return () => window.clearTimeout(timer);
  }, [focusedProvider]);

  async function handleSaveAll() {
    const nextAppPreferences = normalizeAppPreferences(draftAppPreferences);
    applyAppFontCssVariables(nextAppPreferences.fontSize);
    setAppPreferences(nextAppPreferences);
    await saveSettings();
  }

  function previewFontSize(fontSize: number) {
    const nextFontSize = normalizeAppPreferences({ fontSize }).fontSize;
    setDraftAppPreferences((prev) => ({ ...prev, fontSize: nextFontSize }));
    applyAppFontCssVariables(nextFontSize);
  }

  function handleClose() {
    applyAppFontCssVariables(appPreferences.fontSize);
    onClose();
  }

  function previewGlassSettings(partial: Partial<GlassSettings>) {
    const next = normalizeGlassSettings({
      ...previewGlassSettingsRef.current,
      ...partial,
    });
    previewGlassSettingsRef.current = next;
    applyGlassCssVariables(next);
    saveGlassSettingsNow(next);
    // CSS-only during drag — native effect only on commit to avoid flicker
  }

  function commitGlassSettings(partial: Partial<GlassSettings>) {
    const next = normalizeGlassSettings({
      ...previewGlassSettingsRef.current,
      ...partial,
    });
    previewGlassSettingsRef.current = next;
    setDraftGlassSettings(next);
    applyGlassCssVariables(next);
    saveGlassSettingsNow(next);
    setGlassSettings(next);
  }

  function keyDraftFor(provider: Provider) {
    if (provider === "deepseek") {
      return {
        draft: deepseekKeyDraft,
        setDraft: setDeepseekKeyDraft,
        placeholder: "sk-...",
      };
    }
    if (provider === "qwen") {
      return {
        draft: qwenKeyDraft,
        setDraft: setQwenKeyDraft,
        placeholder: "sk-...",
      };
    }
    if (provider === "kimi") {
      return {
        draft: kimiKeyDraft,
        setDraft: setKimiKeyDraft,
        placeholder: "sk-...",
      };
    }
    return {
      draft: glmKeyDraft,
      setDraft: setGlmKeyDraft,
      placeholder: "xxxxxxxx.xxxx",
    };
  }

  const language = draftAppPreferences.language;

  return (
    <div className={`modal-backdrop ${isPreviewingGlass ? "previewing-glass" : ""}`}>
      <section className="settings-modal glass-modal" role="dialog" aria-modal="true">
        <ModalHeader
          title={t("settingsTitle", language)}
          onClose={handleClose}
          closeLabel={t("closeWindow", language)}
        />

        <div className="settings-content">
          <section className="settings-group">
            <GroupHeader title={t("appSectionTitle", language)} desc={t("appSectionDesc", language)} />
            <Field label={t("languageLabel", language)} hint={t("languageHint", language)}>
              <select
                className={inputCls}
                value={draftAppPreferences.language}
                onChange={(e) =>
                  setDraftAppPreferences((prev) => ({
                    ...prev,
                    language: e.target.value === "en-US" ? "en-US" : "zh-CN",
                  }))
                }
              >
                <option value="zh-CN">简体中文</option>
                <option value="en-US">English</option>
              </select>
            </Field>

            <Field label={t("fontSizeLabel", language)} hint={t("fontSizeHint", language)}>
              <StableRange
                ariaLabel={t("fontSizeLabel", language)}
                className="inline-slider"
                min={11}
                max={22}
                value={draftAppPreferences.fontSize}
                suffix="px"
                onPreview={previewFontSize}
                onCommit={previewFontSize}
              />
            </Field>

            <Field label={t("storageLabel", language)} hint={t("storageHint", language)}>
              <select
                className={inputCls}
                value={draftAppPreferences.storageLocation}
                onChange={(e) =>
                  setDraftAppPreferences((prev) => ({
                    ...prev,
                    storageLocation: e.target.value === "project" ? "project" : "appdata",
                  }))
                }
              >
                <option value="appdata">{t("storageAppData", language)}</option>
                <option value="project">{t("storageProject", language)}</option>
              </select>
            </Field>

            <div className="switch-grid">
              <SwitchField
                label={t("notificationsLabel", language)}
                checked={draftAppPreferences.notifications}
                onChange={(checked) =>
                  setDraftAppPreferences((prev) => ({ ...prev, notifications: checked }))
                }
              />
              <SwitchField
                label={t("autoSyncLabel", language)}
                checked={draftAppPreferences.autoSyncObjects}
                onChange={(checked) =>
                  setDraftAppPreferences((prev) => ({ ...prev, autoSyncObjects: checked }))
                }
              />
              <SwitchField
                label={t("autoExportSessionLabel", language)}
                checked={draftAppPreferences.autoExportSessionMarkdown}
                onChange={(checked) =>
                  setDraftAppPreferences((prev) => ({
                    ...prev,
                    autoExportSessionMarkdown: checked,
                  }))
                }
              />
              <SwitchField
                label={t("alwaysOnTopLabel", language)}
                checked={draftAppPreferences.alwaysOnTop}
                onChange={(checked) =>
                  setDraftAppPreferences((prev) => ({ ...prev, alwaysOnTop: checked }))
                }
              />
              <SwitchField
                label={t("reduceMotionLabel", language)}
                checked={draftAppPreferences.reduceMotion}
                onChange={(checked) =>
                  setDraftAppPreferences((prev) => ({ ...prev, reduceMotion: checked }))
                }
              />
              <SwitchField
                label={t("densePanelsLabel", language)}
                checked={draftAppPreferences.densePanels}
                onChange={(checked) =>
                  setDraftAppPreferences((prev) => ({ ...prev, densePanels: checked }))
                }
              />
            </div>
          </section>

          <section className="settings-group">
            <GroupHeader
              title={t("glassSectionTitle", language)}
              desc={t("glassSectionDesc", language)}
            />
            <StableRange
              label={t("transparencyLabel", language)}
              min={0}
              max={90}
              value={draftGlassSettings.transparency}
              suffix="%"
              onPreview={(transparency) => previewGlassSettings({ transparency })}
              onCommit={(transparency) => commitGlassSettings({ transparency })}
              onDragStateChange={setIsPreviewingGlass}
            />

            <StableRange
              label={t("roughnessLabel", language)}
              min={0}
              max={100}
              value={draftGlassSettings.blur}
              suffix="%"
              onPreview={(blur) => previewGlassSettings({ blur })}
              onCommit={(blur) => commitGlassSettings({ blur })}
              onDragStateChange={setIsPreviewingGlass}
            />

            <div className="border-style-field">
              <span>{t("borderStyleLabel", language)}</span>
              <div className="segmented">
                <button
                  type="button"
                  className={draftGlassSettings.border === "pixel" ? "active" : ""}
                  onClick={() => commitGlassSettings({ border: "pixel" })}
                >
                  {t("pixelBorder", language)}
                </button>
                <button
                  type="button"
                  className={draftGlassSettings.border === "glow" ? "active" : ""}
                  onClick={() => commitGlassSettings({ border: "glow" })}
                >
                  {t("glowBorder", language)}
                </button>
              </div>
            </div>

            <button
              type="button"
              className="outline-action reset-glass"
              onClick={() => commitGlassSettings(DEFAULT_GLASS_SETTINGS)}
            >
              {t("resetGlass", language)}
            </button>
            <button
              type="button"
              className="outline-action reset-glass"
              onClick={() => void onRecoverWindow(t("windowRecovered", language))}
            >
              {t("recoverWindow", language)}
            </button>
          </section>

          <section className="settings-group">
            <GroupHeader
              title={t("modelSectionTitle", language)}
              desc={t("modelSectionDesc", language)}
            />
            <SwitchField
              label={t("autoFailover", language)}
              checked={settings.auto_failover}
              onChange={(checked) =>
                setSettings((prev) => ({ ...prev, auto_failover: checked }))
              }
            />
            <label className="settings-field settings-budget-field">
              <span>{t("memoryBudgetSetting", language)}</span>
              <input
                type="number"
                min={200}
                max={8000}
                step={100}
                value={settings.memory_carry_token_budget}
                onChange={(e) =>
                  setSettings((prev) => ({
                    ...prev,
                    memory_carry_token_budget: clampNumber(Number(e.target.value), 200, 8000),
                  }))
                }
                onBlur={(e) =>
                  setSettings((prev) => ({
                    ...prev,
                    memory_carry_token_budget: clampNumber(Number(e.target.value), 200, 8000),
                  }))
                }
              />
              <span className="settings-budget-hint">{t("memoryBudgetSettingHint", language)}</span>
            </label>
            <div
              className={`failover-pool ${
                MODEL_PROVIDERS.some((provider) => providerKeyIsSet(settings, provider.id))
                  ? ""
                  : "empty"
              }`}
            >
              <strong>{t("failoverPoolTitle", language)}</strong>
              <span>{failoverPoolText(settings, language)}</span>
            </div>
            <div className="provider-settings-list">
              {MODEL_PROVIDERS.map((provider) => {
                const keyDraft = keyDraftFor(provider.id);
                const baseUrl = String(settings[provider.baseUrlField] ?? "");
                const cheapModel = String(settings[provider.cheapModelField] ?? "");
                const strongModel = String(settings[provider.strongModelField] ?? "");
                const keySet = Boolean(settings[provider.keySetField]);
                const keyPreview = String(settings[provider.keyPreviewField] ?? "");

                return (
                  <section
                    className={`provider-settings-card ${
                      focusedProvider === provider.id ? "focused" : ""
                    }`}
                    key={provider.id}
                    ref={(node) => {
                      providerCardRefs.current[provider.id] = node;
                    }}
                  >
                    <GroupHeader
                      title={providerDisplay(provider.id, language, "label")}
                      desc={`${t("modelCardDesc", language)} ${
                        keySet
                          ? t("providerConfigured", language)
                          : t("providerMissingKey", language)
                      }`}
                    />
                    <KeyField
                      label={provider.apiLabel}
                      isSet={keySet}
                      preview={keyPreview}
                      draft={keyDraft.draft}
                      onDraftChange={keyDraft.setDraft}
                      placeholder={keyDraft.placeholder}
                      language={language}
                    />
                    <Field label={t("baseUrlLabel", language)} hint={t("baseUrlHint", language)}>
                      <input
                        type="text"
                        className={inputCls}
                        value={baseUrl}
                        onChange={(e) =>
                          setSettings((prev) => ({
                            ...prev,
                            [provider.baseUrlField]: e.target.value,
                          }))
                        }
                      />
                    </Field>
                    <Field label={t("cheapModelLabel", language)}>
                      <ModelSelect
                        ariaLabel={`${providerDisplay(provider.id, language, "label")} ${t("cheapModelLabel", language)}`}
                        models={provider.models}
                        value={cheapModel}
                        onChange={(nextModel) =>
                          setSettings((prev) => ({
                            ...prev,
                            [provider.cheapModelField]: nextModel,
                          }))
                        }
                        language={language}
                      />
                    </Field>
                    <Field label={t("strongModelLabel", language)}>
                      <ModelSelect
                        ariaLabel={`${providerDisplay(provider.id, language, "label")} ${t("strongModelLabel", language)}`}
                        models={provider.models}
                        value={strongModel}
                        onChange={(nextModel) =>
                          setSettings((prev) => ({
                            ...prev,
                            [provider.strongModelField]: nextModel,
                          }))
                        }
                        language={language}
                      />
                    </Field>
                  </section>
                );
              })}
            </div>

            <p className="settings-note">
              {t("modelCostNote", language)}
            </p>
            <p className="settings-note">
              {t("settingsKeyNote", language)}
            </p>
          </section>

          <section className="settings-group developer-tools">
            <GroupHeader
              title={t("developerToolsTitle", language)}
              desc={t("developerToolsDesc", language)}
            />
            <div className="developer-grid">
              <CadDebugCard
                testStatus={testStatus}
                runCadAction={runCadAction}
                language={draftAppPreferences.language}
              />
              <SessionObjectsCard
                sessionObjects={sessionObjects}
                objectReferenceHints={objectReferenceHints}
                sending={sending}
                undoing={undoing}
                syncingObjects={syncingObjects}
                importingSelection={importingSelection}
                handleImportSelectedObjects={handleImportSelectedObjects}
                syncSessionObjects={syncSessionObjects}
                language={draftAppPreferences.language}
              />
            </div>
          </section>
        </div>

        <div className="modal-footer">
          <span>{savedHint ? t("saved", language) : ""}</span>
          <button type="button" className="outline-action" onClick={handleClose}>
            {t("back", language)}
          </button>
          <button type="button" className="primary-action" onClick={() => void handleSaveAll()}>
            {t("saveAppModel", language)}
          </button>
        </div>
      </section>
    </div>
  );
}

function parseValidationPayload(content: string): ElevatorValidation | null {
  const trimmed = content.trim();
  if (!trimmed.startsWith("{")) return null;
  try {
    const parsed = JSON.parse(trimmed);
    if (
      parsed &&
      typeof parsed.ok === "boolean" &&
      Array.isArray(parsed.checks) &&
      typeof parsed.material_table === "object"
    ) {
      return parsed as ElevatorValidation;
    }
  } catch {
    return null;
  }
  return null;
}

function lifecycleSummary(validation: ElevatorValidation, language: "zh-CN" | "en-US") {
  const lc = validation.lifecycle;
  if (!lc) return "";
  if (lc.state === "temporarily_removed") {
    const missing = [
      !lc.removal_reason && (language === "en-US" ? "removal reason" : "拆除原因"),
      !lc.replacement_protection && (language === "en-US" ? "replacement protection" : "替代防护"),
      !lc.responsible_person && (language === "en-US" ? "responsible person" : "责任人"),
      !lc.restore_time && (language === "en-US" ? "restore time" : "恢复时间"),
    ].filter(Boolean);
    if (language === "en-US") {
      return missing.length > 0
        ? `Temporary removal: missing records (${missing.join(", ")}).`
        : `Temporary removal: replacement protection recorded, responsible ${lc.responsible_person}, restore by ${lc.restore_time}.`;
    }
    return missing.length > 0
      ? `临时拆除：缺少管理记录（${missing.join("、")}）。`
      : `临时拆除：替代防护 ${lc.replacement_protection}，责任人 ${lc.responsible_person}，恢复时间 ${lc.restore_time}。`;
  }
  if (lc.state === "restored") {
    return language === "en-US"
      ? `Restored${lc.acceptance_status === "accepted" ? " and accepted" : ", acceptance pending"}.`
      : `已恢复${lc.acceptance_status === "accepted" ? "并已验收" : "，待验收"}。`;
  }
  return "";
}

function validationSummary(validation: ElevatorValidation, language: "zh-CN" | "en-US") {
  const warnings = validation.warnings ?? [];
  const lifecycle = lifecycleSummary(validation, language);
  if (language === "en-US") {
    if (validation.ok) {
      const parts = [
        "Validation passed",
        lifecycle,
        warnings.length > 0
          ? `recommendations: ${warnings.slice(0, 3).join(", ")}`
          : null,
      ].filter(Boolean);
      return parts.join(" · ") + ".";
    }
    const issues = validation.issues.slice(0, 3).join(", ");
    return `Validation failed: ${issues || "see safety panel"}${lifecycle ? " · " + lifecycle : ""}.`;
  }

  if (validation.ok) {
    const parts = [
      "校核通过",
      lifecycle,
      warnings.length > 0 ? `建议项提醒：${warnings.slice(0, 3).join("、")}` : null,
    ].filter(Boolean);
    return parts.join(" · ") + "。";
  }
  const issues = validation.issues.slice(0, 3).join("、");
  return `校核未通过：${issues || "请查看右侧安全校核面板"}${lifecycle ? " · " + lifecycle : ""}。`;
}

function toolCallArgsPreview(call: ToolCall, language: "zh-CN" | "en-US") {
  if (call.name === "validate_elevator_shaft_protection") {
    return language === "en-US" ? "Validation parameters collapsed" : "校核参数已折叠";
  }
  return compactToolArgs(call.args);
}

function toolResultDisplayText(message: Extract<Message, { role: "tool" }>, language: "zh-CN" | "en-US") {
  if (message.name === "validate_elevator_shaft_protection") {
    const validation = parseValidationPayload(message.content);
    if (validation) return validationSummary(validation, language);
    return message.ok
      ? language === "en-US"
        ? "Validation completed."
        : "校核已完成。"
      : message.content;
  }
  return message.content;
}

function completionSummary(
  pending: {
    toolCalls: string[];
    params: Record<string, unknown>;
    validation: ElevatorValidation | null;
    summary: string;
  },
  language: "zh-CN" | "en-US"
) {
  const drewElevator = pending.toolCalls.includes("draw_elevator_shaft_protection");
  const validation = pending.validation;
  if (language === "en-US") {
    if (drewElevator && validation) {
      return validation.ok
        ? "Task complete: the elevator shaft protection drawing was generated and passed safety validation."
        : `Task complete, with validation issues: ${validation.issues.slice(0, 3).join(", ") || "see safety panel"}.`;
    }
    if (drewElevator) return "Task complete: the elevator shaft protection drawing was generated.";
    if (validation) return validationSummary(validation, language);
    return pending.summary ? `Task complete: ${pending.summary}` : "Task complete.";
  }

  if (drewElevator && validation) {
    return validation.ok
      ? "任务完成：已生成电梯井口防护门，并通过安全校核。"
      : `任务完成，但校核发现问题：${validation.issues.slice(0, 3).join("、") || "请查看右侧安全校核面板"}。`;
  }
  if (drewElevator) return "任务完成：已生成电梯井口防护门。";
  if (validation) return validationSummary(validation, language);
  return pending.summary ? `任务完成：${pending.summary}` : "任务完成。";
}

function renderMessage(
  m: Message,
  i: number,
  onConfirmToolCall: (messageIndex: number, call: ToolCall) => void,
  completedToolIds: Set<string>,
  language: "zh-CN" | "en-US"
) {
  if (m.role === "user") {
    return (
      <div key={i} className="message-bubble user-message">
        {m.content}
      </div>
    );
  }

  if (m.role === "plan") {
    return (
      <details key={i} className="message-bubble plan-message" open>
        <summary>
          {t("executionPlan", language)} · {t("stepCount", language, { count: String(m.tool_calls.length) })} · {planSummary(m.tool_calls)}
        </summary>
        {m.text && <div className="message-text">{m.text}</div>}
        <div className="tool-call-list">
          {m.tool_calls.map((tc) => {
            const done = completedToolIds.has(tc.id);
            return (
              <article key={tc.id} className={done ? "tool-done" : "tool-pending"}>
                <span className="tool-status">{done ? <IconCheck /> : <span className="spinner" />}</span>
                <strong>{tc.name}</strong>
                <code>{toolCallArgsPreview(tc, language)}</code>
              </article>
            );
          })}
        </div>
      </details>
    );
  }

  if (m.role === "assistant") {
    return (
      <div key={i} className="assistant-stack">
        {m.text && <div className="message-bubble assistant-message">{m.text}</div>}
        {m.tool_calls.map((tc) => (
          <article key={tc.id} className="tool-call">
            <strong>{tc.name}</strong>
            <code>{JSON.stringify(tc.args)}</code>
          </article>
        ))}
      </div>
    );
  }

  return (
    <div
      key={i}
      className={`message-bubble tool-message ${
        m.confirmation_required ? "confirm" : m.ok ? "ok" : "bad"
      }`}
    >
      <strong>
        {m.confirmation_required ? "!" : m.ok ? <IconCheck /> : <IconCross />} {m.name}
      </strong>
      <span>{toolResultDisplayText(m, language)}</span>
      {m.confirmation_required && m.pending_call && !m.confirmed && (
        <button type="button" onClick={() => onConfirmToolCall(i, m.pending_call!)}>
          {t("confirmExecute", language)}
        </button>
      )}
      {m.confirmation_required && m.confirmed && <em>{t("confirmedHint", language)}</em>}
    </div>
  );
}

function PanelHeader({
  title,
  status,
}: {
  title: string;
  status?: "online" | "error" | "idle";
}) {
  return (
    <header className="panel-header">
      <h2>{title}</h2>
      {status && <span className={`led ${status}`} />}
    </header>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="metric">
      <span>{label}</span>
      <b>{value}</b>
    </div>
  );
}

function StatusPill({ state, label }: { state: "online" | "error" | "idle"; label: string }) {
  return (
    <div className="status-pill">
      <span className={`led ${state}`} />
      <span>{label}</span>
    </div>
  );
}

function ThinkingTrace({
  text,
  language,
  traceRef,
}: {
  text: string;
  language: "zh-CN" | "en-US";
  traceRef: RefObject<HTMLDivElement | null>;
}) {
  return (
    <section className="thinking-trace" aria-live="polite">
      <header>
        <span className="streaming-cursor" />
        <strong>{t("thinkingProcessTitle", language)}</strong>
        <em>{t("thinkingProcessHint", language)}</em>
      </header>
      <div ref={traceRef} className="thinking-trace-body">
        {text ? (
          <pre>{text}</pre>
        ) : (
          <div className="typing-indicator compact" aria-label={t("waitingForModel", language)}>
            <span />
            <span />
            <span />
            <b>{t("waitingForModel", language)}</b>
          </div>
        )}
      </div>
    </section>
  );
}

function ModalHeader({
  title,
  onClose,
  closeLabel = "关闭",
}: {
  title: string;
  onClose: () => void;
  closeLabel?: string;
}) {
  return (
    <header className="modal-header">
      <h2>{title}</h2>
      <button type="button" className="icon-button" onClick={onClose} aria-label={closeLabel}>
        <IconCross />
      </button>
    </header>
  );
}

const inputCls = "settings-input";

function KeyField({
  label,
  isSet,
  preview,
  draft,
  onDraftChange,
  placeholder,
  language,
}: {
  label: string;
  isSet: boolean;
  preview: string;
  draft: string | null;
  onDraftChange: (v: string | null) => void;
  placeholder: string;
  language: "zh-CN" | "en-US";
}) {
  const editing = draft !== null;
  const hint = isSet
    ? editing
      ? t("keyWillOverwrite", language)
      : t("keySavedHint", language)
    : t("keyEmptyHint", language);

  return (
    <Field label={label} hint={hint}>
      {editing ? (
        <div className="key-edit-row">
          <input
            type="password"
            className={inputCls}
            placeholder={placeholder}
            value={draft ?? ""}
            onChange={(e) => onDraftChange(e.target.value)}
            spellCheck={false}
            autoComplete="off"
            autoFocus
          />
          <button type="button" className="outline-action" onClick={() => onDraftChange(null)}>
            {t("cancel", language)}
          </button>
        </div>
      ) : (
        <div className="key-edit-row">
          <div className={`${inputCls} key-preview`}>{isSet ? preview : t("keyNotSet", language)}</div>
          <button type="button" className="outline-action" onClick={() => onDraftChange("")}>
            {isSet ? t("modify", language) : t("set", language)}
          </button>
        </div>
      )}
    </Field>
  );
}

function GroupHeader({ title, desc }: { title: string; desc: string }) {
  return (
    <header className="group-header">
      <h3>{title}</h3>
      <p>{desc}</p>
    </header>
  );
}

function SwitchField({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="switch-field">
      <span>{label}</span>
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      <b aria-hidden="true" />
    </label>
  );
}

function StableRange({
  label,
  ariaLabel,
  className,
  min,
  max,
  value,
  suffix,
  onPreview,
  onCommit,
  onDragStateChange,
}: {
  label?: string;
  ariaLabel?: string;
  className?: string;
  min: number;
  max: number;
  value: number;
  suffix: string;
  onPreview?: (value: number) => void;
  onCommit: (value: number) => void;
  onDragStateChange?: (dragging: boolean) => void;
}) {
  const inputRef = useRef<HTMLInputElement | null>(null);
  const outputRef = useRef<HTMLElement | null>(null);
  const valueRef = useRef(value);
  const draggingRef = useRef(false);
  const rangeLabel = label ?? ariaLabel ?? "滑块";
  const classNames = className ? `slider-field ${className}` : "slider-field";
  const commitKeys = new Set([
    "ArrowLeft",
    "ArrowRight",
    "ArrowUp",
    "ArrowDown",
    "Home",
    "End",
    "PageUp",
    "PageDown",
  ]);

  function format(next: number) {
    return `${Math.round(next)}${suffix}`;
  }

  function readValue() {
    return Math.round(clamp(inputRef.current?.valueAsNumber ?? valueRef.current, min, max));
  }

  function previewValue() {
    const next = readValue();
    if (outputRef.current) {
      outputRef.current.textContent = format(next);
    }
    onPreview?.(next);
  }

  function commitValue() {
    const next = readValue();
    const previous = valueRef.current;
    valueRef.current = next;
    if (inputRef.current) {
      inputRef.current.value = String(next);
    }
    if (outputRef.current) {
      outputRef.current.textContent = format(next);
    }
    if (next !== previous) {
      onCommit(next);
    }
  }

  function setDragging(next: boolean) {
    if (draggingRef.current === next) return;
    draggingRef.current = next;
    onDragStateChange?.(next);
  }

  function endInteraction() {
    commitValue();
    setDragging(false);
  }

  function handleKeyUp(e: ReactKeyboardEvent<HTMLInputElement>) {
    if (commitKeys.has(e.key)) {
      commitValue();
    }
  }

  useEffect(() => {
    const next = Math.round(clamp(value, min, max));
    valueRef.current = next;
    if (inputRef.current) {
      inputRef.current.value = String(next);
    }
    if (outputRef.current) {
      outputRef.current.textContent = format(next);
    }
  }, [value, min, max, suffix]);

  return (
    <div className={classNames}>
      {label && (
        <label>
          <span>{label}</span>
          <b ref={outputRef}>{format(value)}</b>
        </label>
      )}
      <input
        ref={inputRef}
        type="range"
        min={min}
        max={max}
        defaultValue={value}
        aria-label={rangeLabel}
        onInput={previewValue}
        onPointerDown={(e) => {
          setDragging(true);
          try {
            e.currentTarget.setPointerCapture?.(e.pointerId);
          } catch {
            // Some WebView range controls do not allow pointer capture; drag still works without it.
          }
        }}
        onPointerUp={endInteraction}
        onPointerCancel={endInteraction}
        onLostPointerCapture={() => setDragging(false)}
        onTouchEnd={endInteraction}
        onBlur={endInteraction}
        onKeyUp={handleKeyUp}
      />
      {!label && <b ref={outputRef}>{format(value)}</b>}
    </div>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <div className="field">
      <label>{label}</label>
      {children}
      {hint && <span>{hint}</span>}
    </div>
  );
}

function EggLogo({ large = false }: { large?: boolean }) {
  return (
    <svg
      className={`pixel-icon egg-logo ${large ? "large" : ""}`}
      viewBox="0 0 32 32"
      aria-hidden="true"
    >
      <rect x="8" y="6" width="16" height="4" />
      <rect x="4" y="10" width="24" height="12" />
      <rect x="8" y="22" width="16" height="4" />
      <rect className="cut" x="8" y="12" width="4" height="4" />
      <rect className="cut" x="20" y="12" width="4" height="4" />
      <rect className="cut" x="14" y="18" width="4" height="4" />
      <rect className="accent" x="14" y="2" width="4" height="4" />
      <rect className="accent" x="2" y="14" width="4" height="4" />
      <rect className="accent" x="26" y="14" width="4" height="4" />
    </svg>
  );
}

function IconPlus() {
  return (
    <svg className="pixel-icon" viewBox="0 0 16 16" aria-hidden="true">
      <rect x="7" y="3" width="2" height="10" />
      <rect x="3" y="7" width="10" height="2" />
    </svg>
  );
}

function IconSend() {
  return (
    <svg className="pixel-icon" viewBox="0 0 16 16" aria-hidden="true">
      <rect x="3" y="3" width="10" height="2" />
      <rect x="5" y="5" width="8" height="2" />
      <rect x="7" y="7" width="6" height="2" />
      <rect x="5" y="9" width="8" height="2" />
      <rect x="3" y="11" width="10" height="2" />
    </svg>
  );
}

function IconDownload() {
  return (
    <svg className="pixel-icon" viewBox="0 0 16 16" aria-hidden="true">
      <rect x="7" y="2" width="2" height="7" />
      <rect x="5" y="7" width="2" height="2" />
      <rect x="9" y="7" width="2" height="2" />
      <rect x="3" y="10" width="10" height="2" />
      <rect x="3" y="12" width="2" height="2" />
      <rect x="11" y="12" width="2" height="2" />
    </svg>
  );
}

function IconGear() {
  return (
    <svg className="pixel-icon" viewBox="0 0 16 16" aria-hidden="true">
      <rect x="6" y="1" width="4" height="3" />
      <rect x="6" y="12" width="4" height="3" />
      <rect x="1" y="6" width="3" height="4" />
      <rect x="12" y="6" width="3" height="4" />
      <rect x="5" y="5" width="6" height="6" />
      <rect className="cut" x="7" y="7" width="2" height="2" />
    </svg>
  );
}

function IconCheck() {
  return (
    <svg className="pixel-icon" viewBox="0 0 16 16" aria-hidden="true">
      <rect x="3" y="8" width="2" height="2" />
      <rect x="5" y="10" width="2" height="2" />
      <rect x="7" y="8" width="2" height="2" />
      <rect x="9" y="6" width="2" height="2" />
      <rect x="11" y="4" width="2" height="2" />
    </svg>
  );
}

function IconCross() {
  return (
    <svg className="pixel-icon" viewBox="0 0 16 16" aria-hidden="true">
      <rect x="3" y="3" width="2" height="2" />
      <rect x="5" y="5" width="2" height="2" />
      <rect x="7" y="7" width="2" height="2" />
      <rect x="9" y="9" width="2" height="2" />
      <rect x="11" y="11" width="2" height="2" />
      <rect x="11" y="3" width="2" height="2" />
      <rect x="9" y="5" width="2" height="2" />
      <rect x="5" y="9" width="2" height="2" />
      <rect x="3" y="11" width="2" height="2" />
    </svg>
  );
}

// ── Help Panel ──

interface HelpPanelProps {
  language: "zh-CN" | "en-US";
  onClose: () => void;
}

type EvidenceStatus = "primary" | "candidate" | "light" | "legacy";

interface ModelEvidenceRow {
  provider: string;
  model: string;
  status: EvidenceStatus;
  rating: ModelRating;
  officialZh: string;
  officialEn: string;
  cadeggZh: string;
  cadeggEn: string;
  sourceZh: string;
  sourceEn: string;
}

const MODEL_EVIDENCE_TABLE: ModelEvidenceRow[] = [
  {
    provider: "GLM",
    model: "glm-5.2 / glm-5.1 / glm-5",
    status: "candidate",
    rating: 4,
    officialZh: "智谱当前 GLM-5 文本模型系列。",
    officialEn: "Current Zhipu GLM-5 text model family.",
    cadeggZh: "强模型候选；需要 CAD 工具链回归后再设为默认。",
    cadeggEn: "Strong candidate; needs CADEgg tool-chain regression before default use.",
    sourceZh: "智谱模型概览/价格",
    sourceEn: "Zhipu model overview/pricing",
  },
  {
    provider: "GLM",
    model: "glm-5-turbo",
    status: "light",
    rating: 3.5,
    officialZh: "智谱 GLM-5 系列轻量模型。",
    officialEn: "Lightweight model in the Zhipu GLM-5 family.",
    cadeggZh: "适合普通问答、参数解释和低风险辅助。",
    cadeggEn: "Useful for Q&A, parameter explanation, and low-risk assistance.",
    sourceZh: "智谱模型概览/价格",
    sourceEn: "Zhipu model overview/pricing",
  },
  {
    provider: "GLM",
    model: "glm-4.7",
    status: "candidate",
    rating: 4,
    officialZh: "智谱建议从 GLM-4.5 迁移到 GLM-4.7。",
    officialEn: "Zhipu recommends migration from GLM-4.5 to GLM-4.7.",
    cadeggZh: "下一轮优先实测的强模型。",
    cadeggEn: "Priority strong-model target for the next CADEgg test round.",
    sourceZh: "智谱模型概览",
    sourceEn: "Zhipu model overview",
  },
  {
    provider: "GLM",
    model: "glm-4.6",
    status: "candidate",
    rating: 4,
    officialZh: "智谱 GLM-4 系列当前模型。",
    officialEn: "Current model in the Zhipu GLM-4 family.",
    cadeggZh: "可作为 GLM-4.5/4.7 之间的备用强模型。",
    cadeggEn: "Usable as a backup strong model between GLM-4.5 and GLM-4.7.",
    sourceZh: "智谱模型概览/价格",
    sourceEn: "Zhipu model overview/pricing",
  },
  {
    provider: "GLM",
    model: "glm-4.5",
    status: "primary",
    rating: 4,
    officialZh: "官方提示后续应迁移到 GLM-4.7。",
    officialEn: "Official docs indicate migration toward GLM-4.7.",
    cadeggZh: "当前默认强模型；已接入，短期保留稳定性。",
    cadeggEn: "Current default strong model; kept for short-term integration stability.",
    sourceZh: "智谱模型概览",
    sourceEn: "Zhipu model overview",
  },
  {
    provider: "GLM",
    model: "glm-4.7-flashx / glm-4.5-airx / glm-4-flashx-250414",
    status: "light",
    rating: 3.5,
    officialZh: "FlashX/AirX 属于速度优先或轻量变体。",
    officialEn: "FlashX/AirX are speed-oriented or lightweight variants.",
    cadeggZh: "适合轻量问答；不建议独立承担复杂出图。",
    cadeggEn: "Good for light Q&A; not recommended as sole model for complex drawing.",
    sourceZh: "智谱模型概览/价格",
    sourceEn: "Zhipu model overview/pricing",
  },
  {
    provider: "GLM",
    model: "glm-4.5-flash / glm-4-flash-250414",
    status: "light",
    rating: 4,
    officialZh: "官方免费或免费 Flash 模型。",
    officialEn: "Official free or free Flash models.",
    cadeggZh: "可作为免费轻量入口；复杂 CAD 操作前应切强模型。",
    cadeggEn: "Free lightweight entry; switch to a strong model for complex CAD work.",
    sourceZh: "智谱免费模型/价格",
    sourceEn: "Zhipu free models/pricing",
  },
  {
    provider: "GLM",
    model: "glm-4.5-air",
    status: "legacy",
    rating: 3,
    officialZh: "旧版或特定场景 GLM 模型。",
    officialEn: "Older or scenario-specific GLM models.",
    cadeggZh: "保留兼容，不作为自动出图首选。",
    cadeggEn: "Kept for compatibility; not the first choice for autonomous drawing.",
    sourceZh: "智谱模型概览/价格",
    sourceEn: "Zhipu model overview/pricing",
  },
  {
    provider: "DeepSeek",
    model: "deepseek-v4-pro",
    status: "candidate",
    rating: 4,
    officialZh: "DeepSeek 当前高能力 API 模型。",
    officialEn: "Current high-capability DeepSeek API model.",
    cadeggZh: "强模型候选；需要做 CAD 工具调用回归。",
    cadeggEn: "Strong candidate; needs CAD tool-call regression.",
    sourceZh: "DeepSeek API 价格页",
    sourceEn: "DeepSeek API pricing",
  },
  {
    provider: "DeepSeek",
    model: "deepseek-v4-flash",
    status: "light",
    rating: 4.5,
    officialZh: "DeepSeek 当前轻量 API 模型。",
    officialEn: "Current lightweight DeepSeek API model.",
    cadeggZh: "适合低成本问答和失败重试备用。",
    cadeggEn: "Useful for lower-cost Q&A and fallback retries.",
    sourceZh: "DeepSeek API 价格页",
    sourceEn: "DeepSeek API pricing",
  },
  {
    provider: "Qwen",
    model: "qwen3.8-max / qwen3.7-max / qwen3-max",
    status: "candidate",
    rating: 4.5,
    officialZh: "阿里云百炼模型价格页列出的高能力模型。",
    officialEn: "High-capability models listed in Alibaba Cloud Model Studio pricing.",
    cadeggZh: "强模型候选；默认强模型保持 qwen3.8-max。",
    cadeggEn: "Strong candidates; default Qwen strong model remains qwen3.8-max.",
    sourceZh: "阿里云百炼模型价格",
    sourceEn: "Alibaba Cloud Model Studio pricing",
  },
  {
    provider: "Qwen",
    model: "qwen3.7-plus / qwen3.6-plus / qwen3.5-plus",
    status: "candidate",
    rating: 4,
    officialZh: "Plus 系列适合通用任务和结构化输出。",
    officialEn: "Plus models suit general tasks and structured output.",
    cadeggZh: "可用于规划和参数化说明；复杂出图仍需实测。",
    cadeggEn: "Useful for planning and parameterized explanations; complex drawing still needs tests.",
    sourceZh: "阿里云百炼模型价格",
    sourceEn: "Alibaba Cloud Model Studio pricing",
  },
  {
    provider: "Qwen",
    model: "qwen3.7-flash / qwen3.6-flash / qwen3.5-flash",
    status: "light",
    rating: 3,
    officialZh: "Flash/Turbo 为轻量或低延迟模型。",
    officialEn: "Flash/Turbo models are lightweight or low-latency options.",
    cadeggZh: "适合轻量问答、参数补全和兜底，不独立负责复杂出图。",
    cadeggEn: "Good for light Q&A, parameter completion, and fallback; not standalone for complex drawing.",
    sourceZh: "阿里云百炼模型价格",
    sourceEn: "Alibaba Cloud Model Studio pricing",
  },
  {
    provider: "Qwen",
    model: "qwen3-coder-plus / qwen3-coder-flash",
    status: "light",
    rating: 3.5,
    officialZh: "Coder 系列偏代码能力。",
    officialEn: "Coder models are code-oriented.",
    cadeggZh: "可辅助脚本/规则生成；默认仍限制任意 LISP 执行。",
    cadeggEn: "Can assist script/rule generation; arbitrary LISP execution remains restricted.",
    sourceZh: "阿里云百炼模型价格",
    sourceEn: "Alibaba Cloud Model Studio pricing",
  },
  {
    provider: "Kimi",
    model: "kimi-k3",
    status: "candidate",
    rating: 4,
    officialZh: "Kimi 旗舰模型，1M 上下文，面向长程编码和深度推理。",
    officialEn: "Kimi flagship model with 1M context for long-horizon coding and deep reasoning.",
    cadeggZh: "强模型候选；成本最高，默认不自动选，需先做 CAD 工具链回归。",
    cadeggEn: "Strong candidate; highest cost, not default, needs CAD tool-chain regression.",
    sourceZh: "Kimi 官网/帮助中心",
    sourceEn: "Kimi homepage/help center",
  },
  {
    provider: "Kimi",
    model: "kimi-k2.7-code",
    status: "candidate",
    rating: 4.5,
    officialZh: "Kimi Coding 模型，长上下文指令遵循更可靠。",
    officialEn: "Kimi coding model with more reliable long-context instruction following.",
    cadeggZh: "适合脚本、规则和 CAD 代码辅助；不默认执行任意 LISP。",
    cadeggEn: "Good for scripts, rules, and CAD code assistance; arbitrary LISP remains restricted.",
    sourceZh: "Kimi 官网",
    sourceEn: "Kimi homepage",
  },
  {
    provider: "Kimi",
    model: "kimi-k2.6",
    status: "candidate",
    rating: 4,
    officialZh: "Kimi API 当前模型列表中的新模型。",
    officialEn: "New model in the current Kimi API model list.",
    cadeggZh: "Kimi 默认强模型；需要 CAD 工具链回归。",
    cadeggEn: "Default Kimi strong model; needs CADEgg tool-chain regression.",
    sourceZh: "Kimi API 模型列表/价格",
    sourceEn: "Kimi API model list/pricing",
  },
  {
    provider: "Kimi",
    model: "kimi-k2.5",
    status: "light",
    rating: 3,
    officialZh: "Kimi API 当前兼容模型；旧版 moonshot-v1-* 已自动迁移到 k2.5/k2.6。",
    officialEn: "Current compatible Kimi API model; legacy moonshot-v1-* auto-migrate to k2.5/k2.6.",
    cadeggZh: "适合长上下文阅读和轻量问答。",
    cadeggEn: "Useful for long-context reading and light Q&A.",
    sourceZh: "Kimi API 模型列表/价格",
    sourceEn: "Kimi API model list/pricing",
  },
];

const MODEL_SOURCE_LINKS = [
  {
    labelZh: "智谱对话补全 API / 模型概览",
    labelEn: "Zhipu chat completions API / model overview",
    url: "https://docs.bigmodel.cn/api-reference/%E6%A8%A1%E5%9E%8B-api/%E5%AF%B9%E8%AF%9D%E8%A1%A5%E5%85%A8",
  },
  {
    labelZh: "DeepSeek API 模型与价格",
    labelEn: "DeepSeek API models and pricing",
    url: "https://api-docs.deepseek.com/zh-cn/quick_start/pricing/",
  },
  {
    labelZh: "阿里云百炼模型价格",
    labelEn: "Alibaba Cloud Model Studio pricing",
    url: "https://help.aliyun.com/zh/model-studio/model-pricing",
  },
  {
    labelZh: "Kimi API 开放平台",
    labelEn: "Kimi API platform",
    url: "https://platform.kimi.com/",
  },
];

function evidenceStatusLabel(status: EvidenceStatus, language: "zh-CN" | "en-US") {
  const labels: Record<EvidenceStatus, Record<"zh-CN" | "en-US", string>> = {
    primary: { "zh-CN": "当前默认", "en-US": "Default" },
    candidate: { "zh-CN": "强模型候选", "en-US": "Strong Candidate" },
    light: { "zh-CN": "轻量/免费/兼容", "en-US": "Light/Free/Compat" },
    legacy: { "zh-CN": "旧版保留", "en-US": "Legacy" },
  };
  return labels[status][language];
}

function evidenceProviderId(provider: string): Provider | null {
  const normalized = provider.toLowerCase();
  if (normalized === "glm" || normalized === "deepseek" || normalized === "qwen" || normalized === "kimi") {
    return normalized;
  }
  return null;
}

function evidenceRowRating(row: ModelEvidenceRow): ModelRating {
  const providerId = evidenceProviderId(row.provider);
  const sourceModelId = row.model.split("/")[0]?.trim();
  if (providerId && sourceModelId) {
    const rating = modelRatingById(providerId, sourceModelId);
    if (rating) return rating;
  }
  return row.rating;
}

function StarRating({ rating }: { rating: ModelRating }) {
  return (
    <span className="star-rating" aria-label={`${rating}/5`}>
      {ratingStarStates(rating).map((state, index) => (
        <span className={`rating-star ${state}`} key={index} aria-hidden="true">
          ★
        </span>
      ))}
    </span>
  );
}

function HelpPanel({ language, onClose }: HelpPanelProps) {
  const isZh = language === "zh-CN";

  return (
    <div className="modal-backdrop">
      <section className="settings-modal glass-modal help-panel" role="dialog" aria-modal="true">
        <ModalHeader
          title={isZh ? "帮助与说明" : "Help & Guide"}
          onClose={onClose}
          closeLabel={t("closeWindow", language)}
        />

        <div className="settings-content">
          {/* Model Evaluation */}
          <section className="settings-group">
            <GroupHeader
              title={isZh ? "模型接入评估" : "Model Integration Evaluation"}
              desc={isZh
                ? "星级是综合结论：优先看能否发挥 CADEgg 全功能，再看性价比、工具调用/结构化可靠性、上下文/速度和当前接入成熟度。"
                : "Stars are a combined conclusion: CADEgg feature coverage first, then cost-performance, tool/structured-output reliability, context/speed, and current integration maturity."}
            />

            <div className="help-table-wrap">
              <table className="help-table model-evidence-table">
                <thead>
                  <tr>
                    <th>{isZh ? "供应商" : "Provider"}</th>
                    <th>{isZh ? "模型" : "Model"}</th>
                    <th>{isZh ? "状态" : "Status"}</th>
                    <th>{isZh ? "星级" : "Stars"}</th>
                    <th>{isZh ? "官方依据" : "Official Basis"}</th>
                    <th>{isZh ? "CADEgg 建议" : "CADEgg Use"}</th>
                    <th>{isZh ? "来源" : "Source"}</th>
                  </tr>
                </thead>
                <tbody>
                  {MODEL_EVIDENCE_TABLE.map((row) => (
                    <tr key={`${row.provider}-${row.model}`} className={`evidence-${row.status}`}>
                      <td>{row.provider}</td>
                      <td><strong>{row.model}</strong></td>
                      <td>
                        <span className={`model-status ${row.status}`}>
                          {evidenceStatusLabel(row.status, language)}
                        </span>
                      </td>
                      <td><StarRating rating={evidenceRowRating(row)} /></td>
                      <td>{isZh ? row.officialZh : row.officialEn}</td>
                      <td>{isZh ? row.cadeggZh : row.cadeggEn}</td>
                      <td className="source-cell">{isZh ? row.sourceZh : row.sourceEn}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <p className="settings-note">
              {isZh
                ? "说明：模型列表里的星级已于 2026-08-18 按 CADEgg 实测基准（38 个模型 × 固定 6 用例）回写；权重为工具调用可靠性 25% · CAD/规范准确性 25% · 稳定性 15% · 速度 15% · 成本 10% · 长上下文 10%。上表保留官方资料与接入定位。"
                : "Note: the model-list stars were rewritten on 2026-08-18 from CADEgg's measured benchmark (38 models × 6 fixed cases). Weights: tool reliability 25% · CAD/standard accuracy 25% · stability 15% · speed 15% · cost 10% · long context 10%. The table above keeps official docs and integration positioning."}
            </p>
            <div className="help-guide benchmark-evidence-guide">
              <h4>{isZh ? "实测基准要点（2026-08-18）" : "Measured Benchmark Highlights (2026-08-18)"}</h4>
              <ul>
                {isZh ? (
                  <>
                    <li>实测最高分：GLM-5.2（0.878）、通义千问 3-Coder-Plus（0.867）、GLM-4-Flash-250414（0.854）。</li>
                    <li>速度最快：Kimi moonshot 系列约 0.9s/请求；GLM-4.5 最慢（约 9-10s），GLM-4.5-Flash 平均 36s。</li>
                    <li>工具 JSON 与多轮接续普遍稳定；「缺参时是否先追问」仍是所有模型的共同弱项（产品侧会注入知识卡允许按规范默认值出图）。</li>
                    <li>Kimi k2/k3 系列仅接受 temperature=1：基准已自动兼容；Kimi 账户限额（RPM）从 Tier0 升级 Tier1 后不再限流。</li>
                    <li>完整报告：记忆目录 benchmark-results.json / benchmark-results.md。</li>
                  </>
                ) : (
                  <>
                    <li>Top scores: GLM-5.2 (0.878), Qwen 3-Coder-Plus (0.867), GLM-4-Flash-250414 (0.854).</li>
                    <li>Fastest: Kimi moonshot series ≈0.9s/request; GLM-4.5 is slowest (≈9-10s), GLM-4.5-Flash averaged 36s.</li>
                    <li>Tool JSON and multi-turn continuity are generally stable; "clarify missing params first" remains the common weak spot (the product injects knowledge cards allowing standard defaults).</li>
                    <li>Kimi k2/k3 accept only temperature=1: the benchmark auto-falls back. Kimi rate limits no longer throttle after Tier0→Tier1 upgrade.</li>
                    <li>Full report: benchmark-results.json / benchmark-results.md in the memory folder.</li>
                  </>
                )}
              </ul>
            </div>
            <div className="model-source-links">
              <strong>{isZh ? "官方来源" : "Official Sources"}</strong>
              {MODEL_SOURCE_LINKS.map((source) => (
                <a key={source.url} href={source.url} target="_blank" rel="noreferrer">
                  {isZh ? source.labelZh : source.labelEn}
                </a>
              ))}
            </div>
          </section>

          {/* Product Direction */}
          <section className="settings-group">
            <GroupHeader
              title={isZh ? "定位与后续路线" : "Positioning & Roadmap"}
              desc={isZh
                ? "CADEgg 的重点不是把固定 CAD 命令换成大模型，而是把语言、规范、图纸状态和可追溯校核连成工程闭环。"
                : "CADEgg should not replace fixed CAD commands with an LLM; its value is connecting language, standards, drawing state, and traceable validation into an engineering loop."}
            />

            <div className="help-guide">
              <h4>{isZh ? "边界判断" : "Boundary"}</h4>
              <ul>
                <li>{isZh
                  ? "固定参数化绘图、常用尺寸和批量命令，天正类成熟插件通常更高效，CADEgg 不应把这类简单工作变贵。"
                  : "For fixed parametric drafting, common dimensions, and batch commands, mature Tianzheng-style CAD plugins are usually more efficient. CADEgg should not make simple work more expensive."}</li>
                <li>{isZh
                  ? "CADEgg 应只在语言意图不完整、规范约束需要检索、图纸现状需要检查、修改意见需要转译、过程需要留痕时使用模型。"
                  : "CADEgg should use models where intent is incomplete, standards must be retrieved, drawing state must be inspected, review comments must be translated into actions, or the process needs an audit trail."}</li>
                <li>{isZh
                  ? "模型负责理解、规划、解释和校核；实际绘图尽量落到确定性的 CAD 工具、知识卡和规则验证。"
                  : "The model should handle understanding, planning, explanation, and validation, while drawing execution should land on deterministic CAD tools, knowledge cards, and rule checks."}</li>
              </ul>

              <h4>{isZh ? "优先路线" : "Priority Route"}</h4>
              <ol>
                <li>{isZh
                  ? "施工安全规范闭环：缺参追问、知识卡命中、出图、规则校核、报告。"
                  : "Construction-safety standards loop: ask for missing parameters, hit knowledge cards, draw, validate, and report."}</li>
                <li>{isZh
                  ? "已有图纸检查：读取对象分布，输出问题清单和可执行修复建议。"
                  : "Existing drawing inspection: read object distribution, then produce issue lists and actionable fixes."}</li>
                <li>{isZh
                  ? "自然语言改图：把审图意见转成按 handle 可追踪的移动、删除、标注和重画。"
                  : "Natural-language revision: turn review comments into handle-traceable move, delete, annotate, and redraw operations."}</li>
                <li>{isZh
                  ? "项目记忆和规则包：把常用图层、单位、项目做法、规范资料做成本地可维护资料。"
                  : "Project memory and rule packs: keep layers, units, project conventions, and standard documents as local maintainable data."}</li>
                <li>{isZh
                  ? "批量 QA 和交付：多图批检、导出 Markdown/报告、记录模型和耗时数据。"
                  : "Batch QA and delivery: inspect multiple drawings, export Markdown/reports, and record model and duration telemetry."}</li>
                <li>{isZh
                  ? "完成安全场景闭环后，再扩展到更广的工程图纸工作流。"
                  : "After the safety workflow is closed, expand into broader engineering drawing workflows."}</li>
              </ol>
            </div>
          </section>

          {/* Usage Guide */}
          <section className="settings-group">
            <GroupHeader
              title={isZh ? "使用说明" : "Usage Guide"}
              desc={isZh
                ? "CADEgg 是 Windows 桌面 AutoCAD 建筑施工安全助手。当前主攻电梯井口临边防护闭环。"
                : "CADEgg is a Windows desktop AutoCAD construction safety assistant. Currently focused on elevator shaft edge protection."}
            />

            <div className="help-guide">
              <h4>{isZh ? "快速开始" : "Quick Start"}</h4>
              <ol>
                <li>{isZh ? "在设置中配置 API Key（推荐智谱 GLM 或 DeepSeek）" : "Configure API Key in Settings (GLM or DeepSeek recommended)"}</li>
                <li>{isZh ? "在聊天输入框选择模型供应商和具体模型" : "Select model provider and specific model in the chat input area"}</li>
                <li>{isZh ? "输入绘图指令，如「画电梯井口防护，井口宽 2000，高 1800」" : "Enter drawing commands, e.g. 'Draw elevator shaft protection, opening width 2000, height 1800'"}</li>
                <li>{isZh ? "Agent 会自动选择工具、出图并校核" : "The agent will automatically select tools, draw, and validate"}</li>
              </ol>

              <h4>{isZh ? "支持的绘图指令" : "Supported Drawing Commands"}</h4>
              <ul>
                <li>{isZh ? "电梯井口防护门（含防护门扇、踢脚板、警示牌、材料表）" : "Elevator shaft protection door (with door panels, toe board, warning sign, material table)"}</li>
                <li>{isZh ? "基础绘图：直线、圆、矩形、正多边形" : "Basic drawing: line, circle, rectangle, regular polygon"}</li>
                <li>{isZh ? "楼梯：双跑楼梯（含踏步、休息平台、箭头标注）" : "Stairs: double-flight stair (with steps, landing, arrow annotation)"}</li>
              </ul>

              <h4>{isZh ? "知识卡机制" : "Knowledge Card System"}</h4>
              <p>{isZh
                ? "CADEgg 内置建筑施工安全知识卡，基于住建部官方规范（JGJ 80-2016、建办质函〔2019〕90号）。Agent 出图时自动检索知识卡，按规范定值画图，不会自由发挥。井口宽/高是现场实测值，所以 Agent 会追问确认；防护门高（1.5m）和踢脚板（200mm）是规范定值，直接用不追问。"
                : "CADEgg has built-in construction safety knowledge cards based on official Chinese building codes. The agent retrieves knowledge cards when drawing, using standard values without improvisation. Opening width/height are site-measured values, so the agent will ask for confirmation; guard door height (1.5m) and toe board (200mm) are code-specified and used directly."}</p>

              <h4>{isZh ? "模型选择建议" : "Model Selection Tips"}</h4>
              <ul>
                <li>{isZh ? "★★★★★：优先用于出图、校核和多工具规划。" : "★★★★★: preferred for drawing, validation, and multi-tool planning."}</li>
                <li>{isZh ? "★★★★：可作为强模型备用，复杂任务建议保留人工复核。" : "★★★★: usable as strong fallbacks; keep human review for complex work."}</li>
                <li>{isZh ? "★★★ 及以下：用于普通问答、低风险辅助或旧配置兼容。" : "★★★ and below: use for general Q&A, low-risk assistance, or legacy compatibility."}</li>
                <li>{isZh ? "推理模型不默认进入工具调用链，避免只思考不执行。" : "Reasoning-only models are not default tool-chain candidates to avoid thinking without action."}</li>
              </ul>

              <h4>{isZh ? "自动轮转" : "Auto Failover"}</h4>
              <p>{isZh
                ? "聊天输入栏和设置页都可以开关自动轮转。开启时按当前模型 → 同供应商备用模型 → 其他已配置供应商切换；关闭时只使用当前会话选择的模型。"
                : "Auto failover can be toggled from the composer or Settings. When on, CADEgg tries the current model, same-provider fallback, then other configured providers; when off, only the selected session model is used."}</p>
            </div>
          </section>
        </div>
      </section>
    </div>
  );
}
