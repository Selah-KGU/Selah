//! Recording replies are serialized outside UI/async and LIVE state locks.
use super::{current_snapshot, LiveSaveResult, LiveState};
use serde::Serialize;
use tauri::ipc::Response;

const ENCODING_FAILURE: &str = "Live応答の変換失敗";

pub(in crate::live) async fn work<T: Serialize + Send + 'static>(
    failure: &'static str,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<Response, String> {
    crate::background_ipc::respond(failure, ENCODING_FAILURE, work).await
}

pub(in crate::live) async fn encode<T: Serialize + Send + 'static>(
    value: T,
) -> Result<Response, String> {
    work("Live応答処理失敗", move || Ok(value)).await
}

pub(in crate::live) async fn current(state: LiveState) -> Result<Response, String> {
    work("Live状態取得処理失敗", move || {
        Ok(current_snapshot(&state))
    })
    .await
}

pub(in crate::live) async fn saved(
    result: LiveSaveResult,
    reply: FinishReply,
    mut publish: impl FnMut(&'static str, String) + Send + 'static,
) -> Result<Response, String> {
    if matches!(reply, FinishReply::CompactSurface) {
        let was_saved = result.saved;
        return crate::background_ipc::respond_with_event(
            "Live応答処理失敗",
            ENCODING_FAILURE,
            move || Ok(super::surface::CompactSurfaceSaveResult::from(result)),
            move |json| {
                if was_saved {
                    publish("live-surface-compact-saved", json);
                }
            },
        )
        .await;
    }
    if matches!(reply, FinishReply::Surface) {
        let was_saved = result.saved;
        return crate::background_ipc::respond_with_event(
            "Live応答処理失敗",
            ENCODING_FAILURE,
            move || Ok(super::surface::LiveSurfaceSaveResult::from(result)),
            move |json| {
                if was_saved {
                    publish("live-surface-saved", json);
                }
            },
        )
        .await;
    }
    crate::background_ipc::run("Live応答処理失敗", move || {
        // Legacy full saves also publish the small page event so a
        // mounted/reloaded page observes saves initiated by another caller.
        let full = crate::background_ipc::json_text(ENCODING_FAILURE, &result)?;
        let was_saved = result.saved;
        let page = super::surface::LiveSurfaceSaveResult::from(result);
        let page = crate::background_ipc::json_text(ENCODING_FAILURE, &page)?;
        if was_saved {
            publish("live-surface-saved", page.clone());
            publish("live-session-saved", full.clone());
        }
        Ok(Response::new(full))
    })
    .await
}

#[derive(Clone, Copy)]
pub(in crate::live) enum FinishReply {
    Full,
    Surface,
    CompactSurface,
}

#[cfg(test)]
#[path = "response/tests.rs"]
mod tests;
