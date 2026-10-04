//! macOS リアルタイム字幕浮窗 — 灵动岛风格
//!
//! Live 録課モジュールが発行する `live-session-updated` イベントを監聴し、
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
use serde_json::Value;
use tauri::{AppHandle, Listener};

#[path = "macos_subtitle_overlay/caption.rs"]
mod caption;
#[path = "macos_subtitle_overlay/panel.rs"]
mod panel;
#[path = "macos_subtitle_overlay/theme.rs"]
mod theme;

use caption::{schedule_fade_out, show_text};
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
static OVERLAY_OPEN: AtomicBool = AtomicBool::new(false);
/// Last partial show_text time in millis since epoch — used to coalesce STT
/// partials that fire faster than human reading speed.
static LAST_PARTIAL_MS: AtomicU64 = AtomicU64::new(0);
const PARTIAL_MIN_INTERVAL_MS: u64 = 120;

static LAST_CAPTION_SEQ: AtomicU64 = AtomicU64::new(0);

fn claim_caption_seq(seq: u64) -> bool {
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

fn payload_seq(payload: &Value) -> u64 {
    payload
        .get("seq")
        .and_then(|value| value.as_u64())
        .unwrap_or(0)
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
        if text.trim().is_empty() || !claim_caption_seq(0) {
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
        if text.trim().is_empty() || !claim_caption_seq(payload_seq(&payload)) {
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

    let app_stt_final = app.clone();
    let lid_stt_final = app.listen("stt-final", move |event| {
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
        if text.trim().is_empty() || !claim_caption_seq(payload_seq(&payload)) {
            return;
        }
        show_text(&app_stt_final, text, true);
    });

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

    SHARED.lock().unwrap().event_listeners =
        vec![lid_state, lid_line, lid_partial, lid_stt_final, lid_theme];
}

pub fn open_overlay(app: &AppHandle) -> Result<(), String> {
    if OVERLAY_OPEN.load(Ordering::Relaxed) {
        let app_refresh = app.clone();
        let _ = app.run_on_main_thread(move || {
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
    let app2 = app.clone();
    let app_theme = app.clone();
    app.run_on_main_thread(move || {
        build_overlay_panel();
        apply_overlay_theme(effective_is_dark(&app_theme));
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
