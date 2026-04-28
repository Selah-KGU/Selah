#![cfg(target_os = "macos")]

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{msg_send, AnyThread, MainThreadMarker, MainThreadOnly};
use objc2_core_foundation::CFRetained;
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSEventMask, NSFloatingWindowLevel, NSFont,
    NSFontAttributeName, NSForegroundColorAttributeName, NSLineBreakMode, NSMutableParagraphStyle,
    NSPanel, NSParagraphStyleAttributeName, NSScreen, NSTextAlignment, NSTextField, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_graphics::CGPath;
use objc2_foundation::{
    NSArray, NSAttributedString, NSMutableAttributedString, NSNumber, NSPoint, NSRange, NSRect,
    NSSize, NSString,
};
use objc2_quartz_core::{kCAGradientLayerConic, CAGradientLayer, CAShapeLayer, CATransaction};
use serde_json::Value;
use std::ptr::NonNull;
use tauri::{AppHandle, Emitter, Listener, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::agent;
use crate::commands::NativeAgentConfig;
use crate::db::Database;
use crate::stt;

// Direct query of the Fn (kVK_Function) key state. We can't rely on
// `NSEventModifierFlags::Function` alone because macOS sets that bit for
// *any* function-class key — arrow keys, F1–F12, Home/End/Page Up/Down all
// flip it on. Polling modifier flags would mis-detect a held arrow key as
// the Fn shortcut. CGEventSourceKeyState reads the live HID state for a
// specific keycode, so it returns true only when the actual Fn key is held.
const KVK_FUNCTION: u16 = 0x3F; // kVK_Function (Fn / Globe key)
const KCG_EVENT_SOURCE_STATE_COMBINED: i32 = 0; // kCGEventSourceStateCombinedSessionState

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceKeyState(state_id: i32, key: u16) -> bool;
}

fn is_fn_key_down() -> bool {
    // Safety: CGEventSourceKeyState is documented thread-safe and takes a
    // primitive state id + keycode with no out-pointer or allocation.
    unsafe { CGEventSourceKeyState(KCG_EVENT_SOURCE_STATE_COMBINED, KVK_FUNCTION) }
}

const DEFAULT_SHORTCUT: &str = "fn";

// ─ Capsule dimensions ────────────────────────────────────────────────────────
const LISTEN_W: f64 = 540.0;
const LISTEN_H: f64 = 76.0;
const PROCESS_W: f64 = 124.0;
const PROCESS_H: f64 = 52.0;
const RESULT_W: f64 = 720.0;
const RESULT_MIN_H: f64 = 124.0;
const RESULT_MAX_H: f64 = 440.0;
const NOTICE_W: f64 = 460.0;
const NOTICE_H: f64 = 60.0;

const CORNER_RADIUS: f64 = 22.0;
const TOP_MARGIN: f64 = 16.0;
const PAD_X: f64 = 28.0;
const PAD_Y: f64 = 18.0;
const RESULT_PAD_X: f64 = 28.0;
const RESULT_PAD_Y: f64 = 22.0;

// Loading dots
const PROCESS_DOT_SIZE: f64 = 7.0;
const PROCESS_DOT_GAP: f64 = 10.0;

// Typography
const LISTEN_FONT: f64 = 18.5;
const RESULT_BODY_FONT: f64 = 15.0;
const RESULT_CODE_FONT: f64 = 13.5;
const RESULT_H1_FONT: f64 = 20.5;
const RESULT_H2_FONT: f64 = 17.5;
const RESULT_H3_FONT: f64 = 16.0;
const RESULT_LINE_HEIGHT_MUL: f64 = 1.52;
const RESULT_PARAGRAPH_SPACING: f64 = 8.0;
const RESULT_MAX_VISIBLE_LINES: usize = 11;
const NOTICE_FONT: f64 = 14.5;

// Animation
const ANIM_MS: u64 = 16;
const FADE_FRAMES: u64 = 14;
// Critical damping: D ≈ 2·sqrt(K·M) ⇒ no overshoot, no wobble.
const SPRING_K: f64 = 260.0;
const SPRING_D: f64 = 33.0;
const SPRING_M: f64 = 1.0;
const SPRING_DT: f64 = 0.016;
const SPRING_SETTLE: f64 = 0.25;
const RESULT_AUTO_CLOSE_SECS: u64 = 14;
const NOTICE_AUTO_CLOSE_MS: u64 = 1800;
const FN_POLL_IDLE_MS: u64 = 100;
// Held interval must stay well below SHORTCUT_HOLD_MS so a release polled
// at the same instant the hold timer fires can race-update SHORTCUT_DOWN
// before the timer reads it. 25ms preserves the original safety margin.
const FN_POLL_HELD_MS: u64 = 25;
const SHORTCUT_HOLD_MS: u64 = 140;

// Border
const BORDER_IDLE_W: f64 = 1.0;
const BORDER_GRADIENT_W: f64 = 1.7;
const GRADIENT_ROTATION_PERIOD_SEC: f64 = 2.6;

// ─ State tokens ──────────────────────────────────────────────────────────────
static PANEL_OPEN: AtomicBool = AtomicBool::new(false);
static SYSTEM_IS_DARK: AtomicBool = AtomicBool::new(true);
static MORPH_TOKEN: AtomicU64 = AtomicU64::new(0);
static FADE_TOKEN: AtomicU64 = AtomicU64::new(0);
static BORDER_TOKEN: AtomicU64 = AtomicU64::new(0);
static AUTO_CLOSE_TOKEN: AtomicU64 = AtomicU64::new(0);
static FN_PRESSED: AtomicBool = AtomicBool::new(false);
static FN_POLL_TOKEN: AtomicU64 = AtomicU64::new(0);
static SHORTCUT_DOWN: AtomicBool = AtomicBool::new(false);
static SHORTCUT_ARM_TOKEN: AtomicU64 = AtomicU64::new(0);
static DOTS_TOKEN: AtomicU64 = AtomicU64::new(0);
static LISTEN_PULSE_TOKEN: AtomicU64 = AtomicU64::new(0);
static SHORTCUT_REGISTERED: std::sync::LazyLock<Mutex<Option<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

#[derive(Clone, Copy, PartialEq, Eq)]
enum CapsuleMode {
    Listening,
    Processing,
    Result,
    Notice,
}

#[derive(Default)]
struct SharedState {
    mode: Option<CapsuleMode>,
    stop_requested: bool,
    /// All VAD-finalized segments so far, joined with separators.
    finals_accumulated: String,
    /// The current in-flight partial (the latest segment not yet finalized).
    current_speech: String,
    agent_listener: Option<tauri::EventId>,
    result_accumulated: String,
}

fn append_final_segment(sh: &mut SharedState, text: &str) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    if !sh.finals_accumulated.is_empty() {
        let needs_space = !ends_with_cjk(&sh.finals_accumulated) && !starts_with_cjk(trimmed);
        if needs_space {
            sh.finals_accumulated.push(' ');
        }
    }
    sh.finals_accumulated.push_str(trimmed);
    sh.current_speech.clear();
}

fn listening_display_text(sh: &SharedState) -> String {
    let mut out = sh.finals_accumulated.clone();
    let partial = sh.current_speech.trim();
    if !partial.is_empty() {
        if !out.is_empty() {
            let needs_space = !ends_with_cjk(&out) && !starts_with_cjk(partial);
            if needs_space {
                out.push(' ');
            }
        }
        out.push_str(partial);
    }
    out
}

fn consume_all_speech(sh: &mut SharedState) -> String {
    let partial = sh.current_speech.trim().to_string();
    let mut out = std::mem::take(&mut sh.finals_accumulated);
    sh.current_speech.clear();
    if !partial.is_empty() {
        if !out.is_empty() {
            let needs_space = !ends_with_cjk(&out) && !starts_with_cjk(&partial);
            if needs_space {
                out.push(' ');
            }
        }
        out.push_str(&partial);
    }
    out.trim().to_string()
}

