//! Listening, processing, and result transitions.

use super::*;

// ─ Mode transitions ───────────────────────────────────────────────────────────

// Ensure the overlay window exists and is faded in.
// Waits up to ~150 ms for the Win32 thread to initialize on first use.
pub(super) fn ensure_panel(app: &AppHandle) {
    ensure_overlay_window(app);
    for _ in 0..30 {
        if HWND_READY.load(Ordering::Acquire) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    fade_in();
    let hwnd = WINDOW.lock().unwrap_or_else(|e| e.into_inner()).hwnd;
    if hwnd != 0 {
        unsafe {
            ShowWindow(hwnd_from_raw(hwnd), SW_SHOWNOACTIVATE);
        }
    }
}

pub(super) fn listening_display_text(ag: &AgentState) -> String {
    let mut out = ag.finals_accumulated.clone();
    let partial = ag.current_speech.trim();
    if !partial.is_empty() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(partial);
    }
    out
}

pub(super) fn consume_all_speech(ag: &mut AgentState) -> String {
    let partial = ag.current_speech.trim().to_string();
    let mut out = std::mem::take(&mut ag.finals_accumulated);
    ag.current_speech.clear();
    if !partial.is_empty() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&partial);
    }
    out.trim().to_string()
}

pub(super) fn transition_to_listening(app: &AppHandle, initial_text: Option<&str>) {
    cancel_auto_close();
    stop_dots_animation();
    CURRENT_MODE.store(MODE_LISTENING, Ordering::Relaxed);
    ensure_panel(app);
    morph_to(LISTEN_W, LISTEN_H);
    let display = if let Some(t) = initial_text {
        t.to_string()
    } else {
        let ag = AGENT.lock().unwrap_or_else(|e| e.into_inner());
        let s = listening_display_text(&ag);
        if s.trim().is_empty() {
            MUTED_TEXT.to_string()
        } else {
            s
        }
    };
    update_text_content(display);
}

pub(super) fn transition_to_processing(app: &AppHandle) {
    cancel_auto_close();
    CURRENT_MODE.store(MODE_PROCESSING, Ordering::Relaxed);
    {
        let mut ag = AGENT.lock().unwrap_or_else(|e| e.into_inner());
        ag.finals_accumulated.clear();
        ag.current_speech.clear();
    }
    ensure_panel(app);
    morph_to(PROCESS_W, PROCESS_H);
    update_text_content(String::new());
    start_dots_animation();
}

pub(super) fn transition_to_result(app: &AppHandle, text: &str) {
    cancel_auto_close();
    stop_dots_animation();
    CURRENT_MODE.store(MODE_RESULT, Ordering::Relaxed);
    let target_h = estimate_result_height(text);
    ensure_panel(app);
    morph_to(RESULT_W, target_h);
    update_text_content(text.to_string());
    schedule_auto_close(Duration::from_secs(RESULT_AUTO_CLOSE_SECS));
}

pub(super) fn transition_to_notice(app: &AppHandle, message: &str) {
    cancel_auto_close();
    stop_dots_animation();
    CURRENT_MODE.store(MODE_NOTICE, Ordering::Relaxed);
    ensure_panel(app);
    morph_to(NOTICE_W, NOTICE_H);
    update_text_content(message.to_string());
    schedule_auto_close(Duration::from_millis(NOTICE_AUTO_CLOSE_MS));
}

pub fn close_panel(app: &AppHandle, immediate: bool) {
    cancel_auto_close();
    stop_dots_animation();
    CURRENT_MODE.store(MODE_NONE, Ordering::Relaxed);
    clear_agent_listener(app);
    {
        let mut ag = AGENT.lock().unwrap_or_else(|e| e.into_inner());
        ag.stop_requested = false;
        ag.finals_accumulated.clear();
        ag.current_speech.clear();
        ag.result_accumulated.clear();
    }
    if immediate {
        hide_panel();
    } else {
        fade_out_then_hide();
    }
}

