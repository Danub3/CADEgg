import type { SettingsView } from "./types";

export const CLAUDE_MODELS = [
  { id: "claude-opus-4-7", label: "Opus 4.7" },
  { id: "claude-sonnet-4-6", label: "Sonnet 4.6" },
  { id: "claude-haiku-4-5", label: "Haiku 4.5" },
];

export const GEMINI_MODELS = [
  { id: "gemini-2.5-pro", label: "Gemini 2.5 Pro（需付费）" },
  { id: "gemini-2.5-flash", label: "Gemini 2.5 Flash" },
  { id: "gemini-2.0-flash", label: "Gemini 2.0 Flash" },
];

export const GLM_MODELS = [
  { id: "glm-4-flash", label: "GLM-4-Flash（免费）" },
  { id: "glm-4.5-flash", label: "GLM-4.5-Flash（免费）" },
  { id: "glm-4.5", label: "GLM-4.5（付费，最强）" },
  { id: "glm-4-plus", label: "GLM-4-Plus（付费）" },
];

export const DEFAULT_VIEW: SettingsView = {
  provider: "glm",
  work_mode: "competition_mode",
  model: "claude-opus-4-7",
  base_url: "https://api.anthropic.com",
  gemini_model: "gemini-2.0-flash",
  gemini_base_url: "https://generativelanguage.googleapis.com",
  glm_model: "glm-4-flash",
  glm_strong_model: "glm-4.5",
  glm_base_url: "https://open.bigmodel.cn/api/paas/v4",
  anthropic_api_key_set: false,
  anthropic_api_key_preview: "",
  gemini_api_key_set: false,
  gemini_api_key_preview: "",
  glm_api_key_set: false,
  glm_api_key_preview: "",
};