fn is_cjk_char(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x309F   // Hiragana
        | 0x30A0..=0x30FF // Katakana
        | 0x3400..=0x4DBF | 0x4E00..=0x9FFF // CJK Unified
        | 0xF900..=0xFAFF // CJK Compat
        | 0xFF00..=0xFFEF // Halfwidth/Fullwidth
    )
}

fn ends_with_cjk(s: &str) -> bool {
    s.chars().next_back().map(is_cjk_char).unwrap_or(false)
}

fn starts_with_cjk(s: &str) -> bool {
    s.chars().next().map(is_cjk_char).unwrap_or(false)
}

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

static SHARED: std::sync::LazyLock<Mutex<SharedState>> =
    std::sync::LazyLock::new(|| Mutex::new(SharedState::default()));

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

// ─ Theme ─────────────────────────────────────────────────────────────────────
#[derive(Clone, Copy)]
struct Theme {
    is_dark: bool,
}

impl Theme {
    fn current() -> Self {
        Self {
            is_dark: SYSTEM_IS_DARK.load(Ordering::Relaxed),
        }
    }

    fn background(&self) -> (u8, u8, u8, f64) {
        if self.is_dark {
            (22, 20, 28, 0.988)
        } else {
            (250, 248, 253, 0.988)
        }
    }

    fn label_color(&self) -> Retained<NSColor> {
        if self.is_dark {
            srgb(246, 246, 250, 0.985)
        } else {
            srgb(30, 24, 54, 0.985)
        }
    }

    fn muted_label(&self) -> Retained<NSColor> {
        if self.is_dark {
            srgb(184, 184, 204, 0.72)
        } else {
            srgb(94, 90, 122, 0.70)
        }
    }

    fn border_idle(&self) -> (u8, u8, u8, f64) {
        if self.is_dark {
            (167, 132, 232, 0.28)
        } else {
            (161, 98, 222, 0.26)
        }
    }

    fn border_result(&self) -> (u8, u8, u8, f64) {
        if self.is_dark {
            (146, 120, 210, 0.22)
        } else {
            (144, 94, 210, 0.20)
        }
    }

    fn border_notice(&self) -> (u8, u8, u8, f64) {
        if self.is_dark {
            (232, 168, 108, 0.34)
        } else {
            (214, 132, 60, 0.30)
        }
    }

    fn accent(&self) -> Retained<NSColor> {
        if self.is_dark {
            srgb(200, 154, 248, 0.96)
        } else {
            srgb(150, 84, 220, 0.96)
        }
    }

    fn code_bg(&self) -> Retained<NSColor> {
        if self.is_dark {
            srgb(255, 255, 255, 0.06)
        } else {
            srgb(0, 0, 0, 0.05)
        }
    }

    fn code_fg(&self) -> Retained<NSColor> {
        if self.is_dark {
            srgb(255, 176, 214, 0.95)
        } else {
            srgb(190, 60, 130, 0.96)
        }
    }

    // Four-stop conic gradient for the loading border.
    fn gradient_stops(&self) -> [Retained<NSColor>; 5] {
        if self.is_dark {
            [
                srgb(186, 118, 250, 0.92),
                srgb(122, 168, 248, 0.92),
                srgb(238, 138, 196, 0.92),
                srgb(138, 206, 238, 0.92),
                srgb(186, 118, 250, 0.92),
            ]
        } else {
            [
                srgb(158, 88, 224, 0.98),
                srgb(96, 138, 232, 0.98),
                srgb(228, 110, 172, 0.98),
                srgb(110, 188, 232, 0.98),
                srgb(158, 88, 224, 0.98),
            ]
        }
    }

    fn listen_indicator(&self) -> Retained<NSColor> {
        if self.is_dark {
            srgb(255, 120, 158, 0.92)
        } else {
            srgb(228, 78, 132, 0.94)
        }
    }
}

// ─ Public setup ──────────────────────────────────────────────────────────────
pub fn setup(app: &AppHandle) {
    SYSTEM_IS_DARK.store(effective_is_dark(app), Ordering::Relaxed);

    let app_theme = app.clone();
    let _ = app.run_on_main_thread(move || register_appearance_observer(app_theme));

    let app_theme_event = app.clone();
    app.listen("app-theme-changed", move |_event| {
        let dark = effective_is_dark(&app_theme_event);
        SYSTEM_IS_DARK.store(dark, Ordering::Relaxed);
        let app_main = app_theme_event.clone();
        let _ = app_main.run_on_main_thread(move || apply_theme(&Theme::current()));
    });

    let app_final = app.clone();
    app.listen("stt-final", move |event| {
        let payload = serde_json::from_str::<Value>(event.payload()).unwrap_or_default();
        if payload.get("caller").and_then(|c| c.as_str()) != Some("agent") {
            return;
        }
        let text = payload
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string();

        let (display, pending_submit) = {
            let mut sh = SHARED.lock().unwrap();
            append_final_segment(&mut sh, &text);
            let display = listening_display_text(&sh);
            let pending = if sh.stop_requested {
                sh.stop_requested = false;
                Some(consume_all_speech(&mut sh))
            } else {
                None
            };
            (display, pending)
        };

        if should_render_listening_text() && !display.is_empty() {
            update_text(&app_final, CapsuleMode::Listening, &display);
        }

        if let Some(text) = pending_submit {
            if !text.is_empty() {
                submit_to_agent(app_final.clone(), text);
            }
        }
    });

    let app_partial = app.clone();
    app.listen("stt-partial", move |event| {
        let payload = serde_json::from_str::<Value>(event.payload()).unwrap_or_default();
        if payload.get("caller").and_then(|c| c.as_str()) != Some("agent") {
            return;
        }
        let text = payload
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string();
        if text.trim().is_empty() {
            return;
        }
        let display = {
            let mut sh = SHARED.lock().unwrap();
            sh.current_speech = text;
            listening_display_text(&sh)
        };
        if should_render_listening_text() {
            update_text(&app_partial, CapsuleMode::Listening, &display);
        }
    });

    let app_state = app.clone();
    app.listen("stt-state", move |event| {
        let payload = serde_json::from_str::<Value>(event.payload()).unwrap_or_default();
        if payload.get("caller").and_then(|c| c.as_str()) != Some("agent") {
            return;
        }
        let state_name = payload
            .get("state")
            .and_then(|t| t.as_str())
            .unwrap_or_default();
        let listening = matches!(state_name, "initializing" | "listening");

        if listening {
            transition_to_listening(&app_state, None);
            return;
        }

        let pending = {
            let mut sh = SHARED.lock().unwrap();
            if sh.mode == Some(CapsuleMode::Processing) || sh.mode == Some(CapsuleMode::Result) {
                sh.finals_accumulated.clear();
                sh.current_speech.clear();
                None
            } else {
                sh.stop_requested = false;
                Some(consume_all_speech(&mut sh))
            }
        };

        if let Some(text) = pending {
            if text.is_empty() {
                close_panel(&app_state, false);
            } else {
                submit_to_agent(app_state.clone(), text);
            }
        }
    });

    let app_err = app.clone();
    app.listen("stt-error", move |_event| {
        {
            let mut sh = SHARED.lock().unwrap();
            sh.stop_requested = false;
            sh.finals_accumulated.clear();
            sh.current_speech.clear();
        }
        transition_to_notice(&app_err, "音声入力を開始できませんでした");
    });
}

fn should_render_listening_text() -> bool {
    let sh = SHARED.lock().unwrap();
    sh.mode == Some(CapsuleMode::Listening) && !sh.stop_requested
}

