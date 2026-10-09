//! One IPC round-trip for the main window's visibility catch-up.
//! Unchanged rows omit their JSON so a refocus does not re-parse large caches
//! or rebuild the timetable snapshot.

use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::db::{CacheDeltaRow, Database};
use crate::timetable::{build_schedule_snapshot, ScheduleResponse};

const LIVE_TODO_CACHE_KEY: &str = "live_generated_todo";

#[tauri::command]
pub async fn get_backend_task_timestamps<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    keys: Vec<String>,
    include_schedule: bool,
) -> Result<tauri::ipc::Response, String> {
    reply(
        "更新時刻の読み込みに失敗しました",
        move || {
            app.state::<Database>()
                .cache_timestamps(&keys, include_schedule)
        },
    )
    .await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStampQuery {
    pub key: String,
    /// Content revision, independent of the last successful fetch timestamp.
    /// Legacy knownUpdatedAt is ignored: whole seconds cannot prove equality.
    #[serde(default)]
    pub known_revision: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct FrontendCacheBatch {
    pub rows: Vec<CacheDeltaRow>,
    pub schedule_updated_at: i64,
    pub live_todo_updated_at: i64,
    pub schedule_revision: i64,
    pub live_todo_revision: i64,
    pub schedule_unchanged: bool,
    pub schedule_stamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<ScheduleResponse>,
}

#[tauri::command]
pub async fn get_frontend_cache_batch<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    queries: Vec<CacheStampQuery>,
    include_schedule: bool,
    known_schedule_stamp: Option<String>,
) -> Result<tauri::ipc::Response, String> {
    // Read, rebuild, encode and dispose large rows on the same worker.
    reply(
        "キャッシュの読み込みに失敗しました",
        move || load_frontend_cache_batch(&app, queries, include_schedule, known_schedule_stamp),
    )
    .await
}

async fn reply<T: Serialize + Send + 'static>(
    failure: &'static str,
    read: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<tauri::ipc::Response, String> {
    crate::background_ipc::respond(failure, "キャッシュ応答の変換に失敗しました", read).await
}

pub(crate) fn load_frontend_cache_batch<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    queries: Vec<CacheStampQuery>,
    include_schedule: bool,
    known_schedule_stamp: Option<String>,
) -> Result<FrontendCacheBatch, String> {
    let db = app.state::<Database>();
    let mut requests: Vec<_> = queries
        .into_iter()
        .map(|query| (query.key, query.known_revision))
        .collect();
    if include_schedule && !requests.iter().any(|(key, _)| key == LIVE_TODO_CACHE_KEY) {
        requests.push((LIVE_TODO_CACHE_KEY.to_string(), None));
    }
    let (schedule_updated_at, schedule_revision) = if include_schedule {
        db.schedule_snapshot_version()?
    } else {
        (0, 0)
    };
    let rows = db.get_data_cache_deltas(&requests)?;
    let live_row = rows.iter().find(|row| row.key == LIVE_TODO_CACHE_KEY);
    let live_todo_updated_at = live_row.map(|row| row.updated_at).unwrap_or(0);
    let live_todo_revision = live_row.map(|row| row.revision).unwrap_or(0);
    let schedule_stamp = if include_schedule {
        let calendar = crate::commands::load_calendar_config();
        schedule_content_stamp(
            schedule_revision,
            live_todo_revision,
            &chrono::Local::now().date_naive().to_string(),
            &calendar.spring_start,
            &calendar.fall_start,
            db.schedule_ai_cache_updated_at()?,
            crate::db::epoch_secs(),
        )
    } else {
        String::new()
    };
    let schedule_unchanged =
        include_schedule && known_schedule_stamp.as_deref() == Some(schedule_stamp.as_str());

    let schedule = if include_schedule && !schedule_unchanged {
        Some(build_schedule_snapshot(app)?)
    } else {
        None
    };

    Ok(FrontendCacheBatch {
        rows,
        schedule_updated_at,
        live_todo_updated_at,
        schedule_revision,
        live_todo_revision,
        schedule_unchanged,
        schedule_stamp,
        schedule,
    })
}

fn schedule_content_stamp(
    revision: i64,
    live_revision: i64,
    day: &str,
    spring: &str,
    fall: &str,
    ai_updated_at: i64,
    now: i64,
) -> String {
    let ai_expired = now - ai_updated_at > crate::timetable::AI_CACHE_MAX_AGE;
    // Date, semester configuration and expiration affect the derived snapshot
    // without any SQLite write. Encode an opaque stamp owned by the backend.
    serde_json::to_string(&(3, revision, live_revision, day, spring, fall, ai_expired)).unwrap()
}

#[cfg(test)]
mod tests {
    use super::{schedule_content_stamp, CacheStampQuery};

    #[test]
    fn schedule_stamp_changes_for_date_calendar_and_ai_expiration() {
        let stamp = |day: &str, spring: &str, now| {
            schedule_content_stamp(1, 2, day, spring, "2026-09-01", 100, now)
        };
        let first = stamp("2026-10-07", "2026-04-01", 100);
        assert_eq!(first, stamp("2026-10-07", "2026-04-01", 101));
        assert_ne!(first, stamp("2026-10-08", "2026-04-01", 101));
        assert_ne!(first, stamp("2026-10-07", "2026-04-02", 101));
        assert_eq!(
            first,
            stamp(
                "2026-10-07",
                "2026-04-01",
                100 + crate::timetable::AI_CACHE_MAX_AGE
            )
        );
        assert_ne!(
            first,
            stamp(
                "2026-10-07",
                "2026-04-01",
                101 + crate::timetable::AI_CACHE_MAX_AGE
            )
        );
    }

    #[test]
    fn cache_stamp_query_accepts_frontend_camel_case() {
        let query: CacheStampQuery =
            serde_json::from_str(r#"{"key":"luna_todo","knownRevision":42}"#).unwrap();
        assert_eq!(query.key, "luna_todo");
        assert_eq!(query.known_revision, Some(42));

        let missing: CacheStampQuery = serde_json::from_str(r#"{"key":"mail_inbox"}"#).unwrap();
        assert_eq!(missing.known_revision, None);
        let legacy: CacheStampQuery =
            serde_json::from_str(r#"{"key":"mail_inbox","knownUpdatedAt":1710000000}"#).unwrap();
        assert_eq!(legacy.known_revision, None);
    }
}

#[cfg(test)]
#[path = "frontend_cache/ipc_tests.rs"]
mod ipc_tests;
