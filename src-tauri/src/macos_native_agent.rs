#![cfg(target_os = "macos")]

use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSEventMask, NSFloatingWindowLevel, NSFont, NSImage,
    NSPanel, NSScreen, NSTextAlignment, NSTextField, NSView, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use objc2_foundation::{NSData, NSPoint, NSRect, NSSize, NSString};
use serde_json::Value;
use tauri::{AppHandle, Listener, Manager};

use crate::agent;
use crate::db::Database;
use crate::stt;
use crate::tray;

const ORB_WIDTH: f64 = 98.0;
const ORB_HEIGHT: f64 = 44.0;
const ORB_MARGIN_RIGHT: f64 = 24.0;
const ORB_MARGIN_BOTTOM: f64 = 28.0;
const ORB_LOGO_SIZE: f64 = 30.0;
const ORB_LOGO_X: f64 = 7.0;
const ORB_LOGO_HALO_SIZE: f64 = 32.0;
const ORB_MIC_CAPSULE_WIDTH: f64 = 38.0;
const ORB_MIC_CAPSULE_HEIGHT: f64 = 30.0;
const ORB_MIC_CAPSULE_X: f64 = ORB_WIDTH - ORB_MIC_CAPSULE_WIDTH - 6.0;
const ORB_MIC_CAPSULE_Y: f64 = (ORB_HEIGHT - ORB_MIC_CAPSULE_HEIGHT) / 2.0;
const ORB_BAR_WIDTH: f64 = 3.0;
const ORB_BAR_GAP: f64 = 5.0;
const ORB_BAR_IDLE: [f64; 4] = [10.0, 15.0, 7.0, 12.0];

const BUBBLE_MIN_ASSISTANT_HEIGHT: f64 = 40.0;
const BUBBLE_MAX_ASSISTANT_HEIGHT: f64 = 168.0;
const BUBBLE_MARGIN_RIGHT: f64 = 20.0;
const BUBBLE_MARGIN_BOTTOM: f64 = 96.0;
const BUBBLE_STACK_GAP: f64 = 10.0;
const BUBBLE_AUTO_CLOSE_SECS: u64 = 15;

const USER_WIDTH: f64 = 236.0;
const ASSISTANT_WIDTH: f64 = 284.0;
const BUBBLE_PAD_X: f64 = 14.0;
const BUBBLE_PAD_Y: f64 = 9.0;
const BUBBLE_FONT: f64 = 13.0;
const BUBBLE_LINE: f64 = BUBBLE_FONT * 1.44;
const USER_BUBBLE_AUTO_CLOSE_SECS: u64 = 8;

thread_local! {
    static UI_STATE: RefCell<NativeUiState> = RefCell::new(NativeUiState::default());
}

static ORB_WAVE_TOKEN: AtomicU64 = AtomicU64::new(0);
static ORB_WAVE_ACTIVE: AtomicBool = AtomicBool::new(false);
static ORB_HOVER_TOKEN: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
struct NativeUiState {
    orb: Option<Retained<NSPanel>>,
    orb_status: Option<Retained<NSView>>,
    orb_mic_capsule: Option<Retained<NSView>>,
    orb_is_listening: bool,
    orb_mic_hovered: bool,
    orb_wave_bars: Vec<Retained<NSView>>,
    event_monitor: Option<Retained<AnyObject>>,
    stop_requested: bool,
    last_final_text: String,
    bubble_order: Vec<String>,
    bubbles: HashMap<String, BubbleUi>,
}

struct BubbleUi {
    window: Retained<NSPanel>,
    bubble_view: Retained<NSView>,
    text_label: Retained<NSTextField>,
    listener_id: Option<tauri::EventId>,
    width: f64,
    max_lines: usize,
    kind: BubbleKind,
    text: String,
    closing: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BubbleKind {
    UserSpeech,
    AssistantReply,
}

pub fn setup(app: &AppHandle) {
    app.listen("stt-final", move |event| {
        let payload = serde_json::from_str::<Value>(event.payload()).unwrap_or_default();
        if payload.get("caller").and_then(|c| c.as_str()) != Some("agent") {
            return;
        }
        let text = payload.get("text").and_then(|t| t.as_str()).unwrap_or_default().to_owned();
        UI_STATE.with(|state| {
            state.borrow_mut().last_final_text = text;
        });
    });

    let app_handle = app.clone();
    app.listen("stt-state", move |event| {
        let payload = serde_json::from_str::<Value>(event.payload()).unwrap_or_default();
        if payload.get("caller").and_then(|c| c.as_str()) != Some("agent") {
            return;
        }
        let state_name = payload.get("state").and_then(|t| t.as_str()).unwrap_or_default().to_owned();
        let listening = matches!(state_name.as_str(), "initializing" | "listening");
        set_orb_status(&app_handle, listening);

        let pending = UI_STATE.with(|state| {
            let mut state = state.borrow_mut();
            if listening {
                return None;
            }
            let should_submit = state.stop_requested && !state.last_final_text.trim().is_empty();
            let text = if should_submit {
                Some(state.last_final_text.trim().to_string())
            } else {
                None
            };
            state.stop_requested = false;
            state.last_final_text.clear();
            text
        });

        if let Some(text) = pending {
            submit_voice_text(app_handle.clone(), text);
        }
    });

    let app_handle = app.clone();
    app.listen("stt-error", move |_event| {
        UI_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.stop_requested = false;
            state.last_final_text.clear();
        });
        set_orb_status(&app_handle, false);
    });
}