pub fn apply_config(app: &AppHandle, config: &NativeAgentConfig) -> Result<(), String> {
    let shortcut = normalize_shortcut(&config.voice_shortcut);
    let manager = app.global_shortcut();
    let mut registered = SHORTCUT_REGISTERED.lock().unwrap();
    stop_fn_polling();

    if let Some(prev) = registered.clone() {
        if prev != shortcut && manager.is_registered(prev.as_str()) {
            manager
                .unregister(prev.as_str())
                .map_err(|e| format!("failed to unregister voice shortcut: {e}"))?;
        }
    }

    if config.voice_shortcut_enabled {
        if shortcut == "fn" {
            start_fn_polling(app.clone());
        } else if registered.as_deref() != Some(shortcut.as_str())
            || !manager.is_registered(shortcut.as_str())
        {
            let shortcut_for_handler = shortcut.clone();
            manager
                .on_shortcut(shortcut.as_str(), move |app, _shortcut, event| {
                    handle_shortcut_event(app.clone(), shortcut_for_handler.clone(), event.state);
                })
                .map_err(|e| format!("failed to register voice shortcut: {e}"))?;
        }
        *registered = Some(shortcut);
    } else {
        if let Some(prev) = registered.take() {
            if manager.is_registered(prev.as_str()) {
                manager
                    .unregister(prev.as_str())
                    .map_err(|e| format!("failed to unregister voice shortcut: {e}"))?;
            }
        }
        clear_agent_listener(app);
        SHORTCUT_DOWN.store(false, Ordering::Relaxed);
        SHORTCUT_ARM_TOKEN.fetch_add(1, Ordering::Relaxed);
        FN_PRESSED.store(false, Ordering::Relaxed);
        if stt::stt_get_active_caller().as_deref() == Some("agent") {
            let _ = stt::stt_stop_stream();
        }
        close_panel(app, true);
    }

    Ok(())
}

fn handle_shortcut_event(app: AppHandle, _shortcut: String, state: ShortcutState) {
    match state {
        ShortcutState::Pressed => handle_shortcut_pressed(app),
        ShortcutState::Released => handle_shortcut_released(),
    }
}

fn handle_fn_state(app: AppHandle, has_fn: bool) {
    let was_pressed = FN_PRESSED.swap(has_fn, Ordering::Relaxed);
    if has_fn && !was_pressed {
        handle_shortcut_pressed(app);
    } else if !has_fn && was_pressed {
        handle_shortcut_released();
    }
}

fn start_fn_polling(app: AppHandle) {
    let token = FN_POLL_TOKEN
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        loop {
            if FN_POLL_TOKEN.load(Ordering::Relaxed) != token {
                break;
            }

            // CGEventSourceKeyState is thread-safe and answers the question
            // we actually care about ("is the Fn key down right now?")
            // without the Function-flag false-positives caused by arrow /
            // F-keys. No main-thread round-trip needed.
            handle_fn_state(app.clone(), is_fn_key_down());

            let next_ms = if FN_PRESSED.load(Ordering::Relaxed) {
                FN_POLL_HELD_MS
            } else {
                FN_POLL_IDLE_MS
            };
            tokio::time::sleep(Duration::from_millis(next_ms)).await;
        }
    });
}

fn stop_fn_polling() {
    FN_POLL_TOKEN.fetch_add(1, Ordering::Relaxed);
}

fn handle_shortcut_pressed(app: AppHandle) {
    SHORTCUT_DOWN.store(true, Ordering::Relaxed);
    let token = SHORTCUT_ARM_TOKEN
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(SHORTCUT_HOLD_MS)).await;
        if SHORTCUT_ARM_TOKEN.load(Ordering::Relaxed) != token
            || !SHORTCUT_DOWN.load(Ordering::Relaxed)
        {
            return;
        }
        start_agent_capture(app);
    });
}

fn handle_shortcut_released() {
    SHORTCUT_DOWN.store(false, Ordering::Relaxed);
    SHORTCUT_ARM_TOKEN.fetch_add(1, Ordering::Relaxed);
    if stt::stt_get_active_caller().as_deref() != Some("agent") {
        return;
    }
    SHARED.lock().unwrap().stop_requested = true;
    let _ = stt::stt_stop_stream();
}

fn start_agent_capture(app: AppHandle) {
    cancel_auto_close();
    clear_agent_listener(&app);

    if stt::stt_is_running() {
        match stt::stt_get_active_caller().as_deref() {
            Some("agent") => return,
            Some(_) => {
                transition_to_notice(&app, "ほかの音声入力が動作中です");
                return;
            }
            None => {}
        }
    }

    {
        let mut sh = SHARED.lock().unwrap();
        sh.stop_requested = false;
        sh.finals_accumulated.clear();
        sh.current_speech.clear();
        sh.result_accumulated.clear();
        sh.mode = Some(CapsuleMode::Listening);
    }

    transition_to_listening(&app, Some("話してください"));

    if let Err(err) = stt::stt_start_stream(app.clone(), "agent".to_string(), Some(false)) {
        transition_to_notice(&app, &err);
    }
}

// ─ Mode transitions ──────────────────────────────────────────────────────────
fn transition_to_listening(app: &AppHandle, text: Option<&str>) {
    cancel_auto_close();
    SHARED.lock().unwrap().mode = Some(CapsuleMode::Listening);
    ensure_panel(app, LISTEN_W, LISTEN_H);
    let display_text = if let Some(text) = text {
        text.to_string()
    } else {
        let sh = SHARED.lock().unwrap();
        let combined = listening_display_text(&sh);
        if combined.trim().is_empty() {
            "話してください".to_string()
        } else {
            combined
        }
    };
    update_text(app, CapsuleMode::Listening, &display_text);
    update_border(app.clone(), CapsuleMode::Listening);
    start_processing_dots_animation(app.clone(), CapsuleMode::Listening);
    start_listen_pulse_animation(app.clone(), CapsuleMode::Listening);
}

fn transition_to_processing(app: &AppHandle) {
    cancel_auto_close();
    {
        let mut sh = SHARED.lock().unwrap();
        sh.mode = Some(CapsuleMode::Processing);
        sh.current_speech.clear();
        sh.finals_accumulated.clear();
    }
    ensure_panel(app, PROCESS_W, PROCESS_H);
    update_text(app, CapsuleMode::Processing, "");
    update_border(app.clone(), CapsuleMode::Processing);
    start_processing_dots_animation(app.clone(), CapsuleMode::Processing);
    start_listen_pulse_animation(app.clone(), CapsuleMode::Processing);
}

fn transition_to_result(app: &AppHandle, text: &str) {
    cancel_auto_close();
    SHARED.lock().unwrap().mode = Some(CapsuleMode::Result);
    let target_h = compute_result_height(text);
    ensure_panel(app, RESULT_W, target_h);
    update_text(app, CapsuleMode::Result, text);
    update_border(app.clone(), CapsuleMode::Result);
    start_processing_dots_animation(app.clone(), CapsuleMode::Result);
    start_listen_pulse_animation(app.clone(), CapsuleMode::Result);
    schedule_close(app.clone(), Duration::from_secs(RESULT_AUTO_CLOSE_SECS));
}

fn transition_to_notice(app: &AppHandle, message: &str) {
    cancel_auto_close();
    SHARED.lock().unwrap().mode = Some(CapsuleMode::Notice);
    ensure_panel(app, NOTICE_W, NOTICE_H);
    update_text(app, CapsuleMode::Notice, message);
    update_border(app.clone(), CapsuleMode::Notice);
    start_processing_dots_animation(app.clone(), CapsuleMode::Notice);
    start_listen_pulse_animation(app.clone(), CapsuleMode::Notice);
    schedule_close(app.clone(), Duration::from_millis(NOTICE_AUTO_CLOSE_MS));
}

