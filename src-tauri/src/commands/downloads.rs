use crate::client;
use base64::Engine;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Read;
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
    // Strip trailing bare year / year-range, e.g. " 2025", "_2024-2025",
    // "（2025年度）", "(2025)" that don't contain a term keyword.
    static RE_YEAR_SUFFIX: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"[\s_\-–・]*[（(]?\d{4}(?:[\-–]\d{2,4})?(?:年度?)?[)）]?\s*$").unwrap()
    });
    // Strip leading schedule prefix like "水４・金２ " or "月３金４ ".
    // Pattern: one or more (day-char + half/full-width digit) joined by ・/・/space,
    // followed by whitespace.
    static RE_SCHEDULE_PREFIX: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"^(?:[月火水木金土日][０-９0-9][\s・・]*)+").unwrap()
    });

    let s = RE_DEPT_CODE.replace(name, "");
    let s = RE_SCHEDULE_PREFIX.replace(&s, "");
    let s = RE_BRACKET.replace_all(&s, "");
    let s = RE_PAREN_SUFFIX.replace_all(&s, "");
    let s = RE_YEAR_SUFFIX.replace_all(&s, "");
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

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateFileItem {
    pub id: String,
    pub filename: String,
    pub path: String,
    pub course_name: String,
    pub source: String,
    pub size_bytes: u64,
    pub downloaded_at: i64,
    pub file_exists: bool,
    pub is_recommended: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateFileGroup {
    pub content_hash: String,
    pub size_bytes: u64,
    pub items: Vec<DuplicateFileItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateCleanupResult {
    pub deleted_count: usize,
    pub failed_count: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadPreview {
    pub kind: String,
    pub mime: String,
    pub data_url: Option<String>,
    pub text: Option<String>,
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
    // Normalize course name to its simplified form. Without this, downloads
    // recorded by Luna (full name with dept code and term suffix) and
    // entries discovered by scan_download_dir (folder name = simplified)
    // would land in two separate buckets even though they're the same course.
    let course_label = match course_name.map(str::trim).filter(|s| !s.is_empty()) {
        Some(c) => sanitize_path_component(&simplify_course_name(c)),
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
                // Remove any stale (file-missing) records with the same
                // filename so the moved/reclassified file doesn't appear twice.
                // Only remove stale entries for the SAME (filename, course_name)
                // pair so same-named files in different courses are not dropped.
                let fname_lower = rec.filename.to_lowercase();
                let course_lower = rec.course_name.to_lowercase();
                latest.retain(|r| {
                    r.path == rec.path
                        || r.filename.to_lowercase() != fname_lower
                        || r.course_name.to_lowercase() != course_lower
                        || std::path::Path::new(&r.path).exists()
                });
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
            // Immediate parent folder becomes the course label. Normalize the
            // folder name so newly discovered files get the same simplified
            // course_name as records written by record_download (which also
            // calls simplify_course_name). This prevents the sidebar from
            // showing both "日本語" and "日本語 2025" as separate groups.
            let simplified_name = sanitize_path_component(&simplify_course_name(name));
            let label: &str = if simplified_name.is_empty() {
                name
            } else {
                simplified_name.as_str()
            };
            scan_dir_recursive(&path, label, known, discovered, depth + 1);
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
        source: infer_scanned_source(filename, course_folder).to_string(),
        size_bytes: metadata.len(),
        downloaded_at: modified,
        file_exists: true,
    })
}

fn infer_scanned_source(filename: &str, course_folder: &str) -> &'static str {
    let folder = course_folder.trim();
    let file_lower = filename.to_lowercase();
    if folder == "自由ノート" || file_lower.ends_with("_live.md") {
        return "live";
    }
    if folder.is_empty() {
        "scan"
    } else {
        "luna"
    }
}

fn file_sha256(path: &std::path::Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("{} を開けませんでした: {}", path.display(), e))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("{} を読み込めませんでした: {}", path.display(), e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn recommend_duplicate_keep(items: &[DownloadRecord]) -> usize {
    items
        .iter()
        .enumerate()
        .max_by_key(|(_, r)| {
            let source_score = match r.source.as_str() {
                "luna" => 4,
                "live" => 3,
                "mail" => 2,
                "kwic" => 1,
                _ => 0,
            };
            (source_score, r.downloaded_at, r.path.len())
        })
        .map(|(idx, _)| idx)
        .unwrap_or(0)
}

#[tauri::command]
pub fn scan_duplicate_downloads() -> Result<Vec<DuplicateFileGroup>, String> {
    let records = scan_download_dir();
    let mut by_size: HashMap<u64, Vec<DownloadRecord>> = HashMap::new();
    for mut record in records {
        if record.size_bytes == 0 || record.path.trim().is_empty() {
            continue;
        }
        let path = std::path::Path::new(&record.path);
        if !path.is_file() {
            continue;
        }
        if validate_downloads_path(&record.path).is_err() {
            continue;
        }
        record.file_exists = true;
        by_size.entry(record.size_bytes).or_default().push(record);
    }

    let mut groups = Vec::new();
    for (size, same_size) in by_size {
        if same_size.len() < 2 {
            continue;
        }
        let mut by_hash: HashMap<String, Vec<DownloadRecord>> = HashMap::new();
        for record in same_size {
            let hash = file_sha256(std::path::Path::new(&record.path))?;
            by_hash.entry(hash).or_default().push(record);
        }
        for (hash, mut items) in by_hash {
            if items.len() < 2 {
                continue;
            }
            items.sort_by(|a, b| b.downloaded_at.cmp(&a.downloaded_at));
            let keep_idx = recommend_duplicate_keep(&items);
            let flat_items = items
                .into_iter()
                .enumerate()
                .map(|(idx, r)| DuplicateFileItem {
                    id: r.id,
                    filename: r.filename,
                    path: r.path,
                    course_name: r.course_name,
                    source: r.source,
                    size_bytes: r.size_bytes,
                    downloaded_at: r.downloaded_at,
                    file_exists: r.file_exists,
                    is_recommended: idx == keep_idx,
                })
                .collect();
            groups.push(DuplicateFileGroup {
                content_hash: hash,
                size_bytes: size,
                items: flat_items,
            });
        }
    }

    groups.sort_by(|a, b| {
        let a_waste = a
            .size_bytes
            .saturating_mul(a.items.len().saturating_sub(1) as u64);
        let b_waste = b
            .size_bytes
            .saturating_mul(b.items.len().saturating_sub(1) as u64);
        b_waste.cmp(&a_waste)
    });
    Ok(groups)
}

#[tauri::command]
pub fn cleanup_duplicate_downloads(paths: Vec<String>) -> Result<DuplicateCleanupResult, String> {
    delete_downloaded_files(paths)
}

#[tauri::command]
pub fn delete_downloaded_files(paths: Vec<String>) -> Result<DuplicateCleanupResult, String> {
    let mut deleted_count = 0usize;
    let mut failed_count = 0usize;
    let mut errors = Vec::new();

    for path in paths {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            continue;
        }
        match validate_downloads_path(trimmed) {
            Ok(canonical) => match std::fs::remove_file(&canonical) {
                Ok(_) => {
                    deleted_count += 1;
                    remove_download_records_by_path(&canonical.to_string_lossy());
                }
                Err(e) => {
                    failed_count += 1;
                    errors.push(format!("{}: {}", canonical.display(), e));
                }
            },
            Err(e) => {
                failed_count += 1;
                errors.push(format!("{}: {}", trimmed, e));
            }
        }
    }

    Ok(DuplicateCleanupResult {
        deleted_count,
        failed_count,
        errors,
    })
}

#[tauri::command]
pub fn check_file_downloaded(
    filename: String,
    course_name: Option<String>,
) -> Option<DownloadRecord> {
    let records = load_download_history();
    find_downloaded_record(&records, &filename, course_name.as_deref())
}

#[tauri::command]
pub fn check_files_downloaded(
    filenames: Vec<String>,
    course_name: Option<String>,
) -> HashMap<String, DownloadRecord> {
    let records = load_download_history();
    let mut found = HashMap::new();
    for filename in filenames {
        if filename.trim().is_empty() {
            continue;
        }
        if let Some(record) = find_downloaded_record(&records, &filename, course_name.as_deref()) {
            found.insert(filename.clone(), record.clone());
            found.insert(filename.to_lowercase(), record);
        }
    }
    found
}

fn find_downloaded_record(
    records: &[DownloadRecord],
    filename: &str,
    course_name: Option<&str>,
) -> Option<DownloadRecord> {
    let target = filename.to_lowercase();
    // Compare via the simplified/canonical course name. The caller usually
    // passes the full course title (with dept code and term suffix), but
    // stored records hold the simplified form after normalization.
    let query_course = course_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|c| sanitize_path_component(&simplify_course_name(c)));
    let mut found: Option<DownloadRecord> = None;
    for r in records.iter().rev() {
        let rname = r.filename.to_lowercase();
        if rname != target {
            continue;
        }
        // When caller supplies a course name, require an exact match. Records
        // with an empty course_name (legacy, or saved with classify disabled)
        // are treated as non-matches to avoid false positives across courses.
        if let Some(cn) = &query_course {
            let stored = sanitize_path_component(&simplify_course_name(&r.course_name));
            if stored != *cn {
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

fn preview_mime(path: &std::path::Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "svg" => Some("image/svg+xml"),
        _ => None,
    }
}

fn is_text_preview_ext(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "md" | "markdown" | "txt" | "csv" | "json" | "log"
            )
        })
        .unwrap_or(false)
}

