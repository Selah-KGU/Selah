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
const MAX_SEEN_IDS: usize = 500;

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