// ─ Text rendering ────────────────────────────────────────────────────────────
fn update_text(app: &AppHandle, mode: CapsuleMode, text: &str) {
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
                    let color = if mode == CapsuleMode::Listening && text == "話してください" {
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
fn ensure_panel(app: &AppHandle, width: f64, height: f64) {
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

fn build_panel(ui: &mut CapsuleViews) {
    let mtm = MainThreadMarker::new().expect("main thread");
    let theme = Theme::current();

    let visible = NSScreen::mainScreen(mtm)
        .as_ref()
        .map(|s| s.visibleFrame())
        .unwrap_or_else(|| NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1440.0, 900.0)));

    let center_x = visible.origin.x + visible.size.width / 2.0;
    let top_y = visible.origin.y + visible.size.height - TOP_MARGIN;
    let frame = NSRect::new(
        NSPoint::new(center_x - LISTEN_W / 2.0, top_y - LISTEN_H),
        NSSize::new(LISTEN_W, LISTEN_H),
    );

    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        frame,
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

    let root = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(LISTEN_W, LISTEN_H)),
    );
    root.setWantsLayer(true);
    if let Some(layer) = root.layer() {
        layer.setBackgroundColor(Some(&NSColor::clearColor().CGColor()));
    }

    let capsule = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(LISTEN_W, LISTEN_H)),
    );
    capsule.setWantsLayer(true);
    if let Some(layer) = capsule.layer() {
        layer.setCornerRadius(CORNER_RADIUS);
        layer.setBackgroundColor(Some(&NSColor::clearColor().CGColor()));
        // Subtle drop shadow for depth.
        let (sr, sg, sb) = if theme.is_dark { (6, 4, 12) } else { (60, 40, 120) };
        layer.setShadowColor(Some(&srgb(sr, sg, sb, 0.62).CGColor()));
        layer.setShadowOffset(NSSize::new(0.0, -6.0));
        layer.setShadowRadius(22.0);
        layer.setShadowOpacity(if theme.is_dark { 0.30 } else { 0.14 });
    }

    let vfx: Retained<NSVisualEffectView> = unsafe {
        msg_send![
            NSVisualEffectView::alloc(mtm),
            initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(LISTEN_W, LISTEN_H))
        ]
    };
    vfx.setMaterial(NSVisualEffectMaterial::HUDWindow);
    vfx.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    vfx.setState(NSVisualEffectState::Active);
    vfx.setWantsLayer(true);
    if let Some(layer) = vfx.layer() {
        layer.setCornerRadius(CORNER_RADIUS);
        layer.setMasksToBounds(true);
        let (r, g, b, a) = theme.border_idle();
        layer.setBorderColor(Some(&srgb(r, g, b, a).CGColor()));
        layer.setBorderWidth(BORDER_IDLE_W);
    }

    let bg = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(LISTEN_W, LISTEN_H)),
    );
    bg.setWantsLayer(true);
    if let Some(layer) = bg.layer() {
        let (r, g, b, a) = theme.background();
        layer.setBackgroundColor(Some(&srgb(r, g, b, a).CGColor()));
        layer.setCornerRadius(CORNER_RADIUS);
    }
    vfx.addSubview(&bg);

    // Pulsing left-side listen indicator (small orb that breathes during listening).
    let listen_indicator = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(8.0, 8.0)),
    );
    listen_indicator.setWantsLayer(true);
    if let Some(layer) = listen_indicator.layer() {
        layer.setBackgroundColor(Some(&theme.listen_indicator().CGColor()));
        layer.setCornerRadius(4.0);
        layer.setShadowColor(Some(&theme.listen_indicator().CGColor()));
        layer.setShadowRadius(9.0);
        layer.setShadowOpacity(0.6);
        layer.setShadowOffset(NSSize::new(0.0, 0.0));
        layer.setOpacity(0.0);
    }
    vfx.addSubview(&listen_indicator);

    // Main text label
    let label = NSTextField::labelWithString(&NSString::from_str("話してください"), mtm);
    label.setTranslatesAutoresizingMaskIntoConstraints(true);
    label.setFrame(NSRect::new(
        NSPoint::new(PAD_X, PAD_Y),
        NSSize::new(LISTEN_W - PAD_X * 2.0, LISTEN_H - PAD_Y * 2.0),
    ));
    label.setWantsLayer(true);
    label.setTextColor(Some(&theme.muted_label()));
    label.setFont(Some(&NSFont::systemFontOfSize(LISTEN_FONT)));
    label.setAlignment(NSTextAlignment::Center);
    label.setMaximumNumberOfLines(2);
    label.setPreferredMaxLayoutWidth(LISTEN_W - PAD_X * 2.0);
    if let Some(cell) = label.cell() {
        cell.setUsesSingleLineMode(false);
        cell.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    }
    vfx.addSubview(&label);

    // Processing dots (stay in view tree, hidden when not processing)
    let mut processing_dots = Vec::new();
    for _ in 0..3 {
        let dot = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(PROCESS_DOT_SIZE, PROCESS_DOT_SIZE),
            ),
        );
        dot.setWantsLayer(true);
        if let Some(layer) = dot.layer() {
            layer.setBackgroundColor(Some(&theme.accent().CGColor()));
            layer.setCornerRadius(PROCESS_DOT_SIZE / 2.0);
            layer.setOpacity(0.0);
            layer.setShadowColor(Some(&theme.accent().CGColor()));
            layer.setShadowRadius(6.0);
            layer.setShadowOpacity(0.0);
            layer.setShadowOffset(NSSize::new(0.0, 0.0));
        }
        vfx.addSubview(&dot);
        processing_dots.push(dot);
    }

    // Animated gradient border — hidden at rest, only shown during Processing.
    let gradient = CAGradientLayer::new();
    gradient.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(LISTEN_W, LISTEN_H)));
    gradient.setType(unsafe { kCAGradientLayerConic });
    gradient.setStartPoint(NSPoint::new(0.5, 0.5));
    gradient.setEndPoint(NSPoint::new(1.0, 0.5));
    set_gradient_colors(&gradient, &theme);
    set_gradient_locations(&gradient);
    gradient.setHidden(true);

    let mask_shape = CAShapeLayer::new();
    mask_shape.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(LISTEN_W, LISTEN_H)));
    mask_shape.setFillColor(Some(&NSColor::clearColor().CGColor()));
    mask_shape.setStrokeColor(Some(&NSColor::blackColor().CGColor()));
    mask_shape.setLineWidth(BORDER_GRADIENT_W);
    unsafe {
        mask_shape.setPath(Some(&rounded_rect_path(
            LISTEN_W,
            LISTEN_H,
            CORNER_RADIUS,
            BORDER_GRADIENT_W,
        )));
        gradient.setMask(Some(&*mask_shape));
    }
    if let Some(cap_layer) = capsule.layer() {
        cap_layer.addSublayer(&gradient);
    }

    capsule.addSubview(&vfx);
    root.addSubview(&capsule);
    panel.setContentView(Some(&root));
    panel.setAlphaValue(0.0);
    panel.orderFrontRegardless();
    PANEL_OPEN.store(true, Ordering::Relaxed);

    ui.panel = Some(panel);
    ui.root_view = Some(root);
    ui.capsule_view = Some(capsule);
    ui.vfx_view = Some(vfx);
    ui.bg_overlay = Some(bg);
    ui.text_label = Some(label);
    ui.listen_indicator = Some(listen_indicator);
    ui.processing_dots = processing_dots;
    ui.gradient_border = Some(gradient);
    ui.gradient_mask = Some(mask_shape);
    ui.screen_center_x = center_x;
    ui.screen_top_y = top_y;
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
            label.setFrame(NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(0.0, 0.0),
            ));
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

