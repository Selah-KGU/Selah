#![cfg(target_os = "macos")]

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{msg_send, AnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSEventMask, NSFloatingWindowLevel, NSFont, NSImage,
    NSPanel, NSScreen, NSTextAlignment, NSTextField, NSView, NSVisualEffectBlendingMode,
    NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use objc2_foundation::{NSData, NSPoint, NSRect, NSSize, NSString};
use serde_json::Value;
use std::ptr::NonNull;
use tauri::{AppHandle, Listener, Manager};

use crate::agent;
use crate::db::Database;
use crate::stt;
use crate::tray;

#[path = "macos_native_agent/ui_support.rs"]
mod ui_support;

use ui_support::*;

// ── Capsule Dimensions ─────────────────────────────────────────────────────

const CAP_IDLE_W: f64 = 110.0;
const CAP_H: f64 = 46.0;
const CAP_CORNER: f64 = CAP_H / 2.0;
const CAP_MARGIN_RIGHT: f64 = 24.0;
const CAP_MARGIN_BOTTOM: f64 = 28.0;

// Listening / processing / result widths
const CAP_LISTENING_W: f64 = 360.0;
const CAP_PROCESSING_W: f64 = 160.0;
const CAP_RESULT_W: f64 = 380.0;
const CAP_RESULT_MAX_H: f64 = 280.0;

// Idle: logo + mic
const LOGO_SIZE: f64 = 28.0;
const LOGO_HALO_SIZE: f64 = 34.0;
const MIC_CAPSULE_W: f64 = 42.0;
const MIC_CAPSULE_H: f64 = 34.0;
const BAR_W: f64 = 2.5;
const BAR_GAP: f64 = 4.0;
const BAR_IDLE: [f64; 4] = [10.0, 15.0, 7.0, 12.0];

// Listening: speech text + done button
const LISTEN_TEXT_LEFT: f64 = 16.0;
const LISTEN_DONE_SIZE: f64 = 30.0;
const LISTEN_DONE_RIGHT: f64 = 8.0;
const LISTEN_TEXT_FONT: f64 = 13.5;

// Processing: three dots
const DOT_SIZE: f64 = 8.0;
const DOT_GAP: f64 = 10.0;

// Result: text + continue button
const RESULT_PAD_X: f64 = 20.0;
const RESULT_PAD_Y: f64 = 12.0;
const RESULT_FONT: f64 = 13.0;
const RESULT_LINE_H: f64 = RESULT_FONT * 1.44;
const RESULT_MAX_LINES: usize = 10;
const RESULT_BTN_SIZE: f64 = 30.0;
const RESULT_BTN_RIGHT: f64 = 10.0;
const RESULT_AUTO_CLOSE_SECS: u64 = 20;

// ── Animation ───────────────────────────────────────────────────────────────

const ANIM_MS: u64 = 16; // ~60 fps
const FADE_IN_FRAMES: u64 = 18;

// Spring physics for morph — tuned for Dynamic-Island-like feel
const SPRING_STIFFNESS: f64 = 280.0; // Higher = faster snap
const SPRING_DAMPING: f64 = 22.0; // Higher = less oscillation
const SPRING_MASS: f64 = 1.0;
const SPRING_DT: f64 = 0.016; // timestep per frame (60fps)
const SPRING_SETTLE: f64 = 0.3; // velocity+distance threshold to stop

// ── State ───────────────────────────────────────────────────────────────────

// Thread-local: holds NSView references (must stay on main thread)
thread_local! {
    static UI: RefCell<CapsuleViews> = RefCell::new(CapsuleViews::default());
}

// Cross-thread shared state (accessed from event listeners on tokio threads)
static SHARED: std::sync::LazyLock<Mutex<SharedState>> =
    std::sync::LazyLock::new(|| Mutex::new(SharedState::default()));

static WAVE_TOKEN: AtomicU64 = AtomicU64::new(0);
static WAVE_ACTIVE: AtomicBool = AtomicBool::new(false);
static MORPH_TOKEN: AtomicU64 = AtomicU64::new(0);
static DOTS_TOKEN: AtomicU64 = AtomicU64::new(0);
static BORDER_ANIM_TOKEN: AtomicU64 = AtomicU64::new(0);
static RESULT_CLOSE_TOKEN: AtomicU64 = AtomicU64::new(0);
static RESULT_GLOW_TOKEN: AtomicU64 = AtomicU64::new(0);
static CONTENT_FADE_TOKEN: AtomicU64 = AtomicU64::new(0);
static IDLE_WAVE_TOKEN: AtomicU64 = AtomicU64::new(0);
/// Cached system dark-mode flag; cheaply read by animation loops.
static SYSTEM_IS_DARK: AtomicBool = AtomicBool::new(true);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CapsuleMode {
    Idle,
    Listening,
    Processing,
    Result,
}

/// Shared mutable state accessed from any thread (event listeners, tokio tasks).
#[derive(Default)]
struct SharedState {
    mode: Option<CapsuleMode>,
    stop_requested: bool,
    last_final_text: String,
    current_speech: String,
    conv_id: Option<String>,
    agent_listener: Option<tauri::EventId>,
    result_accumulated: String,
}

/// Thread-local view state — only touched on the main thread.
#[derive(Default)]
struct CapsuleViews {
    panel: Option<Retained<NSPanel>>,
    root_view: Option<Retained<NSView>>,
    capsule_view: Option<Retained<NSView>>, // shadow host
    vfx_view: Option<Retained<NSVisualEffectView>>,
    bg_overlay: Option<Retained<NSView>>, // dark opacity layer

    // Idle sub-views
    idle_logo_halo: Option<Retained<NSView>>,
    idle_logo: Option<Retained<NSView>>,
    idle_mic_capsule: Option<Retained<NSView>>,
    idle_separator: Option<Retained<NSView>>,
    idle_wave_bars: Vec<Retained<NSView>>,

    // Listening sub-views
    listen_text: Option<Retained<NSTextField>>,
    listen_done_btn: Option<Retained<NSView>>,
    listen_done_icon: Option<Retained<NSView>>,

    // Processing sub-views
    proc_dots: Vec<Retained<NSView>>,

    // Result sub-views
    result_text: Option<Retained<NSTextField>>,
    result_btn: Option<Retained<NSView>>,
    result_btn_icon: Option<Retained<NSView>>,

    // Interaction
    event_monitor: Option<Retained<AnyObject>>,
}

// ── Easing & Spring ─────────────────────────────────────────────────────────

/// Corner radius that never exceeds the idle-capsule pill radius,
/// so tall result views stay a rounded rectangle instead of an oval.
#[inline]
fn cap_radius(h: f64) -> f64 {
    (h / 2.0).min(CAP_CORNER)
}

#[inline]
fn ease_out_quart(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(4)
}

/// Single-axis critically-damped spring state.
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
    /// Advance one tick. Returns true if still moving.
    fn tick(&mut self) -> bool {
        let dx = self.pos - self.target;
        let accel = (-SPRING_STIFFNESS * dx - SPRING_DAMPING * self.vel) / SPRING_MASS;
        self.vel += accel * SPRING_DT;
        self.pos += self.vel * SPRING_DT;
        dx.abs() > SPRING_SETTLE || self.vel.abs() > SPRING_SETTLE
    }
}

// ── Helpers: NSVisualEffectView ─────────────────────────────────────────────

fn new_vibrancy_view(
    mtm: MainThreadMarker,
    frame: NSRect,
    material: NSVisualEffectMaterial,
    radius: f64,
) -> Retained<NSVisualEffectView> {
    let vfx: Retained<NSVisualEffectView> =
        unsafe { msg_send![NSVisualEffectView::alloc(mtm), initWithFrame: frame] };
    vfx.setMaterial(material);
    vfx.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    vfx.setState(NSVisualEffectState::Active);
    vfx.setWantsLayer(true);
    if let Some(layer) = vfx.layer() {
        layer.setCornerRadius(radius);
        layer.setMasksToBounds(true);
    }
    vfx
}

// ── Appearance Change Observer ───────────────────────────────────────────────