pub fn open_orb(app: &AppHandle) -> Result<(), String> {
    let app_handle = app.clone();
    app.run_on_main_thread(move || {
        UI_STATE.with(|state| {
            let mut state = state.borrow_mut();
            if let Some(panel) = &state.orb {
                panel.orderFrontRegardless();
                panel.makeKeyAndOrderFront(None);
                return;
            }

            let mtm = MainThreadMarker::new().expect("main thread");
            let visible = visible_frame(mtm);
            let rect = NSRect::new(
                NSPoint::new(
                    visible.origin.x + visible.size.width - ORB_WIDTH - ORB_MARGIN_RIGHT,
                    visible.origin.y + ORB_MARGIN_BOTTOM,
                ),
                NSSize::new(ORB_WIDTH, ORB_HEIGHT),
            );

            let panel = base_panel(mtm, rect, true);
            panel.setMovableByWindowBackground(true);
            panel.setAcceptsMouseMovedEvents(true);

            let root =
                NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::new(0.0, 0.0), rect.size));
            root.setWantsLayer(true);
            if let Some(layer) = root.layer() {
                layer.setBackgroundColor(Some(&NSColor::clearColor().CGColor()));
            }

            // Single glass pill — Dictation-indicator style.
            let capsule = NSView::initWithFrame(
                NSView::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(ORB_WIDTH, ORB_HEIGHT)),
            );
            style_surface(
                &capsule,
                &surface_fill(),
                ORB_HEIGHT / 2.0,
                None,
                Some((&shadow_color(0.18), 16.0, 0.14, (0.0, 6.0))),
            );

            // App logo on the left.
            let logo_halo = NSView::initWithFrame(
                NSView::alloc(mtm),
                NSRect::new(
                    NSPoint::new(
                        ORB_LOGO_X - (ORB_LOGO_HALO_SIZE - ORB_LOGO_SIZE) / 2.0,
                        (ORB_HEIGHT - ORB_LOGO_HALO_SIZE) / 2.0,
                    ),
                    NSSize::new(ORB_LOGO_HALO_SIZE, ORB_LOGO_HALO_SIZE),
                ),
            );
            style_surface(
                &logo_halo,
                &surface_fill_emphasis(),
                ORB_LOGO_HALO_SIZE / 2.0,
                None,
                Some((&shadow_color(0.10), 8.0, 0.10, (0.0, 2.0))),
            );
            capsule.addSubview(&logo_halo);

            let logo_view = NSView::initWithFrame(
                NSView::alloc(mtm),
                NSRect::new(
                    NSPoint::new(ORB_LOGO_X, (ORB_HEIGHT - ORB_LOGO_SIZE) / 2.0),
                    NSSize::new(ORB_LOGO_SIZE, ORB_LOGO_SIZE),
                ),
            );
            logo_view.setWantsLayer(true);
            if let Some(layer) = logo_view.layer() {
                layer.setCornerRadius(ORB_LOGO_SIZE / 2.0);
                layer.setMasksToBounds(true);
                if let Some(logo) = load_app_logo() {
                    let obj: &AnyObject = &logo;
                    unsafe { layer.setContents(Some(obj)); }
                }
            }
            capsule.addSubview(&logo_view);

            let mic_capsule = NSView::initWithFrame(
                NSView::alloc(mtm),
                NSRect::new(
                    NSPoint::new(ORB_MIC_CAPSULE_X, ORB_MIC_CAPSULE_Y),
                    NSSize::new(ORB_MIC_CAPSULE_WIDTH, ORB_MIC_CAPSULE_HEIGHT),
                ),
            );
            style_surface(
                &mic_capsule,
                &surface_fill_emphasis(),
                ORB_MIC_CAPSULE_HEIGHT / 2.0,
                None,
                Some((&shadow_color(0.08), 8.0, 0.10, (0.0, 2.0))),
            );
            capsule.addSubview(&mic_capsule);

            // Four-bar waveform on the right, centered inside the mic capsule.
            let bar_count = 4usize;
            let total_bars =
                ORB_BAR_WIDTH * bar_count as f64 + ORB_BAR_GAP * (bar_count as f64 - 1.0);
            let wave_zone_x = ORB_MIC_CAPSULE_X;
            let wave_zone_w = ORB_MIC_CAPSULE_WIDTH;
            let start_x = wave_zone_x + (wave_zone_w - total_bars) / 2.0;
            let cy = ORB_HEIGHT / 2.0;
            let mut wave_bars = Vec::with_capacity(bar_count);
            for (i, h) in ORB_BAR_IDLE.iter().enumerate() {
                let x = start_x + (i as f64) * (ORB_BAR_WIDTH + ORB_BAR_GAP);
                let bar = NSView::initWithFrame(
                    NSView::alloc(mtm),
                    NSRect::new(
                        NSPoint::new(x, cy - h / 2.0),
                        NSSize::new(ORB_BAR_WIDTH, *h),
                    ),
                );
                style_surface(&bar, &glyph_color(), ORB_BAR_WIDTH / 2.0, None, None);
                capsule.addSubview(&bar);
                wave_bars.push(bar);
            }

            root.addSubview(&capsule);
            panel.setContentView(Some(&root));
            panel.orderFrontRegardless();

            state.orb = Some(panel);
            state.orb_status = Some(capsule);
            state.orb_mic_capsule = Some(mic_capsule);
            state.orb_is_listening = false;
            state.orb_mic_hovered = false;
            state.orb_wave_bars = wave_bars;
            install_event_monitor(&mut state, app_handle.clone());
        });
    })
    .map_err(|e| format!("native orb main-thread dispatch failed: {}", e))?;

    let token = ORB_HOVER_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    start_orb_hover_tracking(app.clone(), token);
    Ok(())
}

