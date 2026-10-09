//! macOS リアルタイム字幕浮窗 — 灵动岛风格
//!
//! Live 録課モジュールが発行する `live-transcript-appended` イベントを監聴し、
//! 最新のトランスクリプト行を画面下部の磨砂ガラスカプセルに表示します。
//! STT / Agent とは完全に独立した機能です。
//! また `stt-partial`（caller="live"）を監聴して発話中のリアルタイムテキストも表示します。

// The `#[cfg(target_os = "macos")]` gate lives on the `mod` declaration in
// `lib.rs`; we don't need to repeat it as an inner attribute here.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use objc2::rc::Retained;
use objc2_app_kit::{NSEvent, NSPanel, NSTextField, NSView, NSVisualEffectView};
use tauri::{AppHandle, Listener};

use crate::main_thread_animation::MainThreadAnimation;
use crate::subtitle_events::{Caption, CaptionMailbox};

#[path = "macos_subtitle_overlay/caption.rs"]
mod caption;
#[path = "macos_subtitle_overlay/panel.rs"]
mod panel;
#[path = "macos_subtitle_overlay/theme.rs"]
mod theme;

use caption::{show_caption, status_changed};
use panel::{build_overlay_panel, install_click_monitor};
use theme::{apply_overlay_theme, effective_is_dark};

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
static LIFECYCLE_TOKEN: AtomicU64 = AtomicU64::new(0);
static OVERLAY_OPEN: AtomicBool = AtomicBool::new(false);
static CAPTION_MAILBOX: std::sync::LazyLock<CaptionMailbox> =
    std::sync::LazyLock::new(CaptionMailbox::default);

fn captions_enabled() -> bool {
    OVERLAY_OPEN.load(Ordering::Relaxed)
}

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
    displayed_session_id: Option<String>,
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
pub fn setup(app: &AppHandle) {
    let mut listeners =
        crate::subtitle_events::subscribe(app, captions_enabled, show_caption, status_changed);

    // Refresh overlay colours when the user changes the app theme from
    // the main window, or when the system appearance flips while the app
    // is in "system" mode. macos_native_agent installs the system-level
    // observer and re-emits `app-theme-changed`, so a single subscription
    // here covers both cases.
    let app_theme = app.clone();
    let lid_theme = app.listen("app-theme-changed", move |_event| {
        let dark = effective_is_dark(&app_theme);
        let app_main = app_theme.clone();
        let _ = app_main.run_on_main_thread(move || apply_overlay_theme(dark));
    });

    listeners.push(lid_theme);
    SHARED.lock().unwrap().event_listeners = listeners;
}

pub fn open_overlay(app: &AppHandle) -> Result<(), String> {
    if OVERLAY_OPEN.load(Ordering::Relaxed) {
        let lifecycle = MainThreadAnimation::with_token(
            &LIFECYCLE_TOKEN,
            LIFECYCLE_TOKEN.load(Ordering::Relaxed),
        );
        let app_refresh = app.clone();
        let _ = app.run_on_main_thread(move || {
            if !lifecycle.is_current() || !OVERLAY_OPEN.load(Ordering::Relaxed) {
                return;
            }
            UI.with(|ui| {
                if let Some(p) = &ui.borrow().panel {
                    p.orderFrontRegardless();
                }
            });
            apply_overlay_theme(effective_is_dark(&app_refresh));
        });
        return Ok(());
    }
    OVERLAY_OPEN.store(true, Ordering::Relaxed);
    let lifecycle = MainThreadAnimation::start(&LIFECYCLE_TOKEN);
    let app2 = app.clone();
    let app_theme = app.clone();
    app.run_on_main_thread(move || {
        if !lifecycle.is_current() || !OVERLAY_OPEN.load(Ordering::Relaxed) {
            return;
        }
        // A rapid close/open may invalidate the queued close and reuse the UI.
        let needs_panel = UI.with(|ui| ui.borrow().panel.is_none());
        if needs_panel {
            build_overlay_panel();
            install_click_monitor(app2);
        } else {
            UI.with(|ui| {
                let mut ui = ui.borrow_mut();
                ui.displayed_session_id = None;
                if let Some(label) = &ui.text_label {
                    label.setStringValue(&objc2_foundation::NSString::from_str(""));
                }
                if let Some(panel) = &ui.panel {
                    panel.setAlphaValue(0.0);
                }
            });
        }
        apply_overlay_theme(effective_is_dark(&app_theme));
    })
    .map_err(|e| format!("subtitle overlay open failed: {e}"))
}

pub fn close_overlay(app: &AppHandle) -> Result<(), String> {
    if !OVERLAY_OPEN.load(Ordering::Relaxed) {
        return Ok(());
    }
    OVERLAY_OPEN.store(false, Ordering::Relaxed);
    let lifecycle = MainThreadAnimation::start(&LIFECYCLE_TOKEN);
    CAPTION_MAILBOX.clear();
    HIDE_TOKEN.fetch_add(1, Ordering::Relaxed);
    FADE_TOKEN.fetch_add(1, Ordering::Relaxed);
    MORPH_TOKEN.fetch_add(1, Ordering::Relaxed);

    app.run_on_main_thread(move || {
        if !lifecycle.is_current() {
            return;
        }
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
            ui.displayed_session_id = None;
        });
    })
    .map_err(|e| format!("subtitle overlay close failed: {e}"))
}

pub fn is_open() -> bool {
    OVERLAY_OPEN.load(Ordering::Relaxed)
}

// ── Panel construction ─────────────────────────────────────────────────────────
