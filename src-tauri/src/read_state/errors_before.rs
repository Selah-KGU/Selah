//! Frozen pre-fix DB-backed read-state path, only for regression comparisons.
//! All callers seed their cache. File migration is tested separately with
//! temporary paths; this control must never inspect the user's legacy file.
use super::ReadIdsResponse;
use crate::db::Database;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Default, Deserialize, Serialize)]
struct Data {
    kgc: HashSet<String>,
    luna: HashSet<String>,
    kwic: HashSet<String>,
}
fn load(db: &Database) -> Data {
    match db.get_data_cache("read_state") {
        Ok(Some((json, _))) => serde_json::from_str(&json).unwrap_or_default(),
        _ => Data::default(),
    }
}
fn cap(set: &mut HashSet<String>) {
    if set.len() > 500 {
        let to_remove: Vec<String> = set.iter().take(set.len() - 500).cloned().collect();
        for key in to_remove {
            set.remove(&key);
        }
    }
}
fn persist(db: &Database, data: &Data) {
    if let Ok(json) = serde_json::to_string(data) {
        let _ = db.save_data_cache("read_state", &json);
    }
}
pub(crate) fn mark_read(db: &Database, source: &str, id: &str) {
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
    let inserted = !set.contains(id) && set.insert(id.to_string());
    let needs_cap = set.len() > 500;
    if !inserted && !needs_cap {
        return;
    }
    cap(set);
    persist(db, &data);
}
pub(crate) fn mark_batch_read(db: &Database, source: &str, ids: Vec<String>) {
    let mut data = load(db);
    let set = match source {
        "kgc" => &mut data.kgc,
        "luna" => &mut data.luna,
        "kwic" => &mut data.kwic,
        _ => return,
    };
    let mut changed = false;
    for id in ids {
        if !id.is_empty() && id.len() <= 512 && !set.contains(&id) && set.insert(id) {
            changed = true;
        }
    }
    if !changed && set.len() <= 500 {
        return;
    }
    cap(set);
    persist(db, &data);
}
pub(crate) fn get_all_read_ids(db: &Database) -> ReadIdsResponse {
    let data = load(db);
    ReadIdsResponse {
        kgc: data.kgc.iter().cloned().collect(),
        luna: data.luna.iter().cloned().collect(),
        kwic: data.kwic.iter().cloned().collect(),
    }
}