pub fn close_orb(app: &AppHandle) -> Result<(), String> {
    app.run_on_main_thread(move || {
        UI_STATE.with(|state| {
            let mut state = state.borrow_mut();
            ORB_WAVE_ACTIVE.store(false, Ordering::Relaxed);
            ORB_WAVE_TOKEN.fetch_add(1, Ordering::Relaxed);
            ORB_HOVER_TOKEN.fetch_add(1, Ordering::Relaxed);

            if let Some(panel) = state.orb.take() {
                panel.orderOut(None);
                panel.close();
            }

            state.orb_status = None;
            state.orb_mic_capsule = None;
            state.orb_is_listening = false;
            state.orb_mic_hovered = false;
            state.orb_wave_bars.clear();
            state.stop_requested = false;
            state.last_final_text.clear();
        });
    })
    .map_err(|e| format!("native orb close dispatch failed: {}", e))
}

pub fn open_user_speech_bubble(app: &AppHandle, bubble_id: &str, text: &str) -> Result<(), String> {
    let bubble_id = bubble_id.to_string();
    let text = text.to_string();
    let app_handle = app.clone();
    app.run_on_main_thread(move || {
        UI_STATE.with(|state| {
            let mut state = state.borrow_mut();
            if state.bubbles.contains_key(&bubble_id) {
                return;
            }

            let mtm = MainThreadMarker::new().expect("main thread");
            let bubble_h = bubble_text_height(&text, USER_WIDTH - BUBBLE_PAD_X * 2.0, BUBBLE_FONT, 3)
                .clamp(34.0, 78.0);
            let height = single_bubble_window_height(bubble_h);
            let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(USER_WIDTH + 20.0, height));

            let panel = base_panel(mtm, rect, false);
            panel.setMovableByWindowBackground(true);

            let root =
                NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::new(0.0, 0.0), rect.size));
            root.setWantsLayer(true);
            if let Some(layer) = root.layer() {
                layer.setBackgroundColor(Some(&NSColor::clearColor().CGColor()));
            }

            let bubble_view = NSView::initWithFrame(
                NSView::alloc(mtm),
                NSRect::new(
                    NSPoint::new(10.0, 10.0),
                    NSSize::new(USER_WIDTH, bubble_h),
                ),
            );
            style_surface(
                &bubble_view,
                &bubble_user_fill(),
                18.0,
                None,
                Some((&shadow_color(0.18), 14.0, 0.20, (0.0, 5.0))),
            );
            let text_label = make_wrapping_label(
                mtm,
                &text,
                NSRect::new(
                    NSPoint::new(BUBBLE_PAD_X, BUBBLE_PAD_Y),
                    NSSize::new(USER_WIDTH - BUBBLE_PAD_X * 2.0, bubble_h - BUBBLE_PAD_Y * 2.0),
                ),
                BUBBLE_FONT,
                &bubble_user_ink(),
                NSTextAlignment::Left,
                3,
            );
            bubble_view.addSubview(&text_label);

            root.addSubview(&bubble_view);
            panel.setContentView(Some(&root));
            panel.orderFrontRegardless();

            state.bubble_order.push(bubble_id.clone());
            state.bubbles.insert(
                bubble_id.clone(),
                BubbleUi {
                    window: panel,
                    bubble_view,
                    text_label,
                    listener_id: None,
                    width: USER_WIDTH,
                    max_lines: 3,
                    kind: BubbleKind::UserSpeech,
                    text,
                    closing: false,
                },
            );
            install_event_monitor(&mut state, app_handle.clone());
            reposition_bubbles(&state);
        });
    })
    .map_err(|e| format!("native user bubble main-thread dispatch failed: {}", e))
}

