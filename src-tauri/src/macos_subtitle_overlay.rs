//! macOS リアルタイム字幕浮窗 — 灵动岛风格
//!
//! Live 録課モジュールが発行する `live-session-updated` イベントを監聴し、
//! 最新のトランスクリプト行を画面下部の磨砂ガラスカプセルに表示します。
//! STT / Agent とは完全に独立した機能です。
//! また `stt-partial`（caller="live"）を監聴して発話中のリアルタイムテキストも表示します。

#![cfg(target_os = "macos")]

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyClass;
use objc2::{msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSEventMask, NSFloatingWindowLevel, NSFont, NSPanel,
    NSScreen, NSTextAlignment, NSTextField, NSView, NSVisualEffectBlendingMode,
    NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use serde_json::Value;
use std::ptr::NonNull;
use tauri::{AppHandle, Emitter, Listener, Manager};

// ── Layout ─────────────────────────────────────────────────────────────────────

const SUB_H: f64 = 52.0;
const SUB_CORNER: f64 = SUB_H / 2.0; // full pill
const SUB_MIN_W: f64 = 180.0;
const SUB_MAX_W: f64 = 620.0;
const SUB_PAD_X: f64 = 26.0;
const SUB_FONT: f64 = 18.0;
/// Natural height of a single-line label at SUB_FONT (ascender + descender + leading).
const SUB_LABEL_H: f64 = 26.0;
/// Y offset to vertically centre SUB_LABEL_H inside the SUB_H capsule.
/// Nudged 2pt below the geometric centre to compensate for font leading.
const SUB_LABEL_Y: f64 = (SUB_H - SUB_LABEL_H) / 2.0 - 2.0;
const SUB_MARGIN_BOTTOM: f64 = 64.0;

// ── Timing ─────────────────────────────────────────────────────────────────────

const SUB_FADE_DELAY_SECS: u64 = 6;

// ── Animation ──────────────────────────────────────────────────────────────────

const ANIM_MS: u64 = 16;

const SPRING_K: f64 = 320.0;
const SPRING_D: f64 = 24.0;
const SPRING_M: f64 = 1.0;
const SPRING_DT: f64 = 0.016;
const SPRING_SETTLE: f64 = 0.25;

const FADE_FRAMES: u64 = 20;

// ── Cancellation tokens ────────────────────────────────────────────────────────

static HIDE_TOKEN: AtomicU64 = AtomicU64::new(0);
static FADE_TOKEN: AtomicU64 = AtomicU64::new(0);
static MORPH_TOKEN: AtomicU64 = AtomicU64::new(0);
static OVERLAY_OPEN: AtomicBool = AtomicBool::new(false);
/// Last partial show_text time in millis since epoch — used to coalesce STT
/// partials that fire faster than human reading speed.
static LAST_PARTIAL_MS: AtomicU64 = AtomicU64::new(0);
const PARTIAL_MIN_INTERVAL_MS: u64 = 120;
static SYSTEM_IS_DARK: AtomicBool = AtomicBool::new(true);

// ── Thread-local UI handles ────────────────────────────────────────────────────

#[derive(Default)]
struct OverlayViews {
    panel: Option<Retained<NSPanel>>,
    root_view: Option<Retained<NSView>>,
    capsule_view: Option<Retained<NSView>>,
    vfx_view: Option<Retained<NSVisualEffectView>>,
    bg_overlay: Option<Retained<NSView>>,
    text_label: Option<Retained<NSTextField>>,
    screen_center_x: f64,
    screen_bottom_y: f64,
    /// Local event monitor for click-to-navigate
    event_monitor: Option<Retained<objc2::runtime::AnyObject>>,
}

thread_local! {
    static UI: RefCell<OverlayViews> = RefCell::new(OverlayViews::default());
}

// ── Cross-thread shared state ──────────────────────────────────────────────────

#[derive(Default)]
struct SharedState {
    event_listeners: Vec<tauri::EventId>,
}

static SHARED: std::sync::LazyLock<Mutex<SharedState>> =
    std::sync::LazyLock::new(|| Mutex::new(SharedState::default()));

// ── Spring ─────────────────────────────────────────────────────────────────────

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
    fn set_target(&mut self, t: f64) {
        self.target = t;
    }
    fn tick(&mut self) -> bool {
        let dx = self.pos - self.target;
        let accel = (-SPRING_K * dx - SPRING_D * self.vel) / SPRING_M;
        self.vel += accel * SPRING_DT;
        self.pos += self.vel * SPRING_DT;
        dx.abs() > SPRING_SETTLE || self.vel.abs() > SPRING_SETTLE
    }
}