fn animate_to(app: AppHandle, target_w: f64, target_h: f64) {
    let token = MORPH_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        let (start_w, start_h, cx, top_y) = {
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = app.run_on_main_thread(move || {
                UI.with(|ui| {
                    let ui = ui.borrow();
                    let (w, h) = ui
                        .panel
                        .as_ref()
                        .map(|p| (p.frame().size.width, p.frame().size.height))
                        .unwrap_or((target_w, target_h));
                    let _ = tx.send((w, h, ui.screen_center_x, ui.screen_top_y));
                });
            });
            rx.recv().unwrap_or((target_w, target_h, 0.0, 0.0))
        };

        let mut sw = Spring::new(start_w);
        sw.set_target(target_w);
        let mut sh = Spring::new(start_h);
        sh.set_target(target_h);

        loop {
            if MORPH_TOKEN.load(Ordering::Relaxed) != token {
                break;
            }
            let moving_w = sw.tick();
            let moving_h = sh.tick();
            let w = sw.pos;
            let h = sh.pos;
            let _ = app.run_on_main_thread(move || apply_frame(w, h, cx, top_y));
            if !moving_w && !moving_h {
                break;
            }
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }

        if MORPH_TOKEN.load(Ordering::Relaxed) == token {
            let _ = app.run_on_main_thread(move || apply_frame(target_w, target_h, cx, top_y));
        }
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

fn fade_to(app: AppHandle, target_alpha: f64, token: u64) {
    tauri::async_runtime::spawn(async move {
        let start_alpha = {
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = app.run_on_main_thread(move || {
                UI.with(|ui| {
                    let alpha = ui
                        .borrow()
                        .panel
                        .as_ref()
                        .map(|p| p.alphaValue())
                        .unwrap_or(0.0);
                    let _ = tx.send(alpha);
                });
            });
            rx.recv().unwrap_or(0.0)
        };

        for frame in 0..FADE_FRAMES {
            if FADE_TOKEN.load(Ordering::Relaxed) != token {
                return;
            }
            let t = (frame + 1) as f64 / FADE_FRAMES as f64;
            let alpha = start_alpha + (target_alpha - start_alpha) * ease_out_quart(t);
            let _ = app.run_on_main_thread(move || {
                UI.with(|ui| {
                    if let Some(panel) = &ui.borrow().panel {
                        panel.setAlphaValue(alpha);
                    }
                });
            });
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }

        let _ = app.run_on_main_thread(move || {
            UI.with(|ui| {
                if let Some(panel) = &ui.borrow().panel {
                    panel.setAlphaValue(target_alpha);
                }
            });
        });
    });
}

fn close_panel(app: &AppHandle, immediate: bool) {
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

// ─ Agent submission ──────────────────────────────────────────────────────────
fn submit_to_agent(app: AppHandle, text: String) {
    if text.trim().is_empty() {
        close_panel(&app, false);
        return;
    }

    transition_to_processing(&app);

    let db = app.state::<Database>();
    let conv_id = uuid_v4();
    let _ = db.agent_create_conversation(&conv_id, "Voice Shortcut");

    let cid = conv_id.clone();
    let app_for_listener = app.clone();
    let listener_id = app.listen(format!("agent_stream:{conv_id}"), move |event| {
        handle_agent_stream(&app_for_listener, &cid, event.payload());
    });

    {
        let mut sh = SHARED.lock().unwrap();
        sh.agent_listener = Some(listener_id);
        sh.result_accumulated.clear();
    }

    tauri::async_runtime::spawn(async move {
        let _ = agent::agent_send(app.clone(), conv_id, text, Vec::new()).await;
    });
}

fn handle_agent_stream(app: &AppHandle, _conv_id: &str, payload: &str) {
    let parsed = serde_json::from_str::<Value>(payload).unwrap_or(Value::Null);
    let event_type = parsed
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    match event_type {
        "token" => {
            let chunk = parsed
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if chunk.is_empty() {
                return;
            }
            let mut sh = SHARED.lock().unwrap();
            sh.result_accumulated.push_str(chunk);
        }
        "error" => {
            let msg = parsed
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("エラーが発生しました");
            clear_agent_listener(app);
            {
                let mut sh = SHARED.lock().unwrap();
                sh.result_accumulated = msg.to_string();
            }
            transition_to_notice(app, msg);
        }
        "done" => {
            let final_text = {
                let sh = SHARED.lock().unwrap();
                sh.result_accumulated.trim().to_string()
            };
            clear_agent_listener(app);
            if final_text.is_empty() {
                transition_to_notice(app, "応答を取得できませんでした");
            } else {
                transition_to_result(app, &final_text);
            }
        }
        _ => {}
    }
}

fn clear_agent_listener(app: &AppHandle) {
    let listener_id = SHARED.lock().unwrap().agent_listener.take();
    if let Some(id) = listener_id {
        app.unlisten(id);
    }
}

// ─ Result height measurement ─────────────────────────────────────────────────
fn compute_result_height(text: &str) -> f64 {
    let text_w = RESULT_W - RESULT_PAD_X * 2.0;
    // Rough measurement — each character counted as ~font*0.56 (cjk ~font*0.98).
    let mut total_lines = 0.0_f64;
    for raw in text.split('\n') {
        let trimmed = raw.trim_end();
        if trimmed.is_empty() {
            total_lines += 0.5;
            continue;
        }
        let (font_size, indent) = heading_metrics(trimmed);
        let content = strip_markdown_syntax(trimmed);
        if content.is_empty() {
            total_lines += 0.5;
            continue;
        }
        let glyph_w = font_size * 0.56;
        let cjk_w = font_size * 0.98;
        let eff: f64 = content
            .chars()
            .map(|c| if c.is_ascii() { glyph_w } else { cjk_w })
            .sum();
        let effective_w = (text_w - indent).max(1.0);
        let lines = (eff / effective_w).ceil().max(1.0);
        total_lines += lines;
    }
    let visible = total_lines.min(RESULT_MAX_VISIBLE_LINES as f64).max(2.0);
    let line_px = RESULT_BODY_FONT * RESULT_LINE_HEIGHT_MUL;
    let text_h = visible * line_px + RESULT_PAD_Y * 2.0 + 6.0;
    text_h.clamp(RESULT_MIN_H, RESULT_MAX_H)
}

fn heading_metrics(line: &str) -> (f64, f64) {
    if let Some(rest) = line.strip_prefix("### ") {
        let _ = rest;
        (RESULT_H3_FONT, 0.0)
    } else if let Some(rest) = line.strip_prefix("## ") {
        let _ = rest;
        (RESULT_H2_FONT, 0.0)
    } else if let Some(rest) = line.strip_prefix("# ") {
        let _ = rest;
        (RESULT_H1_FONT, 0.0)
    } else if line.trim_start().starts_with("- ")
        || line.trim_start().starts_with("* ")
        || line.trim_start().starts_with("• ")
    {
        (RESULT_BODY_FONT, 14.0)
    } else {
        (RESULT_BODY_FONT, 0.0)
    }
}

fn strip_markdown_syntax(line: &str) -> String {
    let trimmed = line
        .trim_start_matches("### ")
        .trim_start_matches("## ")
        .trim_start_matches("# ")
        .trim_start();
    let without_bullet = if let Some(rest) = trimmed.strip_prefix("- ") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("* ") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("• ") {
        rest
    } else {
        trimmed
    };
    let mut out = String::with_capacity(without_bullet.len());
    let mut skip = false;
    for c in without_bullet.chars() {
        if c == '*' || c == '_' || c == '`' {
            skip = !skip;
            continue;
        }
        out.push(c);
    }
    out
}

fn schedule_close(app: AppHandle, delay: Duration) {
    let token = AUTO_CLOSE_TOKEN
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        if AUTO_CLOSE_TOKEN.load(Ordering::Relaxed) == token {
            close_panel(&app, false);
        }
    });
}

fn cancel_auto_close() {
    AUTO_CLOSE_TOKEN.fetch_add(1, Ordering::Relaxed);
}

