import type { Provider, SettingsView } from "./types";

// 评级说明：rating 为 CADEgg 实测基准回写（全量 38 模型，测试日期 2026-08-18，结果见记忆目录 benchmark-results.json/md）。
// 评分权重：工具调用可靠性 25% · CAD/规范准确性 25% · 稳定性 15% · 速度 15% · 成本 10% · 长上下文 10%。
// 例外：glm-4.7-flash 实测期间持续 429（访问量过大）未取得完整数据，保留官方评分 3.5。
// 模型标签只标免费，未标注一律按消耗 token/额度处理。

export type ModelTier = "production" | "limited" | "unavailable";
export type ModelRating = 1 | 1.5 | 2 | 2.5 | 3 | 3.5 | 4 | 4.5 | 5;

export interface ModelOption {
  id: string;
  label: string;
  tier: ModelTier;
  rating?: ModelRating;
}

export const GLM_MODELS: ModelOption[] = [
  { id: "glm-5.2", label: "GLM-5.2", tier: "production", rating: 4.5 },
  { id: "glm-5.1", label: "GLM-5.1", tier: "production", rating: 4 },
  { id: "glm-5", label: "GLM-5", tier: "production", rating: 4.5 },
  { id: "glm-5-turbo", label: "GLM-5-Turbo", tier: "limited", rating: 4.5 },
  { id: "glm-4.7", label: "GLM-4.7", tier: "production", rating: 4 },
  { id: "glm-4.7-flashx", label: "GLM-4.7-FlashX", tier: "limited", rating: 4.5 },
  { id: "glm-4.7-flash", label: "GLM-4.7-Flash（免费）", tier: "limited", rating: 3.5 },
  { id: "glm-4.6", label: "GLM-4.6", tier: "production", rating: 4 },
  { id: "glm-4.5", label: "GLM-4.5", tier: "production", rating: 4 },
  { id: "glm-4.5-air", label: "GLM-4.5-Air", tier: "limited", rating: 4 },
  { id: "glm-4.5-airx", label: "GLM-4.5-AirX", tier: "limited", rating: 4 },
  { id: "glm-4.5-flash", label: "GLM-4.5-Flash（免费）", tier: "limited", rating: 4 },
  { id: "glm-4-flash-250414", label: "GLM-4-Flash-250414（免费）", tier: "limited", rating: 4.5 },
  { id: "glm-4-flashx-250414", label: "GLM-4-FlashX-250414", tier: "limited", rating: 4 },
];

export const DEEPSEEK_MODELS: ModelOption[] = [
  { id: "deepseek-v4-pro", label: "DeepSeek V4 Pro", tier: "production", rating: 4 },
  { id: "deepseek-v4-flash", label: "DeepSeek V4 Flash", tier: "production", rating: 4.5 },
];

export const QWEN_MODELS: ModelOption[] = [
  { id: "qwen3.8-max", label: "通义千问 3.8 Max", tier: "production", rating: 4.5 },
  { id: "qwen3.7-max", label: "通义千问 3.7 Max", tier: "production", rating: 4.5 },
  { id: "qwen3.7-plus", label: "通义千问 3.7 Plus", tier: "production", rating: 4.5 },
  { id: "qwen3.7-flash", label: "通义千问 3.7 Flash", tier: "limited", rating: 4 },
  { id: "qwen3.6-plus", label: "通义千问 3.6 Plus", tier: "limited", rating: 4 },
  { id: "qwen3.6-flash", label: "通义千问 3.6 Flash", tier: "limited", rating: 4.5 },
  { id: "qwen3.5-plus", label: "通义千问 3.5 Plus", tier: "limited", rating: 4 },
  { id: "qwen3.5-flash", label: "通义千问 3.5 Flash", tier: "limited", rating: 4 },
  { id: "qwen3-coder-plus", label: "通义千问 Coder Plus", tier: "limited", rating: 4.5 },
  { id: "qwen3-coder-flash", label: "通义千问 Coder Flash", tier: "limited", rating: 4 },
  { id: "qwen3-max", label: "通义千问 3 Max", tier: "production", rating: 4 },
  { id: "qwen-flash", label: "通义千问 Flash", tier: "limited", rating: 4 },
  { id: "qwen-max", label: "通义千问 Max（旧版）", tier: "production", rating: 4 },
  { id: "qwen-plus", label: "通义千问 Plus", tier: "limited", rating: 4 },
  { id: "qwen-turbo", label: "通义千问 Turbo", tier: "limited", rating: 4 },
];

