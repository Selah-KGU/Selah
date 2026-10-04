// The `#[cfg(target_os = "macos")]` gate lives on the `mod` declaration in
// `lib.rs`; we don't need to repeat it as an inner attribute here.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{msg_send, AnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSEventMask, NSFloatingWindowLevel, NSFont,
    NSFontAttributeName, NSForegroundColorAttributeName, NSLineBreakMode, NSMutableParagraphStyle,
    NSPanel, NSParagraphStyleAttributeName, NSScreen, NSTextAlignment, NSTextField, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::CFRetained;
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

#[path = "macos_native_agent/capsule_ui.rs"]
mod capsule_ui;
#[path = "macos_native_agent/flow.rs"]
mod flow;
#[path = "macos_native_agent/markdown.rs"]
mod markdown;
#[path = "macos_native_agent/shortcut.rs"]
mod shortcut;
#[path = "macos_native_agent/state.rs"]
mod state;
#[path = "macos_native_agent/theme.rs"]
mod theme;

use capsule_ui::{
    apply_theme, close_panel, ensure_panel, start_listen_pulse_animation,
    start_processing_dots_animation, update_border, update_text,
};
use markdown::{build_markdown_attributed, srgb};

pub use flow::setup;
use flow::{
    cancel_auto_close, clear_agent_listener, ease_out_quart, submit_to_agent,
    suppress_implicit_animations, transition_to_listening, transition_to_notice,
};
pub use shortcut::apply_config;
use shortcut::schedule_release_finalize;
use state::{
    append_final_segment, consume_all_speech, listening_display_text, CapsuleMode, SHARED,
};
use theme::Theme;

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
const RELEASE_FINALIZE_DELAY_MS: u64 = 180;

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
static RELEASE_FINALIZE_TOKEN: AtomicU64 = AtomicU64::new(0);
static DOTS_TOKEN: AtomicU64 = AtomicU64::new(0);
static LISTEN_PULSE_TOKEN: AtomicU64 = AtomicU64::new(0);
static SHORTCUT_REGISTERED: std::sync::LazyLock<Mutex<Option<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));
