use objc2::msg_send;
use objc2::runtime::AnyClass;
use objc2_app_kit::NSColor;
use objc2_foundation::NSString;
use tauri::{AppHandle, Manager};

use super::*;

pub(in crate::macos_subtitle_overlay) fn srgb(r: u8, g: u8, b: u8, a: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        r as f64 / 255.0,
        g as f64 / 255.0,
        b as f64 / 255.0,
        a,
    )
}

pub(in crate::macos_subtitle_overlay) fn is_dark_mode() -> bool {
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

fn app_theme_mode(app: &AppHandle) -> String {
    let state = app.state::<crate::ThemeState>();
    let guard = state.0.lock().unwrap_or_else(|e| e.into_inner());
    guard.clone()
}

/// Resolves the effective dark/light mode for the overlay, honouring the
/// user's main-window preference ("light" / "dark") and falling back to the
/// real system appearance for "system" mode.
pub(in crate::macos_subtitle_overlay) fn effective_is_dark(app: &AppHandle) -> bool {
    match app_theme_mode(app).as_str() {
        "light" => false,
        "dark" => true,
        _ => is_dark_mode(),
    }
}

/// Re-applies all theme-dependent layer/colour properties on the live panel.
/// Safe to call any time after `build_overlay_panel`; no-op if the panel
/// hasn't been constructed yet.
pub(in crate::macos_subtitle_overlay) fn apply_overlay_theme(dark: bool) {
    SYSTEM_IS_DARK.store(dark, Ordering::Relaxed);
    UI.with(|ui| {
        let ui = ui.borrow();
        if let Some(cap) = &ui.capsule_view {
            if let Some(l) = cap.layer() {
                if dark {
                    l.setShadowColor(Some(&srgb(60, 140, 255, 0.55).CGColor()));
                    l.setShadowRadius(32.0);
                    l.setShadowOpacity(0.30);
                } else {
                    l.setShadowColor(Some(&srgb(0, 0, 0, 0.30).CGColor()));
                    l.setShadowRadius(24.0);
                    l.setShadowOpacity(0.18);
                }
            }
        }
        if let Some(vfx) = &ui.vfx_view {
            // Pin the vibrancy view's appearance so the material renders in
            // the chosen mode regardless of the system value (otherwise
            // forcing light/dark from the main window wouldn't fully
            // override the blur tint).
            unsafe {
                let cls = AnyClass::get(c"NSAppearance").unwrap();
                let name = NSString::from_str(if dark {
                    "NSAppearanceNameDarkAqua"
                } else {
                    "NSAppearanceNameAqua"
                });
                let appearance: *mut objc2::runtime::AnyObject =
                    msg_send![cls, appearanceNamed: &*name];
                if !appearance.is_null() {
                    let _: () = msg_send![&**vfx, setAppearance: appearance];
                }
            }
            if let Some(l) = vfx.layer() {
                if dark {
                    l.setBorderColor(Some(&srgb(120, 180, 255, 0.18).CGColor()));
                } else {
                    l.setBorderColor(Some(&srgb(80, 130, 220, 0.14).CGColor()));
                }
            }
        }
        if let Some(bg) = &ui.bg_overlay {
            if let Some(l) = bg.layer() {
                if dark {
                    l.setBackgroundColor(Some(&srgb(10, 10, 13, 0.94).CGColor()));
                } else {
                    l.setBackgroundColor(Some(&srgb(245, 245, 250, 0.91).CGColor()));
                }
            }
        }
        if let Some(label) = &ui.text_label {
            if dark {
                label.setTextColor(Some(&NSColor::whiteColor()));
            } else {
                label.setTextColor(Some(&NSColor::labelColor()));
            }
        }
    });
}

// ── Text width estimation ──────────────────────────────────────────────────────
