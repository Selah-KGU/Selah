use super::*;

// ─ Public setup ──────────────────────────────────────────────────────────────
pub fn setup(app: &AppHandle) {
    SYSTEM_IS_DARK.store(effective_is_dark(app), Ordering::Relaxed);

    let app_theme = app.clone();
    let _ = app.run_on_main_thread(move || register_appearance_observer(app_theme));

    let app_theme_event = app.clone();
    app.listen("app-theme-changed", move |_event| {
        let dark = effective_is_dark(&app_theme_event);
        SYSTEM_IS_DARK.store(dark, Ordering::Relaxed);
        let app_main = app_theme_event.clone();
        let _ = app_main.run_on_main_thread(move || apply_theme(&Theme::current()));
    });

    let app_final = app.clone();
    app.listen("stt-final", move |event| {
        let payload = serde_json::from_str::<Value>(event.payload()).unwrap_or_default();
        if payload.get("caller").and_then(|c| c.as_str()) != Some("native_agent") {
            return;
        }
        let text = payload
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string();

        let (display, pending_submit) = {
            let mut sh = SHARED.lock().unwrap();
            if sh.mode != Some(CapsuleMode::Listening) {
                return;
            }
            append_final_segment(&mut sh, &text);
            let display = listening_display_text(&sh);
            let pending = if sh.stop_requested {
                sh.stop_requested = false;
                Some(consume_all_speech(&mut sh))
            } else {
                None
            };
            (display, pending)
        };

        if should_render_listening_text() && !display.is_empty() {
            update_text(&app_final, CapsuleMode::Listening, &display);
        }

        if let Some(text) = pending_submit {
            if !text.is_empty() {
                submit_to_agent(app_final.clone(), text);
            }
        }
    });

    let app_partial = app.clone();
    app.listen("stt-partial", move |event| {
        let payload = serde_json::from_str::<Value>(event.payload()).unwrap_or_default();
        if payload.get("caller").and_then(|c| c.as_str()) != Some("native_agent") {
            return;
        }
        let text = payload
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string();
        if text.trim().is_empty() {
            return;
        }
        let display = {
            let mut sh = SHARED.lock().unwrap();
            sh.current_speech = text;
            listening_display_text(&sh)
        };
        if should_render_listening_text() {
            update_text(&app_partial, CapsuleMode::Listening, &display);
        }
    });

    let app_state = app.clone();
    app.listen("stt-state", move |event| {
        let payload = serde_json::from_str::<Value>(event.payload()).unwrap_or_default();
        if payload.get("caller").and_then(|c| c.as_str()) != Some("native_agent") {
            return;
        }
        let state_name = payload
            .get("state")
            .and_then(|t| t.as_str())
            .unwrap_or_default();
        let listening = matches!(state_name, "initializing" | "listening");

        if listening {
            transition_to_listening(&app_state, None);
            return;
        }

        let pending = {
            let mut sh = SHARED.lock().unwrap();
            if sh.mode == Some(CapsuleMode::Processing)
                || sh.mode == Some(CapsuleMode::Result)
                || sh.mode == Some(CapsuleMode::Notice)
            {
                sh.finals_accumulated.clear();
                sh.current_speech.clear();
                None
            } else if sh.stop_requested {
                let text = consume_all_speech(&mut sh);
                if text.is_empty() {
                    None
                } else {
                    sh.stop_requested = false;
                    Some(text)
                }
            } else {
                sh.stop_requested = false;
                Some(consume_all_speech(&mut sh))
            }
        };

        if let Some(text) = pending {
            if text.is_empty() {
                close_panel(&app_state, false);
            } else {
                submit_to_agent(app_state.clone(), text);
            }
        } else {
            schedule_release_finalize(app_state.clone(), RELEASE_FINALIZE_DELAY_MS);
        }
    });

    let app_err = app.clone();
    app.listen("stt-error", move |event| {
        let payload = serde_json::from_str::<Value>(event.payload()).unwrap_or_default();
        if payload.get("caller").and_then(|c| c.as_str()) != Some("native_agent") {
            return;
        }
        {
            let mut sh = SHARED.lock().unwrap();
            sh.stop_requested = false;
            sh.finals_accumulated.clear();
            sh.current_speech.clear();
        }
        transition_to_notice(&app_err, "音声入力を開始できませんでした");
    });
}

fn should_render_listening_text() -> bool {
    let sh = SHARED.lock().unwrap();
    sh.mode == Some(CapsuleMode::Listening) && !sh.stop_requested
}

// ─ Mode transitions ──────────────────────────────────────────────────────────
pub(super) fn transition_to_listening(app: &AppHandle, text: Option<&str>) {
    cancel_auto_close();
    SHARED.lock().unwrap().mode = Some(CapsuleMode::Listening);
    ensure_panel(app, LISTEN_W, LISTEN_H);
    let display_text = if let Some(text) = text {
        text.to_string()
    } else {
        let sh = SHARED.lock().unwrap();
        let combined = listening_display_text(&sh);
        if combined.trim().is_empty() {
            "話してください".to_string()
        } else {
            combined
        }
    };
    update_text(app, CapsuleMode::Listening, &display_text);
    update_border(app.clone(), CapsuleMode::Listening);
    start_processing_dots_animation(app.clone(), CapsuleMode::Listening);
    start_listen_pulse_animation(app.clone(), CapsuleMode::Listening);
}

