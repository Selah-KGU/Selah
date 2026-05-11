use crate::client;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

use super::load_download_config;

pub const OTHER_CATEGORY: &str = "その他";

/// Monotonic per-process counter appended to timestamps so two records created
/// within the same millisecond get distinct ids.
static DOWNLOAD_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Maximum depth for `scan_download_dir` recursion. The expected layout is at
/// most `base/<course>/<free_note_or_similar>/<file>` (3 levels); allow a bit
/// more for user-organized subfolders while still bounding the traversal.
const SCAN_MAX_DEPTH: usize = 6;

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
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let counter = DOWNLOAD_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    let record = DownloadRecord {
        id: format!("{}_{}", now_ms, counter),
        filename: filename.to_string(),
        path: path.to_string(),
        course_name: course_label,
        source: source.to_string(),
        size_bytes,
        downloaded_at: now_ms,
        file_exists: true,
    };
    // Dedupe by path: a prior `scan_download_dir` (e.g. triggered by opening
    // the downloads window while this download was in flight) may have already
    // inserted a `scan_*` record for this exact path. Replace it so the user
    // sees one entry per file, not two.
    if let Some(existing) = records.iter_mut().find(|r| r.path == record.path) {
        *existing = record;
    } else {
        records.push(record);
        if records.len() > 500 {
            records.drain(0..records.len() - 500);
        }
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
    scan_dir_recursive(&base, "", &known_paths, &mut discovered, 0);

    // Re-load so a concurrent record_download (e.g. a Luna download that
    // finished after we built `known_paths`) is not clobbered, and dedupe by
    // path so the same file never appears twice in history.
    if !discovered.is_empty() {
        let mut latest = load_download_history();
        let existing: std::collections::HashSet<String> =
            latest.iter().map(|r| r.path.clone()).collect();
        for rec in discovered {
            if !existing.contains(&rec.path) {
                latest.push(rec);
            }
        }
        if latest.len() > 500 {
            latest.drain(0..latest.len() - 500);
        }
        let _ = save_download_history(&latest);
        records = latest;
    }

    records.retain(|r| !r.path.is_empty());
    for r in &mut records {
        r.file_exists = std::path::Path::new(&r.path).exists();
    }
    records.reverse();
    records
}

