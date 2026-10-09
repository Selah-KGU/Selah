//! Listening, processing, and result transitions.

use super::*;

pub fn close_panel(app: &AppHandle, immediate: bool) {
    close_panel_owned(app, immediate, None, None);
}
pub(super) fn close_panel_for_capture(app: &AppHandle, immediate: bool, expected: Option<u64>) {
    close_panel_owned(app, immediate, expected, None);
}
pub(super) fn close_panel_for_view(app: &AppHandle, lease: &NativeViewLease) {
    close_panel_owned(app, false, None, Some(lease));
}
fn close_panel_owned(
    app: &AppHandle,
    immediate: bool,
    expected: Option<u64>,
    lease: Option<&NativeViewLease>,
) {
    let (closed, lease) = {
        let mut ag = AGENT.lock().unwrap_or_else(|e| e.into_inner());
        if expected.is_some_and(|revision| ag.capture.revision() != revision) {
            return;
        }
        let Some(closed) = ag.close_view(lease) else {
            return;
        };
        cancel_auto_close();
        stop_dots_animation();
        MORPH_TOKEN.fetch_add(1, Ordering::Relaxed);
        FADE_TOKEN.fetch_add(1, Ordering::Relaxed);
        ui::clear_pending_views();
        (closed, ag.view_lease())
    };
    if let Some(id) = closed.input_id {
        let _ = stt::stt_request_native_input_stop(&id);
    }
    enqueue_close(app, lease, immediate);
}

// ─ Shortcut handling ─────────────────────────────────────────────────────────
//
// Windows ショートカット操作モデル（長押し録音・離して送信）:
//   - キー押下 → 音声入力開始 (Listening)
//   - キー離す → 音声送信 → Agent へ (Processing → Result)
//
// キーリピートは held intent で抑止し、状態照会待ちでも追加起動しない。
// Called from WM_AGENT_SHORTCUT_PRESS on the overlay thread.
pub(super) fn handle_shortcut_press(app: AppHandle) {
    let request = {
        let mut state = AGENT.lock().unwrap_or_else(|e| e.into_inner());
        let request = state.press_shortcut();
        if matches!(
            state.mode,
            Some(CapsuleMode::Listening | CapsuleMode::Processing)
        ) {
            return;
        }
        if request.is_some() {
            cancel_auto_close();
        }
        request
    };
    if let Some(request) = request {
        tauri::async_runtime::spawn_blocking(move || start_agent_capture(app, request));
    }
}

// Called from WM_AGENT_SHORTCUT_RELEASE on the overlay thread.
// Hold-to-talk: releasing the key finalizes the recording and submits.
pub(super) fn handle_shortcut_release() {
    stop_listening();
}

// Stop listening and signal STT to finalize.
pub(super) fn stop_listening() {
    let input_id = {
        let mut ag = AGENT.lock().unwrap_or_else(|e| e.into_inner());
        ag.shortcut.release();
        let input_id = ag.capture.id().map(str::to_owned);
        if input_id.is_some() {
            ag.stop_requested = true;
        }
        input_id
    };
    if let Some(id) = input_id {
        let _ = stt::stt_request_native_input_stop(&id);
    }
}

fn start_agent_capture(app: AppHandle, request: crate::native_shortcut::ShortcutRequest) {
    let input_id = uuid_v4();
    let Some(update) = crate::native_shortcut::prepare_capture(
        &AGENT,
        request,
        input_id.clone(),
        MUTED_TEXT,
        stt::stt_get_stream_state,
    ) else {
        return;
    };
    enqueue_view(&app, update.view);
    if !update.start {
        return;
    }

    match stt::stt_start_native_input_now(app.clone(), input_id.clone()) {
        Ok(()) => {
            let stop = {
                let ag = AGENT.lock().unwrap_or_else(|e| e.into_inner());
                !ag.capture.owns(Some(&input_id)) || ag.stop_requested
            };
            if stop {
                let _ = stt::stt_request_native_input_stop(&input_id);
            }
        }
        Err(error) => capture_error(&app, &input_id, &error),
    }
}

// ─ Agent integration ─────────────────────────────────────────────────────────
pub(super) fn capture_error(app: &AppHandle, input_id: &str, message: &str) {
    crate::native_agent_submission::capture_failed(app, input_id, message, &AGENT, enqueue_view);
}

pub(super) fn finish_capture(app: AppHandle, input_id: &str) {
    let conv_id = uuid_v4();
    let (text, start) = {
        let mut ag = AGENT.lock().unwrap_or_else(|e| e.into_inner());
        if !ag.capture.finish(input_id) {
            return;
        }
        ag.stop_requested = false;
        let text = consume_all_speech(&mut ag);
        let revision = ag.capture.revision();
        if text.is_empty() {
            drop(ag);
            close_panel_for_capture(&app, false, Some(revision));
            return;
        }
        // Reserve before UI/DB work so a newer capture cannot lose its speech.
        let start = ag.begin_stream(conv_id);
        (text, start)
    };
    crate::native_agent_submission::submit(app.clone(), start.owner, text, &AGENT, enqueue_view);
    enqueue_view(&app, start.view);
}
