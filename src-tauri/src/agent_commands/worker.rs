//! SQLite locks and JSON conversion never occupy the IPC or async executor.
use super::{AppHandle, Database, Manager};

pub(super) async fn with_database<R: Send + 'static>(
    app: AppHandle,
    work: impl FnOnce(&Database) -> Result<R, String> + Send + 'static,
) -> Result<R, String> {
    let db = app.state::<Database>().scope();
    run(move || work(&db)).await
}

async fn run<R: Send + 'static>(
    work: impl FnOnce() -> Result<R, String> + Send + 'static,
) -> Result<R, String> {
    crate::background_ipc::run("会話の処理に失敗しました", work).await
}

pub(super) fn json_response<T: serde::Serialize>(
    value: &T,
) -> Result<tauri::ipc::Response, String> {
    crate::background_ipc::json_response("会話の変換に失敗しました", value)
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
