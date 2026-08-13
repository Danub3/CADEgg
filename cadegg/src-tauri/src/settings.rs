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
    #[serde(default = "default_gemini_base_url")]
    pub gemini_base_url: String,

    // Zhipu GLM (OpenAI-compatible)
    #[serde(default)]
    pub glm_api_key: String,
    #[serde(default = "default_glm_model")]
    pub glm_model: String,
    #[serde(default = "default_glm_base_url")]
    pub glm_base_url: String,
}

fn default_provider() -> String {
    "glm".to_string()
}
fn default_work_mode() -> WorkMode {
    WorkMode::CompetitionMode
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
fn default_gemini_base_url() -> String {
    "https://generativelanguage.googleapis.com".to_string()
}
fn default_glm_model() -> String {
    "glm-4-flash".to_string()
}
fn default_glm_base_url() -> String {
    "https://open.bigmodel.cn/api/paas/v4".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            work_mode: default_work_mode(),
            anthropic_api_key: String::new(),
            model: default_claude_model(),
            base_url: default_claude_base_url(),
            gemini_api_key: String::new(),
            gemini_model: default_gemini_model(),
            gemini_base_url: default_gemini_base_url(),
            glm_api_key: String::new(),
            glm_model: default_glm_model(),
            glm_base_url: default_glm_base_url(),
        }
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
    pub gemini_base_url: String,
    pub glm_model: String,
    pub glm_base_url: String,
    pub anthropic_api_key_set: bool,
    pub anthropic_api_key_preview: String,
    pub gemini_api_key_set: bool,
    pub gemini_api_key_preview: String,
    pub glm_api_key_set: bool,
    pub glm_api_key_preview: String,
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
            gemini_base_url: s.gemini_base_url.clone(),
            glm_model: s.glm_model.clone(),
            glm_base_url: s.glm_base_url.clone(),
            anthropic_api_key_set: !s.anthropic_api_key.trim().is_empty(),
            anthropic_api_key_preview: preview(&s.anthropic_api_key),
            gemini_api_key_set: !s.gemini_api_key.trim().is_empty(),
            gemini_api_key_preview: preview(&s.gemini_api_key),
            glm_api_key_set: !s.glm_api_key.trim().is_empty(),
            glm_api_key_preview: preview(&s.glm_api_key),
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
    pub model: String,
    pub base_url: String,
    pub gemini_model: String,
    pub gemini_base_url: String,
    pub glm_model: String,
    pub glm_base_url: String,
    #[serde(default)]
    pub anthropic_api_key: Option<String>,
    #[serde(default)]
    pub gemini_api_key: Option<String>,
    #[serde(default)]
    pub glm_api_key: Option<String>,
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
    serde_json::from_str(&content).map_err(|e| format!("解析配置失败: {e}"))
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
    current.gemini_base_url = update.gemini_base_url;
    current.glm_model = update.glm_model;
    current.glm_base_url = update.glm_base_url;
    if let Some(k) = update.anthropic_api_key {
        current.anthropic_api_key = k;
    }
    if let Some(k) = update.gemini_api_key {
        current.gemini_api_key = k;
    }
    if let Some(k) = update.glm_api_key {
        current.glm_api_key = k;
    }

    let path = settings_path(&app)?;
    let content =
        serde_json::to_string_pretty(&current).map_err(|e| format!("序列化配置失败: {e}"))?;
    fs::write(&path, content).map_err(|e| format!("写配置失败: {e}"))
}
