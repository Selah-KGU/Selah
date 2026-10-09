use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSEventMask, NSFloatingWindowLevel, NSFont, NSPanel,
    NSScreen, NSTextAlignment, NSTextField, NSView, NSVisualEffectBlendingMode,
    NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use tauri::{AppHandle, Emitter, Manager};

use super::theme::{is_dark_mode, srgb};
use super::*;
use crate::macos_layer_transaction::suppress_implicit_animations;

pub(in crate::macos_subtitle_overlay) fn build_overlay_panel() {
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
pub(in crate::macos_subtitle_overlay) fn install_click_monitor(app: AppHandle) {
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

pub(in crate::macos_subtitle_overlay) fn apply_frame(w: f64, cx: f64, bottom_y: f64) {
    UI.with(|ui| {
        let ui = ui.borrow();
        let x = cx - w / 2.0;
        if let Some(panel) = &ui.panel {
            // Let AppKit display the updated backing views at its next draw
            // opportunity, after their frames below have been updated together.
            panel.setFrame_display(
                NSRect::new(NSPoint::new(x, bottom_y), NSSize::new(w, SUB_H)),
                false,
            );
        }
        suppress_implicit_animations(|| {
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
    });
}

// ── Spring morph ───────────────────────────────────────────────────────────────