// ── Colour helpers ─────────────────────────────────────────────────────────────

fn srgb(r: u8, g: u8, b: u8, a: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        r as f64 / 255.0,
        g as f64 / 255.0,
        b as f64 / 255.0,
        a,
    )
}

fn ease_out_quart(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(4)
}

fn is_dark_mode() -> bool {
    unsafe {
        let cls = AnyClass::get(c"NSAppearance").unwrap();
        let current: *mut objc2::runtime::AnyObject = msg_send![cls, currentDrawingAppearance];
        if current.is_null() {
            return true;
        }
        let name: *const objc2::runtime::AnyObject = msg_send![current, name];
        if name.is_null() {
            return true;
        }
        let cstr: *const std::os::raw::c_char = msg_send![name, UTF8String];
        if cstr.is_null() {
            return true;
        }
        std::ffi::CStr::from_ptr(cstr)
            .to_string_lossy()
            .contains("Dark")
    }
}

// ── Text width estimation ──────────────────────────────────────────────────────

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

pub fn setup(app: &AppHandle) {
    // `live-session-updated` is now only emitted on summary/cancel/finish —
    // we listen purely to drive the fade-out when the session goes inactive.
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
            schedule_fade_out(&app_state, SUB_FADE_DELAY_SECS);
        }
    });

    // Slim per-line delta event — carries only the new transcript line so
    // the overlay no longer reserialises the full (potentially hundreds of
    // KB) session snapshot on every final.
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
        if text.trim().is_empty() {
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
        if text.trim().is_empty() {
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

    SHARED.lock().unwrap().event_listeners = vec![lid_state, lid_line, lid_partial];
}

pub fn open_overlay(app: &AppHandle) -> Result<(), String> {
    if OVERLAY_OPEN.load(Ordering::Relaxed) {
        let _ = app.run_on_main_thread(|| {
            UI.with(|ui| {
                if let Some(p) = &ui.borrow().panel {
                    p.orderFrontRegardless();
                }
            });
        });
        return Ok(());
    }
    OVERLAY_OPEN.store(true, Ordering::Relaxed);
    let app2 = app.clone();
    app.run_on_main_thread(|| {
        build_overlay_panel();
        install_click_monitor(app2);
    })
    .map_err(|e| format!("subtitle overlay open failed: {e}"))
}

pub fn close_overlay(app: &AppHandle) -> Result<(), String> {
    if !OVERLAY_OPEN.load(Ordering::Relaxed) {
        return Ok(());
    }
    OVERLAY_OPEN.store(false, Ordering::Relaxed);
    HIDE_TOKEN.fetch_add(1, Ordering::Relaxed);
    FADE_TOKEN.fetch_add(1, Ordering::Relaxed);
    MORPH_TOKEN.fetch_add(1, Ordering::Relaxed);

    app.run_on_main_thread(|| {
        UI.with(|ui| {
            let mut ui = ui.borrow_mut();
            // Remove event monitor first
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
        });
    })
    .map_err(|e| format!("subtitle overlay close failed: {e}"))
}

pub fn is_open() -> bool {
    OVERLAY_OPEN.load(Ordering::Relaxed)
}

// ── Panel construction ─────────────────────────────────────────────────────────

fn build_overlay_panel() {
    let mtm = MainThreadMarker::new().expect("main thread");
    let dark = is_dark_mode();
    SYSTEM_IS_DARK.store(dark, Ordering::Relaxed);

    let visible = NSScreen::mainScreen(mtm)
        .as_ref()
        .map(|s| s.visibleFrame())
        .unwrap_or_else(|| NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1440.0, 900.0)));

    let screen_center_x = visible.origin.x + visible.size.width / 2.0;
    let screen_bottom_y = visible.origin.y + SUB_MARGIN_BOTTOM;

    let w0 = SUB_MIN_W;
    let x0 = screen_center_x - w0 / 2.0;
    let rect = NSRect::new(NSPoint::new(x0, screen_bottom_y), NSSize::new(w0, SUB_H));

    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        rect,
        NSWindowStyleMask::NonactivatingPanel,
        NSBackingStoreType::Buffered,
        false,
    );
    panel.setFloatingPanel(true);
    panel.setBecomesKeyOnlyIfNeeded(true);
    panel.setWorksWhenModal(true);
    panel.setOpaque(false);
    panel.setHasShadow(false);
    panel.setHidesOnDeactivate(false);
    panel.setLevel(NSFloatingWindowLevel);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Transient,
    );
    unsafe { panel.setReleasedWhenClosed(false) };

    // Root (fully transparent)
    let root = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w0, SUB_H)),
    );
    root.setWantsLayer(true);
    if let Some(l) = root.layer() {
        l.setBackgroundColor(Some(&NSColor::clearColor().CGColor()));
    }

    // Shadow-host capsule
    let capsule = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w0, SUB_H)),
    );
    capsule.setWantsLayer(true);
    if let Some(l) = capsule.layer() {
        l.setCornerRadius(SUB_CORNER);
        l.setBackgroundColor(Some(&NSColor::clearColor().CGColor()));
        if dark {
            l.setShadowColor(Some(&srgb(60, 140, 255, 0.55).CGColor()));
            l.setShadowRadius(32.0);
            l.setShadowOpacity(0.30);
        } else {
            l.setShadowColor(Some(&srgb(0, 0, 0, 0.30).CGColor()));
            l.setShadowRadius(24.0);
            l.setShadowOpacity(0.18);
        }
        l.setShadowOffset(NSSize::new(0.0, -8.0));
    }

    // Vibrancy layer
    let vfx: Retained<NSVisualEffectView> = unsafe {
        msg_send![
            NSVisualEffectView::alloc(mtm),
            initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w0, SUB_H))
        ]
    };
    vfx.setMaterial(NSVisualEffectMaterial::HUDWindow);
    vfx.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    vfx.setState(NSVisualEffectState::Active);
    vfx.setWantsLayer(true);
    if let Some(l) = vfx.layer() {
        l.setCornerRadius(SUB_CORNER);
        l.setMasksToBounds(true);
        if dark {
            l.setBorderColor(Some(&srgb(120, 180, 255, 0.18).CGColor()));
        } else {
            l.setBorderColor(Some(&srgb(80, 130, 220, 0.14).CGColor()));
        }
        l.setBorderWidth(0.75);
    }

    // Near-opaque overlay (Dynamic Island darkness)
    let bg = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w0, SUB_H)),
    );
    bg.setWantsLayer(true);
    if let Some(l) = bg.layer() {
        if dark {
            l.setBackgroundColor(Some(&srgb(10, 10, 13, 0.94).CGColor()));
        } else {
            l.setBackgroundColor(Some(&srgb(245, 245, 250, 0.91).CGColor()));
        }
        l.setCornerRadius(SUB_CORNER);
    }
    vfx.addSubview(&bg);

    // Text label — sized to one line height, Y-offset centres it inside the capsule
    let label_w = (w0 - SUB_PAD_X * 2.0).max(8.0);
    let label = NSTextField::labelWithString(&NSString::from_str(""), mtm);
    label.setTranslatesAutoresizingMaskIntoConstraints(true);
    label.setFrame(NSRect::new(
        NSPoint::new(SUB_PAD_X, SUB_LABEL_Y),
        NSSize::new(label_w, SUB_LABEL_H),
    ));
    label.setWantsLayer(true);
    if dark {
        label.setTextColor(Some(&NSColor::whiteColor()));
    } else {
        label.setTextColor(Some(&NSColor::labelColor()));
    }
    label.setFont(Some(&NSFont::boldSystemFontOfSize(SUB_FONT)));
    label.setAlignment(NSTextAlignment::Center);
    label.setMaximumNumberOfLines(1);
    label.setPreferredMaxLayoutWidth(label_w);
    if let Some(cell) = label.cell() {
        use objc2_app_kit::NSLineBreakMode;
        cell.setUsesSingleLineMode(true);
        cell.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    }
    vfx.addSubview(&label);

    capsule.addSubview(&vfx);
    root.addSubview(&capsule);
    panel.setContentView(Some(&root));

    panel.setAlphaValue(0.0);
    panel.orderFrontRegardless();

    UI.with(|ui| {
        let mut ui = ui.borrow_mut();
        ui.panel = Some(panel);
        ui.root_view = Some(root);
        ui.capsule_view = Some(capsule);
        ui.vfx_view = Some(vfx);
        ui.bg_overlay = Some(bg);
        ui.text_label = Some(label);
        ui.screen_center_x = screen_center_x;
        ui.screen_bottom_y = screen_bottom_y;
        ui.event_monitor = None;
    });
}

