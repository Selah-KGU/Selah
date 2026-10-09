//! Overlay window creation, frame placement, and lifetime.

use super::*;
use std::mem::size_of;
use std::ptr::{null, null_mut};
use std::sync::atomic::Ordering;
use tauri::{Emitter, Manager};
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    CreateRoundRectRgn, DeleteObject, InvalidateRect, SetWindowRgn, UpdateWindow,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DispatchMessageW, GetMessageW, GetSystemMetrics, LoadCursorW, PostMessageW,
    RegisterClassExW, SetLayeredWindowAttributes, SetWindowPos, ShowWindow, SystemParametersInfoW,
    TranslateMessage, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, IDC_ARROW, LWA_ALPHA, MSG, SM_CXSCREEN,
    SM_CYSCREEN, SPI_GETWORKAREA, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE,
    SW_SHOWNOACTIVATE, WM_CLOSE, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

fn work_area() -> RECT {
    let mut rect = RECT::default();
    unsafe {
        if SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut rect as *mut RECT).cast(), 0) != 0 {
            return rect;
        }
    }
    RECT {
        left: 0,
        top: 0,
        right: unsafe { GetSystemMetrics(SM_CXSCREEN) },
        bottom: unsafe { GetSystemMetrics(SM_CYSCREEN) },
    }
}

fn apply_window_region(hwnd: HWND, width: i32, height: i32) {
    unsafe {
        let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, height, height);
        if !region.is_null() {
            if SetWindowRgn(hwnd, region, 1) == 0 {
                let _ = DeleteObject(region as _);
            }
        }
    }
}

pub(super) fn hwnd_from_raw(raw: RawHwnd) -> HWND {
    raw as HWND
}

pub(super) struct OverlayPaintSnapshot {
    pub(super) width: i32,
    pub(super) text: String,
    pub(super) dark: bool,
}

pub(super) fn window_snapshot() -> Option<OverlayPaintSnapshot> {
    let state = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
    if state.hwnd == 0 {
        None
    } else {
        Some(OverlayPaintSnapshot {
            width: state.width,
            text: state.text.clone(),
            dark: state.dark,
        })
    }
}

pub(super) fn set_theme_mode(dark: bool) {
    let hwnd = {
        let mut state = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
        state.dark = dark;
        state.hwnd
    };
    if hwnd != 0 {
        unsafe {
            let _ = InvalidateRect(hwnd_from_raw(hwnd), null(), 1);
        }
    }
}

pub(super) fn set_alpha(alpha: u8) {
    let (hwnd, was_hidden) = {
        let mut state = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
        if state.hwnd == 0 {
            return;
        }
        let was_hidden = state.alpha == 0;
        state.alpha = alpha;
        (state.hwnd, was_hidden)
    };
    let hwnd = hwnd_from_raw(hwnd);
    unsafe {
        let _ = SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);
        if alpha == 0 {
            ShowWindow(hwnd, SW_HIDE);
        } else if was_hidden {
            // Only call ShowWindow when transitioning from hidden; redundant on
            // subsequent alpha-only changes during an in-progress fade.
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    }
}

pub(super) fn apply_frame(width: i32, center_x: i32, top_y: i32) {
    let hwnd = {
        let mut state = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
        if state.hwnd == 0 {
            return;
        }
        state.width = width;
        state.center_x = center_x;
        state.top_y = top_y;
        state.hwnd
    };
    let hwnd = hwnd_from_raw(hwnd);
    let x = center_x - width / 2;
    unsafe {
        apply_window_region(hwnd, width, SUB_H);
        let _ = SetWindowPos(
            hwnd,
            null_mut(),
            x,
            top_y,
            width,
            SUB_H,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = InvalidateRect(hwnd, null(), 1);
    }
}

pub(super) fn bring_main_window_to_front() {
    let Some(app) = APP_HANDLE.get() else {
        return;
    };
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
    let _ = app.emit("tray-open-tab", "live");
}

pub(super) fn clear_destroyed_window(hwnd: RawHwnd) -> bool {
    let mut state = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
    if state.hwnd != hwnd {
        return false;
    }

    state.hwnd = 0;
    state.width = 0;
    state.alpha = 0;
    state.text.clear();
    state.displayed_session_id = None;
    HWND_READY.store(false, Ordering::Release);
    OVERLAY_OPEN.store(false, Ordering::Relaxed);
    true
}

// Spawns the Win32 overlay window on a dedicated thread and returns immediately.
// CREATING is set to true before calling; the spawned thread clears it once the
// hwnd is stored (or on failure) so that ensure_overlay_window can retry.
fn spawn_overlay_thread(app: &AppHandle) {
    let dark = prefers_dark(app);

    std::thread::spawn(move || unsafe {
        let class_name = wide_null(CLASS_NAME);
        let title = wide_null("Selah Subtitle Overlay");
        let hinstance = GetModuleHandleW(null());

        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(overlay_wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: null_mut(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: null_mut(),
        };

        let _ = RegisterClassExW(&wc);

        let work = work_area();
        let width = SUB_MIN_W;
        let center_x = (work.left + work.right) / 2;
        let top_y = work.bottom - SUB_H - SUB_MARGIN_BOTTOM;
        let x = center_x - width / 2;

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            x,
            top_y,
            width,
            SUB_H,
            null_mut(),
            null_mut(),
            hinstance,
            null(),
        );

        if hwnd.is_null() {
            log::error!(
                "subtitle overlay: CreateWindowExW failed: {}",
                std::io::Error::last_os_error()
            );
            CREATING.store(false, Ordering::SeqCst);
            return;
        }

        apply_window_region(hwnd, width, SUB_H);
        let _ = SetLayeredWindowAttributes(hwnd, 0, 0, LWA_ALPHA);
        ShowWindow(hwnd, SW_HIDE);
        UpdateWindow(hwnd);

        {
            let mut state = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
            state.hwnd = hwnd as RawHwnd;
            state.width = width;
            state.center_x = center_x;
            state.top_y = top_y;
            state.alpha = 0;
            state.text.clear();
            state.displayed_session_id = None;
            state.dark = dark;
        }
        // hwnd is now visible to other threads; publish HWND_READY before clearing CREATING.
        HWND_READY.store(true, Ordering::Release);
        CREATING.store(false, Ordering::SeqCst);

        // close_overlay may have run while CreateWindowExW was in progress and
        // therefore had no hwnd to close. Re-check after publishing the hwnd so
        // a disabled overlay never leaves a hidden Win32 thread behind.
        if !OVERLAY_OPEN.load(Ordering::Relaxed) {
            ShowWindow(hwnd, SW_HIDE);
            let _ = PostMessageW(hwnd, WM_CLOSE, 0, 0);
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}

pub(super) fn ensure_overlay_window(app: &AppHandle) {
    // Fast path: no locks needed when the window is already live.
    if HWND_READY.load(Ordering::Acquire) {
        return;
    }
    let _create_guard = CREATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Re-check under the lock.
    if HWND_READY.load(Ordering::Relaxed) || CREATING.load(Ordering::SeqCst) {
        return;
    }
    CREATING.store(true, Ordering::SeqCst);
    spawn_overlay_thread(app);
}
