use std::time::Duration;

use objc2_foundation::NSString;
use tauri::AppHandle;

use super::panel::apply_frame;
use super::*;

fn ease_out_quart(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(4)
}
fn estimate_text_w(text: &str) -> f64 {
    let w: f64 = text
        .chars()
        .map(|c| {
            let code = c as u32;
            if code > 0x2E7F {
                16.5_f64
            } else {
                8.8_f64
            }
        })
        .sum();
    (w + SUB_PAD_X * 2.0).clamp(SUB_MIN_W, SUB_MAX_W)
}

// ── Public API ─────────────────────────────────────────────────────────────────
fn morph_to(app: AppHandle, target_w: f64) {
    let token = MORPH_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        let (tx, rx) = std::sync::mpsc::sync_channel::<(f64, f64, f64)>(1);
        let _ = app.run_on_main_thread(move || {
            UI.with(|ui| {
                let ui = ui.borrow();
                let cur_w = ui
                    .panel
                    .as_ref()
                    .map(|p| p.frame().size.width)
                    .unwrap_or(target_w);
                let _ = tx.send((cur_w, ui.screen_center_x, ui.screen_bottom_y));
            });
        });
        let (start_w, cx, bottom_y) = rx.recv().unwrap_or((target_w, 0.0, 0.0));

        let mut spring = Spring::new(start_w);
        spring.set_target(target_w);
        spring.vel = (target_w - start_w) * 2.5;

        for _ in 0..90 {
            if MORPH_TOKEN.load(Ordering::Relaxed) != token {
                return;
            }
            if !spring.tick() {
                break;
            }
            let w = spring.pos.clamp(SUB_MIN_W, SUB_MAX_W);
            let _ = app.run_on_main_thread(move || apply_frame(w, cx, bottom_y));
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
        let _ = app.run_on_main_thread(move || apply_frame(target_w, cx, bottom_y));
    });
}

// ── Text update ────────────────────────────────────────────────────────────────

pub(in crate::macos_subtitle_overlay) fn show_text(app: &AppHandle, text: String, is_final: bool) {
    HIDE_TOKEN.fetch_add(1, Ordering::Relaxed);

    if !OVERLAY_OPEN.load(Ordering::Relaxed) {
        return;
    }

    let target_w = estimate_text_w(&text);

    let text_clone = text.clone();
    let _ = app.run_on_main_thread(move || {
        UI.with(|ui| {
            let ui = ui.borrow();
            if let Some(lbl) = &ui.text_label {
                lbl.setStringValue(&NSString::from_str(&text_clone));
            }
            if let Some(panel) = &ui.panel {
                panel.orderFrontRegardless();
            }
        });
    });

    morph_to(app.clone(), target_w);

    let app_fade = app.clone();
    let fade_tok = FADE_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        let (tx, rx) = std::sync::mpsc::sync_channel::<f64>(1);
        let _ = app_fade.run_on_main_thread(move || {
            let a = UI.with(|ui| {
                ui.borrow()
                    .panel
                    .as_ref()
                    .map(|p| p.alphaValue())
                    .unwrap_or(1.0)
            });
            let _ = tx.send(a);
        });
        let start_a = rx.recv().unwrap_or(1.0);
        if start_a >= 0.99 {
            return;
        }
        for i in 0..=FADE_FRAMES {
            if FADE_TOKEN.load(Ordering::Relaxed) != fade_tok {
                return;
            }
            let a = start_a + (1.0 - start_a) * ease_out_quart(i as f64 / FADE_FRAMES as f64);
            let _ = app_fade.run_on_main_thread(move || {
                UI.with(|ui| {
                    if let Some(p) = &ui.borrow().panel {
                        p.setAlphaValue(a);
                    }
                });
            });
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });

    if is_final {
        schedule_fade_out(app, SUB_FADE_DELAY_SECS);
    }
}

// ── Auto-hide ──────────────────────────────────────────────────────────────────

pub(in crate::macos_subtitle_overlay) fn schedule_fade_out(app: &AppHandle, delay_secs: u64) {
    let token = HIDE_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(delay_secs)).await;
        if HIDE_TOKEN.load(Ordering::Relaxed) != token {
            return;
        }
        // Shrink back while fading out
        morph_to(app2.clone(), SUB_MIN_W);

        let fade_tok = FADE_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        for i in (0..=FADE_FRAMES).rev() {
            if FADE_TOKEN.load(Ordering::Relaxed) != fade_tok {
                return;
            }
            let a = ease_out_quart(i as f64 / FADE_FRAMES as f64);
            let _ = app2.run_on_main_thread(move || {
                UI.with(|ui| {
                    if let Some(p) = &ui.borrow().panel {
                        p.setAlphaValue(a);
                    }
                });
            });
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });
}
