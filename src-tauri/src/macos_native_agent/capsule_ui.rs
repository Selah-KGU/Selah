//! Floating capsule panel, theme application, and border animations.

use super::*;
use crate::main_thread_animation::MainThreadAnimation;

#[path = "capsule_ui/border.rs"]
mod border;
#[path = "capsule_ui/build.rs"]
mod build;
#[path = "capsule_ui/motion.rs"]
mod motion;

use border::set_gradient_colors;
pub(super) use border::update_border;
use build::build_panel;
use motion::{animate_to, fade_to};
pub(super) use motion::{start_listen_pulse_animation, start_processing_dots_animation};

#[derive(Default)]
struct CapsuleViews {
    panel: Option<Retained<NSPanel>>,
    displayed_epoch: Option<u64>,
    root_view: Option<Retained<NSView>>,
    capsule_view: Option<Retained<NSView>>,
    vfx_view: Option<Retained<NSVisualEffectView>>,
    bg_overlay: Option<Retained<NSView>>,
    text_label: Option<Retained<NSTextField>>,
    listen_indicator: Option<Retained<NSView>>,
    processing_dots: Vec<Retained<NSView>>,
    gradient_border: Option<Retained<CAGradientLayer>>,
    gradient_mask: Option<Retained<CAShapeLayer>>,
    screen_center_x: f64,
    screen_top_y: f64,
    event_monitor: Option<Retained<AnyObject>>,
}

thread_local! {
    static UI: RefCell<CapsuleViews> = RefCell::new(CapsuleViews::default());
}

#[derive(Clone, Copy)]
struct Spring {
    pos: f64,
    vel: f64,
    target: f64,
}

impl Spring {
    fn new(pos: f64) -> Self {
        Self {
            pos,
            vel: 0.0,
            target: pos,
        }
    }

    fn set_target(&mut self, target: f64) {
        self.target = target;
    }

    fn tick(&mut self) -> bool {
        let dx = self.pos - self.target;
        let accel = (-SPRING_K * dx - SPRING_D * self.vel) / SPRING_M;
        self.vel += accel * SPRING_DT;
        self.pos += self.vel * SPRING_DT;
        dx.abs() > SPRING_SETTLE || self.vel.abs() > SPRING_SETTLE
    }
}

// ─ Text rendering ────────────────────────────────────────────────────────────
static VIEW_MAILBOX: std::sync::LazyLock<LatestUiMailbox<NativeViewUpdate>> =
    std::sync::LazyLock::new(LatestUiMailbox::default);

pub(super) fn enqueue_view_update(app: &AppHandle, view: NativeViewUpdate) {
    let Some(ticket) = VIEW_MAILBOX.push(view) else {
        return;
    };
    let app_update = app.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        let Some(view) = VIEW_MAILBOX.take(ticket) else {
            return;
        };
        if view.is_current() {
            apply_view_update(&app_update, view);
        }
    }) {
        VIEW_MAILBOX.cancel(ticket);
        log::warn!("native Agent view dispatch failed: {error}");
    }
}

