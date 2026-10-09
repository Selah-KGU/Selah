//! IPC status queries wait and encode on workers. Native shortcut callers keep
//! the existing synchronous domain readers and their existing lock ordering.
use serde::Serialize;

async fn reply<T: Serialize + Send + 'static>(
    read: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<tauri::ipc::Response, String> {
    crate::background_ipc::respond("STT状態取得処理失敗", "STT応答の変換失敗", read).await
}

#[tauri::command]
pub(crate) async fn stt_is_running() -> Result<tauri::ipc::Response, String> {
    reply(|| Ok(super::stt_is_running())).await
}

#[tauri::command]
pub(crate) async fn stt_get_active_caller() -> Result<tauri::ipc::Response, String> {
    reply(|| Ok(super::stt_get_active_caller())).await
}

#[tauri::command]
pub(crate) async fn stt_get_stream_state() -> Result<tauri::ipc::Response, String> {
    reply(super::stt_get_stream_state).await
}

#[cfg(test)]
#[path = "ipc_status/tests.rs"]
mod tests;
