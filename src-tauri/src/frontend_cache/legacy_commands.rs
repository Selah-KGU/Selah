// Test-only predecessor commands live outside the production macro scope.
use crate::db::{CacheTimestampBatch, Database};
use crate::frontend_cache::{load_frontend_cache_batch, CacheStampQuery, FrontendCacheBatch};
use crate::timetable::{build_schedule_snapshot, ScheduleResponse};
use tauri::test::MockRuntime;
use tauri::Manager;
#[tauri::command]
async fn get_backend_task_timestamps<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    keys: Vec<String>,
    include_schedule: bool,
) -> Result<CacheTimestampBatch, String> {
    tokio::task::spawn_blocking(move || {
        app.state::<Database>()
            .cache_timestamps(&keys, include_schedule)
    })
    .await
    .map_err(|err| format!("更新時刻の読み込みに失敗しました: {err}"))?
}
#[tauri::command]
async fn get_frontend_cache_batch<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    queries: Vec<CacheStampQuery>,
    include_schedule: bool,
    known_schedule_stamp: Option<String>,
) -> Result<FrontendCacheBatch, String> {
    tokio::task::spawn_blocking(move || {
        load_frontend_cache_batch(&app, queries, include_schedule, known_schedule_stamp)
    })
    .await
    .map_err(|err| format!("キャッシュの読み込みに失敗しました: {err}"))?
}
#[tauri::command]
async fn get_schedule_snapshot<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<ScheduleResponse, String> {
    tokio::task::spawn_blocking(move || build_schedule_snapshot(&app))
        .await
        .map_err(|err| format!("時間割スナップショットの読み込みに失敗しました: {err}"))?
}
pub(crate) fn handler() -> impl Fn(tauri::ipc::Invoke<MockRuntime>) -> bool + Send + Sync + 'static
{
    tauri::generate_handler![
        get_backend_task_timestamps,
        get_frontend_cache_batch,
        get_schedule_snapshot
    ]
}