fn apply_view_update(app: &AppHandle, view: NativeViewUpdate) {
    // Reserve effects before AppKit/Markdown work. A concurrent close can
    // invalidate these tokens; this older render must never claim new ones later.
    let effects = {
        let _state = SHARED.lock().unwrap();
        if !view.is_current() {
            return;
        }
        let changed = UI.with(|ui| ui.borrow().displayed_epoch != Some(view.epoch));
        changed.then(|| {
            let claim = |token: &AtomicU64| token.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
            (
                claim(&FADE_TOKEN),
                claim(&MORPH_TOKEN),
                claim(&BORDER_TOKEN),
                claim(&DOTS_TOKEN),
                claim(&LISTEN_PULSE_TOKEN),
            )
        })
    };
    let changed = UI.with(|ui| {
        let mut ui = ui.borrow_mut();
        if ui.panel.is_none() {
            build_panel(&mut ui);
            install_click_monitor(&mut ui, app.clone());
        }
        let changed = ui.displayed_epoch != Some(view.epoch);
        ui.displayed_epoch = Some(view.epoch);
        changed
    });
    let mode = view.mode;
    let text = view.text;
    suppress_implicit_animations(|| {
        UI.with(|ui| {
            let ui = ui.borrow();
            let Some(label) = &ui.text_label else {
                return;
            };

            match mode {
                CapsuleMode::Processing => {
                    label.setStringValue(&NSString::from_str(""));
                    label.setHidden(true);
                }
                CapsuleMode::Result => {
                    label.setHidden(false);
                    label.setAlignment(NSTextAlignment::Left);
                    let theme = Theme::current();
                    let attr = build_markdown_attributed(&text, theme);
                    label.setAttributedStringValue(&attr);
                    label.setMaximumNumberOfLines(0);
                    if let Some(cell) = label.cell() {
                        cell.setUsesSingleLineMode(false);
                        cell.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
                    }
                }
                CapsuleMode::Listening | CapsuleMode::Notice => {
                    label.setHidden(false);
                    label.setStringValue(&NSString::from_str(&text));
                    label.setAlignment(NSTextAlignment::Center);
                    let theme = Theme::current();
                    let font = if mode == CapsuleMode::Listening {
                        NSFont::systemFontOfSize(LISTEN_FONT)
                    } else {
                        NSFont::systemFontOfSize(NOTICE_FONT)
                    };
                    label.setFont(Some(&font));
                    let color = if mode == CapsuleMode::Listening && text == "話してください"
                    {
                        theme.muted_label()
                    } else {
                        theme.label_color()
                    };
                    label.setTextColor(Some(&color));
                    label.setMaximumNumberOfLines(2);
                    if let Some(cell) = label.cell() {
                        cell.setUsesSingleLineMode(false);
                        cell.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
                    }
                }
            }

            layout_label(&ui, mode);
            layout_processing_dots(&ui, mode);
            layout_listen_indicator(&ui, mode);
        });
    });
    if !changed {
        return;
    }
    let (width, height) = match mode {
        CapsuleMode::Listening => (LISTEN_W, LISTEN_H),
        CapsuleMode::Processing => (PROCESS_W, PROCESS_H),
        CapsuleMode::Result => (RESULT_W, compute_result_height(&text)),
        CapsuleMode::Notice => (NOTICE_W, NOTICE_H),
    };
    let Some((fade, morph, border, dots, pulse)) = effects else {
        return;
    };
    let app_fade = app.clone();
    tauri::async_runtime::spawn(async move {
        fade_to(app_fade, 1.0, fade).await;
    });
    animate_to(app.clone(), width, height, morph);
    update_border(app.clone(), mode, border);
    start_processing_dots_animation(app.clone(), mode, dots);
    start_listen_pulse_animation(app.clone(), mode, pulse);
    match mode {
        CapsuleMode::Result => schedule_close(
            app.clone(),
            Duration::from_secs(RESULT_AUTO_CLOSE_SECS),
            view.lease,
        ),
        CapsuleMode::Notice => schedule_close(
            app.clone(),
            Duration::from_millis(NOTICE_AUTO_CLOSE_MS),
            view.lease,
        ),
        _ => {}
    }
}

// ─ Panel / layout ────────────────────────────────────────────────────────────
fn layout_label(ui: &CapsuleViews, mode: CapsuleMode) {
    let Some(panel) = &ui.panel else {
        return;
    };
    let Some(label) = &ui.text_label else {
        return;
    };
    let frame = panel.frame();
    let width = frame.size.width;
    let height = frame.size.height;

    match mode {
        CapsuleMode::Processing => {
            label.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)));
        }
        CapsuleMode::Result => {
            let x = RESULT_PAD_X;
            let y = RESULT_PAD_Y;
            let w = width - RESULT_PAD_X * 2.0;
            let h = height - RESULT_PAD_Y * 2.0;
            label.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(w, h)));
            label.setPreferredMaxLayoutWidth(w);
        }
        CapsuleMode::Listening => {
            // Leave room on the left for the listening indicator (dot + 10px gap)
            let indicator_slot = 22.0;
            let x = PAD_X + indicator_slot;
            let w = width - (PAD_X + indicator_slot) - PAD_X;
            let fit: NSSize = unsafe { msg_send![label, intrinsicContentSize] };
            let target_h = fit.height.max(22.0).min(height - PAD_Y * 2.0);
            let y = (height - target_h) / 2.0 - 1.0;
            label.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(w, target_h)));
            label.setPreferredMaxLayoutWidth(w);
        }
        CapsuleMode::Notice => {
            let fit: NSSize = unsafe { msg_send![label, intrinsicContentSize] };
            let target_h = fit.height.max(20.0).min(height - PAD_Y * 2.0);
            let y = (height - target_h) / 2.0 - 1.0;
            label.setFrame(NSRect::new(
                NSPoint::new(PAD_X, y),
                NSSize::new(width - PAD_X * 2.0, target_h),
            ));
            label.setPreferredMaxLayoutWidth(width - PAD_X * 2.0);
        }
    }
}

