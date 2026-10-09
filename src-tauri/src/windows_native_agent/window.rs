use super::*;

pub(in crate::windows_native_agent) fn window_snapshot() -> Option<OverlaySnapshot> {
    let s = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
    if s.hwnd == 0 {
        return None;
    }
    Some(OverlaySnapshot {
        width: s.width,
        height: s.height,
        text: s.text.clone(),
        dark: s.dark,
        mode: s.displayed_mode,
    })
}

pub(in crate::windows_native_agent) fn frame_snapshot() -> Option<FrameSnapshot> {
    let s = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
    if s.hwnd == 0 {
        return None;
    }
    Some(FrameSnapshot {
        width: s.width,
        height: s.height,
        center_x: s.center_x,
        top_y: s.top_y,
        alpha: s.alpha,
    })
}

pub(in crate::windows_native_agent) fn set_alpha(alpha: u8) {
    let (hwnd, was_hidden) = {
        let mut s = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
        if s.hwnd == 0 {
            return;
        }
        let was_hidden = s.alpha == 0;
        s.alpha = alpha;
        (s.hwnd, was_hidden)
    };
    let hwnd = hwnd_from_raw(hwnd);
    unsafe {
        let _ = SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);
        if alpha == 0 {
            ShowWindow(hwnd, SW_HIDE);
        } else if was_hidden {
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    }
}

pub(in crate::windows_native_agent) fn apply_frame(
    width: i32,
    height: i32,
    center_x: i32,
    top_y: i32,
) {
    let hwnd = {
        let mut s = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
        if s.hwnd == 0 {
            return;
        }
        s.width = width;
        s.height = height;
        s.center_x = center_x;
        s.top_y = top_y;
        s.hwnd
    };
    let hwnd = hwnd_from_raw(hwnd);
    let x = center_x - width / 2;
    let r = CORNER_RADIUS * 2;
    unsafe {
        let rgn = CreateRoundRectRgn(0, 0, width + 1, height + 1, r, r);
        if !rgn.is_null() {
            if SetWindowRgn(hwnd, rgn, 1) == 0 {
                let _ = DeleteObject(rgn as _);
            }
        }
        let _ = SetWindowPos(
            hwnd,
            null_mut(),
            x,
            top_y,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = InvalidateRect(hwnd, null(), 1);
    }
}

pub(in crate::windows_native_agent) fn update_text_content(text: String) {
    let hwnd = {
        let mut s = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
        if s.hwnd == 0 {
            return;
        }
        s.text = text;
        s.hwnd
    };
    unsafe {
        let _ = InvalidateRect(hwnd_from_raw(hwnd), null(), 1);
    }
}

pub(in crate::windows_native_agent) fn set_theme_dark(dark: bool) {
    let hwnd = {
        let mut s = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
        s.dark = dark;
        s.hwnd
    };
    if hwnd != 0 {
        unsafe {
            let _ = InvalidateRect(hwnd_from_raw(hwnd), null(), 1);
        }
    }
}

// ─ WndProc ────────────────────────────────────────────────────────────────────
pub(in crate::windows_native_agent) unsafe extern "system" fn overlay_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_ERASEBKGND => 1,
        WM_LBUTTONUP => {
            // Any click: bring main window forward and dismiss the overlay.
            if current_mode() == MODE_RESULT {
                bring_main_window_to_front();
            }
            if let Some(app) = APP_HANDLE.get() {
                close_panel(app, false);
            }
            0
        }
        WM_PAINT => {
            paint_overlay(hwnd);
            0
        }
        WM_AGENT_UI_JOB => {
            run_ui_job(hwnd as RawHwnd, wparam as u64);
            0
        }
        WM_AGENT_SHORTCUT_PRESS => {
            // Only cheap input reservation/stop requests happen here. Models,
            // window creation waits, DB work and rendering run elsewhere.
            // Keep press/release ordering on the hook's owning thread.
            if let Some(app) = APP_HANDLE.get() {
                handle_shortcut_press(app.clone());
            }
            0
        }
        WM_AGENT_SHORTCUT_RELEASE => {
            handle_shortcut_release();
            0
        }
        WM_CLOSE => {
            DESTROYING.store(true, Ordering::SeqCst);
            uninstall_ll_hook();
            OVERLAY_HWND.store(0, Ordering::Relaxed);
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            TL_BG_BRUSH.with(|cell| {
                let (_, h) = cell.get();
                if h != 0 {
                    DeleteObject(h as HGDIOBJ);
                }
                cell.set((u32::MAX, 0));
            });
            clear_destroyed_window(hwnd as RawHwnd);
            drop_ui_jobs(hwnd as RawHwnd);
            DESTROYING.store(false, Ordering::SeqCst);
            if HOOK_VK.load(Ordering::Relaxed) != 0 {
                if let Some(app) = APP_HANDLE.get() {
                    ensure_overlay_window(app);
                }
            }
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
