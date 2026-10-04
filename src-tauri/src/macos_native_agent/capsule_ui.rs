//! Floating capsule panel, theme application, and border animations.

use super::*;

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
pub(super) fn update_text(app: &AppHandle, mode: CapsuleMode, text: &str) {
    let text = text.to_string();
    let _ = app.run_on_main_thread(move || {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
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
        let _ = mtm;
    });
}

// ─ Panel / layout ────────────────────────────────────────────────────────────
pub(super) fn ensure_panel(app: &AppHandle, width: f64, height: f64) {
    let app_handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        UI.with(|ui| {
            let mut ui = ui.borrow_mut();
            if ui.panel.is_none() {
                build_panel(&mut ui);
                install_click_monitor(&mut ui, app_handle.clone());
            }
        });
    });

    let token = FADE_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    fade_to(app.clone(), 1.0, token);
    animate_to(app.clone(), width, height);
}

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
    cancel_auto_close();
    BORDER_TOKEN.fetch_add(1, Ordering::Relaxed);
    DOTS_TOKEN.fetch_add(1, Ordering::Relaxed);
    LISTEN_PULSE_TOKEN.fetch_add(1, Ordering::Relaxed);
    MORPH_TOKEN.fetch_add(1, Ordering::Relaxed);

    if immediate {
        let _ = app.run_on_main_thread(remove_panel);
        reset_shared_state();
        return;
    }

    let app = app.clone();
    let token = FADE_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        fade_to(app.clone(), 0.0, token);
        tokio::time::sleep(Duration::from_millis(ANIM_MS * (FADE_FRAMES + 1))).await;
        if FADE_TOKEN.load(Ordering::Relaxed) == token {
            let _ = app.run_on_main_thread(remove_panel);
            reset_shared_state();
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

fn reset_shared_state() {
    let mut sh = SHARED.lock().unwrap();
    sh.mode = None;
    sh.stop_requested = false;
    sh.finals_accumulated.clear();
    sh.current_speech.clear();
    sh.result_accumulated.clear();
    sh.agent_listener = None;
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
}