fn layout_processing_dots(ui: &CapsuleViews, mode: CapsuleMode) {
    let Some(panel) = &ui.panel else {
        return;
    };
    let width = panel.frame().size.width;
    let height = panel.frame().size.height;
    let total_w = PROCESS_DOT_SIZE * 3.0 + PROCESS_DOT_GAP * 2.0;
    let start_x = (width - total_w) / 2.0;
    let y = (height - PROCESS_DOT_SIZE) / 2.0 - 1.0;

    for (idx, dot) in ui.processing_dots.iter().enumerate() {
        let x = start_x + idx as f64 * (PROCESS_DOT_SIZE + PROCESS_DOT_GAP);
        dot.setFrame(NSRect::new(
            NSPoint::new(x, y),
            NSSize::new(PROCESS_DOT_SIZE, PROCESS_DOT_SIZE),
        ));
        if let Some(layer) = dot.layer() {
            if mode != CapsuleMode::Processing {
                layer.setOpacity(0.0);
                layer.setShadowOpacity(0.0);
            }
        }
    }
}

fn layout_listen_indicator(ui: &CapsuleViews, mode: CapsuleMode) {
    let Some(panel) = &ui.panel else {
        return;
    };
    let Some(view) = &ui.listen_indicator else {
        return;
    };
    let height = panel.frame().size.height;
    let x = PAD_X - 2.0;
    let y = (height - 8.0) / 2.0;
    view.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(8.0, 8.0)));
    if let Some(layer) = view.layer() {
        if mode != CapsuleMode::Listening {
            layer.setOpacity(0.0);
        }
    }
}

pub(super) fn close_panel(app: &AppHandle, immediate: bool) {
    close_panel_for_view(app, immediate, None);
}

pub(super) fn close_panel_if_current(app: &AppHandle, lease: &NativeViewLease) {
    close_panel_for_view(app, false, Some(lease));
}

fn close_panel_for_view(app: &AppHandle, immediate: bool, expected: Option<&NativeViewLease>) {
    let (input_id, token) = {
        let mut sh = SHARED.lock().unwrap();
        let Some(closed) = sh.close_view(expected) else {
            return;
        };
        VIEW_MAILBOX.clear();
        cancel_auto_close();
        BORDER_TOKEN.fetch_add(1, Ordering::Relaxed);
        DOTS_TOKEN.fetch_add(1, Ordering::Relaxed);
        LISTEN_PULSE_TOKEN.fetch_add(1, Ordering::Relaxed);
        MORPH_TOKEN.fetch_add(1, Ordering::Relaxed);
        let token = FADE_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        (closed.input_id, token)
    };
    if let Some(id) = input_id {
        let _ = stt::stt_request_native_input_stop(&id);
    }
    let close = MainThreadAnimation::with_token(&FADE_TOKEN, token);

    if immediate {
        let _ = app.run_on_main_thread(move || {
            if close.is_current() {
                remove_panel();
            }
        });
        return;
    }

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if fade_to(app.clone(), 0.0, token).await {
            close
                .frame(&app, || {
                    remove_panel();
                })
                .await;
        }
    });
}

fn remove_panel() {
    UI.with(|ui| {
        let mut ui = ui.borrow_mut();
        if let Some(monitor) = ui.event_monitor.take() {
            unsafe { NSEvent::removeMonitor(&monitor) };
        }
        if let Some(panel) = ui.panel.take() {
            panel.setAlphaValue(0.0);
            panel.orderOut(None);
            panel.close();
        }
        ui.displayed_epoch = None;
        ui.root_view = None;
        ui.capsule_view = None;
        ui.vfx_view = None;
        ui.bg_overlay = None;
        ui.text_label = None;
        ui.listen_indicator = None;
        ui.processing_dots.clear();
        ui.gradient_border = None;
        ui.gradient_mask = None;
    });
    PANEL_OPEN.store(false, Ordering::Relaxed);
}