pub fn open_assistant_bubble(app: &AppHandle, conv_id: &str) -> Result<(), String> {
    let conv_id = conv_id.to_string();
    let app_handle = app.clone();
    app.run_on_main_thread(move || {
        UI_STATE.with(|state| {
            let mut state = state.borrow_mut();
            if state.bubbles.contains_key(&conv_id) {
                return;
            }

            let mtm = MainThreadMarker::new().expect("main thread");
            let placeholder = "考えています…";
            let bubble_h = bubble_text_height(
                placeholder,
                ASSISTANT_WIDTH - BUBBLE_PAD_X * 2.0,
                BUBBLE_FONT,
                6,
            )
            .clamp(BUBBLE_MIN_ASSISTANT_HEIGHT, BUBBLE_MAX_ASSISTANT_HEIGHT);
            let height = single_bubble_window_height(bubble_h);
            let rect =
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(ASSISTANT_WIDTH + 20.0, height));

            let panel = base_panel(mtm, rect, false);
            panel.setMovableByWindowBackground(true);

            let root =
                NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::new(0.0, 0.0), rect.size));
            root.setWantsLayer(true);
            if let Some(layer) = root.layer() {
                layer.setBackgroundColor(Some(&NSColor::clearColor().CGColor()));
            }

            let bubble_view = NSView::initWithFrame(
                NSView::alloc(mtm),
                NSRect::new(NSPoint::new(10.0, 10.0), NSSize::new(ASSISTANT_WIDTH, bubble_h)),
            );
            style_surface(
                &bubble_view,
                &bubble_assistant_fill(),
                18.0,
                Some((&bubble_assistant_border(), 0.5)),
                Some((&shadow_color(0.10), 16.0, 0.12, (0.0, 6.0))),
            );
            let text_label = make_wrapping_label(
                mtm,
                placeholder,
                NSRect::new(
                    NSPoint::new(BUBBLE_PAD_X, BUBBLE_PAD_Y),
                    NSSize::new(ASSISTANT_WIDTH - BUBBLE_PAD_X * 2.0, bubble_h - BUBBLE_PAD_Y * 2.0),
                ),
                BUBBLE_FONT,
                &bubble_assistant_ink(),
                NSTextAlignment::Left,
                6,
            );
            bubble_view.addSubview(&text_label);

            root.addSubview(&bubble_view);
            panel.setContentView(Some(&root));
            panel.orderFrontRegardless();

            let conv_for_listener = conv_id.clone();
            let app_for_listener = app_handle.clone();
            let listener_id = app_handle.listen(format!("agent_stream:{}", conv_id), move |event| {
                handle_agent_stream_event(&app_for_listener, &conv_for_listener, event.payload());
            });

            state.bubble_order.push(conv_id.clone());
            state.bubbles.insert(
                conv_id.clone(),
                BubbleUi {
                    window: panel,
                    bubble_view,
                    text_label,
                    listener_id: Some(listener_id),
                    width: ASSISTANT_WIDTH,
                    max_lines: 6,
                    kind: BubbleKind::AssistantReply,
                    text: placeholder.to_string(),
                    closing: false,
                },
            );
            install_event_monitor(&mut state, app_handle.clone());
            reposition_bubbles(&state);
        });
    })
    .map_err(|e| format!("native assistant bubble main-thread dispatch failed: {}", e))
}

fn handle_orb_click(app: AppHandle) {
    if stt::stt_is_running() {
        UI_STATE.with(|state| {
            state.borrow_mut().stop_requested = true;
        });
        let _ = stt::stt_stop_stream();
        set_orb_status(&app, true);
    } else {
        UI_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.stop_requested = false;
            state.last_final_text.clear();
        });
        let _ = stt::stt_start_stream(app.clone(), "agent".to_string(), Some(true));
        set_orb_status(&app, true);
    }
}

