use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::db::Database;

const CACHE_KEY: &str = "read_state";
const MAX_IDS_PER_SOURCE: usize = 500;

#[derive(Debug, Serialize, Deserialize, Default)]
struct ReadData {
    kgc: HashSet<String>,
    luna: HashSet<String>,
    kwic: HashSet<String>,
}

fn cap(set: &mut HashSet<String>) {
    if set.len() > MAX_IDS_PER_SOURCE {
        let excess = set.len() - MAX_IDS_PER_SOURCE;
        let to_remove: Vec<String> = set.iter().take(excess).cloned().collect();
        for k in to_remove {
            set.remove(&k);
        }
    }
}

fn load(db: &Database) -> Result<ReadData, String> {
    load_from(db, || crate::client::data_dir().join("read_items.json"))
}

fn load_from(
    db: &Database,
    legacy_path: impl FnOnce() -> std::path::PathBuf,
) -> Result<ReadData, String> {
    match db.get_data_cache(CACHE_KEY)? {
        Some((json, _)) => {
            serde_json::from_str(&json).map_err(|error| format!("既読データの解析失敗: {error}"))
        }
        None => {
            // One-time migration: try loading from old JSON file
            let path = legacy_path();
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(ReadData::default());
                }
                Err(error) => return Err(format!("旧既読データの読み取り失敗: {error}")),
            };
            let data: ReadData = serde_json::from_slice(&bytes)
                .map_err(|error| format!("旧既読データの解析失敗: {error}"))?;
            // A failed migration keeps the original bytes and rejects the
            // operation; it cannot acknowledge an uncommitted mark as read.
            persist(db, &data)?;
            if let Err(error) = std::fs::remove_file(&path) {
                log::warn!("read state migration committed but source cleanup failed: {error}");
            }
            log::info!("Migrated read_items.json to database");
            Ok(data)
        }
    }
}

fn persist(db: &Database, data: &ReadData) -> Result<(), String> {
    let json =
        serde_json::to_string(data).map_err(|error| format!("既読データの変換失敗: {error}"))?;
    db.save_data_cache(CACHE_KEY, &json)
}

pub fn mark_read(db: &Database, source: &str, id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 512 || !matches!(source, "kgc" | "luna" | "kwic") {
        return Ok(());
    }
    let mut data = load(db)?;
    let set = match source {
        "kgc" => &mut data.kgc,
        "luna" => &mut data.luna,
        "kwic" => &mut data.kwic,
        _ => unreachable!(),
    };
    // Skip the JSON serialize + DB write when the id is already known —
    // marking the same notification read twice is common (UI re-renders,
    // duplicate clicks) and persisting unchanged data is wasted I/O.
    let inserted = !set.contains(id) && set.insert(id.to_string());
    let needs_cap = set.len() > MAX_IDS_PER_SOURCE;
    if !inserted && !needs_cap {
        return Ok(());
    }
    cap(set);
    persist(db, &data)
}

pub fn mark_batch_read<I, S>(db: &Database, source: &str, ids: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    if !matches!(source, "kgc" | "luna" | "kwic") {
        return Ok(());
    }
    let mut data = load(db)?;
    let set = match source {
        "kgc" => &mut data.kgc,
        "luna" => &mut data.luna,
        "kwic" => &mut data.kwic,
        _ => unreachable!(),
    };
    let mut changed = false;
    for id in ids {
        let id = id.as_ref();
        if !id.is_empty() && id.len() <= 512 && !set.contains(id) && set.insert(id.to_owned()) {
            changed = true;
        }
    }
    if !changed && set.len() <= MAX_IDS_PER_SOURCE {
        return Ok(());
    }
    cap(set);
    persist(db, &data)
}

