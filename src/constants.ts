import type { Provider, SettingsView } from "./types";

export const GLM_MODELS = [
  { id: "glm-4-flash", label: "GLM-4-Flash（免费）" },
  { id: "glm-4.5-flash", label: "GLM-4.5-Flash（免费）" },
  { id: "glm-4.5", label: "GLM-4.5（付费，最强）" },
  { id: "glm-4-plus", label: "GLM-4-Plus（付费）" },
];

export const DEEPSEEK_MODELS = [
  { id: "deepseek-chat", label: "DeepSeek Chat" },
  { id: "deepseek-reasoner", label: "DeepSeek Reasoner" },
];

export const QWEN_MODELS = [
  { id: "qwen-plus", label: "通义千问 Plus" },
  { id: "qwen-turbo", label: "通义千问 Turbo" },
  { id: "qwen-max", label: "通义千问 Max" },
];

export const KIMI_MODELS = [
  { id: "moonshot-v1-8k", label: "Kimi 8K" },
  { id: "moonshot-v1-32k", label: "Kimi 32K" },
  { id: "moonshot-v1-128k", label: "Kimi 128K" },
];

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
  models: Array<{ id: string; label: string }>;
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
  glm_model: "glm-4-flash",
  glm_strong_model: "glm-4.5-flash",
  glm_base_url: "https://open.bigmodel.cn/api/paas/v4",
  deepseek_model: "deepseek-chat",
  deepseek_strong_model: "deepseek-reasoner",
  deepseek_base_url: "https://api.deepseek.com",
  qwen_model: "qwen-plus",
  qwen_strong_model: "qwen-max",
  qwen_base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
  kimi_model: "moonshot-v1-8k",
  kimi_strong_model: "moonshot-v1-32k",
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
