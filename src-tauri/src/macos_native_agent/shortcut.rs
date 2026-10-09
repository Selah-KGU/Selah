use super::*;

// Direct query of the Fn (kVK_Function) key state. We can't rely on
// `NSEventModifierFlags::Function` alone because macOS sets that bit for
// *any* function-class key — arrow keys, F1–F12, Home/End/Page Up/Down all
// flip it on. Polling modifier flags would mis-detect a held arrow key as
// the Fn shortcut. CGEventSourceKeyState reads the live HID state for a
// specific keycode, so it returns true only when the actual Fn key is held.
const KVK_FUNCTION: u16 = 0x3F; // kVK_Function (Fn / Globe key)
const KCG_EVENT_SOURCE_STATE_COMBINED: i32 = 0; // kCGEventSourceStateCombinedSessionState

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceKeyState(state_id: i32, key: u16) -> bool;
}

fn is_fn_key_down() -> bool {
    // Safety: CGEventSourceKeyState is documented thread-safe and takes a
    // primitive state id + keycode with no out-pointer or allocation.
    unsafe { CGEventSourceKeyState(KCG_EVENT_SOURCE_STATE_COMBINED, KVK_FUNCTION) }
}

pub fn apply_config(app: &AppHandle, config: &NativeAgentConfig) -> Result<(), String> {
    let shortcut = normalize_shortcut(&config.voice_shortcut);
    let manager = app.global_shortcut();
    let mut registered = SHORTCUT_REGISTERED.lock().unwrap();
    stop_fn_polling();

    if let Some(prev) = registered.clone() {
        if prev != shortcut && manager.is_registered(prev.as_str()) {
            manager
                .unregister(prev.as_str())
                .map_err(|e| format!("failed to unregister voice shortcut: {e}"))?;
        }
    }

    if config.voice_shortcut_enabled {
        if shortcut == "fn" {
            start_fn_polling(app.clone());
        } else if registered.as_deref() != Some(shortcut.as_str())
            || !manager.is_registered(shortcut.as_str())
        {
            let shortcut_for_handler = shortcut.clone();
            manager
                .on_shortcut(shortcut.as_str(), move |app, _shortcut, event| {
                    handle_shortcut_event(app.clone(), shortcut_for_handler.clone(), event.state);
                })
                .map_err(|e| format!("failed to register voice shortcut: {e}"))?;
        }
        *registered = Some(shortcut);
    } else {
        if let Some(prev) = registered.take() {
            if manager.is_registered(prev.as_str()) {
                manager
                    .unregister(prev.as_str())
                    .map_err(|e| format!("failed to unregister voice shortcut: {e}"))?;
            }
        }
        clear_agent_stream();
        SHARED.lock().unwrap().shortcut.release();
        RELEASE_FINALIZE_TOKEN.fetch_add(1, Ordering::Relaxed);
        FN_PRESSED.store(false, Ordering::Relaxed);
        close_panel(app, true);
    }

    Ok(())
}

fn handle_shortcut_event(app: AppHandle, _shortcut: String, state: ShortcutState) {
    match state {
        ShortcutState::Pressed => handle_shortcut_pressed(app),
        ShortcutState::Released => handle_shortcut_released(app),
    }
}

fn handle_fn_state(app: AppHandle, has_fn: bool) {
    let was_pressed = FN_PRESSED.swap(has_fn, Ordering::Relaxed);
    if has_fn && !was_pressed {
        handle_shortcut_pressed(app);
    } else if !has_fn && was_pressed {
        handle_shortcut_released(app);
    }
}

fn start_fn_polling(app: AppHandle) {
    let token = FN_POLL_TOKEN
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        loop {
            if FN_POLL_TOKEN.load(Ordering::Relaxed) != token {
                break;
            }

            // CGEventSourceKeyState is thread-safe and answers the question
            // we actually care about ("is the Fn key down right now?")
            // without the Function-flag false-positives caused by arrow /
            // F-keys. No main-thread round-trip needed.
            handle_fn_state(app.clone(), is_fn_key_down());

            let next_ms = if FN_PRESSED.load(Ordering::Relaxed) {
                FN_POLL_HELD_MS
            } else {
                FN_POLL_IDLE_MS
            };
            tokio::time::sleep(Duration::from_millis(next_ms)).await;
        }
    });
}

fn stop_fn_polling() {
    FN_POLL_TOKEN.fetch_add(1, Ordering::Relaxed);
}

pub(super) fn schedule_release_finalize(app: AppHandle, delay_ms: u64) {
    let (input_id, token) = {
        let state = SHARED.lock().unwrap();
        let Some(input_id) = state.capture.id().map(str::to_owned) else {
            return;
        };
        let token = RELEASE_FINALIZE_TOKEN
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        (input_id, token)
    };
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        if RELEASE_FINALIZE_TOKEN.load(Ordering::Relaxed) != token {
            return;
        }
        tauri::async_runtime::spawn_blocking(move || {
            let Ok(state) = stt::stt_get_stream_state() else {
                return;
            };
            if state.owner.as_ref().is_some_and(|owner| {
                owner.caller == "native_agent"
                    && owner.input_session_id.as_deref() == Some(&input_id)
            }) {
                return;
            }
            super::flow::finish_capture_for_release(app, &input_id, token);
        });
    });
}

fn handle_shortcut_pressed(app: AppHandle) {
    let request = {
        let mut state = SHARED.lock().unwrap();
        let Some(request) = state.press_shortcut() else {
            return;
        };
        cancel_auto_close();
        request
    };
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(SHORTCUT_HOLD_MS)).await;
        tauri::async_runtime::spawn_blocking(move || start_agent_capture(app, request));
    });
}

fn handle_shortcut_released(app: AppHandle) {
    let input_id = {
        let mut sh = SHARED.lock().unwrap();
        sh.shortcut.release();
        if sh.mode != Some(CapsuleMode::Listening) {
            return;
        }
        let input_id = sh.capture.id().map(str::to_owned);
        sh.stop_requested = true;
        input_id
    };
    if let Some(id) = input_id {
        let _ = stt::stt_request_native_input_stop(&id);
    }
    schedule_release_finalize(app, RELEASE_FINALIZE_DELAY_MS);
}

fn start_agent_capture(app: AppHandle, request: crate::native_shortcut::ShortcutRequest) {
    let input_id = uuid::Uuid::new_v4().to_string();
    let Some(update) = crate::native_shortcut::prepare_capture(
        &SHARED,
        request,
        input_id.clone(),
        "話してください",
        stt::stt_get_stream_state,
    ) else {
        return;
    };
    enqueue_view_update(&app, update.view);
    if !update.start {
        return;
    }

    match stt::stt_start_native_input_now(app.clone(), input_id.clone()) {
        Ok(_) => {
            let stop = {
                let sh = SHARED.lock().unwrap();
                if !sh.capture.owns(Some(&input_id)) {
                    true
                } else {
                    sh.stop_requested
                }
            };
            if stop {
                let _ = stt::stt_request_native_input_stop(&input_id);
                schedule_release_finalize(app, RELEASE_FINALIZE_DELAY_MS);
            }
        }
        Err(err) => capture_error(&app, &input_id, &err),
    }
}

fn normalize_shortcut(shortcut: &str) -> String {
    let value = shortcut.trim().to_ascii_lowercase();
    if value.is_empty() {
        return DEFAULT_SHORTCUT.into();
    }
    // Accept the user-friendly "option+space" alias by mapping it onto the
    // accelerator string the global-shortcut plugin actually parses.
    if value == "option+space" {
        return "alt+space".into();
    }
    value
}