// ── Click-to-navigate monitor ──────────────────────────────────────────────────

/// Install a local mouse-up monitor.  When the user clicks anywhere on the
/// subtitle panel we bring the main window to the front and emit
/// `tray-open-tab` with `"live"` so the frontend navigates to the Live page.
fn install_click_monitor(app: AppHandle) {
    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            NSEventMask::LeftMouseUp,
            &RcBlock::new(move |event: NonNull<NSEvent>| {
                let win_num = event.as_ref().windowNumber();

                let is_our_panel = UI.with(|ui| {
                    let ui = ui.borrow();
                    ui.panel
                        .as_ref()
                        .map(|p| p.windowNumber() == win_num)
                        .unwrap_or(false)
                });

                if is_our_panel {
                    // Bring main window to front and navigate to Live page
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.unminimize();
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                    let _ = app.emit("tray-open-tab", "live");
                }

                event.as_ptr()
            }),
        )
    };

    UI.with(|ui| {
        ui.borrow_mut().event_monitor = monitor;
    });
}

// ── Frame application (main thread) ───────────────────────────────────────────

fn apply_frame(w: f64, cx: f64, bottom_y: f64) {
    UI.with(|ui| {
        let ui = ui.borrow();
        let x = cx - w / 2.0;
        if let Some(panel) = &ui.panel {
            panel.setFrame_display(
                NSRect::new(NSPoint::new(x, bottom_y), NSSize::new(w, SUB_H)),
                true,
            );
        }
        let sz = NSSize::new(w, SUB_H);
        let origin = NSPoint::new(0.0, 0.0);
        let r = SUB_CORNER.min(w / 2.0);
        let label_w = (w - SUB_PAD_X * 2.0).max(8.0);

        if let Some(v) = &ui.root_view {
            v.setFrame(NSRect::new(origin, sz));
        }
        if let Some(v) = &ui.capsule_view {
            v.setFrame(NSRect::new(origin, sz));
            if let Some(l) = v.layer() {
                l.setCornerRadius(r);
            }
        }
        if let Some(v) = &ui.vfx_view {
            v.setFrame(NSRect::new(origin, sz));
            if let Some(l) = v.layer() {
                l.setCornerRadius(r);
            }
        }
        if let Some(v) = &ui.bg_overlay {
            v.setFrame(NSRect::new(origin, sz));
            if let Some(l) = v.layer() {
                l.setCornerRadius(r);
            }
        }
        if let Some(lbl) = &ui.text_label {
            lbl.setFrame(NSRect::new(
                NSPoint::new(SUB_PAD_X, SUB_LABEL_Y),
                NSSize::new(label_w, SUB_LABEL_H),
            ));
        }
    });
}

// ── Spring morph ───────────────────────────────────────────────────────────────

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

fn show_text(app: &AppHandle, text: String, is_final: bool) {
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
                    .map(|p| p.alphaValue() as f64)
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

fn schedule_fade_out(app: &AppHandle, delay_secs: u64) {
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
