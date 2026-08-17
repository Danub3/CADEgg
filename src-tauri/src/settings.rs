use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::Manager;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkMode {
    #[default]
    CompetitionMode,
    SafetyDemoMode,
}

/// On-disk settings — never returned to the frontend as-is.
#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default = "default_work_mode")]
    pub work_mode: WorkMode,

    // Claude
    #[serde(default)]
    pub anthropic_api_key: String,
    #[serde(default = "default_claude_model")]
    pub model: String,
    #[serde(default = "default_claude_base_url")]
    pub base_url: String,

    // Gemini
    #[serde(default)]
    pub gemini_api_key: String,
    #[serde(default = "default_gemini_model")]
    pub gemini_model: String,
    #[serde(default = "default_gemini_strong_model")]
    pub gemini_strong_model: String,
    #[serde(default = "default_gemini_base_url")]
    pub gemini_base_url: String,

    // Zhipu GLM (OpenAI-compatible)
    #[serde(default)]
    pub glm_api_key: String,
    #[serde(default = "default_glm_model")]
    pub glm_model: String,
    #[serde(default = "default_glm_strong_model")]
    pub glm_strong_model: String,
    #[serde(default = "default_glm_base_url")]
    pub glm_base_url: String,

    // DeepSeek (OpenAI-compatible)
    #[serde(default)]
    pub deepseek_api_key: String,
    #[serde(default = "default_deepseek_model")]
    pub deepseek_model: String,
    #[serde(default = "default_deepseek_strong_model")]
    pub deepseek_strong_model: String,
    #[serde(default = "default_deepseek_base_url")]
    pub deepseek_base_url: String,

    // Qwen / DashScope compatible mode (OpenAI-compatible)
    #[serde(default)]
    pub qwen_api_key: String,
    #[serde(default = "default_qwen_model")]
    pub qwen_model: String,
    #[serde(default = "default_qwen_strong_model")]
    pub qwen_strong_model: String,
    #[serde(default = "default_qwen_base_url")]
    pub qwen_base_url: String,

    // Kimi / Moonshot (OpenAI-compatible)
    #[serde(default)]
    pub kimi_api_key: String,
    #[serde(default = "default_kimi_model")]
    pub kimi_model: String,
    #[serde(default = "default_kimi_strong_model")]
    pub kimi_strong_model: String,
    #[serde(default = "default_kimi_base_url")]
    pub kimi_base_url: String,
}

fn default_provider() -> String {
    "glm".to_string()
}
fn default_work_mode() -> WorkMode {
    WorkMode::SafetyDemoMode
}
fn default_claude_model() -> String {
    "claude-opus-4-7".to_string()
}
fn default_claude_base_url() -> String {
    "https://api.anthropic.com".to_string()
}
fn default_gemini_model() -> String {
    "gemini-2.0-flash".to_string()
}
fn default_gemini_strong_model() -> String {
    "gemini-2.5-pro".to_string()
}
fn default_gemini_base_url() -> String {
    "https://generativelanguage.googleapis.com".to_string()
}
fn default_glm_model() -> String {
    "glm-4-plus".to_string()
}
fn default_glm_strong_model() -> String {
    "glm-4.5".to_string()
}
fn default_glm_base_url() -> String {
    "https://open.bigmodel.cn/api/paas/v4".to_string()
}
fn default_deepseek_model() -> String {
    "deepseek-chat".to_string()
}
fn default_deepseek_strong_model() -> String {
    "deepseek-chat".to_string()
}
fn default_deepseek_base_url() -> String {
    "https://api.deepseek.com".to_string()
}
fn default_qwen_model() -> String {
    "qwen-plus".to_string()
}
fn default_qwen_strong_model() -> String {
    "qwen-max".to_string()
}
fn default_qwen_base_url() -> String {
    "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string()
}
fn default_kimi_model() -> String {
    "moonshot-v1-8k".to_string()
}
fn default_kimi_strong_model() -> String {
    "moonshot-v1-32k".to_string()
}
fn default_kimi_base_url() -> String {
    "https://api.moonshot.cn/v1".to_string()
}

const GLM_MODELS: &[&str] = &["glm-4-flash", "glm-4.5-flash", "glm-4.5", "glm-4-plus"];
const DEEPSEEK_MODELS: &[&str] = &["deepseek-chat"];
const QWEN_MODELS: &[&str] = &["qwen-plus", "qwen-turbo", "qwen-max"];
const KIMI_MODELS: &[&str] = &["moonshot-v1-8k", "moonshot-v1-32k", "moonshot-v1-128k"];

