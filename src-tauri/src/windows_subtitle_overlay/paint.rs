//! Cached GDI objects, text measurement, and overlay painting.

use super::*;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, GetDC,
    GetStockObject, GetTextExtentPoint32W, ReleaseDC, RoundRect, SelectObject, SetBkMode,
    SetDCPenColor, SetTextColor, DEFAULT_CHARSET, DEFAULT_PITCH, DEFAULT_QUALITY, DT_CENTER,
    DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FF_DONTCARE, FW_BOLD, HGDIOBJ,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, TRANSPARENT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DestroyWindow, PostQuitMessage, MA_NOACTIVATE, WM_CLOSE, WM_DESTROY,
    WM_ERASEBKGND, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_PAINT,
};

fn create_overlay_font() -> HGDIOBJ {
    let font_name = wide_null("Segoe UI");
    unsafe {
        CreateFontW(
            -SUB_FONT_PX,
            0,
            0,
            0,
            FW_BOLD as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            0,
            DEFAULT_QUALITY as u32,
            (DEFAULT_PITCH | FF_DONTCARE) as u32,
            font_name.as_ptr(),
        ) as HGDIOBJ
    }
}

// Returns the process-lifetime cached font, creating it on first call.
// Safe to call from any thread; GDI font handles are process-global.
fn get_overlay_font() -> HGDIOBJ {
    *CACHED_FONT_HANDLE.get_or_init(|| create_overlay_font() as isize) as HGDIOBJ
}

// Returns a cached solid brush for `color` on the calling thread.
// Only valid on the overlay Win32 thread (where WM_PAINT runs).
// The stale brush is deleted and recreated when the color changes (theme switch).
unsafe fn get_bg_brush(color: u32) -> HGDIOBJ {
    TL_BG_BRUSH.with(|cell| {
        let (cached_color, cached_handle) = cell.get();
        if cached_color == color && cached_handle != 0 {
            return cached_handle as HGDIOBJ;
        }
        if cached_handle != 0 {
            DeleteObject(cached_handle as HGDIOBJ);
        }
        let new_handle = CreateSolidBrush(color) as usize;
        cell.set((color, new_handle));
        new_handle as HGDIOBJ
    })
}

pub(super) fn estimate_text_w(text: &str) -> i32 {
    let text_wide = wide_null(text);
    unsafe {
        let hdc = GetDC(null_mut());
        if !hdc.is_null() {
            let font = get_overlay_font();
            let old_font = SelectObject(hdc, font);
            let mut size = SIZE::default();
            let measured = GetTextExtentPoint32W(
                hdc,
                text_wide.as_ptr(),
                text_wide.len().saturating_sub(1) as i32,
                &mut size,
            );
            SelectObject(hdc, old_font);
            // Font is cached; do not DeleteObject.
            let _ = ReleaseDC(null_mut(), hdc);
            if measured != 0 {
                return (size.cx + SUB_PAD_X * 2).clamp(SUB_MIN_W, SUB_MAX_W);
            }
        }
    }

    let fallback_width: f64 = text
        .chars()
        .map(|c| if (c as u32) > 0x2E7F { 16.5 } else { 8.8 })
        .sum();
    ((fallback_width + (SUB_PAD_X * 2) as f64).round() as i32).clamp(SUB_MIN_W, SUB_MAX_W)
}

pub(super) unsafe extern "system" fn overlay_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_ERASEBKGND => 1,
        WM_LBUTTONUP => {
            bring_main_window_to_front();
            0
        }
        WM_PAINT => {
            paint_overlay(hwnd);
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            // Release the cached brush that was allocated on this thread.
            TL_BG_BRUSH.with(|cell| {
                let (_, handle) = cell.get();
                if handle != 0 {
                    DeleteObject(handle as HGDIOBJ);
                }
                cell.set((u32::MAX, 0));
            });
            clear_destroyed_window(hwnd as RawHwnd);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn paint_overlay(hwnd: HWND) {
    let Some(state) = window_snapshot() else {
        return;
    };

    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    if hdc.is_null() {
        return;
    }

    let mut rect = RECT {
        left: 0,
        top: 0,
        right: state.width,
        bottom: SUB_H,
    };

    let bg = if state.dark {
        rgb(10, 10, 13)
    } else {
        rgb(245, 245, 250)
    };
    let text_color = if state.dark {
        rgb(255, 255, 255)
    } else {
        rgb(24, 24, 28)
    };

    // All three GDI objects are cached — no allocation per frame.
    let brush = get_bg_brush(bg); // thread-local; recreated only on theme change
    let pen = GetStockObject(DC_PEN_STOCK); // stock object; color set below via SetDCPenColor
    let font = get_overlay_font(); // process-lifetime OnceLock

    let old_brush = SelectObject(hdc, brush);
    let old_pen = SelectObject(hdc, pen);
    let old_font = SelectObject(hdc, font);
    // Match pen color to background so RoundRect fills the 1px outer edge
    // without showing a visible border. NULL_PEN would leave that edge unpainted.
    SetDCPenColor(hdc, bg);

    RoundRect(hdc, 0, 0, state.width, SUB_H, SUB_H, SUB_H);
    SetBkMode(hdc, TRANSPARENT as i32);
    SetTextColor(hdc, text_color);

    rect.left += SUB_PAD_X;
    rect.right -= SUB_PAD_X;
    let text_wide = wide_null(&state.text);
    DrawTextW(
        hdc,
        text_wide.as_ptr(),
        -1,
        &mut rect,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
    );

    SelectObject(hdc, old_font);
    SelectObject(hdc, old_pen);
    SelectObject(hdc, old_brush);
    // Cached objects are not deleted here.

    EndPaint(hwnd, &ps);
}