export const KIMI_MODELS: ModelOption[] = [
  { id: "kimi-k3", label: "Kimi K3", tier: "production", rating: 4 },
  { id: "kimi-k2.7-code", label: "Kimi K2.7 Code", tier: "production", rating: 4.5 },
  { id: "kimi-k2.6", label: "Kimi K2.6", tier: "production", rating: 4 },
  { id: "kimi-k2.5", label: "Kimi K2.5", tier: "limited", rating: 4.5 },
  { id: "moonshot-v1-8k", label: "Kimi 8K（旧版）", tier: "unavailable", rating: 4 },
  { id: "moonshot-v1-32k", label: "Kimi 32K（旧版）", tier: "unavailable", rating: 4 },
  { id: "moonshot-v1-128k", label: "Kimi 128K（旧版）", tier: "unavailable", rating: 4 },
];

export function modelRating(model: ModelOption): ModelRating {
  if (model.rating) return model.rating;
  if (model.tier === "production") return 4;
  if (model.tier === "limited") return 3;
  return 2;
}

export const MODEL_PROVIDERS: Array<{
  id: Provider;
  label: string;
  shortLabel: string;
  apiLabel: string;
  baseUrlField: keyof SettingsView;
  cheapModelField: keyof SettingsView;
  strongModelField: keyof SettingsView;
  keySetField: keyof SettingsView;
  keyPreviewField: keyof SettingsView;
  models: ModelOption[];
}> = [
  {
    id: "glm",
    label: "智谱 GLM",
    shortLabel: "GLM",
    apiLabel: "GLM API Key",
    baseUrlField: "glm_base_url",
    cheapModelField: "glm_model",
    strongModelField: "glm_strong_model",
    keySetField: "glm_api_key_set",
    keyPreviewField: "glm_api_key_preview",
    models: GLM_MODELS,
  },
  {
    id: "deepseek",
    label: "DeepSeek",
    shortLabel: "DeepSeek",
    apiLabel: "DeepSeek API Key",
    baseUrlField: "deepseek_base_url",
    cheapModelField: "deepseek_model",
    strongModelField: "deepseek_strong_model",
    keySetField: "deepseek_api_key_set",
    keyPreviewField: "deepseek_api_key_preview",
    models: DEEPSEEK_MODELS,
  },
  {
    id: "qwen",
    label: "通义千问",
    shortLabel: "千问",
    apiLabel: "DashScope API Key",
    baseUrlField: "qwen_base_url",
    cheapModelField: "qwen_model",
    strongModelField: "qwen_strong_model",
    keySetField: "qwen_api_key_set",
    keyPreviewField: "qwen_api_key_preview",
    models: QWEN_MODELS,
  },
  {
    id: "kimi",
    label: "Kimi",
    shortLabel: "Kimi",
    apiLabel: "Moonshot API Key",
    baseUrlField: "kimi_base_url",
    cheapModelField: "kimi_model",
    strongModelField: "kimi_strong_model",
    keySetField: "kimi_api_key_set",
    keyPreviewField: "kimi_api_key_preview",
    models: KIMI_MODELS,
  },
];

export function providerMeta(provider: Provider) {
  return MODEL_PROVIDERS.find((item) => item.id === provider) ?? MODEL_PROVIDERS[0];
}

export const DEFAULT_VIEW: SettingsView = {
  provider: "glm",
  work_mode: "safety_demo_mode",
  auto_failover: true,
  memory_carry_token_budget: 1200,
  glm_model: "glm-4.5-air",
  glm_strong_model: "glm-4.5",
  glm_base_url: "https://open.bigmodel.cn/api/paas/v4",
  deepseek_model: "deepseek-v4-flash",
  deepseek_strong_model: "deepseek-v4-pro",
  deepseek_base_url: "https://api.deepseek.com",
  qwen_model: "qwen3.7-flash",
  qwen_strong_model: "qwen3.8-max",
  qwen_base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
  kimi_model: "kimi-k2.5",
  kimi_strong_model: "kimi-k2.6",
  kimi_base_url: "https://api.moonshot.cn/v1",
  glm_api_key_set: false,
  glm_api_key_preview: "",
  deepseek_api_key_set: false,
  deepseek_api_key_preview: "",
  qwen_api_key_set: false,
  qwen_api_key_preview: "",
  kimi_api_key_set: false,
  kimi_api_key_preview: "",
};