#[tauri::command]
pub fn get_download_preview(path: String) -> Result<Option<DownloadPreview>, String> {
    const IMAGE_PREVIEW_MAX_BYTES: u64 = 10 * 1024 * 1024;
    const TEXT_PREVIEW_MAX_BYTES: u64 = 512 * 1024;
    const TEXT_PREVIEW_CHARS: usize = 700;

    let canonical = validate_downloads_path(&path)?;
    let meta = std::fs::metadata(&canonical).map_err(|e| format!("読み込み失敗: {}", e))?;
    if !meta.is_file() {
        return Ok(None);
    }

    if let Some(mime) = preview_mime(&canonical) {
        if meta.len() > IMAGE_PREVIEW_MAX_BYTES {
            return Ok(None);
        }
        let bytes = std::fs::read(&canonical).map_err(|e| format!("読み込み失敗: {}", e))?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        return Ok(Some(DownloadPreview {
            kind: "image".to_string(),
            mime: mime.to_string(),
            data_url: Some(format!("data:{};base64,{}", mime, encoded)),
            text: None,
        }));
    }

    if is_text_preview_ext(&canonical) {
        if meta.len() > TEXT_PREVIEW_MAX_BYTES {
            return Ok(None);
        }
        let bytes = std::fs::read(&canonical).map_err(|e| format!("読み込み失敗: {}", e))?;
        let raw = String::from_utf8_lossy(&bytes);
        let preview = raw
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let text: String = preview.chars().take(TEXT_PREVIEW_CHARS).collect();
        if text.trim().is_empty() {
            return Ok(None);
        }
        return Ok(Some(DownloadPreview {
            kind: "text".to_string(),
            mime: "text/plain".to_string(),
            data_url: None,
            text: Some(text),
        }));
    }

    Ok(None)
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

/// Share a downloaded/material file through the native OS share surface. The
/// path is restricted to the same managed download roots used by file opening.
#[tauri::command]
pub fn share_downloaded_file_native(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let canonical = validate_downloads_path(&path)?;
    let file_name = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Markdown")
        .to_string();
    super::app_config::share_file_path_native(&app, &canonical, &file_name)
}

#[tauri::command]
pub fn share_downloaded_files_native(
    app: tauri::AppHandle,
    paths: Vec<String>,
) -> Result<(), String> {
    if paths.is_empty() {
        return Err("共有するファイルが選択されていません".into());
    }
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let canonical = validate_downloads_path(&path)?;
        let file_name = canonical
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        files.push((canonical, file_name));
    }
    super::app_config::share_file_paths_native(&app, &files)
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

fn markdown_payload_for_file(canonical: &std::path::Path, filename: &str) -> serde_json::Value {
    let path_str = canonical.to_string_lossy().to_string();
    match std::fs::read(canonical) {
        Ok(bytes) => serde_json::json!({
            "path": path_str,
            "filename": filename,
            "markdown": String::from_utf8_lossy(&bytes).to_string(),
            "error": serde_json::Value::Null,
        }),
        Err(e) => serde_json::json!({
            "path": path_str,
            "filename": filename,
            "markdown": "",
            "error": format!("読み込み失敗: {}", e),
        }),
    }
}

fn queue_markdown_payload_emit(
    win: tauri::WebviewWindow,
    label: String,
    canonical: std::path::PathBuf,
    filename: String,
) {
    use tauri::Emitter;
    tauri::async_runtime::spawn(async move {
        let error_path = canonical.to_string_lossy().to_string();
        let error_filename = filename.clone();
        let payload = match tokio::task::spawn_blocking(move || {
            markdown_payload_for_file(&canonical, &filename)
        })
        .await
        {
            Ok(payload) => payload,
            Err(e) => serde_json::json!({
                "path": error_path,
                "filename": error_filename,
                "markdown": "",
                "error": format!("読み込み失敗: {}", e),
            }),
        };
        if let Ok(mut map) = PENDING_MARKDOWN_PAYLOADS.lock() {
            map.insert(label.clone(), payload.clone());
        }
        let _ = win.emit_to(&label, "markdown-content", &payload);

        let payload_clone = payload.clone();
        let label_clone = label.clone();
        let win_clone = win.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
            let _ = win_clone.emit_to(&label_clone, "markdown-content", &payload_clone);
        });
    });
}

