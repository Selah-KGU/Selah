//! Detect and delete duplicate downloaded files.

use super::*;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::Read;

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
pub async fn scan_duplicate_downloads() -> Result<tauri::ipc::Response, String> {
    crate::background_ipc::respond(
        "Duplicate scan worker failed",
        "Duplicate scan encoding failed",
        || find_duplicate_downloads(scan_download_dir_snapshot()?, validate_downloads_path),
    )
    .await
}

fn find_duplicate_downloads(
    records: Vec<DownloadRecord>,
    validate: impl Fn(&str) -> Result<std::path::PathBuf, String>,
) -> Result<Vec<DuplicateFileGroup>, String> {
    let mut by_size: HashMap<u64, Vec<DownloadRecord>> = HashMap::new();
    for mut record in records {
        if record.size_bytes == 0 || record.path.trim().is_empty() {
            continue;
        }
        let path = std::path::Path::new(&record.path);
        if !path.is_file() {
            continue;
        }
        if validate(&record.path).is_err() {
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
pub async fn cleanup_duplicate_downloads(
    paths: Vec<String>,
) -> Result<tauri::ipc::Response, String> {
    delete_downloaded_files(paths).await
}

#[tauri::command]
pub async fn delete_downloaded_files(paths: Vec<String>) -> Result<tauri::ipc::Response, String> {
    crate::background_ipc::respond(
        "File deletion worker failed",
        "File deletion encoding failed",
        move || {
            delete_files_and_history(
                paths,
                validate_downloads_path,
                super::history::remove_download_records_by_paths,
            )
        },
    )
    .await
}

fn delete_files_and_history(
    paths: Vec<String>,
    validate: impl Fn(&str) -> Result<std::path::PathBuf, String>,
    remove_history: impl FnOnce(&HashSet<String>) -> Result<(), String>,
) -> Result<DuplicateCleanupResult, String> {
    let mut deleted_count = 0usize;
    let mut failed_count = 0usize;
    let mut errors = Vec::new();
    let mut deleted_paths = HashSet::new();

    for path in paths {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            continue;
        }
        match validate(trimmed) {
            Ok(canonical) => match std::fs::remove_file(&canonical) {
                Ok(_) => {
                    deleted_count += 1;
                    deleted_paths.insert(canonical.to_string_lossy().to_string());
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
    if !deleted_paths.is_empty() {
        if let Err(error) = remove_history(&deleted_paths) {
            // Files are already deleted; retain their accurate deletion counts
            // and report the metadata failure without claiming deletion failed.
            errors.push(format!("Download history cleanup failed: {error}"));
        }
    }
    Ok(DuplicateCleanupResult {
        deleted_count,
        failed_count,
        errors,
    })
}

#[cfg(test)]
#[path = "duplicates_tests.rs"]
mod tests;
