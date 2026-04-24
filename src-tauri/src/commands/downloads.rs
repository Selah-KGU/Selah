use crate::client;
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::load_download_config;

pub const OTHER_CATEGORY: &str = "その他";

/// Sanitize a string to be safe as a directory/file name component.
fn sanitize_path_component(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            _ => c,
        })
        .collect();
    let trimmed = s.trim().trim_matches('.');
    if trimmed.is_empty() {
        "_".into()
    } else {
        trimmed.to_string()
    }
}

/// Simplify a course name for use as a folder name.
pub fn simplify_course_name(name: &str) -> String {
    static RE_DEPT_CODE: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"^.+\s\d{7,8}\s+").unwrap());
    static RE_BRACKET: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"[\[［]\d+[\]］]").unwrap());
    static RE_PAREN_SUFFIX: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"[（(][^)）]*(?:学期|限|クラス|組|セメスター|Quarter|Semester)[^)）]*[)）]\s*$")
            .unwrap()
    });

    let s = RE_DEPT_CODE.replace(name, "");
    let s = RE_BRACKET.replace_all(&s, "");
    let s = RE_PAREN_SUFFIX.replace_all(&s, "");
    let s: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = s.trim().to_string();
    if s.is_empty() {
        name.trim().to_string()
    } else {
        s
    }
}

/// Default download base directory: ~/Documents/Selah (created if needed).
pub fn default_download_dir() -> std::path::PathBuf {
    let doc = dirs::document_dir().unwrap_or_else(|| {
        dirs::home_dir()
            .map(|h| h.join("Documents"))
            .unwrap_or_else(std::env::temp_dir)
    });
    let dir = doc.join("Selah");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Resolve the download directory with optional course classification.
pub fn resolve_download_dir(course_name: Option<&str>) -> std::path::PathBuf {
    let config = load_download_config();
    let base = if config.download_dir.is_empty() {
        default_download_dir()
    } else {
        std::path::PathBuf::from(&config.download_dir)
    };

    if config.classify_by_course {
        let folder = match course_name.map(str::trim).filter(|s| !s.is_empty()) {
            Some(course) => sanitize_path_component(&simplify_course_name(course)),
            None => OTHER_CATEGORY.to_string(),
        };
        let dir = base.join(&folder);
        let _ = std::fs::create_dir_all(&dir);
        return dir;
    }

    base
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRecord {
    pub id: String,
    pub filename: String,
    pub path: String,
    pub course_name: String,
    pub source: String,
    pub size_bytes: u64,
    pub downloaded_at: i64,
    #[serde(default)]
    pub file_exists: bool,
}

fn download_history_path() -> std::path::PathBuf {
    client::data_dir().join("download_history.json")
}

pub fn load_download_history() -> Vec<DownloadRecord> {
    let path = download_history_path();
    if path.exists() {
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(records) = serde_json::from_str(&data) {
                return records;
            }
        }
    }
    Vec::new()
}

fn save_download_history(records: &[DownloadRecord]) -> Result<(), String> {
    let path = download_history_path();
    let data =
        serde_json::to_string(records).map_err(|e| format!("JSON serialization error: {}", e))?;
    std::fs::write(&path, &data).map_err(|e| format!("Failed to write download history: {}", e))?;
    Ok(())
}

/// Record a new download in the history. Called from save_to_downloads.
pub fn record_download(
    filename: &str,
    path: &str,
    course_name: Option<&str>,
    source: &str,
    size_bytes: u64,
) {
    let mut records = load_download_history();
    let course_label = match course_name.map(str::trim).filter(|s| !s.is_empty()) {
        Some(c) => c.to_string(),
        None if load_download_config().classify_by_course => OTHER_CATEGORY.to_string(),
        None => String::new(),
    };
    let record = DownloadRecord {
        id: format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ),
        filename: filename.to_string(),
        path: path.to_string(),
        course_name: course_label,
        source: source.to_string(),
        size_bytes,
        downloaded_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64,
        file_exists: true,
    };
    records.push(record);
    if records.len() > 500 {
        records.drain(0..records.len() - 500);
    }
    let _ = save_download_history(&records);
}

#[tauri::command]
pub fn list_downloads() -> Vec<DownloadRecord> {
    let mut records = load_download_history();
    records.retain(|r| !r.path.is_empty());
    for r in &mut records {
        r.file_exists = std::path::Path::new(&r.path).exists();
    }
    records.reverse();
    records
}

#[tauri::command]
pub fn scan_download_dir() -> Vec<DownloadRecord> {
    let config = load_download_config();
    let base = if config.download_dir.is_empty() {
        default_download_dir()
    } else {
        std::path::PathBuf::from(&config.download_dir)
    };

    let mut records = load_download_history();
    let known_paths: std::collections::HashSet<String> =
        records.iter().map(|r| r.path.clone()).collect();

    let mut discovered: Vec<DownloadRecord> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(rec) = try_discover_file(&path, "", &known_paths) {
                    discovered.push(rec);
                }
            } else if path.is_dir() {
                let folder_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                if let Ok(sub_entries) = std::fs::read_dir(&path) {
                    for sub in sub_entries.flatten() {
                        let sub_path = sub.path();
                        if sub_path.is_file() {
                            if let Some(rec) =
                                try_discover_file(&sub_path, &folder_name, &known_paths)
                            {
                                discovered.push(rec);
                            }
                        }
                    }
                }
            }
        }
    }

    if !discovered.is_empty() {
        records.extend(discovered);
        if records.len() > 500 {
            records.drain(0..records.len() - 500);
        }
        let _ = save_download_history(&records);
    }

    records.retain(|r| !r.path.is_empty());
    for r in &mut records {
        r.file_exists = std::path::Path::new(&r.path).exists();
    }
    records.reverse();
    records
}

