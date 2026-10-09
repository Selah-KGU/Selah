//! Caption sequencing, width spring, and fade animation.

use super::*;
use crate::main_thread_animation::MainThreadAnimation;
use std::ptr::null;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::Manager;
use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOWNOACTIVATE};

fn morph_to(app: AppHandle, target_w: i32) {
    let animation = MainThreadAnimation::start(&MORPH_TOKEN);
    tauri::async_runtime::spawn(async move {
        let Some(Some(snapshot)) = animation.read(&app, frame_snapshot).await else {
            return;
        };
        let mut spring = Spring::new(snapshot.width as f64);
        spring.set_target(target_w as f64);
        spring.vel = (target_w - snapshot.width) as f64 * 2.5;
        for _ in 0..90 {
            if !animation.is_current() {
                return;
            }
            if !spring.tick() {
                break;
            }
            let width = spring.pos.round() as i32;
            if !animation
                .frame(&app, move || {
                    apply_frame(width, snapshot.center_x, snapshot.top_y)
                })
                .await
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
        animation
            .frame(&app, move || {
                apply_frame(target_w, snapshot.center_x, snapshot.top_y)
            })
            .await;
    });
}

// Only a current debounce may claim a new width animation on the main thread.
fn trigger_morph(app: AppHandle, target_w: i32, debounce: bool) {
    let request = MainThreadAnimation::start(&MORPH_DEBOUNCE_TOKEN);
    if debounce {
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(MORPH_DEBOUNCE_MS)).await;
            let app_morph = app.clone();
            request
                .frame(&app, move || morph_to(app_morph, target_w))
                .await;
        });
    } else {
        morph_to(app, target_w);
    }
}

pub(super) fn show_caption(app: &AppHandle, caption: Caption) {
    if !captions_enabled() {
        return;
    }
    let Some(ticket) = CAPTION_MAILBOX.push(caption) else {
        return;
    };
    ensure_overlay_window(app);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // Creation runs on the Win32 thread. Retain only the mailbox ticket
        // while it is pending, so the first caption is not lost during startup.
        while !HWND_READY.load(Ordering::Acquire) {
            if !captions_enabled() || !CAPTION_MAILBOX.is_scheduled(ticket) {
                CAPTION_MAILBOX.cancel(ticket);
                return;
            }
            if !CREATING.load(Ordering::SeqCst) {
                // Publication may finish between the loop's READY read and
                // CREATING read. Recheck before treating creation as failed.
                if HWND_READY.load(Ordering::Acquire) {
                    break;
                }
                CAPTION_MAILBOX.cancel(ticket);
                return;
            }
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
        let app_update = app.clone();
        if let Err(error) = app.run_on_main_thread(move || {
            if !captions_enabled() {
                CAPTION_MAILBOX.cancel(ticket);
                return;
            }
            let Some(caption) = CAPTION_MAILBOX.take(ticket) else {
                return;
            };
            if !caption.is_current()
                || !app_update
                    .state::<crate::live::LiveState>()
                    .is_session_current(&caption.session_id)
            {
                return;
            }
            apply_caption(&app_update, caption);
        }) {
            CAPTION_MAILBOX.cancel(ticket);
            log::warn!("subtitle overlay: text dispatch failed: {error}");
        }
    });
}

fn apply_caption(app: &AppHandle, caption: Caption) {
    HIDE_TOKEN.fetch_add(1, Ordering::Relaxed);
    let Caption {
        text,
        session_id,
        is_final,
        ..
    } = caption;
    let target_w = estimate_text_w(&text);
    // Extract hwnd and update text while holding the lock, then release the lock
    // before making cross-thread Win32 calls (SendMessage-based calls while holding
    // a mutex shared with the overlay thread's WndProc would deadlock).
    let hwnd = {
        let mut state = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
        if state.hwnd == 0 {
            return;
        }
        state.text = text;
        state.displayed_session_id = Some(session_id.clone());
        state.hwnd
    };
    let hwnd = hwnd_from_raw(hwnd);
    unsafe {
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = InvalidateRect(hwnd, null(), 1);
    }

    // Partials are debounced: the spring only fires after the text has been stable
    // for MORPH_DEBOUNCE_MS, preventing jitter from rapid sequential STT updates.
    // Final transcripts trigger immediately and also cancel any pending debounce.
    trigger_morph(app.clone(), target_w, !is_final);

    let fade = MainThreadAnimation::start(&FADE_TOKEN);
    let app_fade = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(Some(snapshot)) = fade.read(&app_fade, frame_snapshot).await else {
            return;
        };
        if snapshot.alpha >= 250 {
            return;
        }
        for i in 0..=FADE_FRAMES {
            let alpha = snapshot.alpha as f64
                + (255.0 - snapshot.alpha as f64) * ease_out_quart(i as f64 / FADE_FRAMES as f64);
            if !fade
                .frame(&app_fade, move || {
                    set_alpha(alpha.round().clamp(0.0, 255.0) as u8)
                })
                .await
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });

    if is_final {
        schedule_fade_out(app, SUB_FADE_DELAY_SECS, Some(session_id));
    }
}

pub(super) fn schedule_fade_out(app: &AppHandle, delay_secs: u64, expected_owner: Option<String>) {
    let hide = MainThreadAnimation::start(&HIDE_TOKEN);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(delay_secs)).await;
        let app_hide = app.clone();
        let Some(Some((fade, snapshot))) = hide
            .read(&app, move || {
                if !captions_enabled()
                    || app_hide
                        .state::<crate::live::LiveState>()
                        .active_session_id()
                        != expected_owner
                {
                    return None;
                }
                let snapshot = frame_snapshot()?;
                trigger_morph(app_hide, SUB_MIN_W, false);
                Some((MainThreadAnimation::start(&FADE_TOKEN), snapshot))
            })
            .await
        else {
            return;
        };
        for i in (0..=FADE_FRAMES).rev() {
            let alpha = (snapshot.alpha as f64 * ease_out_quart(i as f64 / FADE_FRAMES as f64))
                .round()
                .clamp(0.0, 255.0) as u8;
            if !fade.frame(&app, move || set_alpha(alpha)).await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });
}

pub(super) fn status_changed(app: &AppHandle, active: bool) {
    let app_update = app.clone();
    let _ = app.run_on_main_thread(move || {
        if !captions_enabled() {
            return;
        }
        let owner = app_update
            .state::<crate::live::LiveState>()
            .active_session_id();
        if !active {
            if owner.is_none() {
                schedule_fade_out(&app_update, SUB_FADE_DELAY_SECS, None);
            }
            return;
        }
        let Some(owner) = owner else {
            return;
        };
        ensure_overlay_window(&app_update);
        let changed = {
            let mut state = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
            if state.displayed_session_id.as_deref() == Some(&owner) {
                false
            } else {
                state.displayed_session_id = Some(owner);
                state.text.clear();
                true
            }
        };
        if changed {
            HIDE_TOKEN.fetch_add(1, Ordering::Relaxed);
            FADE_TOKEN.fetch_add(1, Ordering::Relaxed);
            MORPH_TOKEN.fetch_add(1, Ordering::Relaxed);
            MORPH_DEBOUNCE_TOKEN.fetch_add(1, Ordering::Relaxed);
            set_alpha(0);
        }
    });
}