// ─ Border styling & gradient animation ───────────────────────────────────────
fn update_border(app: AppHandle, mode: CapsuleMode) {
    let token = BORDER_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);

    if mode != CapsuleMode::Processing {
        let _ = app.run_on_main_thread(move || {
            UI.with(|ui| {
                let ui = ui.borrow();
                let theme = Theme::current();
                suppress_implicit_animations(|| {
                    if let Some(vfx) = &ui.vfx_view {
                        if let Some(layer) = vfx.layer() {
                            let (r, g, b, a) = match mode {
                                CapsuleMode::Result => theme.border_result(),
                                CapsuleMode::Notice => theme.border_notice(),
                                _ => theme.border_idle(),
                            };
                            layer.setBorderColor(Some(&srgb(r, g, b, a).CGColor()));
                            layer.setBorderWidth(BORDER_IDLE_W);
                        }
                    }
                    if let Some(gradient) = &ui.gradient_border {
                        gradient.setHidden(true);
                    }
                });
            });
        });
        return;
    }

    // Processing: hide solid border, show + rotate gradient border.
    let _ = app.run_on_main_thread(move || {
        UI.with(|ui| {
            let ui = ui.borrow();
            let theme = Theme::current();
            suppress_implicit_animations(|| {
                if let Some(vfx) = &ui.vfx_view {
                    if let Some(layer) = vfx.layer() {
                        layer.setBorderColor(Some(&NSColor::clearColor().CGColor()));
                        layer.setBorderWidth(0.0);
                    }
                }
                if let Some(gradient) = &ui.gradient_border {
                    set_gradient_colors(gradient, &theme);
                    gradient.setHidden(false);
                }
            });
        });
    });

    // Rotating animation loop — each tick writes raw values with implicit
    // animations suppressed so there's no tweening on top of our own motion.
    tauri::async_runtime::spawn(async move {
        let mut frame = 0_u64;
        loop {
            if BORDER_TOKEN.load(Ordering::Relaxed) != token {
                break;
            }
            let t = frame as f64 * (ANIM_MS as f64 / 1000.0);
            let angle = (t / GRADIENT_ROTATION_PERIOD_SEC) * std::f64::consts::TAU;
            let ex = 0.5 + 0.5 * angle.cos();
            let ey = 0.5 + 0.5 * angle.sin();
            let _ = app.run_on_main_thread(move || {
                UI.with(|ui| {
                    let ui = ui.borrow();
                    if let Some(gradient) = &ui.gradient_border {
                        suppress_implicit_animations(|| {
                            gradient.setStartPoint(NSPoint::new(0.5, 0.5));
                            gradient.setEndPoint(NSPoint::new(ex, ey));
                        });
                    }
                });
            });
            frame = frame.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });
}

fn set_gradient_colors(gradient: &CAGradientLayer, theme: &Theme) {
    let stops = theme.gradient_stops();
    let cg_colors: Vec<Retained<objc2_core_graphics::CGColor>> =
        stops.iter().map(|c| c.CGColor()).collect();
    let refs: Vec<&AnyObject> = cg_colors
        .iter()
        .map(|c| unsafe { &*(&**c as *const objc2_core_graphics::CGColor as *const AnyObject) })
        .collect();
    let array: Retained<NSArray<AnyObject>> = NSArray::from_slice(&refs);
    unsafe { gradient.setColors(Some(&array)) };
}

fn set_gradient_locations(gradient: &CAGradientLayer) {
    let locations = [0.0f64, 0.25, 0.5, 0.75, 1.0];
    let numbers: Vec<Retained<NSNumber>> = locations
        .iter()
        .map(|v| NSNumber::new_f64(*v))
        .collect();
    let refs: Vec<&NSNumber> = numbers.iter().map(|n| &**n).collect();
    let array: Retained<NSArray<NSNumber>> = NSArray::from_slice(&refs);
    gradient.setLocations(Some(&array));
}

fn rounded_rect_path(
    width: f64,
    height: f64,
    radius: f64,
    line_width: f64,
) -> CFRetained<CGPath> {
    // Inset by half the stroke so the ring sits inside the bounds.
    let inset = line_width / 2.0;
    let rect = NSRect::new(
        NSPoint::new(inset, inset),
        NSSize::new((width - inset * 2.0).max(0.0), (height - inset * 2.0).max(0.0)),
    );
    let r = radius.min(rect.size.width / 2.0).min(rect.size.height / 2.0);
    unsafe { CGPath::with_rounded_rect(rect, r, r, std::ptr::null()) }
}

// ─ Listening indicator & processing dots animations ──────────────────────────
fn start_listen_pulse_animation(app: AppHandle, mode: CapsuleMode) {
    let token = LISTEN_PULSE_TOKEN
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    if mode != CapsuleMode::Listening {
        let _ = app.run_on_main_thread(move || {
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
            if LISTEN_PULSE_TOKEN.load(Ordering::Relaxed) != token {
                break;
            }
            let t = frame as f64 * 0.075;
            let pulse = (t.sin() * 0.5 + 0.5).powf(1.1);
            let opacity = (0.52 + pulse * 0.42).clamp(0.0, 0.98) as f32;
            let shadow_opacity = (0.32 + pulse * 0.46).clamp(0.0, 0.92) as f32;
            let _ = app.run_on_main_thread(move || {
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
            });
            frame = frame.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });
}

fn start_processing_dots_animation(app: AppHandle, mode: CapsuleMode) {
    let token = DOTS_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    if mode != CapsuleMode::Processing {
        let _ = app.run_on_main_thread(move || {
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
            if DOTS_TOKEN.load(Ordering::Relaxed) != token {
                break;
            }
            let t = frame as f64 * 0.115;
            let _ = app.run_on_main_thread(move || {
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
            });
            frame = frame.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });
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
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.unminimize();
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                    let _ = app.emit("tray-open-tab", "agent");
                }

                event.as_ptr()
            }),
        )
    };

    ui.event_monitor = monitor;
}

fn normalize_shortcut(shortcut: &str) -> String {
    let value = shortcut.trim().to_ascii_lowercase();
    if value.is_empty() {
        return DEFAULT_SHORTCUT.into();
    }
    // Accept the user-friendly "option+space" alias by mapping it onto the
    // accelerator string the global-shortcut plugin actually parses.
    if value == "option+space" {
        return "alt+space".into();
    }
    value
}

fn register_appearance_observer(app: AppHandle) {
    let app_ptr = Box::into_raw(Box::new(app)) as usize;

    unsafe {
        let dnc_cls = AnyClass::get(c"NSDistributedNotificationCenter").unwrap();
        let dnc: *mut AnyObject = msg_send![dnc_cls, defaultCenter];
        let notif_name = NSString::from_str("AppleInterfaceThemeChangedNotification");

        let block = RcBlock::new(move |_notif: *mut AnyObject| {
            let app_ref = &*(app_ptr as *const AppHandle);
            // Only track macOS appearance when the user's app theme is "system".
            if app_theme_mode(app_ref) != "system" {
                return;
            }
            let app_clone = app_ref.clone();
            let _ = app_clone.run_on_main_thread(move || {
                SYSTEM_IS_DARK.store(is_dark_mode(), Ordering::Relaxed);
                apply_theme(&Theme::current());
            });
            // Notify other modules (subtitle overlay etc.) that the
            // effective dark/light state has changed even though the user
            // didn't manually flip the toggle. The same event name is used
            // for explicit user changes so listeners stay simple.
            let _ = app_ref.emit("app-theme-changed", ());
        });

        let _: () = msg_send![
            dnc,
            addObserverForName: &*notif_name,
            object: std::ptr::null::<AnyObject>(),
            queue: std::ptr::null::<AnyObject>(),
            usingBlock: &*block
        ];
    }
}

