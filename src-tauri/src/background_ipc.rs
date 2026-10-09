//! Build large IPC JSON bodies on blocking workers and return raw JSON replies.
use serde::Serialize;
use tauri::ipc::Response;

pub(crate) async fn run<R: Send + 'static>(
    failure: &'static str,
    work: impl FnOnce() -> Result<R, String> + Send + 'static,
) -> Result<R, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| format!("{failure}: {error}"))?
}

pub(crate) fn json_text<T: Serialize + ?Sized>(failure: &str, value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| format!("{failure}: {error}"))
}

pub(crate) fn json_response<T: Serialize + ?Sized>(
    failure: &str,
    value: &T,
) -> Result<Response, String> {
    json_text(failure, value).map(Response::new)
}

pub(crate) async fn respond<T: Serialize + Send + 'static>(
    work_failure: &'static str,
    encoding_failure: &'static str,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<Response, String> {
    run(work_failure, move || {
        let value = work()?;
        json_response(encoding_failure, &value)
    })
    .await
}

/// Event and RPC consumers see identical objects. Serialization runs once;
/// each transport receives its own string buffer without encoding JSON again.
pub(crate) async fn respond_with_event<T: Serialize + Send + 'static>(
    work_failure: &'static str,
    encoding_failure: &'static str,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
    publish: impl FnOnce(String) + Send + 'static,
) -> Result<Response, String> {
    run(work_failure, move || {
        let value = work()?;
        let json = json_text(encoding_failure, &value)?;
        publish(json.clone());
        Ok(Response::new(json))
    })
    .await
}

#[cfg(test)]
#[path = "background_ipc/tests.rs"]
mod tests;
