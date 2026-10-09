//! Download history records.

use super::history_store::HistoryStore;
use super::*;
use crate::client;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonic per-process counter appended to timestamps so two records created
/// within the same millisecond get distinct ids.
static DOWNLOAD_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Theme subfolder the file sits in *within* its course folder (the 第NN回 /
    /// 課題 / 教材 folders the organizer creates), relative and "/"-joined. Empty
    /// when the file is directly in the course root. Surfaced so the file page can
    /// mirror the organized structure instead of flattening it. Derived from the
    /// path on read; not persisted as authoritative.
    #[serde(default)]
    pub subfolder: String,
}

fn download_history_path() -> std::path::PathBuf {
    client::data_dir().join("download_history.json")
}

pub(super) fn download_history_store() -> HistoryStore {
    HistoryStore::new(download_history_path())
}

pub(super) fn load_download_history() -> Result<Vec<DownloadRecord>, String> {
    download_history_store().read()
}

/// Record a new download in the history. Called from save_to_downloads.
pub fn record_download(
    filename: &str,
    path: &str,
    course_name: Option<&str>,
    source: &str,
    size_bytes: u64,
) {
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
        subfolder: String::new(),
    };
    if let Err(error) = download_history_store().update(|records| {
        upsert_download_record(records, record);
        true
    }) {
        log::warn!("Recording download history failed: {error}");
    }
}

// Preserve the existing position when a scan record is replaced by its actual
// download, and only trim the oldest entries when a new path is appended.
pub(super) fn upsert_download_record(records: &mut Vec<DownloadRecord>, record: DownloadRecord) {
    if let Some(existing) = records.iter_mut().find(|r| r.path == record.path) {
        *existing = record;
    } else {
        records.push(record);
        if records.len() > 500 {
            records.drain(0..records.len() - 500);
        }
    }
}

#[tauri::command]
pub async fn list_downloads() -> Result<tauri::ipc::Response, String> {
    crate::background_ipc::respond(
        "Download history worker failed",
        "Download history encoding failed",
        list_downloads_snapshot,
    )
    .await
}

pub(crate) fn list_downloads_snapshot() -> Result<Vec<DownloadRecord>, String> {
    let mut records = load_download_history()?;
    records.retain(|r| !r.path.is_empty());
    annotate_records(&mut records);
    records.reverse();
    Ok(records)
}

/// Fills `file_exists` and the derived `subfolder` for each record.
pub(super) fn annotate_records(records: &mut [DownloadRecord]) {
    let base = download_base();
    for r in records.iter_mut() {
        r.file_exists = std::path::Path::new(&r.path).exists();
        r.subfolder = theme_subfolder(&r.path, &base);
    }
}

#[tauri::command]
pub async fn check_files_downloaded(
    filenames: Vec<String>,
    course_name: Option<String>,
) -> Result<tauri::ipc::Response, String> {
    crate::background_ipc::respond(
        "Download checks worker failed",
        "Download checks encoding failed",
        move || {
            let records = load_download_history()?;
            Ok(check_downloaded_records(
                &records,
                filenames,
                course_name.as_deref(),
            ))
        },
    )
    .await
}

pub(super) fn check_downloaded_records(
    records: &[DownloadRecord],
    filenames: Vec<String>,
    course_name: Option<&str>,
) -> HashMap<String, DownloadRecord> {
    let mut found = HashMap::new();
    for filename in filenames {
        if filename.trim().is_empty() {
            continue;
        }
        if let Some(record) = find_downloaded_record(records, &filename, course_name) {
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

#[tauri::command]
pub async fn remove_download_record(id: String) -> Result<(), String> {
    remove_download_records(vec![id]).await
}

#[tauri::command]
pub async fn remove_download_records(ids: Vec<String>) -> Result<(), String> {
    crate::background_ipc::run("Download history removal worker failed", move || {
        let ids: HashSet<String> = ids.into_iter().collect();
        download_history_store()
            .update(|records| retain_download_records(records, |r| !ids.contains(&r.id)))
            .map(|_| ())
    })
    .await
}

pub(super) fn retain_download_records(
    records: &mut Vec<DownloadRecord>,
    keep: impl FnMut(&DownloadRecord) -> bool,
) -> bool {
    let before = records.len();
    records.retain(keep);
    records.len() != before
}

/// Remove history for successfully deleted files in one transaction.
pub(super) fn remove_download_records_by_paths(paths: &HashSet<String>) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    download_history_store()
        .update(|records| retain_download_records(records, |r| !paths.contains(&r.path)))
        .map(|_| ())
}

/// Used by LIVE cleanup after a partial file is removed.
pub fn remove_download_records_by_path(path: &str) {
    let paths = HashSet::from([path.to_string()]);
    if let Err(error) = remove_download_records_by_paths(&paths) {
        log::warn!("Removing deleted file from download history failed: {error}");
    }
}

#[tauri::command]
pub async fn clear_download_history() -> Result<(), String> {
    crate::background_ipc::run("Download history clear worker failed", || {
        download_history_store().clear()
    })
    .await
}
