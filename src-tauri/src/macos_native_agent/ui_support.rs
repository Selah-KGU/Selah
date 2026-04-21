use super::*;

pub(super) fn visible_frame(mtm: MainThreadMarker) -> NSRect {
    NSScreen::mainScreen(mtm)
        .as_ref()
        .map(|s| s.visibleFrame())
        .unwrap_or_else(|| NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1440.0, 900.0)))
}

pub(super) fn make_wrapping_label(
    mtm: MainThreadMarker,
    text: &str,
    frame: NSRect,
    font_size: f64,
    color: &NSColor,
    align: NSTextAlignment,
    max_lines: usize,
) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    // labelWithString sets translatesAutoresizingMaskIntoConstraints=false (auto-layout).
    // We use frame-based positioning, so re-enable it to honour setFrame.
    label.setTranslatesAutoresizingMaskIntoConstraints(true);
    label.setFrame(frame);
    label.setTextColor(Some(color));
    label.setFont(Some(&NSFont::systemFontOfSize(font_size)));
    label.setAlignment(align);
    label.setMaximumNumberOfLines(max_lines as isize);
    label.setPreferredMaxLayoutWidth(frame.size.width);
    if let Some(cell) = label.cell() {
        use objc2_app_kit::NSLineBreakMode;
        cell.setUsesSingleLineMode(false);
        cell.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    }
    label
}

// ── Palette / Theme ──────────────────────────────────────────────────────────

/// All colors that differ between dark and light mode.
#[derive(Clone, Copy)]
pub(super) struct Theme {
    pub(super) is_dark: bool,
}

impl Theme {
    pub(super) fn current() -> Self {
        Theme {
            is_dark: SYSTEM_IS_DARK.load(Ordering::Relaxed),
        }
    }

    // Background overlay on top of NSVisualEffectView
    pub(super) fn overlay_bg(&self) -> (u8, u8, u8, f64) {
        if self.is_dark {
            (18, 18, 22, 0.92)
        } else {
            (248, 248, 252, 0.88)
        }
    }

    // Text on labels
    pub(super) fn label_color(&self) -> Retained<NSColor> {
        if self.is_dark {
            NSColor::whiteColor()
        } else {
            NSColor::labelColor()
        }
    }

    // Subtle inner border of vfx
    pub(super) fn vfx_border(&self) -> (u8, u8, u8, f64) {
        if self.is_dark {
            (255, 255, 255, 0.08)
        } else {
            (0, 0, 0, 0.08)
        }
    }

    pub(super) fn vfx_border_hover(&self) -> (u8, u8, u8, f64) {
        if self.is_dark {
            (255, 255, 255, 0.18)
        } else {
            (0, 0, 0, 0.18)
        }
    }

    // Outer shadow of capsule
    pub(super) fn capsule_shadow(&self) -> (u8, u8, u8, f64) {
        if self.is_dark {
            (0, 0, 0, 0.65)
        } else {
            (0, 0, 0, 0.20)
        }
    }

    pub(super) fn capsule_shadow_opacity(&self) -> f64 {
        if self.is_dark {
            0.28
        } else {
            0.12
        }
    }

    // Separator between logo and mic
    pub(super) fn separator_color(&self) -> (u8, u8, u8, f64) {
        if self.is_dark {
            (255, 255, 255, 0.10)
        } else {
            (0, 0, 0, 0.12)
        }
    }

    // Mic capsule background
    pub(super) fn mic_bg(&self) -> (u8, u8, u8, f64) {
        if self.is_dark {
            (255, 255, 255, 0.05)
        } else {
            (0, 0, 0, 0.06)
        }
    }

    // Wave bar color
    pub(super) fn bar_color(&self) -> (u8, u8, u8, f64) {
        if self.is_dark {
            (255, 255, 255, 0.70)
        } else {
            (0, 0, 0, 0.55)
        }
    }

    // Logo halo
    pub(super) fn halo_idle(&self) -> (u8, u8, u8, f64) {
        (74, 158, 255, if self.is_dark { 0.06 } else { 0.10 })
    }
    pub(super) fn halo_hover(&self) -> (u8, u8, u8, f64) {
        (74, 158, 255, if self.is_dark { 0.12 } else { 0.18 })
    }
}

/// Query macOS effective appearance. Must be called from main thread.
pub(super) fn is_dark_mode_macos() -> bool {
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
        let s = {
            // Get UTF-8 C string from the name NSString — does NOT transfer ownership
            let cstr: *const std::os::raw::c_char = msg_send![name, UTF8String];
            if cstr.is_null() {
                return true;
            }
            std::ffi::CStr::from_ptr(cstr)
                .to_string_lossy()
                .into_owned()
        };
        s.contains("Dark")
    }
}

/// Apply the current theme to all persistent layer colors (overlay + vfx border + shadow).
/// Called once after build and again whenever appearance changes.
pub(super) fn apply_theme_to_capsule(s: &CapsuleViews, theme: Theme) {
    if let Some(bg) = &s.bg_overlay {
        let (r, g, b, a) = theme.overlay_bg();
        if let Some(l) = bg.layer() {
            l.setBackgroundColor(Some(&srgb(r, g, b, a).CGColor()));
        }
    }
    if let Some(vfx) = &s.vfx_view {
        let (r, g, b, a) = theme.vfx_border();
        if let Some(l) = vfx.layer() {
            l.setBorderColor(Some(&srgb(r, g, b, a).CGColor()));
        }
    }
    if let Some(capsule) = &s.capsule_view {
        let (r, g, b, a) = theme.capsule_shadow();
        if let Some(l) = capsule.layer() {
            l.setShadowColor(Some(&srgb(r, g, b, a).CGColor()));
            l.setShadowOpacity(theme.capsule_shadow_opacity() as f32);
        }
    }
    // Update mode-specific text colors
    let tc = theme.label_color();
    if let Some(tf) = &s.listen_text {
        tf.setTextColor(Some(&tc));
    }
    if let Some(tf) = &s.result_text {
        tf.setTextColor(Some(&tc));
    }
    // Idle separator
    if let Some(sep) = &s.idle_separator {
        let (r, g, b, a) = theme.separator_color();
        if let Some(l) = sep.layer() {
            l.setBackgroundColor(Some(&srgb(r, g, b, a).CGColor()));
        }
    }
    // Mic capsule
    if let Some(mic) = &s.idle_mic_capsule {
        let (r, g, b, a) = theme.mic_bg();
        if let Some(l) = mic.layer() {
            l.setBackgroundColor(Some(&srgb(r, g, b, a).CGColor()));
        }
    }
    // Wave bars
    for bar in &s.idle_wave_bars {
        let (r, g, b, a) = theme.bar_color();
        if let Some(l) = bar.layer() {
            l.setBackgroundColor(Some(&srgb(r, g, b, a).CGColor()));
        }
    }
}

pub(super) fn srgb(r: u8, g: u8, b: u8, a: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
        a,
    )
}

pub(super) fn load_app_logo() -> Option<Retained<NSImage>> {
    static LOGO_BYTES: &[u8] = include_bytes!("../../icons/icon.png");
    let data = unsafe {
        NSData::dataWithBytes_length(
            LOGO_BYTES.as_ptr() as *const core::ffi::c_void,
            LOGO_BYTES.len(),
        )
    };
    NSImage::initWithData(NSImage::alloc(), &data)
}

pub(super) fn uuid_v4() -> String {
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