fn transition_to_processing(app: &AppHandle) {
    cancel_auto_close();
    {
        let mut sh = SHARED.lock().unwrap();
        sh.mode = Some(CapsuleMode::Processing);
        sh.current_speech.clear();
        sh.finals_accumulated.clear();
    }
    ensure_panel(app, PROCESS_W, PROCESS_H);
    update_text(app, CapsuleMode::Processing, "");
    update_border(app.clone(), CapsuleMode::Processing);
    start_processing_dots_animation(app.clone(), CapsuleMode::Processing);
    start_listen_pulse_animation(app.clone(), CapsuleMode::Processing);
}

fn transition_to_result(app: &AppHandle, text: &str) {
    cancel_auto_close();
    SHARED.lock().unwrap().mode = Some(CapsuleMode::Result);
    let target_h = compute_result_height(text);
    ensure_panel(app, RESULT_W, target_h);
    update_text(app, CapsuleMode::Result, text);
    update_border(app.clone(), CapsuleMode::Result);
    start_processing_dots_animation(app.clone(), CapsuleMode::Result);
    start_listen_pulse_animation(app.clone(), CapsuleMode::Result);
    schedule_close(app.clone(), Duration::from_secs(RESULT_AUTO_CLOSE_SECS));
}

pub(super) fn transition_to_notice(app: &AppHandle, message: &str) {
    cancel_auto_close();
    SHARED.lock().unwrap().mode = Some(CapsuleMode::Notice);
    ensure_panel(app, NOTICE_W, NOTICE_H);
    update_text(app, CapsuleMode::Notice, message);
    update_border(app.clone(), CapsuleMode::Notice);
    start_processing_dots_animation(app.clone(), CapsuleMode::Notice);
    start_listen_pulse_animation(app.clone(), CapsuleMode::Notice);
    schedule_close(app.clone(), Duration::from_millis(NOTICE_AUTO_CLOSE_MS));
}

// ─ Agent submission ──────────────────────────────────────────────────────────
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
    let app_for_listener = app.clone();
    let listener_id = app.listen(format!("agent_stream:{conv_id}"), move |event| {
        handle_agent_stream(&app_for_listener, &cid, event.payload());
    });

    {
        let mut sh = SHARED.lock().unwrap();
        sh.agent_listener = Some(listener_id);
        sh.result_accumulated.clear();
    }

    tauri::async_runtime::spawn(async move {
        let _ = agent::agent_send(app.clone(), conv_id, text, Vec::new()).await;
    });
}

fn handle_agent_stream(app: &AppHandle, _conv_id: &str, payload: &str) {
    let parsed = serde_json::from_str::<Value>(payload).unwrap_or(Value::Null);
    let event_type = parsed
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    match event_type {
        "token" => {
            let chunk = parsed
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if chunk.is_empty() {
                return;
            }
            let mut sh = SHARED.lock().unwrap();
            sh.result_accumulated.push_str(chunk);
        }
        "error" => {
            let msg = parsed
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("エラーが発生しました");
            clear_agent_listener(app);
            {
                let mut sh = SHARED.lock().unwrap();
                sh.result_accumulated = msg.to_string();
            }
            transition_to_notice(app, msg);
        }
        "done" => {
            let final_text = {
                let sh = SHARED.lock().unwrap();
                sh.result_accumulated.trim().to_string()
            };
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
    let listener_id = SHARED.lock().unwrap().agent_listener.take();
    if let Some(id) = listener_id {
        app.unlisten(id);
    }
}

// ─ Result height measurement ─────────────────────────────────────────────────
fn compute_result_height(text: &str) -> f64 {
    let text_w = RESULT_W - RESULT_PAD_X * 2.0;
    // Rough measurement — each character counted as ~font*0.56 (cjk ~font*0.98).
    let mut total_lines = 0.0_f64;
    for raw in text.split('\n') {
        let trimmed = raw.trim_end();
        if trimmed.is_empty() {
            total_lines += 0.5;
            continue;
        }
        let (font_size, indent) = heading_metrics(trimmed);
        let content = strip_markdown_syntax(trimmed);
        if content.is_empty() {
            total_lines += 0.5;
            continue;
        }
        let glyph_w = font_size * 0.56;
        let cjk_w = font_size * 0.98;
        let eff: f64 = content
            .chars()
            .map(|c| if c.is_ascii() { glyph_w } else { cjk_w })
            .sum();
        let effective_w = (text_w - indent).max(1.0);
        let lines = (eff / effective_w).ceil().max(1.0);
        total_lines += lines;
    }
    let visible = total_lines.min(RESULT_MAX_VISIBLE_LINES as f64).max(2.0);
    let line_px = RESULT_BODY_FONT * RESULT_LINE_HEIGHT_MUL;
    let text_h = visible * line_px + RESULT_PAD_Y * 2.0 + 6.0;
    text_h.clamp(RESULT_MIN_H, RESULT_MAX_H)
}

fn heading_metrics(line: &str) -> (f64, f64) {
    if let Some(rest) = line.strip_prefix("### ") {
        let _ = rest;
        (RESULT_H3_FONT, 0.0)
    } else if let Some(rest) = line.strip_prefix("## ") {
        let _ = rest;
        (RESULT_H2_FONT, 0.0)
    } else if let Some(rest) = line.strip_prefix("# ") {
        let _ = rest;
        (RESULT_H1_FONT, 0.0)
    } else if line.trim_start().starts_with("- ")
        || line.trim_start().starts_with("* ")
        || line.trim_start().starts_with("• ")
    {
        (RESULT_BODY_FONT, 14.0)
    } else {
        (RESULT_BODY_FONT, 0.0)
    }
}

fn strip_markdown_syntax(line: &str) -> String {
    let trimmed = line
        .trim_start_matches("### ")
        .trim_start_matches("## ")
        .trim_start_matches("# ")
        .trim_start();
    let without_bullet = if let Some(rest) = trimmed.strip_prefix("- ") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("* ") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("• ") {
        rest
    } else {
        trimmed
    };
    let mut out = String::with_capacity(without_bullet.len());
    let mut skip = false;
    for c in without_bullet.chars() {
        if c == '*' || c == '_' || c == '`' {
            skip = !skip;
            continue;
        }
        out.push(c);
    }
    out
}

fn schedule_close(app: AppHandle, delay: Duration) {
    let token = AUTO_CLOSE_TOKEN
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        if AUTO_CLOSE_TOKEN.load(Ordering::Relaxed) == token {
            close_panel(&app, false);
        }
    });
}

