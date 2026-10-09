//! Keep manual subtitle acceptance synchronous; serialize its reply afterward.
use super::{current_snapshot, response, LiveState};
use serde::Deserialize;
use std::future::Future;
use tauri::{
    ipc::{Invoke, InvokeBody, Response},
    Manager,
};

#[derive(Deserialize)]
struct AppendFields<'a> {
    text: &'a str,
}
fn append_text(body: &InvokeBody) -> Result<&str, String> {
    let InvokeBody::Json(value) = body else {
        return Err("字幕は JSON で送信してください".into());
    };
    AppendFields::deserialize(value)
        .map(|fields| fields.text)
        .map_err(|error| format!("字幕の形式が正しくありません: {error}"))
}

fn append_response(
    state: &LiveState,
    append: impl FnOnce() -> Result<bool, String>,
) -> impl Future<Output = Result<Response, String>> + Send + 'static {
    // Neither a dropped reply nor a delayed first poll can revoke accepted text.
    // The response retains this exact snapshot if another recording starts.
    let snapshot = append().map(|_| current_snapshot(state));
    async move { response::encode(snapshot?).await }
}

pub(crate) fn admission_handler(
    other: impl Fn(Invoke) -> bool + Send + Sync + 'static,
) -> impl Fn(Invoke) -> bool + Send + Sync + 'static {
    move |invoke| {
        if invoke.message.command() != "live_append_transcript" {
            return other(invoke);
        }
        let text = match append_text(invoke.message.payload()) {
            Ok(text) => text,
            Err(error) => {
                invoke.resolver.reject(error);
                return true;
            }
        };
        let app = invoke.message.webview_ref().app_handle();
        let state = app.state::<LiveState>();
        let completion = append_response(state.inner(), || {
            super::commands::append_transcript(app, state.inner(), text, None, None)
        });
        invoke
            .resolver
            .respond_async(async move { completion.await.map_err(Into::into) });
        true
    }
}

#[cfg(test)]
#[path = "admission/tests.rs"]
mod tests;