/// Registers for macOS distributed notification `AppleInterfaceThemeChangedNotification`.
/// Must be called from the main thread.
fn register_appearance_observer(app: AppHandle) {
    // Store app handle in a leaked Box so the raw pointer lives forever.
    let app_ptr = Box::into_raw(Box::new(app)) as usize;

    unsafe {
        // Get NSDistributedNotificationCenter defaultCenter
        let dnc_cls = AnyClass::get(c"NSDistributedNotificationCenter").unwrap();
        let dnc: *mut AnyObject = msg_send![dnc_cls, defaultCenter];

        // Notification name
        let notif_name = NSString::from_str("AppleInterfaceThemeChangedNotification");

        // Use a raw block via block2
        let block = RcBlock::new(move |_notif: *mut AnyObject| {
            // Reconstruct (borrow) the AppHandle from the leaked pointer
            let app_ref = &*(app_ptr as *const AppHandle);
            let app_clone = app_ref.clone();
            let _ = app_clone.run_on_main_thread(move || {
                let dark = is_dark_mode_macos();
                SYSTEM_IS_DARK.store(dark, Ordering::Relaxed);
                UI.with(|s| {
                    let s = s.borrow();
                    if s.panel.is_some() {
                        apply_theme_to_capsule(&s, Theme { is_dark: dark });
                    }
                });
            });
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

// ── Public API ──────────────────────────────────────────────────────────────

pub fn setup(app: &AppHandle) {
    // Observe macOS appearance changes (Dark ↔ Light) — must run on main thread
    let app_theme = app.clone();
    let _ = app.run_on_main_thread(move || {
        register_appearance_observer(app_theme);
    });

    // stt-final: user finished speaking a phrase
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
            .to_owned();

        // Always show final text in the listening label
        if !text.trim().is_empty() {
            update_listening_text(&app_final, &text);
        }

        let pending = {
            let mut sh = SHARED.lock().unwrap();
            if sh.stop_requested && !text.trim().is_empty() {
                sh.stop_requested = false;
                sh.last_final_text.clear();
                Some(text.trim().to_string())
            } else {
                sh.last_final_text = text;
                None
            }
        };

        if let Some(text) = pending {
            submit_to_agent(app_final.clone(), text);
        }
    });

    // stt-partial: live transcription updates
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
            .to_owned();
        if text.trim().is_empty() {
            return;
        }
        update_listening_text(&app_partial, &text);
    });

    // stt-state: listening started/stopped
    let app_state = app.clone();
    app.listen("stt-state", move |event| {
        let payload = serde_json::from_str::<Value>(event.payload()).unwrap_or_default();
        if payload.get("caller").and_then(|c| c.as_str()) != Some("agent") {
            return;
        }
        let state_name = payload
            .get("state")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_owned();
        let listening = matches!(state_name.as_str(), "initializing" | "listening");

        if listening {
            // Transition to listening mode (if not already)
            transition_to_listening(&app_state);
        } else {
            // STT stopped — check if we have text to submit
            let pending = {
                let mut sh = SHARED.lock().unwrap();
                // If already transitioned to processing (stt-final fast-path submitted),
                // don't submit again.
                if sh.mode == Some(CapsuleMode::Processing) || sh.mode == Some(CapsuleMode::Result)
                {
                    sh.last_final_text.clear();
                    sh.current_speech.clear();
                    None
                } else if sh.stop_requested {
                    // User clicked done — use final text if available, otherwise try current speech
                    let text = if !sh.last_final_text.trim().is_empty() {
                        sh.last_final_text.trim().to_string()
                    } else {
                        sh.current_speech.trim().to_string()
                    };
                    sh.stop_requested = false;
                    sh.last_final_text.clear();
                    sh.current_speech.clear();
                    Some(text) // may be empty -> will go to idle
                } else {
                    // Natural end of STT — auto-submit the accumulated speech
                    let text = if !sh.current_speech.trim().is_empty() {
                        sh.current_speech.trim().to_string()
                    } else {
                        sh.last_final_text.trim().to_string()
                    };
                    sh.last_final_text.clear();
                    sh.current_speech.clear();
                    Some(text) // may be empty -> will go to idle
                }
            };

            if let Some(text) = pending {
                if text.is_empty() {
                    transition_to_idle(&app_state);
                } else {
                    submit_to_agent(app_state.clone(), text);
                }
            }
        }
    });

    let app_err = app.clone();
    app.listen("stt-error", move |_event| {
        {
            let mut sh = SHARED.lock().unwrap();
            sh.stop_requested = false;
            sh.last_final_text.clear();
            sh.current_speech.clear();
        }
        transition_to_idle(&app_err);
    });
}

pub fn open_orb(app: &AppHandle) -> Result<(), String> {
    let app_handle = app.clone();
    app.run_on_main_thread(move || {
        UI.with(|s| {
            let mut s = s.borrow_mut();
            if let Some(panel) = &s.panel {
                panel.setAlphaValue(1.0);
                panel.orderFrontRegardless();
                panel.makeKeyAndOrderFront(None);
                return;
            }
            build_capsule(&mut s, &app_handle);
            install_event_monitor(&mut s, app_handle.clone());
        });
    })
    .map_err(|e| format!("capsule open failed: {}", e))?;

    // Fade in
    let app_anim = app.clone();
    tauri::async_runtime::spawn(async move {
        for i in 1..=FADE_IN_FRAMES {
            let alpha = ease_out_quart(i as f64 / FADE_IN_FRAMES as f64);
            let _ = app_anim.run_on_main_thread(move || {
                UI.with(|s| {
                    if let Some(panel) = &s.borrow().panel {
                        panel.setAlphaValue(alpha);
                    }
                });
            });
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });

    Ok(())
}

pub fn close_orb(app: &AppHandle) -> Result<(), String> {
    // Cancel all animations
    WAVE_ACTIVE.store(false, Ordering::Relaxed);
    WAVE_TOKEN.fetch_add(1, Ordering::Relaxed);
    MORPH_TOKEN.fetch_add(1, Ordering::Relaxed);
    DOTS_TOKEN.fetch_add(1, Ordering::Relaxed);
    BORDER_ANIM_TOKEN.fetch_add(1, Ordering::Relaxed);
    RESULT_CLOSE_TOKEN.fetch_add(1, Ordering::Relaxed);
    RESULT_GLOW_TOKEN.fetch_add(1, Ordering::Relaxed);
    CONTENT_FADE_TOKEN.fetch_add(1, Ordering::Relaxed);
    IDLE_WAVE_TOKEN.fetch_add(1, Ordering::Relaxed);

    app.run_on_main_thread(move || {
        UI.with(|s| {
            let mut s = s.borrow_mut();
            if let Some(panel) = s.panel.take() {
                panel.setAlphaValue(0.0);
                panel.orderOut(None);
                panel.close();
            }
            // Clear views
            s.root_view = None;
            s.capsule_view = None;
            s.vfx_view = None;
            s.bg_overlay = None;
            s.idle_logo_halo = None;
            s.idle_logo = None;
            s.idle_mic_capsule = None;
            s.idle_separator = None;
            s.idle_wave_bars.clear();
            s.listen_text = None;
            s.listen_done_btn = None;
            s.listen_done_icon = None;
            s.proc_dots.clear();
            s.result_text = None;
            s.result_btn = None;
            s.result_btn_icon = None;
        });
        // Clear shared state
        let mut sh = SHARED.lock().unwrap();
        sh.mode = None;
        sh.stop_requested = false;
        sh.last_final_text.clear();
        sh.current_speech.clear();
        sh.conv_id = None;
        sh.agent_listener = None;
        sh.result_accumulated.clear();
    })
    .map_err(|e| format!("capsule close failed: {}", e))
}

// ── Capsule Construction ────────────────────────────────────────────────────

fn build_capsule(s: &mut CapsuleViews, app: &AppHandle) {
    let mtm = MainThreadMarker::new().expect("main thread");
    let visible = visible_frame(mtm);
    let rect = NSRect::new(
        NSPoint::new(
            visible.origin.x + visible.size.width - CAP_IDLE_W - CAP_MARGIN_RIGHT,
            visible.origin.y + CAP_MARGIN_BOTTOM,
        ),
        NSSize::new(CAP_IDLE_W, CAP_H),
    );

    let panel = base_panel(mtm, rect, true);
    panel.setMovableByWindowBackground(true);
    panel.setAcceptsMouseMovedEvents(true);
    panel.setAlphaValue(0.0);

    // Detect and cache current dark/light mode
    let dark = is_dark_mode_macos();
    SYSTEM_IS_DARK.store(dark, Ordering::Relaxed);
    let theme = Theme { is_dark: dark };

    // Root clear view
    let root = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(CAP_IDLE_W, CAP_H)),
    );
    root.setWantsLayer(true);
    if let Some(layer) = root.layer() {
        layer.setBackgroundColor(Some(&NSColor::clearColor().CGColor()));
    }

    // Shadow host capsule
    let capsule = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(CAP_IDLE_W, CAP_H)),
    );
    capsule.setWantsLayer(true);
    if let Some(layer) = capsule.layer() {
        layer.setCornerRadius(CAP_CORNER);
        layer.setBackgroundColor(Some(&NSColor::clearColor().CGColor()));
    }

    // Frosted glass base (subtle vibrancy underneath)
    let vfx = new_vibrancy_view(
        mtm,
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(CAP_IDLE_W, CAP_H)),
        NSVisualEffectMaterial::HUDWindow,
        CAP_CORNER,
    );

    // Near-opaque overlay on top of vibrancy (color depends on mode)
    let overlay = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(CAP_IDLE_W, CAP_H)),
    );
    overlay.setWantsLayer(true);
    if let Some(layer) = overlay.layer() {
        let (r, g, b, a) = theme.overlay_bg();
        layer.setBackgroundColor(Some(&srgb(r, g, b, a).CGColor()));
        layer.setCornerRadius(CAP_CORNER);
    }
    vfx.addSubview(&overlay);

    // Subtle inner border
    if let Some(vfx_layer) = vfx.layer() {
        let (r, g, b, a) = theme.vfx_border();
        vfx_layer.setBorderColor(Some(&srgb(r, g, b, a).CGColor()));
        vfx_layer.setBorderWidth(0.5);
    }

    // Shadow
    if let Some(layer) = capsule.layer() {
        let (r, g, b, a) = theme.capsule_shadow();
        layer.setShadowColor(Some(&srgb(r, g, b, a).CGColor()));
        layer.setShadowRadius(30.0);
        layer.setShadowOpacity(theme.capsule_shadow_opacity() as f32);
        layer.setShadowOffset(NSSize::new(0.0, -10.0));
    }

    capsule.addSubview(&vfx);
    root.addSubview(&capsule);
    panel.setContentView(Some(&root));
    panel.orderFrontRegardless();

    s.panel = Some(panel);
    s.root_view = Some(root);
    s.capsule_view = Some(capsule);
    s.vfx_view = Some(Retained::clone(&vfx));
    s.bg_overlay = Some(overlay);
    SHARED.lock().unwrap().mode = Some(CapsuleMode::Idle);

    // Build idle sub-views
    build_idle_views(s, &vfx, mtm);

    // Start idle wave animation
    let token = IDLE_WAVE_TOKEN
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    start_idle_wave_anim(app.clone(), token);
}

