use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tauri::Manager;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStorageLocation {
    Appdata,
    Project,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMemorySaveResult {
    pub latest_markdown_path: String,
    pub summary_markdown_path: String,
    pub events_path: String,
    pub index_path: String,
    pub global_memory_path: String,
}

/// 记忆包里的单个文件信息（用于「查看记忆包」入口）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryFileInfo {
    pub name: String,
    pub size_bytes: u64,
    pub updated_at_ms: u64,
}

/// 记忆包只读快照：目录路径、文件清单、全局记忆内容。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBundleInfo {
    pub dir: String,
    pub files: Vec<MemoryFileInfo>,
    pub global_memory: String,
    pub global_memory_exists: bool,
}

/// 递归收集记忆目录下的文件信息（相对路径、大小、修改时间），最多 limit 个。
fn collect_memory_files(dir: &Path, base: &Path, out: &mut Vec<MemoryFileInfo>, limit: usize) {
    if out.len() >= limit {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut items: Vec<_> = entries.flatten().collect();
    items.sort_by_key(|entry| entry.file_name());
    for entry in items {
        if out.len() >= limit {
            break;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_memory_files(&path, base, out, limit);
        } else if path.is_file() {
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let metadata = fs::metadata(&path).ok();
            out.push(MemoryFileInfo {
                name: rel,
                size_bytes: metadata.as_ref().map(|m| m.len()).unwrap_or(0),
                updated_at_ms: metadata
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
            });
        }
    }
}

/// 读取记忆包只读信息：目录、文件清单、global-memory.md 内容。
/// 只读命令，不修改任何文件，也不注入模型上下文。
#[tauri::command]
pub fn read_memory_bundle(
    app: tauri::AppHandle,
    location: SessionStorageLocation,
) -> Result<MemoryBundleInfo, String> {
    let root = session_dir(&app, location)?;
    let memory_dir = root.join("memory");
    fs::create_dir_all(&memory_dir).map_err(|e| format!("创建记忆目录失败: {e}"))?;

    let mut files = Vec::new();
    collect_memory_files(&memory_dir, &memory_dir, &mut files, 100);

    let global_memory_path = memory_dir.join("global-memory.md");
    let (global_memory, global_memory_exists) = if global_memory_path.is_file() {
        (fs::read_to_string(&global_memory_path).unwrap_or_default(), true)
    } else {
        (String::new(), false)
    };

    Ok(MemoryBundleInfo {
        dir: memory_dir.display().to_string(),
        files,
        global_memory,
        global_memory_exists,
    })
}

fn safe_filename(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars().take(96) {
        if matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || ch.is_control() {
            out.push('_');
        } else {
            out.push(ch);
        }
    }
    let trimmed = out.trim_matches([' ', '.']).trim();
    if trimmed.is_empty() {
        "CADEgg-session.md".to_string()
    } else if trimmed.to_ascii_lowercase().ends_with(".md") {
        trimmed.to_string()
    } else {
        format!("{trimmed}.md")
    }
}

fn safe_path_segment(name: &str, fallback: &str) -> String {
    let mut out = String::new();
    for ch in name.chars().take(80) {
        if matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || ch.is_control() {
            out.push('_');
        } else {
            out.push(ch);
        }
    }
    let trimmed = out.trim_matches([' ', '.']).trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

fn looks_like_project_root(path: &Path) -> bool {
    path.join("src-tauri").is_dir() && path.join("package.json").is_file()
}

fn project_dir() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("拿不到当前目录: {e}"))?;
    if looks_like_project_root(&cwd) {
        return Ok(cwd);
    }

    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors() {
            if looks_like_project_root(ancestor) {
                return Ok(ancestor.to_path_buf());
            }
        }
    }

    Ok(cwd)
}

/// 记忆包根目录（供 benchmark 等模块复用）。
pub(crate) fn memory_bundle_dir(
    app: &tauri::AppHandle,
    location: SessionStorageLocation,
) -> Result<PathBuf, String> {
    Ok(session_dir(app, location)?.join("memory"))
}

fn session_dir(
    app: &tauri::AppHandle,
    location: SessionStorageLocation,
) -> Result<PathBuf, String> {
    let base = match location {
        SessionStorageLocation::Appdata => app
            .path()
            .app_data_dir()
            .map_err(|e| format!("拿不到 app_data_dir: {e}"))?,
        SessionStorageLocation::Project => project_dir()?.join("cadegg-sessions"),
    };
    let dir = match location {
        SessionStorageLocation::Appdata => base.join("sessions"),
        SessionStorageLocation::Project => base,
    };
    fs::create_dir_all(&dir).map_err(|e| format!("创建会话导出目录失败: {e}"))?;
    Ok(dir)
}

