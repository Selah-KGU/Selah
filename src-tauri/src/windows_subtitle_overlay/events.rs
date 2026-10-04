//! Live-session listeners and overlay open/close entry points.

use super::*;
use serde_json::Value;
use std::sync::atomic::Ordering;
use tauri::Listener;
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, ShowWindow, SW_HIDE, WM_CLOSE};

pub fn setup(app: &AppHandle) {
    let _ = APP_HANDLE.set(app.clone());

    let app_state = app.clone();
    let lid_state = app.listen("live-session-updated", move |event| {
        if !OVERLAY_OPEN.load(Ordering::Relaxed) {
            return;
        }
        let payload = serde_json::from_str::<Value>(event.payload()).unwrap_or_default();
        let active = payload
            .get("active")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !active {
            schedule_fade_out(SUB_FADE_DELAY_SECS);
        } else {
            ensure_overlay_window(&app_state);
        }
    });

    let app_line = app.clone();
    let lid_line = app.listen("live-line-appended", move |event| {
        if !OVERLAY_OPEN.load(Ordering::Relaxed) {
            return;
        }
        let payload = serde_json::from_str::<Value>(event.payload()).unwrap_or_default();
        let text = payload
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_owned();
        if text.trim().is_empty() || !claim_caption_seq(0) {
            return;
        }
        show_text(&app_line, text, true);
    });

    let app_partial = app.clone();
    let lid_partial = app.listen("stt-partial", move |event| {
        if !OVERLAY_OPEN.load(Ordering::Relaxed) {
            return;
        }
        let payload = serde_json::from_str::<Value>(event.payload()).unwrap_or_default();
        if payload.get("caller").and_then(|c| c.as_str()) != Some("live") {
            return;
        }
        let text = payload
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_owned();
        if text.trim().is_empty() || !claim_caption_seq(payload_seq(&payload)) {
            return;
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let last = LAST_PARTIAL_MS.load(Ordering::Relaxed);
        if now_ms.saturating_sub(last) < PARTIAL_MIN_INTERVAL_MS {
            return;
        }
        LAST_PARTIAL_MS.store(now_ms, Ordering::Relaxed);
        show_text(&app_partial, text, false);
    });

    let app_stt_final = app.clone();
    let lid_stt_final = app.listen("stt-final", move |event| {
        if !OVERLAY_OPEN.load(Ordering::Relaxed) {
            return;
        }
        let payload = serde_json::from_str::<Value>(event.payload()).unwrap_or_default();
        if payload.get("caller").and_then(|c| c.as_str()) != Some("live") {
            return;
        }
        let text = payload
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_owned();
        if text.trim().is_empty() || !claim_caption_seq(payload_seq(&payload)) {
            return;
        }
        show_text(&app_stt_final, text, true);
    });

    let app_theme = app.clone();
    let lid_theme = app.listen("app-theme-changed", move |_event| {
        set_theme_mode(prefers_dark(&app_theme));
    });

    SHARED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .event_listeners = vec![lid_state, lid_line, lid_partial, lid_stt_final, lid_theme];
}

pub fn open_overlay(app: &AppHandle) -> Result<(), String> {
    OVERLAY_OPEN.store(true, Ordering::Relaxed);
    set_theme_mode(prefers_dark(app));
    ensure_overlay_window(app);
    set_alpha(frame_snapshot().map(|s| s.alpha).unwrap_or(0));
    Ok(())
}

pub fn close_overlay(_app: &AppHandle) -> Result<(), String> {
    OVERLAY_OPEN.store(false, Ordering::Relaxed);
    HWND_READY.store(false, Ordering::Relaxed);
    HIDE_TOKEN.fetch_add(1, Ordering::Relaxed);
    FADE_TOKEN.fetch_add(1, Ordering::Relaxed);
    MORPH_TOKEN.fetch_add(1, Ordering::Relaxed);
    MORPH_DEBOUNCE_TOKEN.fetch_add(1, Ordering::Relaxed);

    let closing_hwnd = {
        let mut state = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
        if state.hwnd == 0 {
            None
        } else {
            let hwnd = state.hwnd;
            state.hwnd = 0;
            state.width = 0;
            state.alpha = 0;
            state.text.clear();
            Some(hwnd)
        }
    };

    if let Some(hwnd) = closing_hwnd {
        let hwnd = hwnd_from_raw(hwnd);
        unsafe {
            ShowWindow(hwnd, SW_HIDE);
            let _ = PostMessageW(hwnd, WM_CLOSE, 0, 0);
        }
    }
    Ok(())
}

pub fn is_open() -> bool {
    OVERLAY_OPEN.load(Ordering::Relaxed) && HWND_READY.load(Ordering::Relaxed)
}
