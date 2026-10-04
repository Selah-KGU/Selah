//! Voice shortcut parsing for the Windows overlay.

use super::*;

// ─ Shortcut normalization ─────────────────────────────────────────────────────
pub(super) fn normalize_shortcut(s: &str) -> String {
    let v = s.trim().to_ascii_lowercase();
    if v.is_empty() {
        return "lalt".into();
    }
    if v == "option+space" {
        return "alt+space".into();
    }
    // "fn" has no Win32 equivalent; map to Left Alt which is physical and unambiguous on Windows.
    if v == "fn" {
        return "lalt".into();
    }
    v
}

// ─ Shortcut → VK + modifier parsing ──────────────────────────────────────────
// Returns (mod_bits, vk_code). Returns (0, 0) if unparseable.
pub(super) fn parse_shortcut_to_vk(s: &str) -> (u32, u32) {
    let mut mods: u32 = 0;
    let mut vk: u32 = 0;
    for token in s.split('+') {
        let t = token.trim().to_ascii_lowercase();
        match t.as_str() {
            "ctrl" | "control" => mods |= MOD_BIT_CTRL,
            "shift" => mods |= MOD_BIT_SHIFT,
            "alt" | "option" => mods |= MOD_BIT_ALT,
            _ => {
                let parsed = code_to_vk(&t);
                // Win/Meta and unknown tokens are not supported by the current
                // hook. Reject them instead of silently degrading Win+X to X.
                if parsed == 0 || vk != 0 {
                    return (0, 0);
                }
                vk = parsed;
            }
        }
    }
    if vk == 0 {
        return (0, 0);
    }
    (mods, vk)
}

pub(super) fn code_to_vk(code: &str) -> u32 {
    // e.code names (from KeyboardEvent.code) and friendly aliases
    if code.starts_with("key") && code.len() == 4 {
        let ch = code.chars().nth(3).unwrap_or('?').to_ascii_uppercase();
        return ch as u32; // 'A'–'Z' → VK_A–VK_Z
    }
    if code.starts_with("digit") && code.len() == 6 {
        let ch = code.chars().nth(5).unwrap_or('?');
        return ch as u32; // '0'–'9' → VK_0–VK_9
    }
    if let Some(rest) = code.strip_prefix('f') {
        if let Ok(n) = rest.parse::<u32>() {
            if (1..=24).contains(&n) {
                return 0x6F + n; // VK_F1=0x70 … VK_F24=0x87
            }
        }
    }
    match code {
        "space" => 0x20,
        "enter" | "return" | "numpadenter" => 0x0D,
        "escape" | "esc" => 0x1B,
        "tab" => 0x09,
        "backspace" => 0x08,
        "delete" => 0x2E,
        "insert" => 0x2D,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "arrowup" | "up" => 0x26,
        "arrowdown" | "down" => 0x28,
        "arrowleft" | "left" => 0x25,
        "arrowright" | "right" => 0x27,
        // standalone modifier keys as trigger keys
        "lalt" | "altleft" => 0xA4,       // VK_LMENU
        "ralt" | "altright" => 0xA5,      // VK_RMENU
        "lctrl" | "controlleft" => 0xA2,  // VK_LCONTROL
        "rctrl" | "controlright" => 0xA3, // VK_RCONTROL
        "lshift" | "shiftleft" => 0xA0,   // VK_LSHIFT
        "rshift" | "shiftright" => 0xA1,  // VK_RSHIFT
        // single printable chars (a-z fallback, digit fallback)
        s if s.len() == 1 => {
            let ch = s.chars().next().unwrap().to_ascii_uppercase();
            ch as u32
        }
        _ => 0,
    }
}

#[cfg(test)]
mod shortcut_tests {
    use super::*;

    #[test]
    fn parses_supported_windows_shortcuts() {
        assert_eq!(
            parse_shortcut_to_vk("ctrl+shift+KeyA"),
            (MOD_BIT_CTRL | MOD_BIT_SHIFT, 0x41)
        );
        assert_eq!(parse_shortcut_to_vk("lalt"), (0, 0xA4));
        assert_eq!(parse_shortcut_to_vk("alt+space"), (MOD_BIT_ALT, 0x20));
    }

    #[test]
    fn rejects_unsupported_or_ambiguous_shortcuts() {
        assert_eq!(parse_shortcut_to_vk("win+KeyA"), (0, 0));
        assert_eq!(parse_shortcut_to_vk("cmd+KeyA"), (0, 0));
        assert_eq!(parse_shortcut_to_vk("ctrl+unknown"), (0, 0));
        assert_eq!(parse_shortcut_to_vk("ctrl+KeyA+KeyB"), (0, 0));
    }

    #[test]
    fn destroying_stale_window_does_not_clear_replacement_state() {
        {
            let mut state = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
            state.hwnd = 200;
            state.alpha = 255;
        }
        HWND_READY.store(true, Ordering::Release);

        assert!(!clear_destroyed_window(100));
        assert!(HWND_READY.load(Ordering::Acquire));
        assert_eq!(WINDOW.lock().unwrap_or_else(|e| e.into_inner()).hwnd, 200);

        assert!(clear_destroyed_window(200));
        assert!(!HWND_READY.load(Ordering::Acquire));
    }
}
