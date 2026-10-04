//! One IPC round-trip for the main window's visibility catch-up.
//! Unchanged rows omit their JSON so a refocus does not re-parse large caches
//! or rebuild the timetable snapshot.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::db::Database;
use crate::timetable::{build_schedule_snapshot, ScheduleResponse};

const LIVE_TODO_CACHE_KEY: &str = "live_generated_todo";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStampQuery {
    pub key: String,
    /// Absent means the caller has no copy, so the row must include JSON.
    #[serde(default)]
    pub known_updated_at: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct CacheBatchRow {
    pub key: String,
    pub updated_at: i64,
    pub unchanged: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FrontendCacheBatch {
    pub rows: Vec<CacheBatchRow>,
    pub schedule_updated_at: i64,
    pub live_todo_updated_at: i64,
    pub schedule_unchanged: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<ScheduleResponse>,
}

#[tauri::command]
pub async fn get_frontend_cache_batch(
    app: tauri::AppHandle,
    queries: Vec<CacheStampQuery>,
    include_schedule: bool,
    known_schedule_stamp: Option<String>,
) -> Result<FrontendCacheBatch, String> {
    let db = app.state::<Database>();
    let mut keys: Vec<String> = queries.iter().map(|query| query.key.clone()).collect();
    if include_schedule && !keys.iter().any(|key| key == LIVE_TODO_CACHE_KEY) {
        keys.push(LIVE_TODO_CACHE_KEY.to_string());
    }
    let loaded = db.get_data_cache_many(&keys)?;
    let live_todo_updated_at = loaded
        .get(LIVE_TODO_CACHE_KEY)
        .map(|(_, updated_at)| *updated_at)
        .unwrap_or(0);
    let schedule_updated_at = if include_schedule {
        db.schedule_snapshot_updated_at()?
    } else {
        0
    };
    let schedule_stamp = format!("{schedule_updated_at}:{live_todo_updated_at}");
    let schedule_unchanged =
        include_schedule && known_schedule_stamp.as_deref() == Some(schedule_stamp.as_str());

    let mut rows = Vec::with_capacity(queries.len() + 1);
    for query in &queries {
        rows.push(row_for_key(&query.key, query.known_updated_at, &loaded));
    }
    if include_schedule && !rows.iter().any(|row| row.key == LIVE_TODO_CACHE_KEY) {
        rows.push(row_for_key(LIVE_TODO_CACHE_KEY, None, &loaded));
    }
    if include_schedule && !schedule_unchanged {
        if let Some(row) = rows.iter_mut().find(|row| row.key == LIVE_TODO_CACHE_KEY) {
            if row.json.is_none() {
                if let Some((json, _)) = loaded.get(LIVE_TODO_CACHE_KEY) {
                    row.json = Some(json.clone());
                }
            }
        }
    }

    let schedule = if include_schedule && !schedule_unchanged {
        let app = app.clone();
        Some(
            tokio::task::spawn_blocking(move || build_schedule_snapshot(&app))
                .await
                .map_err(|err| {
                    format!("時間割スナップショットの読み込みに失敗しました: {err}")
                })??,
        )
    } else {
        None
    };

    Ok(FrontendCacheBatch {
        rows,
        schedule_updated_at,
        live_todo_updated_at,
        schedule_unchanged,
        schedule,
    })
}

fn row_for_key(
    key: &str,
    known_updated_at: Option<i64>,
    loaded: &HashMap<String, (String, i64)>,
) -> CacheBatchRow {
    match loaded.get(key) {
        Some((json, updated_at)) => {
            let unchanged = known_updated_at == Some(*updated_at);
            CacheBatchRow {
                key: key.to_string(),
                updated_at: *updated_at,
                unchanged,
                json: if unchanged { None } else { Some(json.clone()) },
            }
        }
        None => CacheBatchRow {
            key: key.to_string(),
            updated_at: 0,
            unchanged: known_updated_at == Some(0),
            json: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::CacheStampQuery;

    #[test]
    fn cache_stamp_query_accepts_frontend_camel_case() {
        let query: CacheStampQuery =
            serde_json::from_str(r#"{"key":"luna_todo","knownUpdatedAt":1710000000}"#).unwrap();
        assert_eq!(query.key, "luna_todo");
        assert_eq!(query.known_updated_at, Some(1_710_000_000));

        let missing: CacheStampQuery = serde_json::from_str(r#"{"key":"mail_inbox"}"#).unwrap();
        assert_eq!(missing.known_updated_at, None);
    }
}