pub(super) fn cancel_auto_close() {
    AUTO_CLOSE_TOKEN.fetch_add(1, Ordering::Relaxed);
}

fn register_appearance_observer(app: AppHandle) {
    let app_ptr = Box::into_raw(Box::new(app)) as usize;

    unsafe {
        let dnc_cls = AnyClass::get(c"NSDistributedNotificationCenter").unwrap();
        let dnc: *mut AnyObject = msg_send![dnc_cls, defaultCenter];
        let notif_name = NSString::from_str("AppleInterfaceThemeChangedNotification");

        let block = RcBlock::new(move |_notif: *mut AnyObject| {
            let app_ref = &*(app_ptr as *const AppHandle);
            // Only track macOS appearance when the user's app theme is "system".
            if app_theme_mode(app_ref) != "system" {
                return;
            }
            let app_clone = app_ref.clone();
            let _ = app_clone.run_on_main_thread(move || {
                SYSTEM_IS_DARK.store(is_dark_mode(), Ordering::Relaxed);
                apply_theme(&Theme::current());
            });
            // Notify other modules (subtitle overlay etc.) that the
            // effective dark/light state has changed even though the user
            // didn't manually flip the toggle. The same event name is used
            // for explicit user changes so listeners stay simple.
            let _ = app_ref.emit("app-theme-changed", ());
        });

        let _: () = msg_send![
            dnc,
            addObserverForName: &*notif_name,
            object: std::ptr::null::<AnyObject>(),
            queue: std::ptr::null::<AnyObject>(),
            usingBlock: &*block
        ];
    }
}

fn is_dark_mode() -> bool {
    unsafe {
        let cls = AnyClass::get(c"NSAppearance").unwrap();
        let current: *mut AnyObject = msg_send![cls, currentDrawingAppearance];
        if current.is_null() {
            return true;
        }
        let name: *const AnyObject = msg_send![current, name];
        if name.is_null() {
            return true;
        }
        let cstr: *const std::os::raw::c_char = msg_send![name, UTF8String];
        if cstr.is_null() {
            return true;
        }
        std::ffi::CStr::from_ptr(cstr)
            .to_string_lossy()
            .contains("Dark")
    }
}

fn app_theme_mode(app: &AppHandle) -> String {
    let state = app.state::<crate::ThemeState>();
    let guard = state.0.lock().unwrap_or_else(|e| e.into_inner());
    guard.clone()
}

fn effective_is_dark(app: &AppHandle) -> bool {
    match app_theme_mode(app).as_str() {
        "light" => false,
        "dark" => true,
        _ => is_dark_mode(),
    }
}

/// Run a closure inside a CATransaction that suppresses CoreAnimation's default
/// implicit animations. Without this, every `setFrame` / `setOpacity` /
/// `setStartPoint` on a CALayer triggers a ~0.25s ease animation; when we
/// drive our own motion at 60fps these implicit animations stack and fight,
/// which reads as jitter.
pub(super) fn suppress_implicit_animations<F: FnOnce()>(f: F) {
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    CATransaction::setAnimationDuration(0.0);
    f();
    CATransaction::commit();
}

pub(super) fn ease_out_quart(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(4)
}

fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}
