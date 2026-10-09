//! Commit native speech and answers before publishing optional JSON UI events.
use crate::native_agent_state::{NativeViewUpdate, SharedState, StreamCompletion};
use std::borrow::Cow;
use std::sync::{Mutex, OnceLock};
use tauri::AppHandle;
#[path = "native_agent_events/stream.rs"]
mod stream;
pub(crate) use stream::{StreamEvent, StreamOwner};

struct NativeSink {
    state: &'static Mutex<SharedState>,
    display: fn(&AppHandle, NativeViewUpdate),
    finish: fn(AppHandle, &str),
    failed: fn(&AppHandle, &str, &str),
}
static SINK: OnceLock<NativeSink> = OnceLock::new();

pub(crate) fn install(
    state: &'static Mutex<SharedState>,
    display: fn(&AppHandle, NativeViewUpdate),
    finish: fn(AppHandle, &str),
    failed: fn(&AppHandle, &str, &str),
) {
    if SINK
        .set(NativeSink {
            state,
            display,
            finish,
            failed,
        })
        .is_err()
    {
        log::warn!("native speech sink was already installed");
    }
}

pub(crate) enum NativeSttEvent<'a> {
    Speech { text: Cow<'a, str>, is_final: bool },
    State(&'a str),
    Error(&'a str),
}

fn native_owner<'a>(caller: &str, input_id: Option<&'a str>) -> Option<&'a str> {
    input_id.filter(|id| caller == "native_agent" && !id.is_empty())
}

fn speech_view(
    state: &Mutex<SharedState>,
    input_id: &str,
    text: Cow<'_, str>,
    is_final: bool,
) -> Option<NativeViewUpdate> {
    state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .accept_speech(input_id, text, is_final)
}

fn deleted_stream(state: &Mutex<SharedState>, conversation_id: &str) -> Option<StreamCompletion> {
    state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .complete_conversation(conversation_id, Some("会話が削除されました"))
}

/// Native capsules do not depend on a WebView's deletion listener. A deleted
/// processing conversation gets its own terminal notice; a different stream
/// or an active speech capture is left alone.
pub(crate) fn conversation_deleted(app: &AppHandle, conversation_id: &str) {
    let Some(sink) = SINK.get() else {
        return;
    };
    let Some(completion) = deleted_stream(sink.state, conversation_id) else {
        return;
    };
    (sink.display)(app, completion.view);
}

fn stream_view(
    state: &Mutex<SharedState>,
    conversation_id: &str,
    request_id: &str,
    event: StreamEvent<'_>,
) -> Option<StreamCompletion> {
    state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .accept_stream(conversation_id, request_id, event)
}

/// Update native state before optional JSON event delivery. Typed text is
/// borrowed from the backend; only the owned answer buffer retains it.
pub(crate) fn publish_stream(
    app: &AppHandle,
    conversation_id: &str,
    request_id: &str,
    event: StreamEvent<'_>,
) {
    let Some(sink) = SINK.get() else {
        return;
    };
    if let Some(completion) = stream_view(sink.state, conversation_id, request_id, event) {
        (sink.display)(app, completion.view);
    }
}

/// STT calls this before emitting JSON. Tauri may defer listeners while another
/// callback holds its event lock; accepted input cannot depend on their delivery.
pub(crate) fn publish(
    app: &AppHandle,
    caller: &str,
    input_id: Option<&str>,
    event: NativeSttEvent<'_>,
) {
    let Some(input_id) = native_owner(caller, input_id) else {
        return;
    };
    let Some(sink) = SINK.get() else {
        return;
    };
    match event {
        NativeSttEvent::Speech { text, is_final } => {
            let view = speech_view(sink.state, input_id, text, is_final);
            if let Some(view) = view {
                (sink.display)(app, view);
            }
        }
        NativeSttEvent::State("initializing" | "listening") => {
            let view = sink
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .speech_ready_view(input_id);
            if let Some(view) = view {
                (sink.display)(app, view);
            }
        }
        NativeSttEvent::State("idle") => (sink.finish)(app.clone(), input_id),
        NativeSttEvent::State(_) => {}
        NativeSttEvent::Error(message) => (sink.failed)(app, input_id, message),
    }
}

#[cfg(test)]
#[path = "native_agent_events/tests.rs"]
mod tests;