#[tauri::command]
pub fn save_session_markdown(
    app: tauri::AppHandle,
    filename: String,
    content: String,
    location: SessionStorageLocation,
) -> Result<String, String> {
    let dir = session_dir(&app, location)?;
    let path = dir.join(safe_filename(&filename));
    fs::write(&path, content).map_err(|e| format!("写入会话 Markdown 失败: {e}"))?;
    Ok(path.display().to_string())
}

#[tauri::command]
pub fn save_session_memory_bundle(
    app: tauri::AppHandle,
    session_id: String,
    markdown_content: String,
    summary_content: String,
    event_content: String,
    index_entry: serde_json::Value,
    location: SessionStorageLocation,
) -> Result<SessionMemorySaveResult, String> {
    let root = session_dir(&app, location)?;
    let memory_dir = root.join("memory");
    let session_dir = memory_dir
        .join("sessions")
        .join(safe_path_segment(&session_id, "session"));
    fs::create_dir_all(&session_dir).map_err(|e| format!("创建会话记忆目录失败: {e}"))?;

    let latest_markdown_path = session_dir.join("latest.md");
    let summary_markdown_path = session_dir.join("summary.md");
    let events_path = session_dir.join("events.jsonl");
    let index_path = memory_dir.join("memory-index.json");
    let global_memory_path = memory_dir.join("global-memory.md");

    fs::write(&latest_markdown_path, markdown_content)
        .map_err(|e| format!("写入 latest.md 失败: {e}"))?;
    fs::write(&summary_markdown_path, summary_content)
        .map_err(|e| format!("写入 summary.md 失败: {e}"))?;
    if !global_memory_path.is_file() {
        fs::write(
            &global_memory_path,
            "# CADEgg Global Memory\n\n这个文件用于跨会话共享基础记忆，例如用户偏好、项目规则、常用图层和单位约定。\n当前版本只保存，不会自动注入模型上下文。\n",
        )
        .map_err(|e| format!("创建 global-memory.md 失败: {e}"))?;
    }

    if !event_content.trim().is_empty() {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&events_path)
            .map_err(|e| format!("打开 events.jsonl 失败: {e}"))?;
        writeln!(file, "{}", event_content.trim_end())
            .map_err(|e| format!("写入 events.jsonl 失败: {e}"))?;
    }

    let mut entries = if index_path.is_file() {
        match fs::read_to_string(&index_path) {
            Ok(raw) => serde_json::from_str::<Vec<serde_json::Value>>(&raw).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };
    entries.retain(|entry| {
        entry.get("sessionId") != Some(&serde_json::Value::String(session_id.clone()))
    });
    entries.push(index_entry);
    entries.sort_by(|a, b| {
        let a_updated = a
            .get("updatedAt")
            .and_then(|v| v.as_i64())
            .unwrap_or_default();
        let b_updated = b
            .get("updatedAt")
            .and_then(|v| v.as_i64())
            .unwrap_or_default();
        b_updated.cmp(&a_updated)
    });
    if entries.len() > 200 {
        entries.truncate(200);
    }

    let index_json = serde_json::to_string_pretty(&entries)
        .map_err(|e| format!("序列化 memory-index.json 失败: {e}"))?;
    fs::write(&index_path, index_json).map_err(|e| format!("写入 memory-index.json 失败: {e}"))?;

    Ok(SessionMemorySaveResult {
        latest_markdown_path: latest_markdown_path.display().to_string(),
        summary_markdown_path: summary_markdown_path.display().to_string(),
        events_path: events_path.display().to_string(),
        index_path: index_path.display().to_string(),
        global_memory_path: global_memory_path.display().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_memory_files_lists_nested_files_sorted() {
        let base = std::env::temp_dir().join(format!("cadegg-memory-test-{}", std::process::id()));
        let sub = base.join("sessions").join("abc123");
        fs::create_dir_all(&sub).unwrap();
        fs::write(base.join("global-memory.md"), "# 全局记忆").unwrap();
        fs::write(sub.join("latest.md"), "# latest").unwrap();

        let mut out = Vec::new();
        collect_memory_files(&base, &base, &mut out, 100);

        let names: Vec<&str> = out.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"global-memory.md"));
        assert!(names.contains(&"sessions/abc123/latest.md"));
        assert!(out.iter().all(|f| f.size_bytes > 0));

        let _ = fs::remove_dir_all(&base);
    }
}
