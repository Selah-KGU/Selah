//! Gradient border path and color animation.

use super::*;
use crate::main_thread_animation::MainThreadAnimation;

// ─ Border styling & gradient animation ───────────────────────────────────────
pub(in crate::macos_native_agent) fn update_border(app: AppHandle, mode: CapsuleMode, token: u64) {
    let animation = MainThreadAnimation::with_token(&BORDER_TOKEN, token);

    if mode != CapsuleMode::Processing {
        let _ = app.run_on_main_thread(move || {
            if !animation.is_current() {
                return;
            }
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
        if !animation.is_current() {
            return;
        }
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
            if !animation.is_current() {
                break;
            }
            let t = frame as f64 * (ANIM_MS as f64 / 1000.0);
            let angle = (t / GRADIENT_ROTATION_PERIOD_SEC) * std::f64::consts::TAU;
            let ex = 0.5 + 0.5 * angle.cos();
            let ey = 0.5 + 0.5 * angle.sin();
            if !animation
                .frame(&app, move || {
                    UI.with(|ui| {
                        let ui = ui.borrow();
                        if let Some(gradient) = &ui.gradient_border {
                            suppress_implicit_animations(|| {
                                gradient.setStartPoint(NSPoint::new(0.5, 0.5));
                                gradient.setEndPoint(NSPoint::new(ex, ey));
                            });
                        }
                    });
                })
                .await
            {
                break;
            }
            frame = frame.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });
}

pub(super) fn set_gradient_colors(gradient: &CAGradientLayer, theme: &Theme) {
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

pub(super) fn set_gradient_locations(gradient: &CAGradientLayer) {
    let locations = [0.0f64, 0.25, 0.5, 0.75, 1.0];
    let numbers: Vec<Retained<NSNumber>> =
        locations.iter().map(|v| NSNumber::new_f64(*v)).collect();
    let refs: Vec<&NSNumber> = numbers.iter().map(|n| &**n).collect();
    let array: Retained<NSArray<NSNumber>> = NSArray::from_slice(&refs);
    gradient.setLocations(Some(&array));
}

pub(super) fn rounded_rect_path(
    width: f64,
    height: f64,
    radius: f64,
    line_width: f64,
) -> CFRetained<CGPath> {
    // Inset by half the stroke so the ring sits inside the bounds.
    let inset = line_width / 2.0;
    let rect = NSRect::new(
        NSPoint::new(inset, inset),
        NSSize::new(
            (width - inset * 2.0).max(0.0),
            (height - inset * 2.0).max(0.0),
        ),
    );
    let r = radius
        .min(rect.size.width / 2.0)
        .min(rect.size.height / 2.0);
    unsafe { CGPath::with_rounded_rect(rect, r, r, std::ptr::null()) }
}