fn submit_voice_text(app: AppHandle, text: String) {
    set_orb_status(&app, true);
    let db = app.state::<Database>();
    let conv_id = uuid_v4();
    let speech_bubble_id = format!("speech:{}", conv_id);
    let _ = db.agent_create_conversation(&conv_id, "Float Agent");
    let _ = open_user_speech_bubble(&app, &speech_bubble_id, &text);
    let _ = open_assistant_bubble(&app, &conv_id);
    schedule_bubble_close(
        app.clone(),
        speech_bubble_id,
        Duration::from_secs(USER_BUBBLE_AUTO_CLOSE_SECS),
    );

    tauri::async_runtime::spawn(async move {
        let _ = agent::agent_send(app.clone(), conv_id.clone(), text, Vec::new()).await;
        set_orb_status(&app, false);
    });
}

fn handle_agent_stream_event(app: &AppHandle, conv_id: &str, payload: &str) {
    let parsed = serde_json::from_str::<Value>(payload).unwrap_or(Value::Null);
    let event_type = parsed
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let should_finish = matches!(event_type.as_str(), "done" | "error");
    let event_type_for_ui = event_type.clone();
    let next_text = match event_type.as_str() {
        "token" => parsed
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        "error" => parsed
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Agent error")
            .to_string(),
        _ => String::new(),
    };

    let conv = conv_id.to_string();
    let _ = app.run_on_main_thread(move || {
        UI_STATE.with(|state| {
            let mut state = state.borrow_mut();
            let Some(bubble) = state.bubbles.get_mut(&conv) else {
                return;
            };
            match event_type_for_ui.as_str() {
                "token" => {
                    if bubble.text == "考えています…" {
                        bubble.text.clear();
                    }
                    bubble.text.push_str(&next_text);
                    relayout_bubble(bubble);
                }
                "error" => {
                    bubble.text = format!("問題が発生しました。\n{}", next_text);
                    relayout_bubble(bubble);
                }
                "done" => {}
                _ => {}
            }
            reposition_bubbles(&state);
        });
    });

    if should_finish {
        let listener_id = UI_STATE.with(|state| {
            state
                .borrow()
                .bubbles
                .get(conv_id)
                .and_then(|bubble| bubble.listener_id)
        });
        if let Some(id) = listener_id {
            app.unlisten(id);
        }
        schedule_bubble_close(app.clone(), conv_id.to_string(), Duration::from_secs(BUBBLE_AUTO_CLOSE_SECS));
    }
}

fn schedule_bubble_close(app: AppHandle, conv_id: String, delay: Duration) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        close_bubble(&app, &conv_id);
    });
}

fn close_bubble(app: &AppHandle, conv_id: &str) {
    let conv = conv_id.to_string();
    let app_handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let mut removed_listener = None;
        UI_STATE.with(|state| {
            let mut state = state.borrow_mut();
            let Some(existing) = state.bubbles.get_mut(&conv) else {
                return;
            };
            if existing.closing {
                return;
            }
            existing.closing = true;

            if let Some(bubble) = state.bubbles.remove(&conv) {
                bubble.window.close();
                removed_listener = bubble.listener_id;
            }
            state.bubble_order.retain(|id| id != &conv);
            reposition_bubbles(&state);
        });
        if let Some(listener_id) = removed_listener {
            app_handle.unlisten(listener_id);
        }
    });
}

fn relayout_bubble(bubble: &mut BubbleUi) {
    let bubble_h = bubble_text_height(
        &bubble.text,
        bubble.width - BUBBLE_PAD_X * 2.0,
        BUBBLE_FONT,
        bubble.max_lines,
    );
    let bubble_h = match bubble.kind {
        BubbleKind::UserSpeech => bubble_h.clamp(34.0, 78.0),
        BubbleKind::AssistantReply => {
            bubble_h.clamp(BUBBLE_MIN_ASSISTANT_HEIGHT, BUBBLE_MAX_ASSISTANT_HEIGHT)
        }
    };
    let total_h = single_bubble_window_height(bubble_h);

    bubble.window.setFrame_display(
        NSRect::new(
            bubble.window.frame().origin,
            NSSize::new(bubble.width + 20.0, total_h),
        ),
        true,
    );

    bubble.bubble_view.setFrame(NSRect::new(
        NSPoint::new(10.0, 10.0),
        NSSize::new(bubble.width, bubble_h),
    ));
    bubble.text_label.setFrame(NSRect::new(
        NSPoint::new(BUBBLE_PAD_X, BUBBLE_PAD_Y),
        NSSize::new(bubble.width - BUBBLE_PAD_X * 2.0, bubble_h - BUBBLE_PAD_Y * 2.0),
    ));
    bubble
        .text_label
        .setStringValue(&NSString::from_str(&bubble.text));
}

