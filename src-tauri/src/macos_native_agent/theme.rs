use super::*;

// ─ Theme ─────────────────────────────────────────────────────────────────────
#[derive(Clone, Copy)]
pub(super) struct Theme {
    pub(super) is_dark: bool,
}

impl Theme {
    pub(super) fn current() -> Self {
        Self {
            is_dark: SYSTEM_IS_DARK.load(Ordering::Relaxed),
        }
    }

    pub(super) fn background(&self) -> (u8, u8, u8, f64) {
        if self.is_dark {
            (22, 20, 28, 0.988)
        } else {
            (250, 248, 253, 0.988)
        }
    }

    pub(super) fn label_color(&self) -> Retained<NSColor> {
        if self.is_dark {
            srgb(246, 246, 250, 0.985)
        } else {
            srgb(30, 24, 54, 0.985)
        }
    }

    pub(super) fn muted_label(&self) -> Retained<NSColor> {
        if self.is_dark {
            srgb(184, 184, 204, 0.72)
        } else {
            srgb(94, 90, 122, 0.70)
        }
    }

    pub(super) fn border_idle(&self) -> (u8, u8, u8, f64) {
        if self.is_dark {
            (167, 132, 232, 0.28)
        } else {
            (161, 98, 222, 0.26)
        }
    }

    pub(super) fn border_result(&self) -> (u8, u8, u8, f64) {
        if self.is_dark {
            (146, 120, 210, 0.22)
        } else {
            (144, 94, 210, 0.20)
        }
    }

    pub(super) fn border_notice(&self) -> (u8, u8, u8, f64) {
        if self.is_dark {
            (232, 168, 108, 0.34)
        } else {
            (214, 132, 60, 0.30)
        }
    }

    pub(super) fn accent(&self) -> Retained<NSColor> {
        if self.is_dark {
            srgb(200, 154, 248, 0.96)
        } else {
            srgb(150, 84, 220, 0.96)
        }
    }

    pub(super) fn code_bg(&self) -> Retained<NSColor> {
        if self.is_dark {
            srgb(255, 255, 255, 0.06)
        } else {
            srgb(0, 0, 0, 0.05)
        }
    }

    pub(super) fn code_fg(&self) -> Retained<NSColor> {
        if self.is_dark {
            srgb(255, 176, 214, 0.95)
        } else {
            srgb(190, 60, 130, 0.96)
        }
    }

    // Four-stop conic gradient for the loading border.
    pub(super) fn gradient_stops(&self) -> [Retained<NSColor>; 5] {
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

    pub(super) fn listen_indicator(&self) -> Retained<NSColor> {
        if self.is_dark {
            srgb(255, 120, 158, 0.92)
        } else {
            srgb(228, 78, 132, 0.94)
        }
    }
}
