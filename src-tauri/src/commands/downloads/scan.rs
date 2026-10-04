//! Scan the download directory for files missing from history.

use super::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Maximum depth for `scan_download_dir` recursion. The expected layout is at
/// most `base/<course>/<free_note_or_similar>/<file>` (3 levels); allow a bit
/// more for user-organized subfolders while still bounding the traversal.
const SCAN_MAX_DEPTH: usize = 6;

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
    annotate_records(&mut records);
    records.reverse();
    records
}

pub(super) fn scan_dir_recursive(
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
            // The course label is the FIRST folder under base; any deeper folder
            // is in-course organization (the 第NN回 / 教材 / 課題 theme folders the
            // organizer creates) and must keep the course's name, not the theme's.
            // Otherwise filing a course's files would fragment it into one fake
            // "course" per theme folder in the downloads sidebar / duplicate scan.
            // Normalize the top-level name so scanned files get the same simplified
            // course_name as record_download writes (which also simplifies),
            // keeping "日本語" and "日本語 2025" from splitting into two groups.
            let next_label: String = if course_folder.is_empty() {
                let simplified = sanitize_path_component(&simplify_course_name(name));
                if simplified.is_empty() {
                    name.to_string()
                } else {
                    simplified
                }
            } else {
                course_folder.to_string()
            };
            scan_dir_recursive(&path, &next_label, known, discovered, depth + 1);
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
    // Skip hidden files, OS junk, and Office lock/owner files (`~$report.docx`),
    // which are transient 0-byte artifacts — not real downloads. Surfacing them
    // pollutes the downloads list and the duplicate scanner.
    if filename.starts_with('.')
        || filename.starts_with("~$")
        || filename == "desktop.ini"
        || filename == "Thumbs.db"
    {
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
        subfolder: String::new(),
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
