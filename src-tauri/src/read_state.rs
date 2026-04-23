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

fn load(db: &Database) -> ReadData {
    match db.get_data_cache(CACHE_KEY) {
        Ok(Some((json, _))) => serde_json::from_str(&json).unwrap_or_default(),
        _ => {
            // One-time migration: try loading from old JSON file
            let path = crate::client::data_dir().join("read_items.json");
            if let Ok(bytes) = std::fs::read(&path) {
                if let Ok(data) = serde_json::from_slice::<ReadData>(&bytes) {
                    // Migrate to DB and remove old file
                    if let Ok(json) = serde_json::to_string(&data) {
                        let _ = db.save_data_cache(CACHE_KEY, &json);
                    }
                    let _ = std::fs::remove_file(&path);
                    log::info!("Migrated read_items.json to database");
                    return data;
                }
            }
            ReadData::default()
        }
    }
}

fn persist(db: &Database, data: &ReadData) {
    if let Ok(json) = serde_json::to_string(data) {
        let _ = db.save_data_cache(CACHE_KEY, &json);
    }
}

pub fn mark_read(db: &Database, source: &str, id: &str) {
    if id.is_empty() || id.len() > 512 {
        return;
    }
    let mut data = load(db);
    let set = match source {
        "kgc" => &mut data.kgc,
        "luna" => &mut data.luna,
        "kwic" => &mut data.kwic,
        _ => return,
    };
    set.insert(id.to_string());
    cap(set);
    persist(db, &data);
}

pub fn mark_batch_read(db: &Database, source: &str, ids: Vec<String>) {
    let mut data = load(db);
    let set = match source {
        "kgc" => &mut data.kgc,
        "luna" => &mut data.luna,
        "kwic" => &mut data.kwic,
        _ => return,
    };
    for id in ids {
        if !id.is_empty() && id.len() <= 512 {
            set.insert(id);
        }
    }
    cap(set);
    persist(db, &data);
}

pub fn get_all_read_ids(db: &Database) -> ReadIdsResponse {
    let data = load(db);
    ReadIdsResponse {
        kgc: data.kgc.iter().cloned().collect(),
        luna: data.luna.iter().cloned().collect(),
        kwic: data.kwic.iter().cloned().collect(),
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReadIdsResponse {
    pub kgc: Vec<String>,
    pub luna: Vec<String>,
    pub kwic: Vec<String>,
}

// ── Seen notification IDs (push dedup) ──

const SEEN_CACHE_PREFIX: &str = "seen_notifs_";
const SEEN_LUNA_OBJECTS_KEY: &str = "seen_notifs_luna_objects";
const SEEN_INIT_PREFIX: &str = "seen_notifs_init_";
const SEEN_BOOTSTRAP_STARTED_AT_KEY: &str = "seen_notifs_bootstrap_started_at";
const SEEN_BOOTSTRAP_COMPLETE_KEY: &str = "seen_notifs_bootstrap_complete";
const MAX_SEEN_IDS: usize = 500;

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
        let _ = db.save_data_cache(&key, &json);
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
        let _ = db.save_data_cache(SEEN_LUNA_OBJECTS_KEY, &json);
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
    let key = format!("{}{}", SEEN_INIT_PREFIX, source);
    if let Ok(json) = serde_json::to_string(&true) {
        let _ = db.save_data_cache(&key, &json);
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