// ─ Click monitor to open main window from Result state ───────────────────────
fn install_click_monitor(ui: &mut CapsuleViews, app: AppHandle) {
    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            NSEventMask::LeftMouseUp,
            &RcBlock::new(move |event: NonNull<NSEvent>| {
                let win_num = event.as_ref().windowNumber();
                let should_open = UI.with(|ui| {
                    let ui = ui.borrow();
                    let is_our_panel = ui
                        .panel
                        .as_ref()
                        .map(|p| p.windowNumber() == win_num)
                        .unwrap_or(false);
                    let mode = SHARED.lock().unwrap().mode;
                    is_our_panel && mode == Some(CapsuleMode::Result)
                });

                if should_open {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ =
                            crate::agent_commands::open_agent_popup(app, None, None, None, None)
                                .await;
                    });
                }

                event.as_ptr()
            }),
        )
    };

    ui.event_monitor = monitor;
}

pub(super) fn apply_theme(theme: &Theme) {
    suppress_implicit_animations(|| {
        UI.with(|ui| {
            let ui = ui.borrow();
            let mode = SHARED.lock().unwrap().mode;
            if let Some(bg) = &ui.bg_overlay {
                let (r, g, b, a) = theme.background();
                if let Some(layer) = bg.layer() {
                    layer.setBackgroundColor(Some(&srgb(r, g, b, a).CGColor()));
                }
            }
            if let Some(capsule) = &ui.capsule_view {
                if let Some(layer) = capsule.layer() {
                    let (sr, sg, sb) = if theme.is_dark {
                        (6, 4, 12)
                    } else {
                        (60, 40, 120)
                    };
                    layer.setShadowColor(Some(&srgb(sr, sg, sb, 0.62).CGColor()));
                    layer.setShadowOpacity(if theme.is_dark { 0.30 } else { 0.14 });
                }
            }
            if let Some(vfx) = &ui.vfx_view {
                if let Some(layer) = vfx.layer() {
                    let (r, g, b, a) = match mode {
                        Some(CapsuleMode::Result) => theme.border_result(),
                        Some(CapsuleMode::Notice) => theme.border_notice(),
                        Some(CapsuleMode::Processing) => {
                            // Gradient border takes over in processing; clear the solid one.
                            layer.setBorderColor(Some(&NSColor::clearColor().CGColor()));
                            layer.setBorderWidth(0.0);
                            theme.border_idle()
                        }
                        _ => theme.border_idle(),
                    };
                    if mode != Some(CapsuleMode::Processing) {
                        layer.setBorderColor(Some(&srgb(r, g, b, a).CGColor()));
                        layer.setBorderWidth(BORDER_IDLE_W);
                    }
                }
            }
            if let Some(view) = &ui.listen_indicator {
                if let Some(layer) = view.layer() {
                    layer.setBackgroundColor(Some(&theme.listen_indicator().CGColor()));
                    layer.setShadowColor(Some(&theme.listen_indicator().CGColor()));
                }
            }
            for dot in &ui.processing_dots {
                if let Some(layer) = dot.layer() {
                    layer.setBackgroundColor(Some(&theme.accent().CGColor()));
                    layer.setShadowColor(Some(&theme.accent().CGColor()));
                }
            }
            if let Some(gradient) = &ui.gradient_border {
                set_gradient_colors(gradient, theme);
            }
            // Re-skin the active label so live theme flips reach currently-visible text.
            if let Some(label) = &ui.text_label {
                match mode {
                    Some(CapsuleMode::Result) => {
                        let source = SHARED.lock().unwrap().result_accumulated.clone();
                        let trimmed = source.trim();
                        if !trimmed.is_empty() {
                            let attr = build_markdown_attributed(trimmed, *theme);
                            label.setAttributedStringValue(&attr);
                        }
                    }
                    Some(CapsuleMode::Listening) => {
                        let combined = listening_display_text(&SHARED.lock().unwrap());
                        let color = if combined.trim().is_empty() {
                            theme.muted_label()
                        } else {
                            theme.label_color()
                        };
                        label.setTextColor(Some(&color));
                    }
                    Some(CapsuleMode::Notice) => {
                        label.setTextColor(Some(&theme.label_color()));
                    }
                    _ => {}
                }
            }
        });
    });
}
