use super::{format_datetime, response, LiveState};
use serde::Serialize;

/// Metadata only: no transcript, course, summaries or whiteboard owners.
#[derive(Debug, Default, Serialize)]
pub(super) struct LiveTrayStatus {
    pub(super) active: bool,
    pub(super) listening: bool,
    pub(super) started_at: Option<String>,
}

pub(super) async fn reply(
    state: LiveState,
    read_microphone: impl FnOnce() -> Result<crate::stt::SttStreamState, String> + Send + 'static,
) -> Result<tauri::ipc::Response, String> {
    response::work("Liveトレイ状態取得処理失敗", move || {
        snapshot(&state, read_microphone)
    })
    .await
}

pub(super) fn snapshot(
    state: &LiveState,
    read_microphone: impl FnOnce() -> Result<crate::stt::SttStreamState, String>,
) -> Result<LiveTrayStatus, String> {
    let session = state
        .session
        .lock()
        .map_err(|_| "Live state lock failed".to_string())?;
    let Some(session) = session.as_ref() else {
        return Ok(LiveTrayStatus::default());
    };
    // Preserve LIVE -> STT admission order and the coherent recording ID. This
    // wait now belongs to a blocking worker, not the IPC or async executor.
    let microphone = read_microphone()?;
    Ok(LiveTrayStatus {
        active: true,
        listening: microphone.phase == crate::stt::SttStreamPhase::Listening
            && microphone.owner.as_ref().is_some_and(|owner| {
                owner.caller == "live"
                    && owner.live_session_id.as_deref() == Some(&session.session_id)
            }),
        started_at: Some(format_datetime(session.started_at)),
    })
}

#[cfg(test)]
mod tests;
