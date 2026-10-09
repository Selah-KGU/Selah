//! Idempotent download-folder migrations.

use super::*;
use std::sync::Mutex;

static DOWNLOAD_MIGRATION_LOCK: Mutex<()> = Mutex::new(());

fn download_migration_lock() -> std::sync::MutexGuard<'static, ()> {
    DOWNLOAD_MIGRATION_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Idempotent download-folder migrations. Order matters: move loose files,
/// rename course folders, normalize history names, then drop duplicate rows.
/// One lock covers the whole sequence so a settings save cannot interleave.
pub fn run_download_migrations() {
    let _guard = download_migration_lock();
    migrate_uncategorized_to_other_impl();
    migrate_rename_course_folders_impl();
    migrate_normalize_course_names_impl();
    migrate_deduplicate_by_filename_impl();
}

pub fn migrate_uncategorized_to_other() {
    let _guard = download_migration_lock();
    migrate_uncategorized_to_other_impl();
}

/// Rewrite each history entry's `course_name` to its simplified, sanitized
/// form. Pre-normalization, the same logical course often appeared under
/// multiple buckets (full dept-coded name from `record_download`, simplified
/// folder name from `scan_download_dir`). Idempotent.
fn migrate_normalize_course_names_impl() {
    apply_history_migration("normalize course names", normalize_course_names);
}

fn apply_history_migration(name: &str, mutate: impl FnOnce(&mut Vec<DownloadRecord>) -> bool) {
    if let Err(error) = download_history_store().update(mutate) {
        log::warn!("Download history migration ({name}) failed: {error}");
    }
}

pub(super) fn normalize_course_names(records: &mut Vec<DownloadRecord>) -> bool {
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
    changed
}

/// Remove duplicate history entries caused by file migration (old path no
/// longer exists but a new path for the same filename was inserted by
/// scan_download_dir). Keeps the entry whose file actually exists; if both
/// exist (unlikely) keeps the more-recent one. Idempotent.
fn migrate_deduplicate_by_filename_impl() {
    apply_history_migration("deduplicate filenames", deduplicate_history_records);
}

pub(super) fn deduplicate_history_records(records: &mut Vec<DownloadRecord>) -> bool {
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
    let mut i = 0;
    records.retain(|_| {
        let keep = keep_set.contains(&i);
        i += 1;
        keep
    });
    records.len() != original_len
}

/// Move files sitting at the root of the download base dir into `その他/`.
/// Only runs when `classify_by_course` is enabled. Idempotent: a second run is a no-op.
fn migrate_uncategorized_to_other_impl() {
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

    apply_history_migration("move uncategorized paths", |records| {
        apply_uncategorized_paths(records, &path_map)
    });
}

pub(super) fn apply_uncategorized_paths(
    records: &mut Vec<DownloadRecord>,
    path_map: &std::collections::HashMap<String, String>,
) -> bool {
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
    changed
}

/// Rename course subdirectories whose name does not match the simplified form
/// (e.g. "水４・金２ 日本語I ４" → "日本語I ４"). Files inside are moved to the
/// canonical folder; history paths are updated accordingly. Idempotent.
fn migrate_rename_course_folders_impl() {
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

    // File moves have completed; rewrite the latest history under its lock.
    apply_history_migration("rename course paths", |records| {
        apply_renamed_course_paths(records, &path_map)
    });
}

pub(super) fn apply_renamed_course_paths(
    records: &mut Vec<DownloadRecord>,
    path_map: &std::collections::HashMap<String, String>,
) -> bool {
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
    changed
}