fn build_idle_views(s: &mut CapsuleViews, vfx: &NSVisualEffectView, mtm: MainThreadMarker) {
    let theme = Theme::current();
    let cy = CAP_H / 2.0;

    // Layout: [logo | gap | separator | gap | mic_capsule] — centered in capsule
    let gap = 6.0;
    let sep_w = 0.5;
    let content_w = LOGO_SIZE + gap + sep_w + gap + MIC_CAPSULE_W;
    let start_x = (CAP_IDLE_W - content_w) / 2.0;

    let logo_x = start_x;
    let sep_x = logo_x + LOGO_SIZE + gap;
    let mic_x = sep_x + sep_w + gap;

    // Logo glow ring
    let halo_offset = (LOGO_HALO_SIZE - LOGO_SIZE) / 2.0;
    let halo = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(
            NSPoint::new(logo_x - halo_offset, cy - LOGO_HALO_SIZE / 2.0),
            NSSize::new(LOGO_HALO_SIZE, LOGO_HALO_SIZE),
        ),
    );
    halo.setWantsLayer(true);
    if let Some(layer) = halo.layer() {
        layer.setCornerRadius(LOGO_HALO_SIZE / 2.0);
        let (r, g, b, a) = theme.halo_idle();
        layer.setBackgroundColor(Some(&srgb(r, g, b, a).CGColor()));
    }
    vfx.addSubview(&halo);

    // Logo image — transparent version
    let logo_view = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(
            NSPoint::new(logo_x, cy - LOGO_SIZE / 2.0),
            NSSize::new(LOGO_SIZE, LOGO_SIZE),
        ),
    );
    logo_view.setWantsLayer(true);
    if let Some(layer) = logo_view.layer() {
        layer.setCornerRadius(LOGO_SIZE / 2.0);
        layer.setMasksToBounds(true);
        if let Some(logo) = load_app_logo() {
            let obj: &AnyObject = &logo;
            unsafe {
                layer.setContents(Some(obj));
            }
        }
    }
    vfx.addSubview(&logo_view);

    // Vertical separator — centered vertically
    let sep_h = CAP_H * 0.48;
    let sep = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(
            NSPoint::new(sep_x, cy - sep_h / 2.0),
            NSSize::new(sep_w, sep_h),
        ),
    );
    sep.setWantsLayer(true);
    if let Some(layer) = sep.layer() {
        let (r, g, b, a) = theme.separator_color();
        layer.setBackgroundColor(Some(&srgb(r, g, b, a).CGColor()));
    }
    vfx.addSubview(&sep);

    // Mic capsule area — centered vertically
    let mic_y = cy - MIC_CAPSULE_H / 2.0;
    let mic_capsule = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(
            NSPoint::new(mic_x, mic_y),
            NSSize::new(MIC_CAPSULE_W, MIC_CAPSULE_H),
        ),
    );
    mic_capsule.setWantsLayer(true);
    if let Some(layer) = mic_capsule.layer() {
        let (r, g, b, a) = theme.mic_bg();
        layer.setBackgroundColor(Some(&srgb(r, g, b, a).CGColor()));
        layer.setCornerRadius(MIC_CAPSULE_H / 2.0);
    }
    vfx.addSubview(&mic_capsule);

    // Wave bars — centered inside mic_capsule
    let bar_count = 4usize;
    let total_bars = BAR_W * bar_count as f64 + BAR_GAP * (bar_count as f64 - 1.0);
    let bars_start_x = mic_x + (MIC_CAPSULE_W - total_bars) / 2.0;
    let mut bars = Vec::with_capacity(bar_count);
    for (i, h) in BAR_IDLE.iter().enumerate() {
        let x = bars_start_x + (i as f64) * (BAR_W + BAR_GAP);
        let bar = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(NSPoint::new(x, cy - h / 2.0), NSSize::new(BAR_W, *h)),
        );
        bar.setWantsLayer(true);
        if let Some(layer) = bar.layer() {
            let (r, g, b, a) = theme.bar_color();
            layer.setBackgroundColor(Some(&srgb(r, g, b, a).CGColor()));
            layer.setCornerRadius(BAR_W / 2.0);
        }
        vfx.addSubview(&bar);
        bars.push(bar);
    }

    s.idle_logo_halo = Some(halo);
    s.idle_logo = Some(logo_view);
    s.idle_mic_capsule = Some(mic_capsule);
    s.idle_separator = Some(sep);
    s.idle_wave_bars = bars;
}