fn reposition_bubbles(state: &NativeUiState) {
    let mtm = MainThreadMarker::new().expect("main thread");
    let visible = visible_frame(mtm);
    let mut current_y = visible.origin.y + BUBBLE_MARGIN_BOTTOM;

    for conv_id in &state.bubble_order {
        let Some(bubble) = state.bubbles.get(conv_id) else {
            continue;
        };
        let frame = bubble.window.frame();
        bubble.window.setFrameOrigin(NSPoint::new(
            visible.origin.x + visible.size.width - frame.size.width - BUBBLE_MARGIN_RIGHT,
            current_y,
        ));
        bubble.window.orderFrontRegardless();
        current_y += frame.size.height + BUBBLE_STACK_GAP;
    }
}

fn point_in_mic_capsule(point: NSPoint) -> bool {
    point.x >= ORB_MIC_CAPSULE_X
        && point.x <= ORB_MIC_CAPSULE_X + ORB_MIC_CAPSULE_WIDTH
        && point.y >= ORB_MIC_CAPSULE_Y
        && point.y <= ORB_MIC_CAPSULE_Y + ORB_MIC_CAPSULE_HEIGHT
}

fn mouse_in_mic_capsule_screen(panel: &NSPanel) -> bool {
    let mouse = NSEvent::mouseLocation();
    let frame = panel.frame();
    let min_x = frame.origin.x + ORB_MIC_CAPSULE_X;
    let max_x = min_x + ORB_MIC_CAPSULE_WIDTH;
    let min_y = frame.origin.y + ORB_MIC_CAPSULE_Y;
    let max_y = min_y + ORB_MIC_CAPSULE_HEIGHT;
    mouse.x >= min_x && mouse.x <= max_x && mouse.y >= min_y && mouse.y <= max_y
}

fn start_orb_hover_tracking(app: AppHandle, token: u64) {
    tauri::async_runtime::spawn(async move {
        loop {
            if ORB_HOVER_TOKEN.load(Ordering::Relaxed) != token {
                break;
            }

            let _ = app.run_on_main_thread(move || {
                UI_STATE.with(|state| {
                    let mut state = state.borrow_mut();
                    let hovered = state
                        .orb
                        .as_ref()
                        .map(|orb| mouse_in_mic_capsule_screen(orb))
                        .unwrap_or(false);
                    if hovered != state.orb_mic_hovered {
                        state.orb_mic_hovered = hovered;
                        update_mic_capsule_appearance(&state);
                    }
                });
            });

            std::thread::sleep(Duration::from_millis(40));
        }
    });
}

fn update_mic_capsule_appearance(state: &NativeUiState) {
    if let Some(mic_capsule) = state.orb_mic_capsule.as_ref() {
        if let Some(layer) = mic_capsule.layer() {
            let (fill, shadow_radius, shadow_opacity, offset_y) = if state.orb_is_listening {
                (surface_fill_active(), 14.0, 0.22f32, 5.0)
            } else if state.orb_mic_hovered {
                (surface_fill_hover_strong(), 14.0, 0.22f32, 5.0)
            } else {
                (surface_fill_emphasis(), 8.0, 0.10f32, 2.0)
            };
            layer.setBackgroundColor(Some(&fill.CGColor()));
            layer.setBorderWidth(0.0);
            layer.setShadowRadius(shadow_radius);
            layer.setShadowOpacity(shadow_opacity);
            layer.setShadowOffset(NSSize::new(0.0, offset_y));
        }
    }
}

fn install_event_monitor(state: &mut NativeUiState, app: AppHandle) {
    if state.event_monitor.is_some() {
        return;
    }

    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            NSEventMask::LeftMouseUp,
            &RcBlock::new(move |event: NonNull<NSEvent>| {
                let window_number = event.as_ref().windowNumber();
                let click_point = event.as_ref().locationInWindow();
                let clicked = UI_STATE.with(|state| {
                    let state = state.borrow();
                    if let Some(orb) = state.orb.as_ref() {
                        if orb.windowNumber() == window_number {
                            let x = click_point.x;
                            let y = click_point.y;
                            let in_logo = x >= ORB_LOGO_X
                                && x <= ORB_LOGO_X + ORB_LOGO_SIZE
                                && y >= (ORB_HEIGHT - ORB_LOGO_SIZE) / 2.0
                                && y <= (ORB_HEIGHT + ORB_LOGO_SIZE) / 2.0;
                            let in_mic_capsule = point_in_mic_capsule(click_point);
                            if in_logo {
                                return (OrbClickTarget::Logo, None);
                            }
                            if in_mic_capsule {
                                return (OrbClickTarget::Mic, None);
                            }
                            return (OrbClickTarget::Body, None);
                        }
                    }
                    let bubble = state
                        .bubbles
                        .iter()
                        .find_map(|(id, bubble)| (bubble.window.windowNumber() == window_number).then(|| id.clone()));
                    (OrbClickTarget::None, bubble)
                });

                match clicked.0 {
                    OrbClickTarget::Logo => {
                        let _ = tray::show_main_agent_window(app.clone());
                    }
                    OrbClickTarget::Mic => {
                        handle_orb_click(app.clone());
                    }
                    OrbClickTarget::Body | OrbClickTarget::None => {}
                }

                if let Some(conv_id) = clicked.1 {
                    let _ = tray::show_main_agent_window(app.clone());
                    close_bubble(&app, &conv_id);
                }

                event.as_ptr()
            }),
        )
    };

    state.event_monitor = monitor;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OrbClickTarget {
    None,
    Body,
    Logo,
    Mic,
}

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