/// Open (or focus) the in-app Markdown reader window for the given file.
#[tauri::command]
pub async fn open_markdown_file_window(app: tauri::AppHandle, path: String) -> Result<(), String> {
    use tauri::Manager;
    let canonical = validate_downloads_path(&path)?;
    let meta = std::fs::metadata(&canonical).map_err(|e| format!("読み込み失敗: {}", e))?;
    if meta.len() > MARKDOWN_MAX_BYTES {
        // Fall back to the system opener for oversized files so the user can
        // still get to them.
        return open_downloaded_file_external(app, canonical.to_string_lossy().to_string());
    }
    let filename = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Markdown")
        .to_string();
    let label = markdown_window_label(&canonical);

    if let Some(win) = app.get_webview_window(&label) {
        let _ = win.set_focus();
        queue_markdown_payload_emit(win, label, canonical, filename);
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

    // Read and parse the file payload after the window is created so opening a
    // large-but-supported note does not block the native window from appearing.
    queue_markdown_payload_emit(win, label, canonical, filename);

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
    std::fs::write(&canonical, contents.as_bytes()).map_err(|e| format!("保存失敗: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn remove_download_record(id: String) -> Result<(), String> {
    let mut records = load_download_history();
    records.retain(|r| r.id != id);
    save_download_history(&records)
}

#[tauri::command]
pub fn remove_download_records(ids: Vec<String>) -> Result<(), String> {
    let ids: std::collections::HashSet<String> = ids.into_iter().collect();
    let mut records = load_download_history();
    records.retain(|r| !ids.contains(&r.id));
    save_download_history(&records)
}

/// Remove any download history entries whose path matches `path`. Used when a
/// file is being deleted from disk and we don't want a dangling "missing file"
/// entry in the downloads list.
pub fn remove_download_records_by_path(path: &str) {
    let mut records = load_download_history();
    let before = records.len();
    records.retain(|r| r.path != path);
    if records.len() != before {
        let _ = save_download_history(&records);
    }
}

#[tauri::command]
pub fn clear_download_history() -> Result<(), String> {
    save_download_history(&[])
}

/// Rewrite each history entry's `course_name` to its simplified, sanitized
/// form. Pre-normalization, the same logical course often appeared under
/// multiple buckets (full dept-coded name from `record_download`, simplified
/// folder name from `scan_download_dir`). Idempotent.
pub fn migrate_normalize_course_names() {
    let mut records = load_download_history();
    let mut changed = false;
    for r in records.iter_mut() {
        let trimmed = r.course_name.trim();
        if trimmed.is_empty() {
            continue;
        }
        let normalized = sanitize_path_component(&simplify_course_name(trimmed));
        if normalized != r.course_name {
            r.course_name = normalized;
            changed = true;
        }
    }
    if changed {
        let _ = save_download_history(&records);
    }
}

/// Remove duplicate history entries caused by file migration (old path no
/// longer exists but a new path for the same filename was inserted by
/// scan_download_dir). Keeps the entry whose file actually exists; if both
/// exist (unlikely) keeps the more-recent one. Idempotent.
pub fn migrate_deduplicate_by_filename() {
    let mut records = load_download_history();
    let original_len = records.len();
    // Group indices by lowercase filename. Prefer live files; among ties keep
    // the one with the larger downloaded_at timestamp.
    // Key on (filename, course_name) so same-named files in different courses
    // are treated as independent records and never collapsed into one.
    let mut keep: std::collections::HashMap<(String, String), usize> =
        std::collections::HashMap::new();
    for (i, r) in records.iter().enumerate() {
        let key = (r.filename.to_lowercase(), r.course_name.to_lowercase());
        let entry = keep.entry(key).or_insert(i);
        let prev = &records[*entry];
        let cur = &records[i];
        let prev_live = std::path::Path::new(&prev.path).exists();
        let cur_live = std::path::Path::new(&cur.path).exists();
        let prefer_current = (!prev_live && cur_live)
            || (prev_live == cur_live && cur.downloaded_at > prev.downloaded_at);
        if prefer_current {
            *entry = i;
        }
    }
    let keep_set: std::collections::HashSet<usize> = keep.values().copied().collect();
    records = records
        .into_iter()
        .enumerate()
        .filter_map(|(i, r)| if keep_set.contains(&i) { Some(r) } else { None })
        .collect();
    if records.len() != original_len {
        let _ = save_download_history(&records);
    }
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

/// Rename course subdirectories whose name does not match the simplified form
/// (e.g. "水４・金２ 日本語I ４" → "日本語I ４"). Files inside are moved to the
/// canonical folder; history paths are updated accordingly. Idempotent.
pub fn migrate_rename_course_folders() {
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

    // Collect all immediate subdirectories whose simplified name differs.
    let mut renames: Vec<(std::path::PathBuf, std::path::PathBuf)> = Vec::new();
    let Ok(entries) = std::fs::read_dir(&base) else {
        return;
    };
    for entry in entries.flatten() {
        let src = entry.path();
        if !src.is_dir() {
            continue;
        }
        let Some(raw_name) = src.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if raw_name.starts_with('.') {
            continue;
        }
        let simplified = sanitize_path_component(&simplify_course_name(raw_name));
        if simplified.is_empty() || simplified == raw_name {
            continue;
        }
        let dest = base.join(&simplified);
        renames.push((src, dest));
    }

    if renames.is_empty() {
        return;
    }

    // Build path rewrite map: old_file_path → new_file_path
    let mut path_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for (src_dir, dest_dir) in &renames {
        if let Err(e) = std::fs::create_dir_all(dest_dir) {
            log::warn!(
                "migrate_rename_course_folders: failed to create {:?}: {}",
                dest_dir,
                e
            );
            continue;
        }

        // Move every file from src_dir into dest_dir.
        let Ok(file_entries) = std::fs::read_dir(src_dir) else {
            continue;
        };
        for fe in file_entries.flatten() {
            let file_src = fe.path();
            if !file_src.is_file() {
                continue;
            }
            let Some(fname) = file_src.file_name().map(|n| n.to_os_string()) else {
                continue;
            };
            let mut file_dest = dest_dir.join(&fname);
            if file_dest.exists() {
                let stem = std::path::Path::new(&fname)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                let ext = std::path::Path::new(&fname)
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_default();
                let mut i = 1u32;
                loop {
                    let candidate = dest_dir.join(format!("{} ({}){}", stem, i, ext));
                    if !candidate.exists() {
                        file_dest = candidate;
                        break;
                    }
                    i += 1;
                    if i > 999 {
                        break;
                    }
                }
            }
            match std::fs::rename(&file_src, &file_dest) {
                Ok(()) => {
                    path_map.insert(
                        file_src.to_string_lossy().to_string(),
                        file_dest.to_string_lossy().to_string(),
                    );
                }
                Err(e) => log::warn!(
                    "migrate_rename_course_folders: move {:?} -> {:?}: {}",
                    file_src,
                    file_dest,
                    e
                ),
            }
        }
        // Remove now-empty source directory (best-effort)
        let _ = std::fs::remove_dir(src_dir);
    }

    // Update history records: fix both path and course_name
    let mut records = load_download_history();
    let mut changed = false;
    for r in records.iter_mut() {
        if let Some(new_path) = path_map.get(&r.path) {
            r.path = new_path.clone();
            changed = true;
        }
        let normalized = sanitize_path_component(&simplify_course_name(&r.course_name));
        if !normalized.is_empty() && normalized != r.course_name {
            r.course_name = normalized;
            changed = true;
        }
    }
    if changed {
        let _ = save_download_history(&records);
    }
}