fn scan_dir_recursive(
    dir: &std::path::Path,
    course_folder: &str,
    known: &std::collections::HashSet<String>,
    discovered: &mut Vec<DownloadRecord>,
    depth: usize,
) {
    if depth > SCAN_MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(rec) = try_discover_file(&path, course_folder, known) {
                discovered.push(rec);
            }
        } else if path.is_dir() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            // Immediate parent folder becomes the course label. This matches
            // how live.rs records free notes (course_name="自由ノート") even
            // though they live under "その他/自由ノート/".
            scan_dir_recursive(&path, name, known, discovered, depth + 1);
        }
    }
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

    let mut hasher = DefaultHasher::new();
    path_str.hash(&mut hasher);
    let path_hash = hasher.finish();

    Some(DownloadRecord {
        id: format!("scan_{:x}", path_hash),
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
    let query_course = course_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let mut found: Option<DownloadRecord> = None;
    for r in records.iter().rev() {
        let rname = r.filename.to_lowercase();
        if rname != target {
            continue;
        }
        // When caller supplies a course name, require an exact match. Records
        // with an empty course_name (legacy, or saved with classify disabled)
        // are treated as non-matches to avoid false positives across courses.
        if let Some(cn) = query_course {
            if r.course_name != cn {
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
    found
}

/// Validate that `path` exists and resolves under one of the allowed download
/// roots (app default `~/Documents/Selah`, OS Downloads, or the user's custom
/// download dir). Returns the canonical path on success.
fn validate_downloads_path(path: &str) -> Result<std::path::PathBuf, String> {
    let p = std::path::Path::new(path);
    if !p.exists() {
        return Err("ファイルが見つかりません".into());
    }
    let canonical = p
        .canonicalize()
        .map_err(|e| format!("パスが無効です: {}", e))?;
    let app_default = default_download_dir()
        .canonicalize()
        .unwrap_or_else(|_| default_download_dir());
    let sys_downloads_raw = dirs::download_dir().unwrap_or_else(|| {
        dirs::home_dir()
            .map(|h| h.join("Downloads"))
            .unwrap_or_else(std::env::temp_dir)
    });
    // Canonicalize sys_downloads so the starts_with comparison works correctly
    // on Windows where canonicalize() adds the \\?\\ extended-path prefix.
    let sys_downloads = sys_downloads_raw
        .canonicalize()
        .unwrap_or(sys_downloads_raw);
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
    Ok(canonical)
}

fn is_markdown_ext(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
        .unwrap_or(false)
}

#[tauri::command]
pub async fn open_downloaded_file(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let canonical = validate_downloads_path(&path)?;
    if is_markdown_ext(&canonical) {
        return open_markdown_file_window(app, canonical.to_string_lossy().to_string()).await;
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(canonical.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("ファイルを開けませんでした: {}", e))?;
    Ok(())
}

/// Open a downloaded file with the OS default app, bypassing the built-in
/// Markdown reader. Used by the reader's "外部で開く" button.
#[tauri::command]
pub fn open_downloaded_file_external(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let canonical = validate_downloads_path(&path)?;
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(canonical.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("ファイルを開けませんでした: {}", e))?;
    Ok(())
}

/// Max size of a markdown file the in-app reader will load. Larger files are
/// pushed to the external opener.
const MARKDOWN_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// Pending payloads keyed by window label. The markdown reader window pulls
/// from here on startup to avoid a race where the backend emits the
/// `markdown-content` event before the page's listener attaches.
static PENDING_MARKDOWN_PAYLOADS: LazyLock<Mutex<HashMap<String, serde_json::Value>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn markdown_window_label(canonical: &std::path::Path) -> String {
    let mut hasher = DefaultHasher::new();
    canonical.to_string_lossy().hash(&mut hasher);
    format!("md-reader-{:x}", hasher.finish())
}

/// Open (or focus) the in-app Markdown reader window for the given file.
#[tauri::command]
pub async fn open_markdown_file_window(app: tauri::AppHandle, path: String) -> Result<(), String> {
    use tauri::{Emitter, Manager};
    let canonical = validate_downloads_path(&path)?;
    let meta = std::fs::metadata(&canonical).map_err(|e| format!("読み込み失敗: {}", e))?;
    if meta.len() > MARKDOWN_MAX_BYTES {
        // Fall back to the system opener for oversized files so the user can
        // still get to them.
        return open_downloaded_file_external(app, canonical.to_string_lossy().to_string());
    }
    let bytes = std::fs::read(&canonical).map_err(|e| format!("読み込み失敗: {}", e))?;
    let markdown = String::from_utf8_lossy(&bytes).to_string();
    let filename = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Markdown")
        .to_string();
    let path_str = canonical.to_string_lossy().to_string();
    let label = markdown_window_label(&canonical);
    let payload = serde_json::json!({
        "path": path_str,
        "filename": filename.clone(),
        "markdown": markdown,
        "error": serde_json::Value::Null,
    });

    if let Ok(mut map) = PENDING_MARKDOWN_PAYLOADS.lock() {
        map.insert(label.clone(), payload.clone());
    }

    if let Some(win) = app.get_webview_window(&label) {
        let _ = win.emit_to(&label, "markdown-content", &payload);
        let _ = win.set_focus();
        return Ok(());
    }

    let win = tauri::WebviewWindowBuilder::new(
        &app,
        &label,
        tauri::WebviewUrl::App("markdown-reader.html".into()),
    )
    .title(&filename)
    .inner_size(820.0, 720.0)
    .min_inner_size(420.0, 360.0)
    .resizable(true)
    .build()
    .map_err(|e| format!("ウィンドウ作成失敗: {}", e))?;

    // Backup emit after the page has had time to attach its listener. The page
    // also pulls via `get_pending_markdown_payload`, so whichever path lands
    // first wins. Use a longer delay on all platforms to account for slower
    // WebView2 initialisation on Windows.
    let payload_clone = payload.clone();
    let label_clone = label.clone();
    let win_clone = win.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        let _ = win_clone.emit_to(&label_clone, "markdown-content", &payload_clone);
    });

    Ok(())
}

/// Called by the markdown reader on init to fetch its payload synchronously.
/// Removes the entry so subsequent calls return null.
#[tauri::command]
pub fn get_pending_markdown_payload(label: String) -> Option<serde_json::Value> {
    PENDING_MARKDOWN_PAYLOADS
        .lock()
        .ok()
        .and_then(|mut m| m.remove(&label))
}

/// Write Markdown contents back to disk. Restricted to .md/.markdown files
/// inside the allowed download roots, with a size cap matching the reader's.
#[tauri::command]
pub fn write_markdown_file(path: String, contents: String) -> Result<(), String> {
    let canonical = validate_downloads_path(&path)?;
    if !is_markdown_ext(&canonical) {
        return Err("Markdown ファイルのみ編集できます".into());
    }
    if contents.len() as u64 > MARKDOWN_MAX_BYTES {
        return Err("ファイルが大きすぎます（8MBを超えるMarkdownはサポートしていません）".into());
    }
    std::fs::write(&canonical, contents.as_bytes())
        .map_err(|e| format!("保存失敗: {}", e))?;
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

    let mut path_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
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