fn visible_frame(mtm: MainThreadMarker) -> NSRect {
    NSScreen::mainScreen(mtm)
        .as_ref()
        .map(|s| s.visibleFrame())
        .unwrap_or_else(|| NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1440.0, 900.0)))
}

fn make_label(
    mtm: MainThreadMarker,
    text: &str,
    frame: NSRect,
    font_size: f64,
    color: &NSColor,
    align: NSTextAlignment,
) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFrame(frame);
    label.setTextColor(Some(color));
    label.setFont(Some(&NSFont::systemFontOfSize(font_size)));
    label.setAlignment(align);
    label
}

fn make_wrapping_label(
    mtm: MainThreadMarker,
    text: &str,
    frame: NSRect,
    font_size: f64,
    color: &NSColor,
    align: NSTextAlignment,
    max_lines: usize,
) -> Retained<NSTextField> {
    let label = make_label(mtm, text, frame, font_size, color, align);
    label.setMaximumNumberOfLines(max_lines as isize);
    label.setPreferredMaxLayoutWidth(frame.size.width);
    if let Some(cell) = label.cell() {
        use objc2_app_kit::NSLineBreakMode;
        cell.setUsesSingleLineMode(false);
        cell.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    }
    label
}

fn style_surface(
    view: &NSView,
    bg: &NSColor,
    radius: f64,
    border: Option<(&NSColor, f64)>,
    shadow: Option<(&NSColor, f64, f32, (f64, f64))>,
) {
    view.setWantsLayer(true);
    if let Some(layer) = view.layer() {
        layer.setBackgroundColor(Some(&bg.CGColor()));
        layer.setCornerRadius(radius);
        if let Some((border_color, border_width)) = border {
            layer.setBorderColor(Some(&border_color.CGColor()));
            layer.setBorderWidth(border_width);
        }
        if let Some((shadow_color, shadow_radius, shadow_opacity, (x, y))) = shadow {
            layer.setShadowColor(Some(&shadow_color.CGColor()));
            layer.setShadowRadius(shadow_radius);
            layer.setShadowOpacity(shadow_opacity);
            layer.setShadowOffset(NSSize::new(x, y));
        }
    }
}

fn bubble_text_height(text: &str, width: f64, font_size: f64, max_lines: usize) -> f64 {
    let effective_chars = text.chars().map(|c| if c.is_ascii() { 1.0 } else { 1.7 }).sum::<f64>();
    let chars_per_line = (width / (font_size * 0.74)).max(8.0);
    let lines = (effective_chars / chars_per_line).ceil().max(1.0).min(max_lines as f64);
    lines * BUBBLE_LINE + BUBBLE_PAD_Y * 2.0
}

fn single_bubble_window_height(bubble_h: f64) -> f64 {
    10.0 + bubble_h + 10.0
}

fn set_orb_status(app: &AppHandle, listening: bool) {
    let start_animation = if listening {
        ORB_WAVE_ACTIVE.store(true, Ordering::Relaxed);
        Some(ORB_WAVE_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1))
    } else {
        ORB_WAVE_ACTIVE.store(false, Ordering::Relaxed);
        ORB_WAVE_TOKEN.fetch_add(1, Ordering::Relaxed);
        None
    };

    let _ = app.run_on_main_thread(move || {
        UI_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.orb_is_listening = listening;
            if let Some(capsule) = state.orb_status.as_ref() {
                if let Some(layer) = capsule.layer() {
                    let fill = if listening {
                        surface_fill_active()
                    } else {
                        surface_fill()
                    };
                    layer.setBackgroundColor(Some(&fill.CGColor()));
                    layer.setBorderWidth(0.0);
                }
            }
            update_mic_capsule_appearance(&state);
            apply_wave_frame(&state.orb_wave_bars, ORB_BAR_IDLE);
        });
    });

    if let Some(token) = start_animation {
        start_orb_wave_animation(app.clone(), token);
    }
}