// ─ Shortcut handling ─────────────────────────────────────────────────────────
//
// Windows ショートカット操作モデル（長押し録音・離して送信）:
//   - キー押下 → 音声入力開始 (Listening)
//   - キー離す → 音声送信 → Agent へ (Processing → Result)
//
// キーリピートは MODE_LISTENING 中の press を無視することで自然に処理される。
// Called from WM_AGENT_SHORTCUT_PRESS on the overlay thread.
pub(super) fn handle_shortcut_press(app: AppHandle) {
    log::info!(
        "[agent] shortcut press, mode={}",
        CURRENT_MODE.load(Ordering::Relaxed)
    );
    match CURRENT_MODE.load(Ordering::Relaxed) {
        MODE_LISTENING => {
            // Key is held down (repeat) or already recording — ignore.
        }
        MODE_PROCESSING => {
            // 処理中は無視
        }
        _ => {
            cancel_auto_close();
            start_agent_capture(app);
        }
    }
}

// Called from WM_AGENT_SHORTCUT_RELEASE on the overlay thread.
// Hold-to-talk: releasing the key finalizes the recording and submits.
pub(super) fn handle_shortcut_release() {
    log::info!(
        "[agent] shortcut release, mode={}",
        CURRENT_MODE.load(Ordering::Relaxed)
    );
    if CURRENT_MODE.load(Ordering::Relaxed) == MODE_LISTENING {
        stop_listening();
    }
}

// Stop listening and signal STT to finalize.
pub(super) fn stop_listening() {
    SHORTCUT_ARM_TOKEN.fetch_add(1, Ordering::Relaxed);
    if stt::stt_get_active_caller().as_deref() != Some("native_agent") {
        // No active native_agent STT session — close the overlay if it is stuck in LISTENING.
        if CURRENT_MODE.load(Ordering::Relaxed) == MODE_LISTENING {
            if let Some(app) = APP_HANDLE.get() {
                close_panel(app, false);
            }
        }
        return;
    }
    AGENT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .stop_requested = true;
    let _ = stt::stt_stop_stream();
}

pub(super) fn start_agent_capture(app: AppHandle) {
    clear_agent_listener(&app);

    if stt::stt_is_running() {
        match stt::stt_get_active_caller().as_deref() {
            Some("native_agent") => return, // already capturing for this caller
            Some(_) => {
                transition_to_notice(&app, "ほかの音声入力が動作中です");
                return;
            }
            None => {}
        }
    }

    {
        let mut ag = AGENT.lock().unwrap_or_else(|e| e.into_inner());
        ag.stop_requested = false;
        ag.finals_accumulated.clear();
        ag.current_speech.clear();
        ag.result_accumulated.clear();
    }

    transition_to_listening(&app, Some(MUTED_TEXT));

    if let Err(err) = stt::stt_start_stream(app.clone(), "native_agent".to_string(), Some(false)) {
        transition_to_notice(&app, &err);
    }
}

// ─ Agent integration ─────────────────────────────────────────────────────────
pub(super) fn submit_to_agent(app: AppHandle, text: String) {
    if text.trim().is_empty() {
        close_panel(&app, false);
        return;
    }

    transition_to_processing(&app);

    let db = app.state::<Database>();
    let conv_id = uuid_v4();
    let _ = db.agent_create_conversation(&conv_id, "Voice Shortcut");

    let cid = conv_id.clone();
    let app_listener = app.clone();
    let lid = app.listen(format!("agent_stream:{conv_id}"), move |event| {
        handle_agent_stream(&app_listener, &cid, event.payload());
    });
    AGENT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .agent_listener = Some(lid);

    tauri::async_runtime::spawn(async move {
        let _ = agent::agent_send(app, conv_id, text, Vec::new()).await;
    });
}

pub(super) fn handle_agent_stream(app: &AppHandle, _conv_id: &str, payload: &str) {
    let parsed = serde_json::from_str::<Value>(payload).unwrap_or(Value::Null);
    let ev = parsed
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    match ev {
        "token" => {
            let chunk = parsed
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if !chunk.is_empty() {
                AGENT
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .result_accumulated
                    .push_str(chunk);
            }
        }
        "error" => {
            let msg = parsed
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("エラーが発生しました");
            clear_agent_listener(app);
            AGENT
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .result_accumulated = msg.to_string();
            transition_to_notice(app, msg);
        }
        "done" => {
            let final_text = AGENT
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .result_accumulated
                .trim()
                .to_string();
            clear_agent_listener(app);
            if final_text.is_empty() {
                transition_to_notice(app, "応答を取得できませんでした");
            } else {
                transition_to_result(app, &final_text);
            }
        }
        _ => {}
    }
}

pub(super) fn clear_agent_listener(app: &AppHandle) {
    if let Some(id) = AGENT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .agent_listener
        .take()
    {
        app.unlisten(id);
    }
}
