//! Caption sequencing, width spring, and fade animation.

use super::*;
use serde_json::Value;
use std::ptr::null;
use std::sync::atomic::Ordering;
use std::time::Duration;
use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOWNOACTIVATE};

pub(super) fn claim_caption_seq(seq: u64) -> bool {
    let last = LAST_CAPTION_SEQ.load(Ordering::SeqCst);
    if seq > 0 && seq < last {
        return false;
    }
    if seq > 0 {
        LAST_CAPTION_SEQ.store(seq, Ordering::SeqCst);
        return true;
    }
    // Unsequenced echo of a line the page already committed. Once a sequenced
    // caption has been shown, that echo must not cover a newer partial.
    last == 0
}

pub(super) fn payload_seq(payload: &Value) -> u64 {
    payload
        .get("seq")
        .and_then(|value| value.as_u64())
        .unwrap_or(0)
}

fn morph_to(target_w: i32) {
    let Some(snapshot) = frame_snapshot() else {
        return;
    };
    let token = MORPH_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        let mut spring = Spring::new(snapshot.width as f64);
        spring.set_target(target_w as f64);
        spring.vel = (target_w - snapshot.width) as f64 * 2.5;

        for _ in 0..90 {
            if MORPH_TOKEN.load(Ordering::Relaxed) != token {
                return;
            }
            if !spring.tick() {
                break;
            }
            apply_frame(spring.pos.round() as i32, snapshot.center_x, snapshot.top_y);
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
        apply_frame(target_w, snapshot.center_x, snapshot.top_y);
    });
}

// For partial updates: debounce the morph so rapid STT partials don't restart the
// spring animation on every word. Text is already displayed; only the width animation
// is delayed until the text stabilizes.
// For final/immediate: cancels any pending debounce and morphs right away.
fn trigger_morph(target_w: i32, debounce: bool) {
    let token = MORPH_DEBOUNCE_TOKEN
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    if debounce {
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(MORPH_DEBOUNCE_MS)).await;
            if MORPH_DEBOUNCE_TOKEN.load(Ordering::Relaxed) != token {
                return;
            }
            morph_to(target_w);
        });
    } else {
        morph_to(target_w);
    }
}

pub(super) fn show_text(app: &AppHandle, text: String, is_final: bool) {
    HIDE_TOKEN.fetch_add(1, Ordering::Relaxed);

    if !OVERLAY_OPEN.load(Ordering::Relaxed) {
        return;
    }

    ensure_overlay_window(app);

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
    trigger_morph(target_w, !is_final);

    let start_alpha = frame_snapshot().map(|s| s.alpha).unwrap_or(0);
    let fade_tok = FADE_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        if start_alpha >= 250 {
            return;
        }
        for i in 0..=FADE_FRAMES {
            if FADE_TOKEN.load(Ordering::Relaxed) != fade_tok {
                return;
            }
            let alpha = start_alpha as f64
                + (255.0 - start_alpha as f64) * ease_out_quart(i as f64 / FADE_FRAMES as f64);
            set_alpha(alpha.round().clamp(0.0, 255.0) as u8);
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });

    if is_final {
        schedule_fade_out(SUB_FADE_DELAY_SECS);
    }
}

pub(super) fn schedule_fade_out(delay_secs: u64) {
    let token = HIDE_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(delay_secs)).await;
        if HIDE_TOKEN.load(Ordering::Relaxed) != token {
            return;
        }
        let Some(snapshot) = frame_snapshot() else {
            return;
        };

        trigger_morph(SUB_MIN_W, false);

        let fade_tok = FADE_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        for i in (0..=FADE_FRAMES).rev() {
            if FADE_TOKEN.load(Ordering::Relaxed) != fade_tok {
                return;
            }
            let alpha = (255.0 * ease_out_quart(i as f64 / FADE_FRAMES as f64))
                .round()
                .clamp(0.0, 255.0) as u8;
            set_alpha(alpha);
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
        set_alpha(0);
        apply_frame(SUB_MIN_W, snapshot.center_x, snapshot.top_y);
    });
}