fn build_listening_views(s: &mut CapsuleViews, vfx: &NSVisualEffectView, mtm: MainThreadMarker) {
    let theme = Theme::current();
    let text_right = LISTEN_DONE_SIZE + LISTEN_DONE_RIGHT * 2.0;
    let text_w = CAP_LISTENING_W - LISTEN_TEXT_LEFT - text_right;
    // Vertically center the text label
    let text_h = CAP_H - 12.0;
    let text_y = (CAP_H - text_h) / 2.0;
    let label = make_wrapping_label(
        mtm,
        "",
        NSRect::new(
            NSPoint::new(LISTEN_TEXT_LEFT, text_y),
            NSSize::new(text_w, text_h),
        ),
        LISTEN_TEXT_FONT,
        &theme.label_color(),
        NSTextAlignment::Left,
        2,
    );
    vfx.addSubview(&label);

    // Done button — solid accent blue circle, centered vertically
    let btn_x = CAP_LISTENING_W - LISTEN_DONE_SIZE - LISTEN_DONE_RIGHT;
    let btn_y = (CAP_H - LISTEN_DONE_SIZE) / 2.0;
    let done_btn = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(
            NSPoint::new(btn_x, btn_y),
            NSSize::new(LISTEN_DONE_SIZE, LISTEN_DONE_SIZE),
        ),
    );
    done_btn.setWantsLayer(true);
    if let Some(layer) = done_btn.layer() {
        layer.setBackgroundColor(Some(&srgb(74, 158, 255, 1.0).CGColor()));
        layer.setCornerRadius(LISTEN_DONE_SIZE / 2.0);
        layer.setShadowColor(Some(&srgb(74, 158, 255, 0.60).CGColor()));
        layer.setShadowRadius(8.0);
        layer.setShadowOpacity(0.35);
        layer.setShadowOffset(NSSize::new(0.0, -2.0));
    }
    vfx.addSubview(&done_btn);

    // Stop-square icon — pure NSView, guaranteed centered
    let sq = 11.0_f64;
    let sq_r = 2.5_f64;
    let sq_x = (LISTEN_DONE_SIZE - sq) / 2.0;
    let sq_y = (LISTEN_DONE_SIZE - sq) / 2.0;
    let icon = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(sq_x, sq_y), NSSize::new(sq, sq)),
    );
    icon.setWantsLayer(true);
    if let Some(layer) = icon.layer() {
        layer.setBackgroundColor(Some(&srgb(255, 255, 255, 1.0).CGColor()));
        layer.setCornerRadius(sq_r);
    }
    done_btn.addSubview(&icon);

    s.listen_text = Some(label);
    s.listen_done_btn = Some(done_btn);
    s.listen_done_icon = Some(icon);
}

fn build_processing_views(s: &mut CapsuleViews, vfx: &NSVisualEffectView, mtm: MainThreadMarker) {
    let total_w = DOT_SIZE * 3.0 + DOT_GAP * 2.0;
    let start_x = (CAP_PROCESSING_W - total_w) / 2.0;
    let cy = CAP_H / 2.0;
    let mut dots = Vec::with_capacity(3);
    for i in 0..3 {
        let x = start_x + (i as f64) * (DOT_SIZE + DOT_GAP);
        let dot = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(
                NSPoint::new(x, cy - DOT_SIZE / 2.0),
                NSSize::new(DOT_SIZE, DOT_SIZE),
            ),
        );
        dot.setWantsLayer(true);
        if let Some(layer) = dot.layer() {
            layer.setBackgroundColor(Some(&srgb(74, 158, 255, 0.85).CGColor()));
            layer.setCornerRadius(DOT_SIZE / 2.0);
            // Per-dot glow
            layer.setShadowColor(Some(&srgb(74, 158, 255, 0.50).CGColor()));
            layer.setShadowRadius(6.0);
            layer.setShadowOpacity(0.30);
            layer.setShadowOffset(NSSize::new(0.0, 0.0));
        }
        vfx.addSubview(&dot);
        dots.push(dot);
    }
    s.proc_dots = dots;
}

fn build_result_views(
    s: &mut CapsuleViews,
    vfx: &NSVisualEffectView,
    mtm: MainThreadMarker,
    cap_w: f64,
    cap_h: f64,
) {
    let theme = Theme::current();
    let text_w = cap_w - RESULT_PAD_X * 2.0 - RESULT_BTN_SIZE - RESULT_BTN_RIGHT;
    let text_h = cap_h - RESULT_PAD_Y * 2.0;
    let label = make_wrapping_label(
        mtm,
        "",
        NSRect::new(
            NSPoint::new(RESULT_PAD_X, RESULT_PAD_Y),
            NSSize::new(text_w, text_h),
        ),
        RESULT_FONT,
        &theme.label_color(),
        NSTextAlignment::Left,
        RESULT_MAX_LINES,
    );
    vfx.addSubview(&label);

    // Continue button — circle, vertically centered on right
    let btn_x = cap_w - RESULT_BTN_SIZE - RESULT_BTN_RIGHT;
    let btn_y = (cap_h - RESULT_BTN_SIZE) / 2.0;
    let btn = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(
            NSPoint::new(btn_x, btn_y),
            NSSize::new(RESULT_BTN_SIZE, RESULT_BTN_SIZE),
        ),
    );
    btn.setWantsLayer(true);
    if let Some(layer) = btn.layer() {
        layer.setBackgroundColor(Some(&srgb(74, 158, 255, 0.15).CGColor()));
        layer.setCornerRadius(RESULT_BTN_SIZE / 2.0);
        layer.setBorderColor(Some(&srgb(74, 158, 255, 0.25).CGColor()));
        layer.setBorderWidth(0.5);
        layer.setShadowColor(Some(&srgb(0, 0, 0, 0.25).CGColor()));
        layer.setShadowRadius(4.0);
        layer.setShadowOpacity(0.12);
        layer.setShadowOffset(NSSize::new(0.0, -1.0));
    }
    vfx.addSubview(&btn);

    // 3 mini wave bars inside button (represents "continue speaking")
    let mini_bar_w = 2.0_f64;
    let mini_bar_gap = 3.0_f64;
    let mini_bar_heights = [7.0_f64, 11.0, 7.0];
    let mini_total = mini_bar_w * 3.0 + mini_bar_gap * 2.0;
    let mini_start_x = (RESULT_BTN_SIZE - mini_total) / 2.0;
    let btn_cy = RESULT_BTN_SIZE / 2.0;
    for (i, bh) in mini_bar_heights.iter().enumerate() {
        let bx = mini_start_x + (i as f64) * (mini_bar_w + mini_bar_gap);
        let mini_bar = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(
                NSPoint::new(bx, btn_cy - bh / 2.0),
                NSSize::new(mini_bar_w, *bh),
            ),
        );
        mini_bar.setWantsLayer(true);
        if let Some(layer) = mini_bar.layer() {
            layer.setBackgroundColor(Some(&srgb(74, 158, 255, 0.90).CGColor()));
            layer.setCornerRadius(mini_bar_w / 2.0);
        }
        btn.addSubview(&mini_bar);
    }

    // Invisible placeholder (kept for struct consistency)
    let icon = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
    );
    btn.addSubview(&icon);

    s.result_text = Some(label);
    s.result_btn = Some(btn);
    s.result_btn_icon = Some(icon);
}

// ── Mode Transitions ────────────────────────────────────────────────────────

fn transition_to_listening(app: &AppHandle) {
    IDLE_WAVE_TOKEN.fetch_add(1, Ordering::Relaxed);
    {
        let mut sh = SHARED.lock().unwrap();
        if sh.mode == Some(CapsuleMode::Listening) {
            return;
        }
        sh.current_speech.clear();
        sh.mode = Some(CapsuleMode::Listening);
    }

    let _ = app.run_on_main_thread(move || {
        UI.with(|s| {
            let mut s = s.borrow_mut();
            if s.panel.is_none() {
                return;
            }
            clear_mode_views(&mut s);
            let mtm = MainThreadMarker::new().expect("main thread");
            let vfx = s.vfx_view.clone();
            if let Some(vfx) = &vfx {
                build_listening_views(&mut s, vfx, mtm);
            }
            // Start content at 0 opacity for fade-in
            set_content_opacity(&s, 0.0);
        });
    });

    morph_capsule_to(app.clone(), CAP_LISTENING_W, CAP_H);
    fade_content_in(app.clone());

    let token = BORDER_ANIM_TOKEN
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    start_listening_border_anim(app.clone(), token);
}