fn start_orb_wave_animation(app: AppHandle, token: u64) {
    tauri::async_runtime::spawn(async move {
        let mut frame_index = 0usize;
        loop {
            if !ORB_WAVE_ACTIVE.load(Ordering::Relaxed)
                || ORB_WAVE_TOKEN.load(Ordering::Relaxed) != token
            {
                break;
            }

            let progress = (frame_index as f64) * 0.19;

            let _ = app.run_on_main_thread(move || {
                UI_STATE.with(|state| {
                    let state = state.borrow();
                    apply_orb_animation_frame(state.orb_status.as_ref(), &state.orb_wave_bars, progress);
                });
            });

            frame_index = frame_index.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(42)).await;
        }
    });
}

fn apply_wave_frame(bars: &[Retained<NSView>], heights: [f64; 4]) {
    let cy = ORB_HEIGHT / 2.0;
    for (i, bar) in bars.iter().enumerate() {
        let x = bar.frame().origin.x;
        let h = heights[i];
        bar.setFrame(NSRect::new(
            NSPoint::new(x, cy - h / 2.0),
            NSSize::new(ORB_BAR_WIDTH, h),
        ));
    }
}

fn apply_orb_animation_frame(capsule: Option<&Retained<NSView>>, bars: &[Retained<NSView>], progress: f64) {
    let phases = [0.0f64, 0.95, 1.8, 2.7];
    let mut heights = ORB_BAR_IDLE;

    for (i, h) in heights.iter_mut().enumerate() {
        let wave = ((progress + phases[i]).sin() + 1.0) * 0.5;
        let accent = ((progress * 1.7 + phases[i] * 0.6).sin() + 1.0) * 0.5;
        *h = 7.0 + wave * 9.0 + accent * 2.5;
    }
    apply_wave_frame(bars, heights);

    for (i, bar) in bars.iter().enumerate() {
        if let Some(layer) = bar.layer() {
            let shimmer = 0.62 + (((progress * 1.5) + phases[i]).sin() + 1.0) * 0.19;
            layer.setOpacity(shimmer as f32);
        }
    }

    if let Some(capsule) = capsule {
        if let Some(layer) = capsule.layer() {
            let glow = ((progress * 0.8).sin() + 1.0) * 0.5;
            layer.setShadowRadius(16.0 + glow * 5.0);
            layer.setShadowOpacity((0.14 + glow * 0.10) as f32);
            layer.setShadowOffset(NSSize::new(0.0, 6.0 + glow * 1.5));
        }
    }
}

fn srgb(r: u8, g: u8, b: u8, a: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
        a,
    )
}

// ── Monochrome palette — adapts to light/dark via NSColor dynamic system colors. ──

fn surface_fill() -> Retained<NSColor> {
    NSColor::controlBackgroundColor()
}

fn surface_fill_emphasis() -> Retained<NSColor> {
    NSColor::textBackgroundColor().colorWithAlphaComponent(0.92)
}

fn surface_fill_hover_strong() -> Retained<NSColor> {
    NSColor::selectedContentBackgroundColor().colorWithAlphaComponent(0.92)
}

fn surface_fill_active() -> Retained<NSColor> {
    NSColor::windowBackgroundColor()
}

fn glyph_color() -> Retained<NSColor> {
    NSColor::labelColor()
}

fn bubble_user_fill() -> Retained<NSColor> {
    NSColor::labelColor()
}

fn bubble_user_ink() -> Retained<NSColor> {
    NSColor::textBackgroundColor()
}

fn bubble_assistant_fill() -> Retained<NSColor> {
    NSColor::textBackgroundColor()
}

fn bubble_assistant_ink() -> Retained<NSColor> {
    NSColor::labelColor()
}

fn bubble_assistant_border() -> Retained<NSColor> {
    NSColor::separatorColor()
}

fn shadow_color(alpha: f64) -> Retained<NSColor> {
    srgb(0, 0, 0, alpha)
}

fn load_app_logo() -> Option<Retained<NSImage>> {
    static LOGO_BYTES: &[u8] = include_bytes!("../icons/128x128@2x.png");
    let data = unsafe {
        NSData::dataWithBytes_length(
            LOGO_BYTES.as_ptr() as *const core::ffi::c_void,
            LOGO_BYTES.len(),
        )
    };
    NSImage::initWithData(NSImage::alloc(), &data)
}

fn uuid_v4() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], bytes[8],
        bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    )
}
