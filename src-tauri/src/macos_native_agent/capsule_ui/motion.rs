//! Capsule resize, fade, and indicator animations.

use super::border::rounded_rect_path;
use super::*;
use crate::main_thread_animation::MainThreadAnimation;

pub(super) fn animate_to(app: AppHandle, target_w: f64, target_h: f64, token: u64) {
    let animation = MainThreadAnimation::with_token(&MORPH_TOKEN, token);
    tauri::async_runtime::spawn(async move {
        let Some(Some((start_w, start_h, cx, top_y))) = animation
            .read(&app, || {
                UI.with(|ui| {
                    let ui = ui.borrow();
                    let panel = ui.panel.as_ref()?;
                    let frame = panel.frame();
                    Some((
                        frame.size.width,
                        frame.size.height,
                        ui.screen_center_x,
                        ui.screen_top_y,
                    ))
                })
            })
            .await
        else {
            return;
        };

        let mut sw = Spring::new(start_w);
        sw.set_target(target_w);
        let mut sh = Spring::new(start_h);
        sh.set_target(target_h);

        loop {
            if !animation.is_current() {
                return;
            }
            let moving_w = sw.tick();
            let moving_h = sh.tick();
            let w = sw.pos;
            let h = sh.pos;
            if !animation
                .frame(&app, move || apply_frame(w, h, cx, top_y))
                .await
            {
                return;
            }
            if !moving_w && !moving_h {
                break;
            }
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
        animation
            .frame(&app, move || apply_frame(target_w, target_h, cx, top_y))
            .await;
    });
}

fn apply_frame(width: f64, height: f64, center_x: f64, top_y: f64) {
    UI.with(|ui| {
        let ui = ui.borrow();
        let x = center_x - width / 2.0;
        let y = top_y - height;
        let origin = NSPoint::new(0.0, 0.0);
        let size = NSSize::new(width, height);
        let radius = CORNER_RADIUS.min(height / 2.0);
        let mode = SHARED
            .lock()
            .unwrap()
            .mode
            .unwrap_or(CapsuleMode::Listening);

        // setFrame_display on the NSPanel is a window-server op and must run
        // outside the CATransaction (it's not a CALayer op). Pass display:false
        // so the window server doesn't force a synchronous redraw each tick —
        // the backing layers repaint on their own anyway.
        if let Some(panel) = &ui.panel {
            panel.setFrame_display(NSRect::new(NSPoint::new(x, y), size), false);
        }

        suppress_implicit_animations(|| {
            if let Some(root) = &ui.root_view {
                root.setFrame(NSRect::new(origin, size));
            }
            if let Some(capsule) = &ui.capsule_view {
                capsule.setFrame(NSRect::new(origin, size));
                if let Some(layer) = capsule.layer() {
                    layer.setCornerRadius(radius);
                }
            }
            if let Some(vfx) = &ui.vfx_view {
                vfx.setFrame(NSRect::new(origin, size));
                if let Some(layer) = vfx.layer() {
                    layer.setCornerRadius(radius);
                }
            }
            if let Some(bg) = &ui.bg_overlay {
                bg.setFrame(NSRect::new(origin, size));
                if let Some(layer) = bg.layer() {
                    layer.setCornerRadius(radius);
                }
            }
            if let (Some(gradient), Some(mask)) = (&ui.gradient_border, &ui.gradient_mask) {
                gradient.setFrame(NSRect::new(origin, size));
                mask.setFrame(NSRect::new(origin, size));
                mask.setPath(Some(&rounded_rect_path(
                    width,
                    height,
                    radius,
                    BORDER_GRADIENT_W,
                )));
            }
            if ui.text_label.is_some() {
                layout_label(&ui, mode);
            }
            layout_processing_dots(&ui, mode);
            layout_listen_indicator(&ui, mode);
        });
    });
}

pub(super) async fn fade_to(app: AppHandle, target_alpha: f64, token: u64) -> bool {
    let animation = MainThreadAnimation::with_token(&FADE_TOKEN, token);
    let Some(Some(start_alpha)) = animation
        .read(&app, || {
            UI.with(|ui| ui.borrow().panel.as_ref().map(|p| p.alphaValue()))
        })
        .await
    else {
        return false;
    };

    for frame in 0..FADE_FRAMES {
        let t = (frame + 1) as f64 / FADE_FRAMES as f64;
        let alpha = start_alpha + (target_alpha - start_alpha) * ease_out_quart(t);
        if !animation
            .frame(&app, move || {
                UI.with(|ui| {
                    if let Some(panel) = &ui.borrow().panel {
                        panel.setAlphaValue(alpha);
                    }
                });
            })
            .await
        {
            return false;
        }
        if frame + 1 < FADE_FRAMES {
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    }
    true
}

// ─ Listening indicator & processing dots animations ──────────────────────────
pub(in crate::macos_native_agent) fn start_listen_pulse_animation(
    app: AppHandle,
    mode: CapsuleMode,
    token: u64,
) {
    let animation = MainThreadAnimation::with_token(&LISTEN_PULSE_TOKEN, token);
    if mode != CapsuleMode::Listening {
        let _ = app.run_on_main_thread(move || {
            if !animation.is_current() {
                return;
            }
            UI.with(|ui| {
                let ui = ui.borrow();
                if let Some(view) = &ui.listen_indicator {
                    if let Some(layer) = view.layer() {
                        suppress_implicit_animations(|| layer.setOpacity(0.0));
                    }
                }
            });
        });
        return;
    }
    tauri::async_runtime::spawn(async move {
        let mut frame = 0_u64;
        loop {
            if !animation.is_current() {
                break;
            }
            let t = frame as f64 * 0.075;
            let pulse = (t.sin() * 0.5 + 0.5).powf(1.1);
            let opacity = (0.52 + pulse * 0.42).clamp(0.0, 0.98) as f32;
            let shadow_opacity = (0.32 + pulse * 0.46).clamp(0.0, 0.92) as f32;
            if !animation
                .frame(&app, move || {
                    UI.with(|ui| {
                        let ui = ui.borrow();
                        if let Some(view) = &ui.listen_indicator {
                            if let Some(layer) = view.layer() {
                                suppress_implicit_animations(|| {
                                    layer.setOpacity(opacity);
                                    layer.setShadowOpacity(shadow_opacity);
                                    layer.setShadowRadius(8.0 + pulse * 3.5);
                                });
                            }
                        }
                    });
                })
                .await
            {
                break;
            }
            frame = frame.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });
}

pub(in crate::macos_native_agent) fn start_processing_dots_animation(
    app: AppHandle,
    mode: CapsuleMode,
    token: u64,
) {
    let animation = MainThreadAnimation::with_token(&DOTS_TOKEN, token);
    if mode != CapsuleMode::Processing {
        let _ = app.run_on_main_thread(move || {
            if !animation.is_current() {
                return;
            }
            UI.with(|ui| {
                let ui = ui.borrow();
                suppress_implicit_animations(|| {
                    for dot in &ui.processing_dots {
                        if let Some(layer) = dot.layer() {
                            layer.setOpacity(0.0);
                            layer.setShadowOpacity(0.0);
                        }
                    }
                });
            });
        });
        return;
    }

    tauri::async_runtime::spawn(async move {
        let mut frame = 0_u64;
        loop {
            if !animation.is_current() {
                break;
            }
            let t = frame as f64 * 0.115;
            if !animation
                .frame(&app, move || {
                    let theme = Theme::current();
                    UI.with(|ui| {
                        let ui = ui.borrow();
                        let Some(panel) = &ui.panel else { return };
                        let panel_w = panel.frame().size.width;
                        let panel_h = panel.frame().size.height;
                        let total_w = PROCESS_DOT_SIZE * 3.0 + PROCESS_DOT_GAP * 2.0;
                        let start_x = (panel_w - total_w) / 2.0;
                        let base_y = (panel_h - PROCESS_DOT_SIZE) / 2.0 - 1.0;
                        suppress_implicit_animations(|| {
                            for (idx, dot) in ui.processing_dots.iter().enumerate() {
                                let phase = t - idx as f64 * 0.48;
                                let wave = (phase.sin() * 0.5 + 0.5).powf(1.25);
                                let rise = wave * 2.6;
                                let x = start_x + idx as f64 * (PROCESS_DOT_SIZE + PROCESS_DOT_GAP);
                                let y = base_y - rise;
                                dot.setFrame(NSRect::new(
                                    NSPoint::new(x, y),
                                    NSSize::new(PROCESS_DOT_SIZE, PROCESS_DOT_SIZE),
                                ));
                                if let Some(layer) = dot.layer() {
                                    layer.setBackgroundColor(Some(&theme.accent().CGColor()));
                                    layer.setShadowColor(Some(&theme.accent().CGColor()));
                                    layer.setOpacity((0.45 + wave * 0.5) as f32);
                                    layer.setShadowRadius(5.0 + wave * 3.0);
                                    layer.setShadowOpacity((0.22 + wave * 0.32) as f32);
                                }
                            }
                        });
                    });
                })
                .await
            {
                break;
            }
            frame = frame.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });
}