fn transition_to_processing(app: &AppHandle) {
    // Stop border animation
    BORDER_ANIM_TOKEN.fetch_add(1, Ordering::Relaxed);
    WAVE_ACTIVE.store(false, Ordering::Relaxed);
    WAVE_TOKEN.fetch_add(1, Ordering::Relaxed);

    SHARED.lock().unwrap().mode = Some(CapsuleMode::Processing);

    let _ = app.run_on_main_thread(move || {
        UI.with(|s| {
            let mut s = s.borrow_mut();
            if s.panel.is_none() {
                return;
            }
            clear_mode_views(&mut s);

            let mtm = MainThreadMarker::new().expect("main thread");
            let vfx = s.vfx_view.clone();
            if let Some(vfx) = &vfx {
                build_processing_views(&mut s, vfx, mtm);
            }

            // Reset capsule shadow to neutral
            let theme = Theme::current();
            if let Some(capsule) = &s.capsule_view {
                if let Some(layer) = capsule.layer() {
                    let (r, g, b, a) = theme.capsule_shadow();
                    layer.setShadowColor(Some(&srgb(r, g, b, a).CGColor()));
                    layer.setShadowRadius(30.0);
                    layer.setShadowOpacity(theme.capsule_shadow_opacity() as f32);
                    layer.setShadowOffset(NSSize::new(0.0, -10.0));
                    layer.setBorderWidth(0.0);
                }
            }
            // Reset vfx border
            if let Some(vfx) = &s.vfx_view {
                if let Some(layer) = vfx.layer() {
                    let (r, g, b, a) = theme.vfx_border();
                    layer.setBorderColor(Some(&srgb(r, g, b, a).CGColor()));
                    layer.setBorderWidth(0.5);
                }
            }
        });
    });

    morph_capsule_to(app.clone(), CAP_PROCESSING_W, CAP_H);

    // Start dot bounce animation
    let token = DOTS_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    start_dots_animation(app.clone(), token);
}

fn transition_to_result(app: &AppHandle) {
    DOTS_TOKEN.fetch_add(1, Ordering::Relaxed);
    BORDER_ANIM_TOKEN.fetch_add(1, Ordering::Relaxed);

    let text = {
        let mut sh = SHARED.lock().unwrap();
        sh.mode = Some(CapsuleMode::Result);
        sh.result_accumulated.clone()
    };
    let result_h = compute_result_height(&text);

    // Flash the border bright on transition
    let app_flash = app.clone();
    let _ = app.run_on_main_thread(move || {
        UI.with(|s| {
            let s = s.borrow();
            if let Some(capsule) = &s.capsule_view {
                if let Some(layer) = capsule.layer() {
                    layer.setShadowColor(Some(&srgb(74, 158, 255, 0.55).CGColor()));
                    layer.setShadowRadius(40.0);
                    layer.setShadowOpacity(0.50);
                }
            }
            if let Some(vfx) = &s.vfx_view {
                if let Some(layer) = vfx.layer() {
                    layer.setBorderColor(Some(&srgb(74, 158, 255, 0.40).CGColor()));
                    layer.setBorderWidth(1.5);
                }
            }
        });
    });
    // Fade the flash over ~200ms (handled by the result glow anim taking over)

    let text_for_ui = text.clone();
    let _ = app_flash.run_on_main_thread(move || {
        UI.with(|s| {
            let mut s = s.borrow_mut();
            if s.panel.is_none() {
                return;
            }
            clear_mode_views(&mut s);

            let cap_h = compute_result_height(&text_for_ui);
            let mtm = MainThreadMarker::new().expect("main thread");
            let vfx = s.vfx_view.clone();
            if let Some(vfx) = &vfx {
                build_result_views(&mut s, vfx, mtm, CAP_RESULT_W, cap_h);
                if let Some(label) = &s.result_text {
                    label.setStringValue(&NSString::from_str(&text_for_ui));
                }
            }
        });
    });

    morph_capsule_to(app.clone(), CAP_RESULT_W, result_h);

    // Start subtle breathing glow for result mode
    let glow_token = RESULT_GLOW_TOKEN
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    start_result_glow_anim(app.clone(), glow_token);

    // Auto-close timer
    let close_token = RESULT_CLOSE_TOKEN
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    let app_close = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(RESULT_AUTO_CLOSE_SECS)).await;
        if RESULT_CLOSE_TOKEN.load(Ordering::Relaxed) == close_token {
            transition_to_idle(&app_close);
        }
    });
}

/// Update result text and smoothly animate capsule to fit new content.
fn update_result_live(app: &AppHandle, text: String, cap_w: f64, cap_h: f64) {
    // Update text + internal content frames on main thread
    let _ = app.run_on_main_thread(move || {
        UI.with(|s| {
            let s = s.borrow();

            // Update text label
            if let Some(label) = &s.result_text {
                label.setStringValue(&NSString::from_str(&text));
                let text_w = cap_w - RESULT_PAD_X * 2.0 - RESULT_BTN_SIZE - RESULT_BTN_RIGHT;
                let text_h = cap_h - RESULT_PAD_Y * 2.0;
                label.setFrame(NSRect::new(
                    NSPoint::new(RESULT_PAD_X, RESULT_PAD_Y),
                    NSSize::new(text_w, text_h),
                ));
            }

            // Reposition continue button
            if let Some(btn) = &s.result_btn {
                let btn_x = cap_w - RESULT_BTN_SIZE - RESULT_BTN_RIGHT;
                let btn_y = (cap_h - RESULT_BTN_SIZE) / 2.0;
                btn.setFrame(NSRect::new(
                    NSPoint::new(btn_x, btn_y),
                    NSSize::new(RESULT_BTN_SIZE, RESULT_BTN_SIZE),
                ));
            }
        });
    });

    // Animate the capsule frame via spring physics (reuses morph token)
    morph_capsule_to(app.clone(), cap_w, cap_h);
}

fn transition_to_idle(app: &AppHandle) {
    // Cancel all mode animations
    WAVE_ACTIVE.store(false, Ordering::Relaxed);
    WAVE_TOKEN.fetch_add(1, Ordering::Relaxed);
    BORDER_ANIM_TOKEN.fetch_add(1, Ordering::Relaxed);
    DOTS_TOKEN.fetch_add(1, Ordering::Relaxed);
    RESULT_CLOSE_TOKEN.fetch_add(1, Ordering::Relaxed);
    RESULT_GLOW_TOKEN.fetch_add(1, Ordering::Relaxed);
    CONTENT_FADE_TOKEN.fetch_add(1, Ordering::Relaxed);

    {
        let mut sh = SHARED.lock().unwrap();
        sh.mode = Some(CapsuleMode::Idle);
        sh.current_speech.clear();
        sh.result_accumulated.clear();
        sh.conv_id = None;
        sh.agent_listener = None;
    }

    let _ = app.run_on_main_thread(move || {
        UI.with(|s| {
            let mut s = s.borrow_mut();
            if s.panel.is_none() {
                return;
            }

            clear_mode_views(&mut s);

            let mtm = MainThreadMarker::new().expect("main thread");
            let vfx = s.vfx_view.clone();
            if let Some(vfx) = &vfx {
                build_idle_views(&mut s, vfx, mtm);
            }

            // Reset capsule shadow to neutral
            let theme = Theme::current();
            if let Some(capsule) = &s.capsule_view {
                if let Some(layer) = capsule.layer() {
                    let (r, g, b, a) = theme.capsule_shadow();
                    layer.setShadowColor(Some(&srgb(r, g, b, a).CGColor()));
                    layer.setShadowRadius(30.0);
                    layer.setShadowOpacity(theme.capsule_shadow_opacity() as f32);
                    layer.setShadowOffset(NSSize::new(0.0, -10.0));
                    layer.setBorderWidth(0.0);
                }
            }
            // Reset vfx border
            if let Some(vfx) = &s.vfx_view {
                if let Some(layer) = vfx.layer() {
                    let (r, g, b, a) = theme.vfx_border();
                    layer.setBorderColor(Some(&srgb(r, g, b, a).CGColor()));
                    layer.setBorderWidth(0.5);
                }
            }
        });
    });

    morph_capsule_to(app.clone(), CAP_IDLE_W, CAP_H);

    // Restart idle wave animation
    let token = IDLE_WAVE_TOKEN
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    start_idle_wave_anim(app.clone(), token);
}

