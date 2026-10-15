use std::time::Duration;

use objc2::runtime::AnyObject;
use objc2_app_kit::{NSFont, NSFontAttributeName, NSStringDrawing};
use objc2_foundation::{NSDictionary, NSString};
use tauri::{AppHandle, Manager};

use crate::main_thread_animation::MainThreadAnimation;

use super::panel::apply_frame;
use super::*;

fn ease_out_quart(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(4)
}
fn estimate_text_w(text: &str) -> f64 {
    let font = NSFont::boldSystemFontOfSize(SUB_FONT);
    let attributes = NSDictionary::<NSString, AnyObject>::from_slices(
        &[unsafe { NSFontAttributeName }],
        &[font.as_ref()],
    );
    // Match the actual label font; per-character estimates shrink English
    // pages below their text width. Reserve the text cell's interior padding.
    let width = unsafe { NSString::from_str(text).sizeWithAttributes(Some(&attributes)) }.width;
    (width.ceil() + 4.0 + SUB_PAD_X * 2.0).clamp(SUB_MIN_W, SUB_MAX_W)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translated_pages_fit_the_actual_label_font() {
        objc2::rc::autoreleasepool(|_| {
            for text in [
                "Momentum measures movement. These words all belong to one complete translation.",
                "WWW WWW WWW WWW WWW WWW WWW WWW WWW WWW WWW WWW",
                "第一句讲期望值的定义以及如何计算。第二句解释方差和标准差的区别。",
                "👨‍👩‍👧‍👦 と cafe\u{301} の話です。次の行も表示します。",
            ] {
                for page in crate::subtitle_events::pages::split_pages(text) {
                    let font = NSFont::boldSystemFontOfSize(SUB_FONT);
                    let attributes = NSDictionary::<NSString, AnyObject>::from_slices(
                        &[unsafe { NSFontAttributeName }],
                        &[font.as_ref()],
                    );
                    let actual =
                        unsafe { NSString::from_str(&page).sizeWithAttributes(Some(&attributes)) }
                            .width;
                    let label_width = estimate_text_w(&page) - SUB_PAD_X * 2.0 - 4.0;
                    assert!(actual <= label_width, "page is clipped: {page:?}");
                }
            }
        });
    }
}

// ── Public API ─────────────────────────────────────────────────────────────────
fn morph_to(app: AppHandle, target_w: f64) {
    let animation = MainThreadAnimation::start(&MORPH_TOKEN);
    tauri::async_runtime::spawn(async move {
        let Some(Some((start_w, cx, bottom_y))) = animation
            .read(&app, || {
                UI.with(|ui| {
                    let ui = ui.borrow();
                    let panel = ui.panel.as_ref()?;
                    Some((
                        panel.frame().size.width,
                        ui.screen_center_x,
                        ui.screen_bottom_y,
                    ))
                })
            })
            .await
        else {
            return;
        };

        // Repeated captions can have the same capped width. Cancel the old
        // morph but avoid rewriting five view frames when already at target.
        if start_w == target_w {
            return;
        }

        let mut spring = Spring::new(start_w);
        spring.set_target(target_w);
        spring.vel = (target_w - start_w) * 2.5;

        for _ in 0..90 {
            if !animation.is_current() {
                return;
            }
            if !spring.tick() {
                break;
            }
            let w = spring.pos.clamp(SUB_MIN_W, SUB_MAX_W);
            if !animation
                .frame(&app, move || apply_frame(w, cx, bottom_y))
                .await
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
        animation
            .frame(&app, move || apply_frame(target_w, cx, bottom_y))
            .await;
    });
}

fn fade_in(app: AppHandle) {
    let animation = MainThreadAnimation::start(&FADE_TOKEN);
    tauri::async_runtime::spawn(async move {
        let Some(Some(start_a)) = animation
            .read(&app, || {
                UI.with(|ui| ui.borrow().panel.as_ref().map(|p| p.alphaValue()))
            })
            .await
        else {
            return;
        };
        if start_a >= 0.99 {
            return;
        }
        for i in 0..=FADE_FRAMES {
            let a = start_a + (1.0 - start_a) * ease_out_quart(i as f64 / FADE_FRAMES as f64);
            if !animation
                .frame(&app, move || {
                    UI.with(|ui| {
                        if let Some(p) = &ui.borrow().panel {
                            p.setAlphaValue(a);
                        }
                    });
                })
                .await
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });
}

// ── Text update ────────────────────────────────────────────────────────────────

pub(in crate::macos_subtitle_overlay) fn show_caption(app: &AppHandle, caption: Caption) {
    if !captions_enabled() {
        return;
    }
    let Some(ticket) = CAPTION_MAILBOX.push(caption) else {
        return;
    };
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
        HIDE_TOKEN.fetch_add(1, Ordering::Relaxed);
        let target_w = estimate_text_w(&caption.text);
        UI.with(|ui| {
            let mut ui = ui.borrow_mut();
            ui.displayed_session_id = Some(caption.session_id.clone());
            if let Some(lbl) = &ui.text_label {
                lbl.setStringValue(&NSString::from_str(&caption.text));
            }
            if let Some(panel) = &ui.panel {
                panel.orderFrontRegardless();
            }
        });
        morph_to(app_update.clone(), target_w);
        fade_in(app_update.clone());
        if caption.is_final {
            schedule_fade_out(&app_update, SUB_FADE_DELAY_SECS, Some(caption.session_id));
        }
    }) {
        CAPTION_MAILBOX.cancel(ticket);
        log::warn!("subtitle overlay: text dispatch failed: {error}");
    }
}

pub(in crate::macos_subtitle_overlay) fn status_changed(app: &AppHandle, active: bool) {
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
        let changed = UI.with(|ui| {
            let mut ui = ui.borrow_mut();
            if ui.displayed_session_id.as_deref() == Some(&owner) {
                return false;
            }
            ui.displayed_session_id = Some(owner);
            if let Some(label) = &ui.text_label {
                label.setStringValue(&NSString::from_str(""));
            }
            if let Some(panel) = &ui.panel {
                panel.setAlphaValue(0.0);
            }
            true
        });
        if changed {
            HIDE_TOKEN.fetch_add(1, Ordering::Relaxed);
            FADE_TOKEN.fetch_add(1, Ordering::Relaxed);
            MORPH_TOKEN.fetch_add(1, Ordering::Relaxed);
        }
    });
}

// ── Auto-hide ──────────────────────────────────────────────────────────────────

pub(in crate::macos_subtitle_overlay) fn schedule_fade_out(
    app: &AppHandle,
    delay_secs: u64,
    expected_owner: Option<String>,
) {
    let hide = MainThreadAnimation::start(&HIDE_TOKEN);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(delay_secs)).await;
        let app_hide = app.clone();
        let Some(Some(animation)) = hide
            .read(&app, move || {
                if !captions_enabled()
                    || app_hide
                        .state::<crate::live::LiveState>()
                        .active_session_id()
                        != expected_owner
                {
                    return None;
                }
                morph_to(app_hide, SUB_MIN_W);
                Some(MainThreadAnimation::start(&FADE_TOKEN))
            })
            .await
        else {
            return;
        };
        for i in (0..=FADE_FRAMES).rev() {
            let a = ease_out_quart(i as f64 / FADE_FRAMES as f64);
            if !animation
                .frame(&app, move || {
                    UI.with(|ui| {
                        if let Some(p) = &ui.borrow().panel {
                            p.setAlphaValue(a);
                        }
                    });
                })
                .await
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });
}
