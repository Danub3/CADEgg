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

import "./App.css";
import type {
  AgentEvent,
  DemoLogEntry,
  ElevatorValidation,
  Message,
  ObjectUpdate,
  Provider,
  SessionObject,
  SettingsView,
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
  notifications: boolean;
  autoSyncObjects: boolean;
  alwaysOnTop: boolean;
  reduceMotion: boolean;
  densePanels: boolean;
}

const DEFAULT_APP_PREFERENCES: AppPreferences = {
  language: "zh-CN",
  fontSize: 14,
  storageLocation: "appdata",
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
  validationFailed: { "zh-CN": "安全校核未通过", "en-US": "Safety Check Failed" },
  riskItems: { "zh-CN": "风险项：{items}", "en-US": "Risks: {items}" },
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
    "zh-CN": "拖动时仅预览数值，点击保存后应用到会话文字和输入区。",
    "en-US": "Drag to preview; saving applies it to messages and the composer.",
  },
  storageLabel: { "zh-CN": "存储位置", "en-US": "Storage Location" },
  storageHint: {
    "zh-CN": "模型 Key 和模型配置由 Rust 后端保存；当前使用系统 AppData，避免把密钥写入项目目录。",
    "en-US": "Keys and model settings are stored by the Rust backend in system AppData.",
  },
  storageAppData: { "zh-CN": "系统 AppData（推荐）", "en-US": "System AppData (Recommended)" },
  storageProject: {
    "zh-CN": "项目目录（仅记录偏好，后端暂不迁移密钥）",
    "en-US": "Project directory (preference only; keys stay in AppData)",
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
  baseUrlLabel: { "zh-CN": "API Base URL", "en-US": "API Base URL" },
  baseUrlHint: {
    "zh-CN": "兼容 OpenAI /chat/completions 的官方或中转地址。",
    "en-US": "Official or relay endpoint compatible with OpenAI /chat/completions.",
  },
  cheapModelLabel: { "zh-CN": "轻量模型", "en-US": "Cheap Model" },
  strongModelLabel: { "zh-CN": "强模型", "en-US": "Strong Model" },
  settingsKeyNote: {
    "zh-CN": "API Key 仅保存在本机 AppData/settings.json。界面不会明文回显已保存的 key。",
    "en-US": "API keys are stored only in local AppData/settings.json and are never shown in full.",
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
  "glm-5.3": { "zh-CN": "GLM-5.3", "en-US": "GLM-5.3" },
  "glm-5.2": { "zh-CN": "GLM-5.2", "en-US": "GLM-5.2" },
  "glm-5.1": { "zh-CN": "GLM-5.1", "en-US": "GLM-5.1" },
  "glm-5-plus": { "zh-CN": "GLM-5-Plus", "en-US": "GLM-5-Plus" },
  "glm-5-turbo": { "zh-CN": "GLM-5-Turbo", "en-US": "GLM-5-Turbo" },
  "glm-4.7": { "zh-CN": "GLM-4.7", "en-US": "GLM-4.7" },
  "glm-4.7-flashx": { "zh-CN": "GLM-4.7-FlashX", "en-US": "GLM-4.7-FlashX" },
  "glm-4.7-flash": { "zh-CN": "GLM-4.7-Flash", "en-US": "GLM-4.7-Flash" },
  "glm-4.6": { "zh-CN": "GLM-4.6", "en-US": "GLM-4.6" },
  "glm-4.5": { "zh-CN": "GLM-4.5（付费）", "en-US": "GLM-4.5 (Paid)" },
  "glm-4.5-air": { "zh-CN": "GLM-4.5-Air", "en-US": "GLM-4.5-Air" },
  "glm-4.5-airx": { "zh-CN": "GLM-4.5-AirX", "en-US": "GLM-4.5-AirX" },
  "glm-4.5-flash": { "zh-CN": "GLM-4.5-Flash（免费）", "en-US": "GLM-4.5-Flash (Free)" },
  "glm-4-plus": { "zh-CN": "GLM-4-Plus（付费）", "en-US": "GLM-4-Plus (Paid)" },
  "glm-4-air-250414": { "zh-CN": "GLM-4-Air-250414", "en-US": "GLM-4-Air-250414" },
  "glm-4-long": { "zh-CN": "GLM-4-Long", "en-US": "GLM-4-Long" },
  "glm-4-flash-250414": { "zh-CN": "GLM-4-Flash-250414", "en-US": "GLM-4-Flash-250414" },
  "glm-4-flashx-250414": { "zh-CN": "GLM-4-FlashX-250414", "en-US": "GLM-4-FlashX-250414" },
  "glm-4-flash": { "zh-CN": "GLM-4-Flash（旧版）", "en-US": "GLM-4-Flash (Legacy)" },
  "deepseek-v4-pro": { "zh-CN": "DeepSeek V4 Pro", "en-US": "DeepSeek V4 Pro" },
  "deepseek-v4-flash": { "zh-CN": "DeepSeek V4 Flash", "en-US": "DeepSeek V4 Flash" },
  "deepseek-chat": { "zh-CN": "DeepSeek Chat", "en-US": "DeepSeek Chat" },
  "qwen3.8-max": { "zh-CN": "通义千问 3.8 Max", "en-US": "Qwen 3.8 Max" },
  "qwen3.8-max-preview": { "zh-CN": "通义千问 3.8 Max Preview", "en-US": "Qwen 3.8 Max Preview" },
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
  "qwen-flash": { "zh-CN": "通义千问 Flash", "en-US": "Qwen Flash" },
  "qwen-max": { "zh-CN": "通义千问 Max（旧版）", "en-US": "Qwen Max (Legacy)" },
  "qwen-plus": { "zh-CN": "通义千问 Plus", "en-US": "Qwen Plus" },
  "qwen-turbo": { "zh-CN": "通义千问 Turbo", "en-US": "Qwen Turbo" },
  "kimi-k3": { "zh-CN": "Kimi K3", "en-US": "Kimi K3" },
  "kimi-k2.7-code": { "zh-CN": "Kimi K2.7 Code", "en-US": "Kimi K2.7 Code" },
  "kimi-k2.7-code-highspeed": { "zh-CN": "Kimi K2.7 Code Highspeed", "en-US": "Kimi K2.7 Code Highspeed" },
  "kimi-k2.6": { "zh-CN": "Kimi K2.6", "en-US": "Kimi K2.6" },
  "kimi-k2.5": { "zh-CN": "Kimi K2.5", "en-US": "Kimi K2.5" },
  "moonshot-v1-8k": { "zh-CN": "Kimi 8K（旧版）", "en-US": "Kimi 8K (Legacy)" },
  "moonshot-v1-32k": { "zh-CN": "Kimi 32K（旧版）", "en-US": "Kimi 32K (Legacy)" },
  "moonshot-v1-128k": { "zh-CN": "Kimi 128K（旧版）", "en-US": "Kimi 128K (Legacy)" },
};

function providerDisplay(provider: Provider, lang: "zh-CN" | "en-US", kind: "label" | "short") {
  return PROVIDER_UI[provider]?.[lang]?.[kind] ?? providerMeta(provider)[kind === "short" ? "shortLabel" : "label"];
}

function modelDisplay(model: ModelOption, lang: "zh-CN" | "en-US") {
  return MODEL_UI[model.id]?.[lang] ?? model.label;
}

function starsForRating(rating: ModelRating) {
  return `${"★".repeat(rating)}${"☆".repeat(5 - rating)}`;
}

function modelStarDisplay(model: ModelOption) {
  return starsForRating(modelRating(model));
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
}

interface StoredChatSessions {
  activeSessionId: string;
  sessions: ChatSession[];
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

function createChatSession(settings: SettingsView): ChatSession {
  const now = Date.now();
  return {
    id: makeId("session"),
    title: "新会话",
    createdAt: now,
    updatedAt: now,
    provider: settings.provider,
    model: currentModelFor(settings),
    messages: [],
    sessionObjects: [],
    demoLog: [],
    lastValidation: null,
    lastDrawParams: null,
  };
}

function sessionTitleFromMessages(messages: Message[]) {
  const lastUserMessage = [...messages].reverse().find((message) => message.role === "user");
  if (!lastUserMessage || !("content" in lastUserMessage)) return "新会话";
  return lastUserMessage.content.replace(/\s+/g, " ").trim().slice(0, 28) || "新会话";
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
    ...(typeof fontSize === "number" ? { "--content-font-size": `${fontSize}px` } : {}),
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
    fontSize: clamp(Number(value.fontSize ?? DEFAULT_APP_PREFERENCES.fontSize), 12, 18),
    storageLocation: value.storageLocation === "project" ? "project" : "appdata",
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
  const [activeSessionId, setActiveSessionId] = useState(initialSession.id);

  const [input, setInput] = useState("");
  const [messages, setMessages] = useState<Message[]>(initialSession.messages);
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

  const [testStatus, setTestStatus] = useState<{ ok: boolean; msg: string } | null>(null);
  const [undoing, setUndoing] = useState(false);
  const [syncingObjects, setSyncingObjects] = useState(false);
  const [importingSelection, setImportingSelection] = useState(false);

  const [demoLog, setDemoLog] = useState<DemoLogEntry[]>(initialSession.demoLog);
  const [lastValidation, setLastValidation] = useState<ElevatorValidation | null>(
    initialSession.lastValidation
  );
  const [lastDrawParams, setLastDrawParams] = useState<Record<string, unknown> | null>(
    initialSession.lastDrawParams
  );

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
  const [completedToolIds, setCompletedToolIds] = useState<Set<string>>(new Set());
  const lastUserInputRef = useRef("");
  const pendingLogRef = useRef<{
    toolCalls: string[];
    params: Record<string, unknown>;
    validation: ElevatorValidation | null;
    summary: string;
  } | null>(null);

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
    setSessions((prev) =>
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
        assistantDraftRef.current += e.delta;
        setAssistantDraft(assistantDraftRef.current);
      } else if (e.kind === "assistant") {
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
        setMessages((prev) => [
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
            setLastValidation(validation);
            if (pendingLogRef.current) {
              pendingLogRef.current.validation = validation;
            }
          }
        }
        if (e.result.name === "draw_elevator_shaft_protection") {
          if (e.result.ok) {
            const call = pendingToolCallsRef.current[e.result.id];
            if (call) setLastDrawParams(call.args);
          }
          if (pendingLogRef.current) {
            pendingLogRef.current.summary = e.result.ok
              ? e.result.content
              : `绘图失败：${e.result.content}`;
          }
        }
        setMessages((prev) => [
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
        commitUndoSnapshotIfNeeded();
        const pending = pendingLogRef.current;
        pendingLogRef.current = null;
        if (pending && pending.toolCalls.length > 0) {
          if (!e.text.trim()) {
            const summary = completionSummary(pending, appPreferencesRef.current.language);
            setMessages((prev) => [...prev, { role: "assistant", text: summary, tool_calls: [] }]);
          }
          const entry: DemoLogEntry = {
            time: new Date().toLocaleTimeString("zh-CN", { hour12: false }),
            user_input: lastUserInputRef.current,
            tool_calls: pending.toolCalls,
            params: pending.params,
            validation: pending.validation ?? undefined,
            summary: pending.summary || "完成",
          };
          setDemoLog((prev) => [entry, ...prev].slice(0, 30));
        }
        const shouldPostRunSync = pendingPostRunSyncRef.current;
        pendingPostRunSyncRef.current = false;
        if (shouldPostRunSync) {
          void (async () => {
            await syncSessionObjects(false);
            setSending(false);
          })();
        } else {
          setSending(false);
        }
      } else if (e.kind === "error") {
        if (assistantDraftRef.current) {
          assistantDraftRef.current = "";
          setAssistantDraft("");
        }
        commitUndoSnapshotIfNeeded();
        pendingPostRunSyncRef.current = false;
        setErrorMsg(e.message);
        setSending(false);
        if (e.message.includes("API Key")) setView("settings");
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

    const newHistory: Message[] = [...messages, { role: "user", content: text }];
    setMessages(newHistory);
    setInput("");
    assistantDraftRef.current = "";
    setAssistantDraft("");
    setSending(true);
    setErrorMsg(null);
    completedToolIdsRef.current = new Set();
    setCompletedToolIds(new Set());

    try {
      let syncedObjects = sessionObjectsRef.current;
      if (syncedObjects.length > 0) {
        const latest = await syncSessionObjects(false);
        if (latest === null) {
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
      };

      // run_agent emits agent:event for each step; resolve only means the backend loop ended.
      await invoke("run_agent", {
        userInput: text,
        history: buildHistoryPayload(messages),
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
    setTestStatus({ ok: true, msg: "执行中..." });
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
      setMessages((prev) => [...prev, { role: "assistant", text: msg, tool_calls: [] }]);
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
    setErrorMsg(null);
    try {
      const result = await invoke<{
        id: string;
        name: string;
        ok: boolean;
        content: string;
        confirmation_required: boolean;
        object_updates: ObjectUpdate[];
      }>("confirm_tool_call", { call });

      setMessages((prev) =>
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
    } catch (e) {
      setErrorMsg(String(e));
    } finally {
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
    sessionObjectsRef.current = session.sessionObjects;
    setActiveSessionId(session.id);
    setMessages(session.messages);
    setAssistantDraft("");
    setSessionObjects(session.sessionObjects);
    setDemoLog(session.demoLog);
    setLastValidation(session.lastValidation);
    setLastDrawParams(session.lastDrawParams);
    setInput("");
    setErrorMsg(null);
  }

  function handleNewConversation() {
    if (sending) return;
    const session = createChatSession(settings);
    setSessions((prev) => [session, ...prev]);
    applySession(session);
  }

  function handleSelectSession(id: string) {
    if (sending || id === activeSessionId) return;
    const session = sessions.find((item) => item.id === id);
    if (session) applySession(session, true);
  }

  function handleDeleteSession(id: string) {
    if (sending) return;
    const remaining = sessions.filter((session) => session.id !== id);
    if (remaining.length > 0) {
      setSessions(remaining);
      if (id === activeSessionId) applySession(remaining[0], true);
      return;
    }
    const next = createChatSession(settings);
    setSessions([next]);
    applySession(next);
  }

  async function handleModelChange(provider: Provider, model: string) {
    const nextModel = normalizeModelForProvider(provider, model, currentModelFor(settings, provider));
    setSessions((prev) =>
      prev.map((s) =>
        s.id === activeSessionId
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

  const activeSession = sessions.find((session) => session.id === activeSessionId);
  const activeProvider = normalizeProvider(activeSession?.provider ?? settings.provider);
  const activeModel = normalizeModelForProvider(
    activeProvider,
    activeSession?.model,
    currentModelFor(settings, activeProvider)
  );
  const selectedProviderMeta = providerMeta(activeProvider);
  const providerLabel = providerDisplay(activeProvider, appPreferences.language, "short");
  const bridgeState =
    testStatus === null ? "idle" : testStatus.ok ? "online" : "error";
  const bridgeLabel =
    bridgeState === "online" ? t("bridgeOnline", appPreferences.language) : bridgeState === "error" ? t("bridgeError", appPreferences.language) : t("bridgeIdle", appPreferences.language);
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

        <div className="session-title drag-zone" title={sessionTitle} onMouseDown={handleWindowDrag}>
          {sessionTitle}
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
            {sessions.map((session) => (
              <button
                type="button"
                className={`session-card ${session.id === activeSessionId ? "active" : ""}`}
                onClick={() => handleSelectSession(session.id)}
                key={session.id}
              >
                <strong>{session.title}</strong>
                <span>
                  {providerDisplay(session.provider, appPreferences.language, "short")} ·{" "}
                  {session.model || t("defaultModel", appPreferences.language)}
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
            ))}
          </div>

          <div className="sidebar-spacer" />

          <button type="button" className="sidebar-settings" onClick={() => setView("settings")}>
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
                onOpenSettings={() => setView("settings")}
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
              onOpenSettings={() => setView("settings")}
              language={appPreferences.language}
            />
            <div className="composer-hint">{t("composerHint", appPreferences.language)}</div>
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
            language={appPreferences.language}
          />
          <DrawResultCard lastDrawParams={lastDrawParams} language={appPreferences.language} />
          <ValidationCard lastValidation={lastValidation} language={appPreferences.language} />
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
          onClose={() => setView("chat")}
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
  const hasPreset = meta.models.some((item) => item.id === model);

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

      <select
        aria-label={t("sessionModelAria", language)}
        value={hasPreset ? model : "__custom"}
        onChange={(e) => {
          if (e.target.value !== "__custom") {
            void onModelChange(provider, e.target.value);
          }
        }}
      >
        {!hasPreset && <option value="__custom">{model}</option>}
        {meta.models.map((item) => (
          <option key={item.id} value={item.id}>
            {modelDisplay(item, language)} · {modelStarDisplay(item)}
          </option>
        ))}
      </select>

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

function GenerationActionsCard({
  undoing,
  sending,
  syncingObjects,
  handleUndoLastGeneration,
  language,
}: {
  undoing: boolean;
  sending: boolean;
  syncingObjects: boolean;
  handleUndoLastGeneration: () => Promise<void>;
  language: "zh-CN" | "en-US";
}) {
  return (
    <section className="rail-card action-card">
      <PanelHeader title={t("actionCardTitle", language)} />
      <p>{t("actionCardDesc", language)}</p>
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
  return (
    <section className="debug-panel">
      <PanelHeader title={t("cadDebugTitle", language)} status={testStatus?.ok ? "online" : "idle"} />
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

  return (
    <section className={`rail-card validation-card ${lastValidation.ok ? "valid" : "invalid"}`}>
      <PanelHeader title={t("validationTitle", language)} status={lastValidation.ok ? "online" : "error"} />
      <strong>{lastValidation.ok ? t("validationPassed", language) : t("validationFailed", language)}</strong>
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

  useEffect(() => {
    setDraftAppPreferences(normalizeAppPreferences(appPreferences));
  }, [appPreferences]);

  useEffect(() => {
    const next = normalizeGlassSettings(glassSettings);
    previewGlassSettingsRef.current = next;
    setDraftGlassSettings(next);
  }, [glassSettings]);

  async function handleSaveAll() {
    const nextAppPreferences = normalizeAppPreferences(draftAppPreferences);
    setAppPreferences(nextAppPreferences);
    await saveSettings();
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
          onClose={onClose}
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
                min={12}
                max={18}
                value={draftAppPreferences.fontSize}
                suffix="px"
                onCommit={(fontSize) =>
                  setDraftAppPreferences((prev) => ({
                    ...prev,
                    fontSize,
                  }))
                }
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
            <div className="provider-settings-list">
              {MODEL_PROVIDERS.map((provider) => {
                const keyDraft = keyDraftFor(provider.id);
                const baseUrl = String(settings[provider.baseUrlField] ?? "");
                const cheapModel = String(settings[provider.cheapModelField] ?? "");
                const strongModel = String(settings[provider.strongModelField] ?? "");
                const keySet = Boolean(settings[provider.keySetField]);
                const keyPreview = String(settings[provider.keyPreviewField] ?? "");

                return (
                  <section className="provider-settings-card" key={provider.id}>
                    <GroupHeader
                      title={providerDisplay(provider.id, language, "label")}
                      desc={t("modelCardDesc", language)}
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
                      <select
                        className={inputCls}
                        value={cheapModel}
                        onChange={(e) =>
                          setSettings((prev) => ({
                            ...prev,
                            [provider.cheapModelField]: e.target.value,
                          }))
                        }
                      >
                        {provider.models.map((model) => (
                          <option key={model.id} value={model.id}>
                            {modelDisplay(model, language)} · {modelStarDisplay(model)}
                          </option>
                        ))}
                      </select>
                    </Field>
                    <Field label={t("strongModelLabel", language)}>
                      <select
                        className={inputCls}
                        value={strongModel}
                        onChange={(e) =>
                          setSettings((prev) => ({
                            ...prev,
                            [provider.strongModelField]: e.target.value,
                          }))
                        }
                      >
                        {provider.models.map((model) => (
                          <option key={model.id} value={model.id}>
                            {modelDisplay(model, language)} · {modelStarDisplay(model)}
                          </option>
                        ))}
                      </select>
                    </Field>
                  </section>
                );
              })}
            </div>

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
          <button type="button" className="outline-action" onClick={onClose}>
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

function validationSummary(validation: ElevatorValidation, language: "zh-CN" | "en-US") {
  if (language === "en-US") {
    if (validation.ok) {
      return "Validation passed: dimensions, guard height, toe board, warning sign, and material table are checked.";
    }
    const issues = validation.issues.slice(0, 3).join(", ");
    return `Validation failed: ${issues || "see safety panel"}.`;
  }

  if (validation.ok) {
    return "校核通过：井口尺寸、防护门高度、踢脚板、警示牌和材料表已检查。";
  }
  const issues = validation.issues.slice(0, 3).join("、");
  return `校核未通过：${issues || "请查看右侧安全校核面板"}。`;
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

const MODEL_EVAL_TABLE = [
  { model: "GLM-5.3 / GLM-5.2", score: "9.3", rating: 5 as ModelRating, noteZh: "强规划与工具调用，适合复杂出图和复核", noteEn: "Strong planning and tool calling for complex drawing and review" },
  { model: "GLM-4.5", score: "8.8", rating: 5 as ModelRating, noteZh: "当前已打通，函数调用稳定，适合作为默认强模型", noteEn: "Currently verified, stable function calling, good default strong model" },
  { model: "DeepSeek V4 Pro / Chat", score: "8.7", rating: 5 as ModelRating, noteZh: "工具选择和多步编排表现好，成本友好", noteEn: "Good tool selection and multi-step orchestration, cost friendly" },
  { model: "Qwen 3.8 Max / 3 Max", score: "8.5", rating: 5 as ModelRating, noteZh: "结构化输出强，适合规划和参数化任务", noteEn: "Strong structured output for planning and parametric tasks" },
  { model: "Kimi K3 / K2.7 Code", score: "8.2", rating: 4 as ModelRating, noteZh: "代码与长上下文能力好，工具链仍需实测监督", noteEn: "Good coding and long-context ability; tool chain still needs supervision" },
  { model: "GLM-4.7 / GLM-4.6", score: "8.0", rating: 4 as ModelRating, noteZh: "可作为 GLM-4.5 的升级候选或备用强模型", noteEn: "Good candidates as GLM-4.5 upgrades or strong fallbacks" },
  { model: "GLM Air / FlashX / Qwen Flash", score: "7.0", rating: 3 as ModelRating, noteZh: "适合轻量问答和低风险辅助，不建议独立承担复杂出图", noteEn: "Useful for light Q&A and low-risk assistance, not ideal for complex drawing alone" },
  { model: "Moonshot v1 / 旧版 Flash", score: "5.5", rating: 2 as ModelRating, noteZh: "保留兼容入口，不建议用于 agent 自动出图", noteEn: "Kept for compatibility; not recommended for autonomous drawing" },
];

function StarRating({ rating }: { rating: ModelRating }) {
  return (
    <span className={`star-rating rating-${rating}`} aria-label={`${rating}/5`}>
      {starsForRating(rating)}
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
              title={isZh ? "模型能力评估" : "Model Capability Evaluation"}
              desc={isZh
                ? "基于公开函数调用能力、当前接入状态和 CADEgg 工具链风险分层。五星优先用于强模型，三星以内只适合轻量或兼容场景。"
                : "Based on public function-calling ability, current integration status, and CADEgg tool-chain risk. Five stars are preferred for strong models; three or fewer are for light or compatibility use."}
            />

            <div className="help-table-wrap">
              <table className="help-table">
                <thead>
                  <tr>
                    <th>{isZh ? "模型" : "Model"}</th>
                    <th>{isZh ? "评分" : "Score"}</th>
                    <th>{isZh ? "星级" : "Stars"}</th>
                    <th>{isZh ? "说明" : "Notes"}</th>
                  </tr>
                </thead>
                <tbody>
                  {MODEL_EVAL_TABLE.map((row) => (
                    <tr key={row.model} className={`eval-rating-${row.rating}`}>
                      <td><strong>{row.model}</strong></td>
                      <td>{row.score}</td>
                      <td><StarRating rating={row.rating} /></td>
                      <td>{isZh ? row.noteZh : row.noteEn}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
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