fn clear_mode_views(s: &mut CapsuleViews) {
    // Remove all sub-views from vfx
    if let Some(vfx) = &s.vfx_view {
        let subviews = vfx.subviews();
        let views: Vec<_> = subviews.iter().collect();
        for sv in views.iter().rev() {
            sv.removeFromSuperview();
        }
        // Re-add the persistent dark overlay
        if let Some(bg) = &s.bg_overlay {
            vfx.addSubview(bg);
        }
    }
    s.idle_logo_halo = None;
    s.idle_logo = None;
    s.idle_mic_capsule = None;
    s.idle_separator = None;
    s.idle_wave_bars.clear();
    s.listen_text = None;
    s.listen_done_btn = None;
    s.listen_done_icon = None;
    s.proc_dots.clear();
    s.result_text = None;
    s.result_btn = None;
    s.result_btn_icon = None;
}

// ── Morph Animation (Spring Physics) ─────────────────────────────────────────

fn morph_capsule_to(app: AppHandle, target_w: f64, target_h: f64) {
    let token = MORPH_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        // Capture current frame ON THE MAIN THREAD
        let (tx, rx) = std::sync::mpsc::sync_channel::<(f64, f64, f64, f64)>(1);
        let tw = target_w;
        let th = target_h;
        let _ = app.run_on_main_thread(move || {
            let result = UI.with(|s| {
                let s = s.borrow();
                if let Some(panel) = &s.panel {
                    let f = panel.frame();
                    let right = f.origin.x + f.size.width;
                    (f.size.width, f.size.height, right, f.origin.y)
                } else {
                    (tw, th, 0.0, 0.0)
                }
            });
            let _ = tx.send(result);
        });
        let (start_w, start_h, right_edge, bottom_y) =
            rx.recv().unwrap_or((target_w, target_h, 0.0, 0.0));

        let mut sw = Spring::new(start_w);
        let mut sh = Spring::new(start_h);
        sw.set_target(target_w);
        sh.set_target(target_h);

        // Inject a small initial velocity kick for that "punchy" feel
        let dw = target_w - start_w;
        let dh = target_h - start_h;
        sw.vel = dw * 2.5; // velocity boost in direction of travel
        sh.vel = dh * 2.5;

        let max_frames = 90u64; // safety cap ~1.5s
        for _ in 0..max_frames {
            if MORPH_TOKEN.load(Ordering::Relaxed) != token {
                return;
            }
            let w_moving = sw.tick();
            let h_moving = sh.tick();
            if !w_moving && !h_moving {
                break;
            }
            let w = sw.pos;
            let h = sh.pos;
            let new_x = right_edge - w;

            let _ = app.run_on_main_thread(move || {
                apply_capsule_frame(w, h, new_x, bottom_y);
            });
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
        // Snap to exact target on final frame
        let final_x = right_edge - target_w;
        let _ = app.run_on_main_thread(move || {
            apply_capsule_frame(target_w, target_h, final_x, bottom_y);
        });
    });
}

/// Shared helper: set panel + all internal layers to (w, h) at screen position (x, y).
fn apply_capsule_frame(w: f64, h: f64, x: f64, y: f64) {
    UI.with(|s| {
        let s = s.borrow();
        if let Some(panel) = &s.panel {
            panel.setFrame_display(NSRect::new(NSPoint::new(x, y), NSSize::new(w, h)), true);
        }
        let r = cap_radius(h);
        let origin = NSPoint::new(0.0, 0.0);
        let sz = NSSize::new(w, h);
        if let Some(root) = &s.root_view {
            root.setFrame(NSRect::new(origin, sz));
        }
        if let Some(capsule) = &s.capsule_view {
            capsule.setFrame(NSRect::new(origin, sz));
            if let Some(layer) = capsule.layer() {
                layer.setCornerRadius(r);
            }
        }
        if let Some(vfx) = &s.vfx_view {
            vfx.setFrame(NSRect::new(origin, sz));
            if let Some(layer) = vfx.layer() {
                layer.setCornerRadius(r);
            }
        }
        if let Some(bg) = &s.bg_overlay {
            bg.setFrame(NSRect::new(origin, sz));
            if let Some(layer) = bg.layer() {
                layer.setCornerRadius(r);
            }
        }
    });
}

// ── Click Handling ──────────────────────────────────────────────────────────

fn install_event_monitor(s: &mut CapsuleViews, app: AppHandle) {
    if s.event_monitor.is_some() {
        return;
    }

    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            NSEventMask::LeftMouseUp,
            &RcBlock::new(move |event: NonNull<NSEvent>| {
                let window_number = event.as_ref().windowNumber();
                let click_point = event.as_ref().locationInWindow();

                let action = UI.with(|s| {
                    let s = s.borrow();
                    let Some(panel) = &s.panel else {
                        return ClickAction::None;
                    };
                    if panel.windowNumber() != window_number {
                        return ClickAction::None;
                    }

                    let x = click_point.x;
                    let y = click_point.y;
                    let mode = SHARED.lock().unwrap().mode.unwrap_or(CapsuleMode::Idle);
                    let panel_w = panel.frame().size.width;

                    match mode {
                        CapsuleMode::Idle => {
                            // Logo area — compute same layout as build_idle_views
                            let gap = 6.0_f64;
                            let sep_w = 0.5_f64;
                            let content_w = LOGO_SIZE + gap + sep_w + gap + MIC_CAPSULE_W;
                            let logo_x = (panel_w - content_w) / 2.0;
                            let cy = CAP_H / 2.0;
                            let in_logo = x >= logo_x
                                && x <= logo_x + LOGO_SIZE
                                && y >= cy - LOGO_SIZE / 2.0
                                && y <= cy + LOGO_SIZE / 2.0;
                            if in_logo {
                                return ClickAction::OpenMainWindow;
                            }
                            // Mic area — anywhere else in the capsule
                            ClickAction::StartListening
                        }
                        CapsuleMode::Listening => {
                            // Done button area (right side)
                            let btn_x = panel_w - LISTEN_DONE_SIZE - LISTEN_DONE_RIGHT;
                            if x >= btn_x && x <= btn_x + LISTEN_DONE_SIZE {
                                return ClickAction::FinishListening;
                            }
                            ClickAction::None
                        }
                        CapsuleMode::Processing => ClickAction::None,
                        CapsuleMode::Result => {
                            // Continue button — vertically centered on right
                            let panel_h = panel.frame().size.height;
                            let btn_x = panel_w - RESULT_BTN_SIZE - RESULT_BTN_RIGHT;
                            let btn_y = (panel_h - RESULT_BTN_SIZE) / 2.0;
                            if x >= btn_x
                                && x <= btn_x + RESULT_BTN_SIZE
                                && y >= btn_y
                                && y <= btn_y + RESULT_BTN_SIZE
                            {
                                return ClickAction::ContinueAsking;
                            }
                            // Click anywhere else on result opens main window
                            ClickAction::OpenMainWindow
                        }
                    }
                });

                match action {
                    ClickAction::OpenMainWindow => {
                        let _ = tray::show_main_agent_window(app.clone());
                    }
                    ClickAction::StartListening => {
                        handle_mic_click(app.clone());
                    }
                    ClickAction::FinishListening => {
                        handle_done_click(app.clone());
                    }
                    ClickAction::ContinueAsking => {
                        transition_to_idle(&app);
                    }
                    ClickAction::None => {}
                }

                event.as_ptr()
            }),
        )
    };

    s.event_monitor = monitor;
}

#[derive(Clone, Copy)]
enum ClickAction {
    None,
    OpenMainWindow,
    StartListening,
    FinishListening,
    ContinueAsking,
}

fn handle_mic_click(app: AppHandle) {
    if stt::stt_is_running() {
        // Already running — stop and submit
        handle_done_click(app);
    } else {
        {
            let mut sh = SHARED.lock().unwrap();
            sh.stop_requested = false;
            sh.last_final_text.clear();
            sh.current_speech.clear();
        }
        let _ = stt::stt_start_stream(app.clone(), "agent".to_string(), Some(true));
    }
}

fn handle_done_click(_app: AppHandle) {
    SHARED.lock().unwrap().stop_requested = true;
    let _ = stt::stt_stop_stream();
}

