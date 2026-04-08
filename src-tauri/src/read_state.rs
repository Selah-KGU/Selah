use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::client::data_dir;

const FILE_NAME: &str = "read_items.json";
const MAX_IDS_PER_SOURCE: usize = 500;

#[derive(Debug, Serialize, Deserialize, Default)]
struct ReadData {
    kgc: HashSet<String>,
    luna: HashSet<String>,
    kwic: HashSet<String>,
}

pub struct ReadState {
    data: Mutex<ReadData>,
}

impl ReadState {
    pub fn new() -> Self {
        let mut data = Self::load().unwrap_or_default();
        Self::trim(&mut data);
        Self {
            data: Mutex::new(data),
        }
    }

    fn file_path() -> PathBuf {
        data_dir().join(FILE_NAME)
    }

    fn load() -> Option<ReadData> {
        let bytes = std::fs::read(Self::file_path()).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn persist(data: &ReadData) {
        if let Ok(json) = serde_json::to_string(data) {
            let tmp = Self::file_path().with_extension("json.tmp");
            if std::fs::write(&tmp, &json).is_ok() {
                let _ = std::fs::rename(&tmp, Self::file_path());
            }
        }
    }

    /// Keep each source set within MAX_IDS_PER_SOURCE by discarding arbitrary old entries.
    fn trim(data: &mut ReadData) {
        fn cap(set: &mut HashSet<String>) {
            if set.len() > MAX_IDS_PER_SOURCE {
                let excess = set.len() - MAX_IDS_PER_SOURCE;
                let to_remove: Vec<String> = set.iter().take(excess).cloned().collect();
                for k in to_remove {
                    set.remove(&k);
                }
            }
        }
        cap(&mut data.kgc);
        cap(&mut data.luna);
        cap(&mut data.kwic);
    }

    pub fn mark_read(&self, source: &str, id: &str) {
        if id.is_empty() || id.len() > 512 {
            return;
        }
        let mut data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        let set = match source {
            "kgc" => &mut data.kgc,
            "luna" => &mut data.luna,
            "kwic" => &mut data.kwic,
            _ => return,
        };
        if set.insert(id.to_string()) {
            Self::persist(&data);
        }
    }

    pub fn get_all_read_ids(&self) -> ReadIdsResponse {
        let data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        ReadIdsResponse {
            kgc: data.kgc.iter().cloned().collect(),
            luna: data.luna.iter().cloned().collect(),
            kwic: data.kwic.iter().cloned().collect(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReadIdsResponse {
    pub kgc: Vec<String>,
    pub luna: Vec<String>,
    pub kwic: Vec<String>,
}
