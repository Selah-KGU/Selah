use super::*;

// ─ WH_KEYBOARD_LL hook ───────────────────────────────────────────────────────
unsafe extern "system" fn ll_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let target_vk = HOOK_VK.load(Ordering::Relaxed);
        if target_vk != 0 {
            let kb = &*(lparam as *const KBDLLHOOKSTRUCT);
            if kb.vkCode == target_vk {
                // When the trigger key is itself a modifier (e.g. Left Alt = 0xA4),
                // GetKeyState for that modifier is already asserted, so comparing
                // cur_mods against target_mods=0 would always fail.  Skip the
                // modifier check entirely for standalone modifier-key triggers.
                const MODIFIER_VKS: &[u32] =
                    &[0x10, 0x11, 0x12, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5];
                let mods_ok = if MODIFIER_VKS.contains(&target_vk) {
                    true
                } else {
                    let target_mods = HOOK_MODS.load(Ordering::Relaxed);
                    // GetKeyState is safe from an LL hook callback (called on the installing thread).
                    let ctrl = (GetKeyState(VK_CONTROL as i32) as u16 & 0x8000) != 0;
                    let shift = (GetKeyState(VK_SHIFT as i32) as u16 & 0x8000) != 0;
                    let alt = (GetKeyState(VK_MENU as i32) as u16 & 0x8000) != 0;
                    let cur_mods = if ctrl { MOD_BIT_CTRL } else { 0 }
                        | if shift { MOD_BIT_SHIFT } else { 0 }
                        | if alt { MOD_BIT_ALT } else { 0 };
                    cur_mods == target_mods
                };
                if mods_ok {
                    let hwnd = OVERLAY_HWND.load(Ordering::Relaxed) as HWND;
                    let w = wparam as u32;
                    if w == WM_KEYDOWN || w == WM_SYSKEYDOWN {
                        PostMessageW(hwnd, WM_AGENT_SHORTCUT_PRESS, 0, 0);
                        // Consume the key so IME doesn't also act on it.
                        return 1;
                    } else if w == WM_KEYUP || w == WM_SYSKEYUP {
                        PostMessageW(hwnd, WM_AGENT_SHORTCUT_RELEASE, 0, 0);
                        return 1;
                    }
                }
            }
        }
    }
    CallNextHookEx(
        HOOK_HANDLE.load(Ordering::Relaxed) as HHOOK,
        code,
        wparam,
        lparam,
    )
}

pub(in crate::windows_native_agent) fn install_ll_hook() {
    unsafe {
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll_hook_proc), null_mut(), 0);
        if hook.is_null() {
            log::error!(
                "agent overlay: SetWindowsHookExW failed: {}",
                std::io::Error::last_os_error()
            );
        } else {
            HOOK_HANDLE.store(hook as isize, Ordering::Relaxed);
            log::info!("agent overlay: LL keyboard hook installed");
        }
    }
}

pub(in crate::windows_native_agent) fn uninstall_ll_hook() {
    let h = HOOK_HANDLE.swap(0, Ordering::Relaxed) as HHOOK;
    if !h.is_null() {
        unsafe {
            UnhookWindowsHookEx(h);
        }
        log::info!("agent overlay: LL keyboard hook removed");
    }
}

// ─ Public API ────────────────────────────────────────────────────────────────
pub fn setup(app: &AppHandle) {
    let _ = APP_HANDLE.set(app.clone());

    let app_theme = app.clone();
    let lid_theme = app.listen("app-theme-changed", move |_| {
        let dark = prefers_dark(&app_theme);
        if let Err(error) = enqueue_ui_job(Box::new(move || set_theme_dark(dark))) {
            log::debug!("Agent theme dispatch skipped: {error}");
        }
    });

    crate::native_agent_events::install(&AGENT, enqueue_view, finish_capture, capture_error);
    AGENT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .event_listeners = vec![lid_theme];
}

pub fn apply_config(app: &AppHandle, config: &NativeAgentConfig) -> Result<(), String> {
    let shortcut = normalize_shortcut(&config.voice_shortcut);
    log::info!(
        "[agent] apply_config: enabled={}, shortcut={}",
        config.voice_shortcut_enabled,
        shortcut
    );

    if config.voice_shortcut_enabled {
        let (mods, vk) = parse_shortcut_to_vk(&shortcut);
        if vk == 0 {
            return Err(format!("cannot parse shortcut key: {shortcut}"));
        }
        log::info!("[agent] shortcut parsed: mods={mods:#03b} vk={vk:#04x}");
        // Atomically update hook targets. The LL hook callback reads these on its next invocation.
        HOOK_MODS.store(mods, Ordering::Relaxed);
        HOOK_VK.store(vk, Ordering::Relaxed);
        // If the overlay window isn't up yet, create it so the hook thread runs.
        ensure_overlay_window(app);
    } else {
        // Disable: zero hook targets (hook becomes no-op), then destroy window.
        HOOK_VK.store(0, Ordering::Relaxed);
        HOOK_MODS.store(0, Ordering::Relaxed);
        AGENT
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .shortcut
            .release();
        close_panel(app, true);
        force_destroy_panel();
    }

    Ok(())
}
