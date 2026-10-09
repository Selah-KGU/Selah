//! Live-session listeners and overlay open/close entry points.

use super::*;
use std::sync::atomic::Ordering;
use tauri::Listener;
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, ShowWindow, SW_HIDE, WM_CLOSE};

pub fn setup(app: &AppHandle) {
    let _ = APP_HANDLE.set(app.clone());

    let mut listeners =
        crate::subtitle_events::subscribe(app, captions_enabled, show_caption, status_changed);

    let app_theme = app.clone();
    let lid_theme = app.listen("app-theme-changed", move |_event| {
        set_theme_mode(prefers_dark(&app_theme));
    });

    listeners.push(lid_theme);
    SHARED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .event_listeners = listeners;
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
    CAPTION_MAILBOX.clear();
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
            state.displayed_session_id = None;
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