pub fn normalize_provider_id(value: &str) -> String {
    match value.trim() {
        "deepseek" | "qwen" | "kimi" | "glm" => value.trim().to_string(),
        _ => default_provider(),
    }
}

fn supported_models(provider: &str) -> &'static [&'static str] {
    match provider {
        "deepseek" => DEEPSEEK_MODELS,
        "qwen" => QWEN_MODELS,
        "kimi" => KIMI_MODELS,
        _ => GLM_MODELS,
    }
}

pub fn default_strong_model_for_provider(provider: &str) -> String {
    match provider {
        "deepseek" => default_deepseek_strong_model(),
        "qwen" => default_qwen_strong_model(),
        "kimi" => default_kimi_strong_model(),
        _ => default_glm_strong_model(),
    }
}

fn default_cheap_model_for_provider(provider: &str) -> String {
    match provider {
        "deepseek" => default_deepseek_model(),
        "qwen" => default_qwen_model(),
        "kimi" => default_kimi_model(),
        _ => default_glm_model(),
    }
}

pub fn normalize_model_for_provider(provider: &str, value: &str, fallback: &str) -> String {
    let models = supported_models(provider);
    let candidate = value.trim();
    if models.contains(&candidate) {
        return candidate.to_string();
    }
    let fallback_candidate = fallback.trim();
    if models.contains(&fallback_candidate) {
        return fallback_candidate.to_string();
    }
    models.first().copied().unwrap_or("").to_string()
}

fn sanitize_settings(mut settings: Settings) -> Settings {
    settings.provider = normalize_provider_id(&settings.provider);
    settings.glm_model = normalize_model_for_provider(
        "glm",
        &settings.glm_model,
        &default_cheap_model_for_provider("glm"),
    );
    settings.glm_strong_model = normalize_model_for_provider(
        "glm",
        &settings.glm_strong_model,
        &default_strong_model_for_provider("glm"),
    );
    settings.deepseek_model = normalize_model_for_provider(
        "deepseek",
        &settings.deepseek_model,
        &default_cheap_model_for_provider("deepseek"),
    );
    settings.deepseek_strong_model = normalize_model_for_provider(
        "deepseek",
        &settings.deepseek_strong_model,
        &default_strong_model_for_provider("deepseek"),
    );
    settings.qwen_model = normalize_model_for_provider(
        "qwen",
        &settings.qwen_model,
        &default_cheap_model_for_provider("qwen"),
    );
    settings.qwen_strong_model = normalize_model_for_provider(
        "qwen",
        &settings.qwen_strong_model,
        &default_strong_model_for_provider("qwen"),
    );
    settings.kimi_model = normalize_model_for_provider(
        "kimi",
        &settings.kimi_model,
        &default_cheap_model_for_provider("kimi"),
    );
    settings.kimi_strong_model = normalize_model_for_provider(
        "kimi",
        &settings.kimi_strong_model,
        &default_strong_model_for_provider("kimi"),
    );
    settings
}

impl Default for Settings {
    fn default() -> Self {
        sanitize_settings(Self {
            provider: default_provider(),
            work_mode: default_work_mode(),
            anthropic_api_key: String::new(),
            model: default_claude_model(),
            base_url: default_claude_base_url(),
            gemini_api_key: String::new(),
            gemini_model: default_gemini_model(),
            gemini_strong_model: default_gemini_strong_model(),
            gemini_base_url: default_gemini_base_url(),
            glm_api_key: String::new(),
            glm_model: default_glm_model(),
            glm_strong_model: default_glm_strong_model(),
            glm_base_url: default_glm_base_url(),
            deepseek_api_key: String::new(),
            deepseek_model: default_deepseek_model(),
            deepseek_strong_model: default_deepseek_strong_model(),
            deepseek_base_url: default_deepseek_base_url(),
            qwen_api_key: String::new(),
            qwen_model: default_qwen_model(),
            qwen_strong_model: default_qwen_strong_model(),
            qwen_base_url: default_qwen_base_url(),
            kimi_api_key: String::new(),
            kimi_model: default_kimi_model(),
            kimi_strong_model: default_kimi_strong_model(),
            kimi_base_url: default_kimi_base_url(),
        })
    }
}