fn apply_theme(theme: &Theme) {
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
                let (sr, sg, sb) = if theme.is_dark { (6, 4, 12) } else { (60, 40, 120) };
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

fn is_dark_mode() -> bool {
    unsafe {
        let cls = AnyClass::get(c"NSAppearance").unwrap();
        let current: *mut AnyObject = msg_send![cls, currentDrawingAppearance];
        if current.is_null() {
            return true;
        }
        let name: *const AnyObject = msg_send![current, name];
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

fn app_theme_mode(app: &AppHandle) -> String {
    let state = app.state::<crate::ThemeState>();
    let guard = state.0.lock().unwrap_or_else(|e| e.into_inner());
    guard.clone()
}

fn effective_is_dark(app: &AppHandle) -> bool {
    match app_theme_mode(app).as_str() {
        "light" => false,
        "dark" => true,
        _ => is_dark_mode(),
    }
}

/// Run a closure inside a CATransaction that suppresses CoreAnimation's default
/// implicit animations. Without this, every `setFrame` / `setOpacity` /
/// `setStartPoint` on a CALayer triggers a ~0.25s ease animation; when we
/// drive our own motion at 60fps these implicit animations stack and fight,
/// which reads as jitter.
fn suppress_implicit_animations<F: FnOnce()>(f: F) {
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    CATransaction::setAnimationDuration(0.0);
    f();
    CATransaction::commit();
}

// ─ Markdown → NSAttributedString ─────────────────────────────────────────────
enum MdBlock {
    Paragraph(Vec<MdInline>),
    Heading(u8, Vec<MdInline>),
    Bullet(Vec<MdInline>),
    Ordered(u32, Vec<MdInline>),
    HRule,
    Blank,
}

#[derive(Clone)]
enum MdInline {
    Text(String),
    Bold(String),
    Italic(String),
    BoldItalic(String),
    Code(String),
}

fn parse_markdown(text: &str) -> Vec<MdBlock> {
    let mut blocks = Vec::new();
    for raw in text.split('\n') {
        let trimmed = raw.trim_end_matches('\r');
        if trimmed.trim().is_empty() {
            blocks.push(MdBlock::Blank);
            continue;
        }
        if trimmed.trim() == "---" || trimmed.trim() == "***" || trimmed.trim() == "___" {
            blocks.push(MdBlock::HRule);
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("### ") {
            blocks.push(MdBlock::Heading(3, parse_inlines(rest)));
        } else if let Some(rest) = trimmed.strip_prefix("## ") {
            blocks.push(MdBlock::Heading(2, parse_inlines(rest)));
        } else if let Some(rest) = trimmed.strip_prefix("# ") {
            blocks.push(MdBlock::Heading(1, parse_inlines(rest)));
        } else if let Some(rest) = trimmed
            .trim_start()
            .strip_prefix("- ")
            .or_else(|| trimmed.trim_start().strip_prefix("* "))
            .or_else(|| trimmed.trim_start().strip_prefix("• "))
        {
            blocks.push(MdBlock::Bullet(parse_inlines(rest)));
        } else if let Some((num, rest)) = parse_ordered_prefix(trimmed.trim_start()) {
            blocks.push(MdBlock::Ordered(num, parse_inlines(rest)));
        } else {
            blocks.push(MdBlock::Paragraph(parse_inlines(trimmed)));
        }
    }
    blocks
}

fn parse_ordered_prefix(line: &str) -> Option<(u32, &str)> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 || i >= bytes.len() {
        return None;
    }
    if bytes[i] != b'.' && bytes[i] != b')' {
        return None;
    }
    if i + 1 >= bytes.len() || bytes[i + 1] != b' ' {
        return None;
    }
    let n: u32 = line[..i].parse().ok()?;
    Some((n, &line[i + 2..]))
}

fn parse_inlines(line: &str) -> Vec<MdInline> {
    let chars: Vec<char> = line.chars().collect();
    let mut out: Vec<MdInline> = Vec::new();
    let mut buf = String::new();
    let mut i = 0;
    let n = chars.len();

    let flush_buf = |buf: &mut String, out: &mut Vec<MdInline>| {
        if !buf.is_empty() {
            out.push(MdInline::Text(std::mem::take(buf)));
        }
    };

    while i < n {
        let c = chars[i];
        if c == '`' {
            let mut j = i + 1;
            while j < n && chars[j] != '`' {
                j += 1;
            }
            if j < n {
                flush_buf(&mut buf, &mut out);
                out.push(MdInline::Code(chars[i + 1..j].iter().collect()));
                i = j + 1;
                continue;
            }
        }
        if c == '*' && i + 2 < n && chars[i + 1] == '*' && chars[i + 2] == '*' {
            let start = i + 3;
            let mut j = start;
            while j + 2 < n && !(chars[j] == '*' && chars[j + 1] == '*' && chars[j + 2] == '*') {
                j += 1;
            }
            if j + 2 < n && chars[j] == '*' && chars[j + 1] == '*' && chars[j + 2] == '*' {
                flush_buf(&mut buf, &mut out);
                out.push(MdInline::BoldItalic(chars[start..j].iter().collect()));
                i = j + 3;
                continue;
            }
        }
        if c == '*' && i + 1 < n && chars[i + 1] == '*' {
            let start = i + 2;
            let mut j = start;
            while j + 1 < n && !(chars[j] == '*' && chars[j + 1] == '*') {
                j += 1;
            }
            if j + 1 < n && chars[j] == '*' && chars[j + 1] == '*' {
                flush_buf(&mut buf, &mut out);
                out.push(MdInline::Bold(chars[start..j].iter().collect()));
                i = j + 2;
                continue;
            }
        }
        if c == '*' && i + 1 < n {
            let start = i + 1;
            let mut j = start;
            while j < n && chars[j] != '*' {
                j += 1;
            }
            if j < n && chars[j] == '*' {
                flush_buf(&mut buf, &mut out);
                out.push(MdInline::Italic(chars[start..j].iter().collect()));
                i = j + 1;
                continue;
            }
        }
        buf.push(c);
        i += 1;
    }
    flush_buf(&mut buf, &mut out);
    out
}

fn build_markdown_attributed(text: &str, theme: Theme) -> Retained<NSMutableAttributedString> {
    let blocks = parse_markdown(text);
    let out = NSMutableAttributedString::new();

    let body_color = theme.label_color();
    let muted_color = theme.muted_label();
    let accent_color = theme.accent();
    let code_fg = theme.code_fg();
    let code_bg = theme.code_bg();

    let mut first_block = true;
    let mut prev_blank = false;
    for block in &blocks {
        match block {
            MdBlock::Blank => {
                prev_blank = true;
                continue;
            }
            _ => {}
        }

        if !first_block {
            append_plain(&out, "\n", &body_color, RESULT_BODY_FONT, None, 0.0);
        }
        first_block = false;

        match block {
            MdBlock::Heading(level, inlines) => {
                let (font_size, spacing_before) = match level {
                    1 => (RESULT_H1_FONT, 10.0),
                    2 => (RESULT_H2_FONT, 8.0),
                    _ => (RESULT_H3_FONT, 6.0),
                };
                let color = if *level == 1 { body_color.clone() } else { accent_color.clone() };
                let style = make_paragraph_style(
                    RESULT_LINE_HEIGHT_MUL,
                    RESULT_PARAGRAPH_SPACING,
                    if first_block { 0.0 } else { spacing_before },
                    0.0,
                    0.0,
                    NSTextAlignment::Left,
                );
                append_inlines(
                    &out,
                    inlines,
                    &BlockCtx {
                        base_font_size: font_size,
                        bold: true,
                        color: &color,
                        accent: &accent_color,
                        code_fg: &code_fg,
                        code_bg: &code_bg,
                        paragraph: &style,
                    },
                );
            }
            MdBlock::Paragraph(inlines) => {
                let style = make_paragraph_style(
                    RESULT_LINE_HEIGHT_MUL,
                    if prev_blank { RESULT_PARAGRAPH_SPACING } else { 2.0 },
                    0.0,
                    0.0,
                    0.0,
                    NSTextAlignment::Left,
                );
                append_inlines(
                    &out,
                    inlines,
                    &BlockCtx {
                        base_font_size: RESULT_BODY_FONT,
                        bold: false,
                        color: &body_color,
                        accent: &accent_color,
                        code_fg: &code_fg,
                        code_bg: &code_bg,
                        paragraph: &style,
                    },
                );
            }
            MdBlock::Bullet(inlines) => {
                let style = make_paragraph_style(
                    RESULT_LINE_HEIGHT_MUL,
                    2.0,
                    0.0,
                    14.0,
                    14.0,
                    NSTextAlignment::Left,
                );
                append_plain(&out, "•  ", &accent_color, RESULT_BODY_FONT, Some(&style), 0.0);
                append_inlines(
                    &out,
                    inlines,
                    &BlockCtx {
                        base_font_size: RESULT_BODY_FONT,
                        bold: false,
                        color: &body_color,
                        accent: &accent_color,
                        code_fg: &code_fg,
                        code_bg: &code_bg,
                        paragraph: &style,
                    },
                );
            }
            MdBlock::Ordered(n, inlines) => {
                let style = make_paragraph_style(
                    RESULT_LINE_HEIGHT_MUL,
                    2.0,
                    0.0,
                    18.0,
                    18.0,
                    NSTextAlignment::Left,
                );
                let marker = format!("{n}.  ");
                append_plain(&out, &marker, &accent_color, RESULT_BODY_FONT, Some(&style), 0.0);
                append_inlines(
                    &out,
                    inlines,
                    &BlockCtx {
                        base_font_size: RESULT_BODY_FONT,
                        bold: false,
                        color: &body_color,
                        accent: &accent_color,
                        code_fg: &code_fg,
                        code_bg: &code_bg,
                        paragraph: &style,
                    },
                );
            }
            MdBlock::HRule => {
                let style = make_paragraph_style(
                    0.9,
                    6.0,
                    6.0,
                    0.0,
                    0.0,
                    NSTextAlignment::Left,
                );
                let rule: String = "─".repeat(48);
                append_plain(&out, &rule, &muted_color, RESULT_BODY_FONT * 0.7, Some(&style), 0.0);
            }
            MdBlock::Blank => {}
        }
        prev_blank = false;
    }
    out
}

struct BlockCtx<'a> {
    base_font_size: f64,
    bold: bool,
    color: &'a NSColor,
    accent: &'a NSColor,
    code_fg: &'a NSColor,
    code_bg: &'a NSColor,
    paragraph: &'a NSMutableParagraphStyle,
}

fn append_inlines(out: &NSMutableAttributedString, inlines: &[MdInline], ctx: &BlockCtx) {
    for inline in inlines {
        match inline {
            MdInline::Text(s) => {
                let font = if ctx.bold {
                    NSFont::boldSystemFontOfSize(ctx.base_font_size)
                } else {
                    NSFont::systemFontOfSize(ctx.base_font_size)
                };
                append_attr(
                    out,
                    s,
                    Some(&font),
                    Some(ctx.color),
                    None,
                    Some(ctx.paragraph),
                );
            }
            MdInline::Bold(s) => {
                let font = NSFont::boldSystemFontOfSize(ctx.base_font_size);
                append_attr(
                    out,
                    s,
                    Some(&font),
                    Some(ctx.color),
                    None,
                    Some(ctx.paragraph),
                );
            }
            MdInline::Italic(s) => {
                let font = italic_system_font(ctx.base_font_size);
                append_attr(
                    out,
                    s,
                    Some(&font),
                    Some(ctx.accent),
                    None,
                    Some(ctx.paragraph),
                );
            }
            MdInline::BoldItalic(s) => {
                let font = bold_italic_system_font(ctx.base_font_size);
                append_attr(
                    out,
                    s,
                    Some(&font),
                    Some(ctx.accent),
                    None,
                    Some(ctx.paragraph),
                );
            }
            MdInline::Code(s) => {
                let font = NSFont::monospacedSystemFontOfSize_weight(
                    RESULT_CODE_FONT,
                    unsafe { objc2_app_kit::NSFontWeightMedium },
                );
                // Subtle padding around inline code using hair-space around the text.
                let padded = format!("\u{2009}{s}\u{2009}");
                append_attr(
                    out,
                    &padded,
                    Some(&font),
                    Some(ctx.code_fg),
                    Some(ctx.code_bg),
                    Some(ctx.paragraph),
                );
            }
        }
    }
}

fn append_plain(
    out: &NSMutableAttributedString,
    text: &str,
    color: &NSColor,
    font_size: f64,
    paragraph: Option<&NSMutableParagraphStyle>,
    _kern: f64,
) {
    let font = NSFont::systemFontOfSize(font_size);
    append_attr(out, text, Some(&font), Some(color), None, paragraph);
}

fn append_attr(
    out: &NSMutableAttributedString,
    text: &str,
    font: Option<&NSFont>,
    fg: Option<&NSColor>,
    bg: Option<&NSColor>,
    paragraph: Option<&NSMutableParagraphStyle>,
) {
    if text.is_empty() {
        return;
    }
    let ns = NSString::from_str(text);
    let start = out.length();
    let piece = NSAttributedString::initWithString(NSAttributedString::alloc(), &ns);
    out.appendAttributedString(&piece);
    let end = out.length();
    if end <= start {
        return;
    }
    let range = NSRange::new(start, end - start);

    unsafe {
        if let Some(font) = font {
            out.addAttribute_value_range(NSFontAttributeName, font.as_ref(), range);
        }
        if let Some(fg) = fg {
            out.addAttribute_value_range(NSForegroundColorAttributeName, fg.as_ref(), range);
        }
        if let Some(bg) = bg {
            out.addAttribute_value_range(
                objc2_app_kit::NSBackgroundColorAttributeName,
                bg.as_ref(),
                range,
            );
        }
        if let Some(paragraph) = paragraph {
            out.addAttribute_value_range(
                NSParagraphStyleAttributeName,
                paragraph.as_ref(),
                range,
            );
        }
    }
}

fn make_paragraph_style(
    line_height_mul: f64,
    paragraph_spacing: f64,
    spacing_before: f64,
    head_indent: f64,
    first_head_indent: f64,
    alignment: NSTextAlignment,
) -> Retained<NSMutableParagraphStyle> {
    let style = NSMutableParagraphStyle::new();
    style.setLineHeightMultiple(line_height_mul);
    style.setParagraphSpacing(paragraph_spacing);
    style.setParagraphSpacingBefore(spacing_before);
    style.setHeadIndent(head_indent);
    style.setFirstLineHeadIndent(first_head_indent);
    style.setAlignment(alignment);
    style
}

fn italic_system_font(size: f64) -> Retained<NSFont> {
    unsafe {
        let cls = AnyClass::get(c"NSFontManager").unwrap();
        let shared: *mut AnyObject = msg_send![cls, sharedFontManager];
        let base = NSFont::systemFontOfSize(size);
        let italic_traits: i64 = 1; // NSItalicFontMask
        let result: *mut NSFont = msg_send![
            shared,
            convertFont: &*base,
            toHaveTrait: italic_traits
        ];
        if result.is_null() {
            base
        } else {
            Retained::retain(result).unwrap_or_else(|| NSFont::systemFontOfSize(size))
        }
    }
}

fn bold_italic_system_font(size: f64) -> Retained<NSFont> {
    unsafe {
        let cls = AnyClass::get(c"NSFontManager").unwrap();
        let shared: *mut AnyObject = msg_send![cls, sharedFontManager];
        let base = NSFont::boldSystemFontOfSize(size);
        let italic_traits: i64 = 1; // NSItalicFontMask
        let result: *mut NSFont = msg_send![
            shared,
            convertFont: &*base,
            toHaveTrait: italic_traits
        ];
        if result.is_null() {
            base
        } else {
            Retained::retain(result).unwrap_or_else(|| NSFont::boldSystemFontOfSize(size))
        }
    }
}

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

fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}