pub fn get_all_read_ids(db: &Database) -> Result<ReadIdsResponse, String> {
    let data = load(db)?;
    Ok(ReadIdsResponse {
        kgc: data.kgc.into_iter().collect(),
        luna: data.luna.into_iter().collect(),
        kwic: data.kwic.into_iter().collect(),
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReadIdsResponse {
    pub kgc: Vec<String>,
    pub luna: Vec<String>,
    pub kwic: Vec<String>,
}

#[cfg(test)]
#[path = "read_state/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "read_state/errors_before.rs"]
pub(crate) mod errors_before;

// ── Seen notification IDs (push dedup) ──

const SEEN_CACHE_PREFIX: &str = "seen_notifs_";
const SEEN_LUNA_OBJECTS_KEY: &str = "seen_notifs_luna_objects";
const SEEN_INIT_PREFIX: &str = "seen_notifs_init_";
const SEEN_BOOTSTRAP_STARTED_AT_KEY: &str = "seen_notifs_bootstrap_started_at";
const SEEN_BOOTSTRAP_COMPLETE_KEY: &str = "seen_notifs_bootstrap_complete";
const SEEN_FORMAT_VERSION_KEY: &str = "seen_notifs_format_version";
// v2: luna revision keys strip the volatile trailing timestamp from the body.
pub const CURRENT_SEEN_NOTIF_FORMAT_VERSION: u32 = 2;
const MAX_SEEN_IDS: usize = 2000;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LunaNotifSeenEntry {
    pub base_key: String,
    pub revision_key: String,
}

pub fn get_seen_notif_ids(db: &Database, source: &str) -> Vec<String> {
    let key = format!("{}{}", SEEN_CACHE_PREFIX, source);
    match db.get_data_cache(&key) {
        Ok(Some((json, _))) => serde_json::from_str(&json).unwrap_or_default(),
        _ => Vec::new(),
    }
}

pub fn save_seen_notif_ids(db: &Database, source: &str, ids: Vec<String>) {
    let key = format!("{}{}", SEEN_CACHE_PREFIX, source);
    // Keep only last MAX_SEEN_IDS
    let trimmed: Vec<String> = if ids.len() > MAX_SEEN_IDS {
        let skip = ids.len() - MAX_SEEN_IDS;
        ids.into_iter().skip(skip).collect()
    } else {
        ids
    };
    if let Ok(json) = serde_json::to_string(&trimmed) {
        let _ = db.save_data_cache_if_changed(&key, &json);
    }
}

pub fn get_luna_notif_seen_entries(db: &Database) -> Vec<LunaNotifSeenEntry> {
    match db.get_data_cache(SEEN_LUNA_OBJECTS_KEY) {
        Ok(Some((json, _))) => serde_json::from_str(&json).unwrap_or_default(),
        _ => Vec::new(),
    }
}

pub fn save_luna_notif_seen_entries(db: &Database, entries: Vec<LunaNotifSeenEntry>) {
    let trimmed: Vec<LunaNotifSeenEntry> = if entries.len() > MAX_SEEN_IDS {
        let skip = entries.len() - MAX_SEEN_IDS;
        entries.into_iter().skip(skip).collect()
    } else {
        entries
    };
    if let Ok(json) = serde_json::to_string(&trimmed) {
        let _ = db.save_data_cache_if_changed(SEEN_LUNA_OBJECTS_KEY, &json);
    }
}

pub fn is_seen_notif_initialized(db: &Database, source: &str) -> bool {
    let key = format!("{}{}", SEEN_INIT_PREFIX, source);
    match db.get_data_cache(&key) {
        Ok(Some((json, _))) => serde_json::from_str::<bool>(&json).unwrap_or(false),
        _ => false,
    }
}

pub fn mark_seen_notif_initialized(db: &Database, source: &str) {
    if is_seen_notif_initialized(db, source) {
        return;
    }
    let key = format!("{}{}", SEEN_INIT_PREFIX, source);
    if let Ok(json) = serde_json::to_string(&true) {
        let _ = db.save_data_cache_if_changed(&key, &json);
    }
}

pub fn has_seen_notif_state(db: &Database, source: &str) -> bool {
    is_seen_notif_initialized(db, source) || !get_seen_notif_ids(db, source).is_empty()
}

pub fn get_seen_notif_bootstrap_started_at(db: &Database) -> Option<i64> {
    match db.get_data_cache(SEEN_BOOTSTRAP_STARTED_AT_KEY) {
        Ok(Some((json, _))) => serde_json::from_str::<i64>(&json).ok(),
        _ => None,
    }
}

pub fn mark_seen_notif_bootstrap_started_at(db: &Database, started_at: i64) {
    if let Ok(json) = serde_json::to_string(&started_at) {
        let _ = db.save_data_cache(SEEN_BOOTSTRAP_STARTED_AT_KEY, &json);
    }
}

pub fn is_seen_notif_bootstrap_complete(db: &Database) -> bool {
    match db.get_data_cache(SEEN_BOOTSTRAP_COMPLETE_KEY) {
        Ok(Some((json, _))) => serde_json::from_str::<bool>(&json).unwrap_or(false),
        _ => false,
    }
}

pub fn mark_seen_notif_bootstrap_complete(db: &Database) {
    if let Ok(json) = serde_json::to_string(&true) {
        let _ = db.save_data_cache(SEEN_BOOTSTRAP_COMPLETE_KEY, &json);
    }
}

pub fn get_seen_notif_format_version(db: &Database) -> u32 {
    match db.get_data_cache(SEEN_FORMAT_VERSION_KEY) {
        Ok(Some((json, _))) => serde_json::from_str::<u32>(&json).unwrap_or(0),
        _ => 0,
    }
}

pub fn mark_seen_notif_format_version(db: &Database, version: u32) {
    if let Ok(json) = serde_json::to_string(&version) {
        let _ = db.save_data_cache(SEEN_FORMAT_VERSION_KEY, &json);
    }
}

/// Reset all seen-notification state across sources. Used for format migrations;
/// after reset, each source goes back through the silent seed path on next sync.
pub fn reset_all_seen_notif_state(db: &Database) {
    for source in &["kgc", "luna", "kwic", "mail"] {
        let _ = db.save_data_cache(&format!("{}{}", SEEN_CACHE_PREFIX, source), "[]");
        let _ = db.save_data_cache(&format!("{}{}", SEEN_INIT_PREFIX, source), "false");
    }
    let _ = db.save_data_cache(SEEN_LUNA_OBJECTS_KEY, "[]");
    let _ = db.save_data_cache(SEEN_BOOTSTRAP_STARTED_AT_KEY, "null");
    let _ = db.save_data_cache(SEEN_BOOTSTRAP_COMPLETE_KEY, "false");
}