fn try_discover_file(
    path: &std::path::Path,
    course_folder: &str,
    known: &std::collections::HashSet<String>,
) -> Option<DownloadRecord> {
    let path_str = path.to_string_lossy().to_string();
    if known.contains(&path_str) {
        return None;
    }
    let filename = path.file_name()?.to_str()?;
    if filename.starts_with('.') || filename == "desktop.ini" || filename == "Thumbs.db" {
        return None;
    }
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as i64;

    Some(DownloadRecord {
        id: format!("scan_{}", modified),
        filename: filename.to_string(),
        path: path_str,
        course_name: course_folder.to_string(),
        source: "scan".to_string(),
        size_bytes: metadata.len(),
        downloaded_at: modified,
        file_exists: true,
    })
}

#[tauri::command]
pub fn check_file_downloaded(
    filename: String,
    course_name: Option<String>,
) -> Option<DownloadRecord> {
    let records = load_download_history();
    let target = filename.to_lowercase();
    let mut found: Option<DownloadRecord> = None;
    for r in records.iter().rev() {
        let rname = r.filename.to_lowercase();
        if rname == target {
            if let Some(ref cn) = course_name {
                if !cn.is_empty() && !r.course_name.is_empty() && r.course_name != *cn {
                    continue;
                }
            }
            let mut rec = r.clone();
            rec.file_exists = std::path::Path::new(&rec.path).exists();
            if rec.file_exists {
                return Some(rec);
            }
            if found.is_none() {
                found = Some(rec);
            }
        }
    }
    found
}

#[tauri::command]
pub fn open_downloaded_file(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    if !p.exists() {
        return Err("ファイルが見つかりません".into());
    }
    let canonical = p
        .canonicalize()
        .map_err(|e| format!("パスが無効です: {}", e))?;
    let app_default = default_download_dir()
        .canonicalize()
        .unwrap_or_else(|_| default_download_dir());
    let sys_downloads = dirs::download_dir().unwrap_or_else(|| {
        dirs::home_dir()
            .map(|h| h.join("Downloads"))
            .unwrap_or_else(std::env::temp_dir)
    });
    let dl_config = load_download_config();
    let custom_dir = if dl_config.download_dir.is_empty() {
        None
    } else {
        std::path::Path::new(&dl_config.download_dir)
            .canonicalize()
            .ok()
    };
    let allowed = canonical.starts_with(&app_default)
        || canonical.starts_with(&sys_downloads)
        || custom_dir
            .as_ref()
            .is_some_and(|d| canonical.starts_with(d));
    if !allowed {
        return Err("ダウンロードフォルダ外のファイルは開けません".into());
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(canonical.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("ファイルを開けませんでした: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn remove_download_record(id: String) -> Result<(), String> {
    let mut records = load_download_history();
    records.retain(|r| r.id != id);
    save_download_history(&records)
}

#[tauri::command]
pub fn clear_download_history() -> Result<(), String> {
    save_download_history(&[])
}

/// Move files sitting at the root of the download base dir into `その他/`.
/// Only runs when `classify_by_course` is enabled. Idempotent: a second run is a no-op.
pub fn migrate_uncategorized_to_other() {
    let config = load_download_config();
    if !config.classify_by_course {
        return;
    }
    let base = if config.download_dir.is_empty() {
        default_download_dir()
    } else {
        std::path::PathBuf::from(&config.download_dir)
    };
    if !base.is_dir() {
        return;
    }
    let target_dir = base.join(OTHER_CATEGORY);

    let mut pending: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.starts_with('.') || name == "desktop.ini" || name == "Thumbs.db" {
                continue;
            }
            pending.push(path);
        }
    }
    if pending.is_empty() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&target_dir) {
        log::warn!("migrate: failed to create {:?}: {}", target_dir, e);
        return;
    }

    let mut path_map: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for src in pending {
        let Some(name) = src.file_name().map(|n| n.to_os_string()) else {
            continue;
        };
        let mut dest = target_dir.join(&name);
        if dest.exists() {
            let stem = std::path::Path::new(&name)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let ext = std::path::Path::new(&name)
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            let mut i = 1u32;
            loop {
                let candidate = target_dir.join(format!("{} ({}){}", stem, i, ext));
                if !candidate.exists() {
                    dest = candidate;
                    break;
                }
                i += 1;
                if i > 999 {
                    break;
                }
            }
        }
        match std::fs::rename(&src, &dest) {
            Ok(()) => {
                path_map.insert(
                    src.to_string_lossy().to_string(),
                    dest.to_string_lossy().to_string(),
                );
            }
            Err(e) => log::warn!("migrate: failed to move {:?} -> {:?}: {}", src, dest, e),
        }
    }

    if path_map.is_empty() {
        return;
    }

    let mut records = load_download_history();
    let mut changed = false;
    for r in records.iter_mut() {
        if let Some(new_path) = path_map.get(&r.path) {
            r.path = new_path.clone();
            if r.course_name.trim().is_empty() {
                r.course_name = OTHER_CATEGORY.to_string();
            }
            changed = true;
        }
    }
    if changed {
        let _ = save_download_history(&records);
    }
}