/// Sanitized view sent to the frontend — no raw key bytes.
#[derive(Serialize)]
pub struct SettingsView {
    pub provider: String,
    pub work_mode: WorkMode,
    pub model: String,
    pub base_url: String,
    pub gemini_model: String,
    pub gemini_strong_model: String,
    pub gemini_base_url: String,
    pub glm_model: String,
    pub glm_strong_model: String,
    pub glm_base_url: String,
    pub deepseek_model: String,
    pub deepseek_strong_model: String,
    pub deepseek_base_url: String,
    pub qwen_model: String,
    pub qwen_strong_model: String,
    pub qwen_base_url: String,
    pub kimi_model: String,
    pub kimi_strong_model: String,
    pub kimi_base_url: String,
    pub anthropic_api_key_set: bool,
    pub anthropic_api_key_preview: String,
    pub gemini_api_key_set: bool,
    pub gemini_api_key_preview: String,
    pub glm_api_key_set: bool,
    pub glm_api_key_preview: String,
    pub deepseek_api_key_set: bool,
    pub deepseek_api_key_preview: String,
    pub qwen_api_key_set: bool,
    pub qwen_api_key_preview: String,
    pub kimi_api_key_set: bool,
    pub kimi_api_key_preview: String,
}

fn preview(key: &str) -> String {
    let k = key.trim();
    if k.is_empty() {
        return String::new();
    }
    let len = k.chars().count();
    if len <= 8 {
        return "•".repeat(len);
    }
    let head: String = k.chars().take(4).collect();
    let tail: String = k.chars().skip(len - 4).collect();
    format!("{head}••••{tail}")
}

impl From<&Settings> for SettingsView {
    fn from(s: &Settings) -> Self {
        Self {
            provider: s.provider.clone(),
            work_mode: s.work_mode,
            model: s.model.clone(),
            base_url: s.base_url.clone(),
            gemini_model: s.gemini_model.clone(),
            gemini_strong_model: s.gemini_strong_model.clone(),
            gemini_base_url: s.gemini_base_url.clone(),
            glm_model: s.glm_model.clone(),
            glm_strong_model: s.glm_strong_model.clone(),
            glm_base_url: s.glm_base_url.clone(),
            deepseek_model: s.deepseek_model.clone(),
            deepseek_strong_model: s.deepseek_strong_model.clone(),
            deepseek_base_url: s.deepseek_base_url.clone(),
            qwen_model: s.qwen_model.clone(),
            qwen_strong_model: s.qwen_strong_model.clone(),
            qwen_base_url: s.qwen_base_url.clone(),
            kimi_model: s.kimi_model.clone(),
            kimi_strong_model: s.kimi_strong_model.clone(),
            kimi_base_url: s.kimi_base_url.clone(),
            anthropic_api_key_set: !s.anthropic_api_key.trim().is_empty(),
            anthropic_api_key_preview: preview(&s.anthropic_api_key),
            gemini_api_key_set: !s.gemini_api_key.trim().is_empty(),
            gemini_api_key_preview: preview(&s.gemini_api_key),
            glm_api_key_set: !s.glm_api_key.trim().is_empty(),
            glm_api_key_preview: preview(&s.glm_api_key),
            deepseek_api_key_set: !s.deepseek_api_key.trim().is_empty(),
            deepseek_api_key_preview: preview(&s.deepseek_api_key),
            qwen_api_key_set: !s.qwen_api_key.trim().is_empty(),
            qwen_api_key_preview: preview(&s.qwen_api_key),
            kimi_api_key_set: !s.kimi_api_key.trim().is_empty(),
            kimi_api_key_preview: preview(&s.kimi_api_key),
        }
    }
}

/// Update payload from the frontend.
/// `*_api_key: None` → keep existing. `Some("")` → clear. `Some(value)` → overwrite.
#[derive(Deserialize)]
pub struct SettingsUpdate {
    pub provider: String,
    #[serde(default = "default_work_mode")]
    pub work_mode: WorkMode,
    #[serde(default = "default_claude_model")]
    pub model: String,
    #[serde(default = "default_claude_base_url")]
    pub base_url: String,
    #[serde(default = "default_gemini_model")]
    pub gemini_model: String,
    #[serde(default = "default_gemini_strong_model")]
    pub gemini_strong_model: String,
    #[serde(default = "default_gemini_base_url")]
    pub gemini_base_url: String,
    #[serde(default = "default_glm_model")]
    pub glm_model: String,
    #[serde(default = "default_glm_strong_model")]
    pub glm_strong_model: String,
    #[serde(default = "default_glm_base_url")]
    pub glm_base_url: String,
    #[serde(default = "default_deepseek_model")]
    pub deepseek_model: String,
    #[serde(default = "default_deepseek_strong_model")]
    pub deepseek_strong_model: String,
    #[serde(default = "default_deepseek_base_url")]
    pub deepseek_base_url: String,
    #[serde(default = "default_qwen_model")]
    pub qwen_model: String,
    #[serde(default = "default_qwen_strong_model")]
    pub qwen_strong_model: String,
    #[serde(default = "default_qwen_base_url")]
    pub qwen_base_url: String,
    #[serde(default = "default_kimi_model")]
    pub kimi_model: String,
    #[serde(default = "default_kimi_strong_model")]
    pub kimi_strong_model: String,
    #[serde(default = "default_kimi_base_url")]
    pub kimi_base_url: String,
    #[serde(default)]
    pub anthropic_api_key: Option<String>,
    #[serde(default)]
    pub gemini_api_key: Option<String>,
    #[serde(default)]
    pub glm_api_key: Option<String>,
    #[serde(default)]
    pub deepseek_api_key: Option<String>,
    #[serde(default)]
    pub qwen_api_key: Option<String>,
    #[serde(default)]
    pub kimi_api_key: Option<String>,
}