// ── Speech Updates ──────────────────────────────────────────────────────────

fn update_listening_text(app: &AppHandle, text: &str) {
    // Update shared state (accessible from any thread)
    SHARED.lock().unwrap().current_speech = text.to_string();

    // Update UI label (must be on main thread)
    let text = text.to_string();
    let _ = app.run_on_main_thread(move || {
        UI.with(|s| {
            let s = s.borrow();
            if let Some(label) = &s.listen_text {
                label.setStringValue(&NSString::from_str(&text));
                // Re-center vertically: measure actual content height,
                // then reposition the label so its text sits at capsule midpoint.
                let fit: NSSize = unsafe { msg_send![label, intrinsicContentSize] };
                let text_h = fit.height.max(18.0).min(CAP_H - 4.0);
                let y = (CAP_H - text_h) / 2.0;
                let cur = label.frame();
                label.setFrame(NSRect::new(
                    NSPoint::new(cur.origin.x, y),
                    NSSize::new(cur.size.width, text_h),
                ));
            }
        });
    });
}

// ── Agent Submission ────────────────────────────────────────────────────────

fn submit_to_agent(app: AppHandle, text: String) {
    // Transition to processing
    transition_to_processing(&app);

    let db = app.state::<Database>();
    let conv_id = uuid_v4();
    let _ = db.agent_create_conversation(&conv_id, "Float Agent");

    // Register stream listener
    let cid = conv_id.clone();
    let app_for_listener = app.clone();
    let listener_id = app.listen(format!("agent_stream:{}", conv_id), move |event| {
        handle_agent_stream(&app_for_listener, &cid, event.payload());
    });

    {
        let mut sh = SHARED.lock().unwrap();
        sh.conv_id = Some(conv_id.clone());
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
        .unwrap_or_default()
        .to_string();

    match event_type.as_str() {
        "token" => {
            let text = parsed
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            let (full_text, should_transition) = {
                let mut sh = SHARED.lock().unwrap();
                sh.result_accumulated.push_str(&text);
                let full = sh.result_accumulated.clone();
                let transition = sh.mode == Some(CapsuleMode::Processing);
                (full, transition)
            };

            if should_transition {
                transition_to_result(app);
            } else {
                // Already in result mode — update text and resize capsule to fit
                let new_h = compute_result_height(&full_text);
                update_result_live(app, full_text, CAP_RESULT_W, new_h);
            }
        }
        "error" => {
            let msg = parsed
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("エラーが発生しました")
                .to_string();
            SHARED.lock().unwrap().result_accumulated = msg;
            transition_to_result(app);
        }
        "done" => {
            // Unlisten
            let lid = SHARED.lock().unwrap().agent_listener.take();
            if let Some(id) = lid {
                app.unlisten(id);
            }
        }
        _ => {}
    }
}

// ── Listening Border Animation ──────────────────────────────────────────────

fn start_listening_border_anim(app: AppHandle, token: u64) {
    tauri::async_runtime::spawn(async move {
        let mut frame: u64 = 0;
        loop {
            if BORDER_ANIM_TOKEN.load(Ordering::Relaxed) != token {
                break;
            }
            let t = frame as f64 * 0.06;
            let _ = app.run_on_main_thread(move || {
                UI.with(|s| {
                    let s = s.borrow();
                    if let Some(capsule) = &s.capsule_view {
                        if let Some(layer) = capsule.layer() {
                            // Multi-harmonic breathing glow
                            let w1 = ((t * 0.7).sin() + 1.0) * 0.5;
                            let w2 = ((t * 1.1 + 0.5).sin() + 1.0) * 0.5;
                            let w3 = ((t * 0.3).sin() + 1.0) * 0.5;
                            let combined = w1 * 0.5 + w2 * 0.3 + w3 * 0.2;

                            // Blue-cyan gradient shift
                            let r = (60.0 + combined * 30.0) as u8;
                            let g = (140.0 + combined * 40.0) as u8;
                            let a = 0.50 + combined * 0.35;

                            let glow_color = srgb(r, g, 255, a);
                            layer.setShadowColor(Some(&glow_color.CGColor()));
                            layer.setShadowRadius(20.0 + combined * 18.0);
                            layer.setShadowOpacity((0.35 + combined * 0.25) as f32);
                            layer.setShadowOffset(NSSize::new(0.0, -4.0 - combined * 4.0));

                            // Animated border glow
                            let border_a = 0.30 + combined * 0.45;
                            let border_color = srgb(
                                (80.0 + combined * 40.0) as u8,
                                (160.0 + combined * 30.0) as u8,
                                255,
                                border_a,
                            );
                            layer.setBorderColor(Some(&border_color.CGColor()));
                            layer.setBorderWidth(1.2 + combined * 0.8);
                        }
                    }
                });
            });
            frame = frame.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });
}

// ── Processing Dots Animation ───────────────────────────────────────────────

fn start_dots_animation(app: AppHandle, token: u64) {
    tauri::async_runtime::spawn(async move {
        let mut frame: u64 = 0;
        loop {
            if DOTS_TOKEN.load(Ordering::Relaxed) != token {
                break;
            }
            let t = frame as f64 * 0.08;
            let _ = app.run_on_main_thread(move || {
                UI.with(|s| {
                    let s = s.borrow();
                    let cy = CAP_H / 2.0;
                    let total_w = DOT_SIZE * 3.0 + DOT_GAP * 2.0;
                    let start_x = (CAP_PROCESSING_W - total_w) / 2.0;

                    for (i, dot) in s.proc_dots.iter().enumerate() {
                        let phase = i as f64 * 0.9;
                        // Elastic bounce
                        let raw = (t + phase).sin();
                        let bounce = raw.max(0.0).powf(0.7) * 8.0;
                        // Size pulse
                        let pulse = 0.85 + (raw + 1.0) * 0.15;
                        let size = DOT_SIZE * pulse;
                        // Opacity wave
                        let alpha = 0.55 + (raw + 1.0) * 0.225;

                        let base_x = start_x + (i as f64) * (DOT_SIZE + DOT_GAP);
                        dot.setFrame(NSRect::new(
                            NSPoint::new(
                                base_x + (DOT_SIZE - size) / 2.0,
                                cy - size / 2.0 + bounce,
                            ),
                            NSSize::new(size, size),
                        ));
                        if let Some(layer) = dot.layer() {
                            layer.setOpacity(alpha as f32);
                            layer.setCornerRadius(size / 2.0);
                            // Dynamic glow intensity
                            let glow = raw.max(0.0);
                            layer.setShadowOpacity((0.20 + glow * 0.35) as f32);
                            layer.setShadowRadius(4.0 + glow * 8.0);
                        }
                    }

                    // Capsule border shimmer sweep
                    if let Some(capsule) = &s.capsule_view {
                        if let Some(layer) = capsule.layer() {
                            let sweep = ((t * 0.4).sin() + 1.0) * 0.5;
                            let border_a = 0.06 + sweep * 0.10;
                            layer.setBorderColor(Some(&srgb(74, 158, 255, border_a).CGColor()));
                            layer.setBorderWidth(0.5 + sweep * 0.5);
                        }
                    }
                });
            });
            frame = frame.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });
}

// ── Content Fade Helpers ────────────────────────────────────────────────────

/// Set layer opacity on all mode-specific content subviews (not the bg_overlay).
fn set_content_opacity(s: &CapsuleViews, opacity: f64) {
    let op = opacity as f32;
    let set = |v: &Retained<NSView>| {
        if let Some(l) = v.layer() {
            l.setOpacity(op);
        }
    };
    let set_tf = |v: &Retained<NSTextField>| {
        if let Some(l) = v.layer() {
            l.setOpacity(op);
        } else {
            // NSTextField may not be layer-backed yet — ensure it is
            v.setWantsLayer(true);
            if let Some(l) = v.layer() {
                l.setOpacity(op);
            }
        }
    };
    for bar in &s.idle_wave_bars {
        set(bar);
    }
    if let Some(v) = &s.idle_logo {
        set(v);
    }
    if let Some(v) = &s.idle_logo_halo {
        set(v);
    }
    if let Some(v) = &s.idle_mic_capsule {
        set(v);
    }
    if let Some(v) = &s.listen_text {
        set_tf(v);
    }
    if let Some(v) = &s.listen_done_btn {
        set(v);
    }
    for dot in &s.proc_dots {
        set(dot);
    }
    if let Some(v) = &s.result_text {
        set_tf(v);
    }
    if let Some(v) = &s.result_btn {
        set(v);
    }
}

/// Fade all mode-specific content from 0→1 over ~18 frames (~300ms).
fn fade_content_in(app: AppHandle) {
    let token = CONTENT_FADE_TOKEN
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        for i in 1..=FADE_IN_FRAMES {
            if CONTENT_FADE_TOKEN.load(Ordering::Relaxed) != token {
                return;
            }
            let alpha = ease_out_quart(i as f64 / FADE_IN_FRAMES as f64);
            let _ = app.run_on_main_thread(move || {
                UI.with(|s| {
                    set_content_opacity(&s.borrow(), alpha);
                });
            });
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });
}

// ── Result Glow Animation ───────────────────────────────────────────────────

fn start_result_glow_anim(app: AppHandle, token: u64) {
    tauri::async_runtime::spawn(async move {
        let mut frame: u64 = 0;
        loop {
            if RESULT_GLOW_TOKEN.load(Ordering::Relaxed) != token {
                break;
            }
            let t = frame as f64 * 0.04;
            let _ = app.run_on_main_thread(move || {
                UI.with(|s| {
                    let s = s.borrow();
                    // Gentle breathing shadow
                    if let Some(capsule) = &s.capsule_view {
                        if let Some(layer) = capsule.layer() {
                            let w1 = ((t * 0.5).sin() + 1.0) * 0.5;
                            let w2 = ((t * 0.8 + 1.0).sin() + 1.0) * 0.5;
                            let combined = w1 * 0.6 + w2 * 0.4;

                            let r = (40.0 + combined * 25.0) as u8;
                            let g = (120.0 + combined * 50.0) as u8;
                            let a = 0.15 + combined * 0.20;
                            layer.setShadowColor(Some(&srgb(r, g, 255, a).CGColor()));
                            layer.setShadowRadius(18.0 + combined * 14.0);
                            layer.setShadowOpacity((0.18 + combined * 0.15) as f32);
                            layer.setShadowOffset(NSSize::new(0.0, -3.0 - combined * 3.0));
                        }
                    }
                    // Subtle border shimmer
                    if let Some(vfx) = &s.vfx_view {
                        if let Some(layer) = vfx.layer() {
                            let shimmer = ((t * 0.6).sin() + 1.0) * 0.5;
                            let border_a = 0.06 + shimmer * 0.08;
                            layer.setBorderColor(Some(&srgb(74, 158, 255, border_a).CGColor()));
                            layer.setBorderWidth(0.5 + shimmer * 0.3);
                        }
                    }
                });
            });
            frame = frame.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });
}

// ── Idle Wave Animation (with hover detection) ─────────────────────────────

fn start_idle_wave_anim(app: AppHandle, token: u64) {
    tauri::async_runtime::spawn(async move {
        let mut frame: u64 = 0;
        loop {
            if IDLE_WAVE_TOKEN.load(Ordering::Relaxed) != token {
                break;
            }
            if SHARED.lock().unwrap().mode != Some(CapsuleMode::Idle) {
                break;
            }
            let f = frame;
            let _ = app.run_on_main_thread(move || {
                UI.with(|s| {
                    let s = s.borrow();
                    if s.panel.is_none() {
                        return;
                    }

                    // Detect hover via mouse position
                    let hovering = if let Some(panel) = &s.panel {
                        let mouse: NSPoint = unsafe {
                            let cls = AnyClass::get(c"NSEvent").unwrap();
                            msg_send![cls, mouseLocation]
                        };
                        let pf = panel.frame();
                        mouse.x >= pf.origin.x
                            && mouse.x <= pf.origin.x + pf.size.width
                            && mouse.y >= pf.origin.y
                            && mouse.y <= pf.origin.y + pf.size.height
                    } else {
                        false
                    };

                    let speed = if hovering { 0.12 } else { 0.05 };
                    let t = f as f64 * speed;
                    let cy = CAP_H / 2.0;

                    // Animate wave bars
                    for (i, bar) in s.idle_wave_bars.iter().enumerate() {
                        let phase = i as f64 * 1.2;
                        let breathe = ((t + phase).sin() + 1.0) * 0.5;
                        let base_h = BAR_IDLE[i.min(BAR_IDLE.len() - 1)];
                        let extra = if hovering { 7.0 } else { 4.0 };
                        let h = base_h + breathe * extra;
                        let alpha = if hovering {
                            0.65 + breathe * 0.35
                        } else {
                            0.45 + breathe * 0.35
                        };
                        bar.setFrame(NSRect::new(
                            NSPoint::new(bar.frame().origin.x, cy - h / 2.0),
                            NSSize::new(BAR_W, h),
                        ));
                        if let Some(layer) = bar.layer() {
                            layer.setOpacity(alpha as f32);
                        }
                    }

                    let theme = Theme::current();
                    // Hover: expand shadow, brighten border
                    if let Some(capsule) = &s.capsule_view {
                        if let Some(layer) = capsule.layer() {
                            if hovering {
                                layer.setShadowColor(Some(&srgb(74, 158, 255, 0.18).CGColor()));
                                layer.setShadowRadius(38.0);
                                layer.setShadowOpacity(0.35);
                            } else {
                                let r = 28.0 + ((t * 0.3).sin() + 1.0) * 1.5;
                                let (sr, sg, sb, sa) = theme.capsule_shadow();
                                layer.setShadowColor(Some(&srgb(sr, sg, sb, sa).CGColor()));
                                layer.setShadowRadius(r);
                                layer.setShadowOpacity(theme.capsule_shadow_opacity() as f32);
                            }
                        }
                    }

                    // Hover: brighten inner border
                    if let Some(vfx) = &s.vfx_view {
                        if let Some(layer) = vfx.layer() {
                            let (r, g, b, a) = if hovering {
                                theme.vfx_border_hover()
                            } else {
                                theme.vfx_border()
                            };
                            layer.setBorderColor(Some(&srgb(r, g, b, a).CGColor()));
                        }
                    }

                    // Hover: logo halo glow
                    if let Some(halo) = &s.idle_logo_halo {
                        if let Some(layer) = halo.layer() {
                            let (r, g, b, a) = if hovering {
                                theme.halo_hover()
                            } else {
                                theme.halo_idle()
                            };
                            layer.setBackgroundColor(Some(&srgb(r, g, b, a).CGColor()));
                        }
                    }
                });
            });
            frame = frame.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });
}

// ── Result Height Computation ───────────────────────────────────────────────

fn compute_result_height(text: &str) -> f64 {
    let text_w = CAP_RESULT_W - RESULT_PAD_X * 2.0 - RESULT_BTN_SIZE - RESULT_BTN_RIGHT;
    let chars_per_line = (text_w / (RESULT_FONT * 0.74)).max(8.0);

    // Count lines: each explicit newline is at least one line,
    // each visual line within a paragraph wraps by char count.
    let mut total_lines = 0.0_f64;
    for paragraph in text.split('\n') {
        let eff = paragraph
            .chars()
            .map(|c| if c.is_ascii() { 1.0 } else { 1.7 })
            .sum::<f64>();
        total_lines += (eff / chars_per_line).ceil().max(1.0);
    }
    let lines = total_lines.max(1.0).min(RESULT_MAX_LINES as f64);
    let text_h = lines * RESULT_LINE_H + RESULT_PAD_Y * 2.0;
    text_h.clamp(CAP_H, CAP_RESULT_MAX_H)
}

// ── Panel / View Construction ───────────────────────────────────────────────

fn base_panel(mtm: MainThreadMarker, rect: NSRect, shadow: bool) -> Retained<NSPanel> {
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
    panel.setHasShadow(shadow);
    panel.setHidesOnDeactivate(false);
    panel.setLevel(NSFloatingWindowLevel);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Transient,
    );
    unsafe { panel.setReleasedWhenClosed(false) };
    panel
}
