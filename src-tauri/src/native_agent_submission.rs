//! Submission outlives the floating view, but never occupies STT teardown.
use crate::native_agent_events::StreamOwner;
use crate::native_agent_state::{NativeViewUpdate, SharedState, StreamCompletion};
use std::future::Future;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};
#[path = "native_agent_submission/storage.rs"]
mod storage;

pub(crate) fn submit(
    app: AppHandle,
    owner: Arc<StreamOwner>,
    text: String,
    state: &'static Mutex<SharedState>,
    display: fn(&AppHandle, NativeViewUpdate),
) {
    let result_app = app.clone();
    let result_owner = owner.clone();
    let submission = crate::agent::submit_voice_turn(
        app.clone(),
        owner.conversation_id().to_owned(),
        owner.request_id().to_owned(),
        voice_save_job(app, owner.conversation_id().to_owned(), text),
    );
    tauri::async_runtime::spawn(async move {
        let result = submission.await;
        if let Err(error) = &result {
            log::warn!(
                "[native agent] submission failed conv_id={}: {}",
                result_owner.conversation_id(),
                error
            );
        }
        if let Some(completion) = submission_finished(state, &result_owner, &result) {
            display(&result_app, completion.view);
        }
    });
}

// Normal terminal events already consumed their native owner synchronously.
// Superseded requests suppress their public terminal event; finish
// the still-owned capsule explicitly so it cannot wait for another request.
fn submission_finished(
    state: &Mutex<SharedState>,
    owner: &StreamOwner,
    result: &Result<(), String>,
) -> Option<StreamCompletion> {
    let message = result
        .as_ref()
        .err()
        .map(String::as_str)
        .unwrap_or("応答が取り消されました");
    state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .complete_stream(owner.conversation_id(), owner.request_id(), Some(message))
}

/// An interrupted capture saves recognized input without starting model/tools.
/// Both platform callbacks share this path and the normal submission's storage.
pub(crate) fn capture_failed(
    app: &AppHandle,
    input_id: &str,
    message: &str,
    state: &'static Mutex<SharedState>,
    display: fn(&AppHandle, NativeViewUpdate),
) {
    let Some(failure) = state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .fail_capture(input_id, message)
    else {
        return;
    };
    let lease = failure.view.lease.clone();
    if !failure.text.is_empty() {
        // Register persistence before touching the floating UI.
        let recovery = prepare_and_run(
            voice_save_job(app.clone(), uuid::Uuid::new_v4().to_string(), failure.text),
            |_| async { Ok(()) },
        );
        let result_app = app.clone();
        tauri::async_runtime::spawn(async move {
            let result = recovery.await;
            if let Err(error) = &result {
                log::warn!("interrupted native speech storage failed: {error}");
            }
            let view = state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .complete_capture_recovery(
                    &lease,
                    &failure.message,
                    result.as_ref().err().map(String::as_str),
                );
            if let Some(view) = view {
                display(&result_app, view);
            }
        });
    }
    display(app, failure.view);
}

fn voice_save_job(
    app: AppHandle,
    conversation_id: String,
    text: String,
) -> impl FnOnce() -> Result<crate::agent::SavedVoiceInput, String> + Send + 'static {
    let permit = crate::pending_persistence::INPUT_SAVES.reserve();
    let attempt = storage::STORAGE.accept(conversation_id, text);
    move || save_and_notify(&app, permit?, attempt)
}

fn save_and_notify(
    app: &AppHandle,
    permit: crate::pending_persistence::SavePermit,
    attempt: storage::VoiceAttempt,
) -> Result<crate::agent::SavedVoiceInput, String> {
    let input = save_attempt(app, attempt);
    permit.complete(input.as_ref().map(|_| ()).map_err(String::as_str));
    let input = input?;
    let _ = app.emit("agent-conversations-changed", input.conversation_id());
    Ok(input)
}

fn save_attempt(
    app: &AppHandle,
    attempt: storage::VoiceAttempt,
) -> Result<crate::agent::SavedVoiceInput, String> {
    attempt.persist_with(|input, retry| {
        let db = app.state::<crate::db::Database>();
        if retry {
            crate::agent::retry_voice_input(&db, input.conversation_id.clone(), input.text.clone())
        } else {
            crate::agent::save_voice_input(&db, input.conversation_id.clone(), input.text.clone())
        }
    })
}

/// Retry only failed attempts. In-flight jobs retain their own save permits.
/// Recovery persists speech; it does not restart inference during application quit.
pub(crate) fn retry_failed(app: &AppHandle) {
    for attempt in storage::STORAGE.retry_failed() {
        let app = app.clone();
        let permit = crate::pending_persistence::INPUT_SAVES.reserve();
        tauri::async_runtime::spawn_blocking(move || {
            let permit = match permit {
                Ok(permit) => permit,
                Err(error) => {
                    log::warn!("voice retry rejected: {error}");
                    return;
                }
            };
            if let Err(error) = save_and_notify(&app, permit, attempt) {
                log::warn!("voice retry failed: {error}");
            }
        });
    }
}

fn prepare_and_run<T: Send + 'static, S: Future<Output = Result<(), String>>>(
    save: impl FnOnce() -> Result<T, String> + Send + 'static,
    run: impl FnOnce(T) -> S,
) -> impl Future<Output = Result<(), String>> {
    // Start storage immediately, so aborting a queued async continuation cannot
    // drop accepted speech before its blocking job has even been registered.
    let saved = tauri::async_runtime::spawn_blocking(save);
    async move {
        let input = saved
            .await
            .map_err(|error| format!("音声会話の準備に失敗しました: {error}"))??;
        run(input).await
    }
}
#[cfg(test)]
#[path = "native_agent_submission/tests.rs"]
mod tests;