fn settings_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("拿不到 app_data_dir: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("创建配置目录失败: {e}"))?;
    Ok(dir.join("settings.json"))
}

pub fn load(app: &tauri::AppHandle) -> Result<Settings, String> {
    let path = settings_path(app)?;
    if !path.exists() {
        return Ok(Settings::default());
    }
    let content = fs::read_to_string(&path).map_err(|e| format!("读配置失败: {e}"))?;
    let settings = serde_json::from_str(&content).map_err(|e| format!("解析配置失败: {e}"))?;
    Ok(sanitize_settings(settings))
}

#[tauri::command]
pub fn get_settings(app: tauri::AppHandle) -> Result<SettingsView, String> {
    let s = load(&app)?;
    Ok(SettingsView::from(&s))
}

#[tauri::command]
pub fn save_settings(app: tauri::AppHandle, update: SettingsUpdate) -> Result<(), String> {
    let mut current = load(&app).unwrap_or_default();
    current.provider = update.provider;
    current.work_mode = update.work_mode;
    current.model = update.model;
    current.base_url = update.base_url;
    current.gemini_model = update.gemini_model;
    current.gemini_strong_model = update.gemini_strong_model;
    current.gemini_base_url = update.gemini_base_url;
    current.glm_model = update.glm_model;
    current.glm_strong_model = update.glm_strong_model;
    current.glm_base_url = update.glm_base_url;
    current.deepseek_model = update.deepseek_model;
    current.deepseek_strong_model = update.deepseek_strong_model;
    current.deepseek_base_url = update.deepseek_base_url;
    current.qwen_model = update.qwen_model;
    current.qwen_strong_model = update.qwen_strong_model;
    current.qwen_base_url = update.qwen_base_url;
    current.kimi_model = update.kimi_model;
    current.kimi_strong_model = update.kimi_strong_model;
    current.kimi_base_url = update.kimi_base_url;
    if let Some(k) = update.anthropic_api_key {
        current.anthropic_api_key = k;
    }
    if let Some(k) = update.gemini_api_key {
        current.gemini_api_key = k;
    }
    if let Some(k) = update.glm_api_key {
        current.glm_api_key = k;
    }
    if let Some(k) = update.deepseek_api_key {
        current.deepseek_api_key = k;
    }
    if let Some(k) = update.qwen_api_key {
        current.qwen_api_key = k;
    }
    if let Some(k) = update.kimi_api_key {
        current.kimi_api_key = k;
    }

    current = sanitize_settings(current);
    let path = settings_path(&app)?;
    let content =
        serde_json::to_string_pretty(&current).map_err(|e| format!("序列化配置失败: {e}"))?;
    fs::write(&path, content).map_err(|e| format!("写配置失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_removed_deepseek_reasoner() {
        let settings = Settings {
            provider: "deepseek".to_string(),
            deepseek_strong_model: "deepseek-reasoner".to_string(),
            ..Default::default()
        };

        let sanitized = sanitize_settings(settings);

        assert_eq!(sanitized.provider, "deepseek");
        assert_eq!(sanitized.deepseek_strong_model, "deepseek-chat");
    }

    #[test]
    fn normalize_unknown_provider_and_model() {
        let settings = Settings {
            provider: "claude".to_string(),
            glm_strong_model: "missing-model".to_string(),
            ..Default::default()
        };

        let sanitized = sanitize_settings(settings);

        assert_eq!(sanitized.provider, "glm");
        assert_eq!(sanitized.glm_strong_model, "glm-4.5");
    }
}
