//! Windows リアルタイム字幕浮窗 — ネイティブカプセル版
//!
//! Live 録課モジュールが発行する `live-session-updated` / `live-line-appended`
//! / `stt-partial` を監聴し、最新の転写テキストを画面下部のネイティブ浮窗に
//! 表示します。macOS 版と同様、STT / Agent 本体とは分離された補助 UI です。

#![cfg(target_os = "windows")]

use serde_json::Value;
use std::mem::size_of;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Listener, Manager};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateRoundRectRgn, CreateSolidBrush, DeleteObject, DrawTextW,
    EndPaint, GetDC, GetStockObject, GetTextExtentPoint32W, InvalidateRect, ReleaseDC, RoundRect,
    SelectObject, SetBkMode, SetDCPenColor, SetTextColor, SetWindowRgn, UpdateWindow,
    DEFAULT_CHARSET, DEFAULT_PITCH, DEFAULT_QUALITY, DT_CENTER, DT_END_ELLIPSIS, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, FF_DONTCARE, FW_BOLD, HGDIOBJ, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
    TRANSPARENT,
};
// Win32 stock-object identifier for the per-DC custom-color pen; no allocation needed.
// Set the actual color via SetDCPenColor after selecting it into the DC.
const DC_PEN_STOCK: i32 = 19;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetSystemMetrics, LoadCursorW, PostMessageW, PostQuitMessage, RegisterClassExW,
    SetLayeredWindowAttributes, SetWindowPos, ShowWindow, SystemParametersInfoW, TranslateMessage,
    CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, IDC_ARROW, LWA_ALPHA, MA_NOACTIVATE, MSG, SM_CXSCREEN,
    SM_CYSCREEN, SPI_GETWORKAREA, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE,
    SW_SHOWNOACTIVATE, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_LBUTTONUP, WM_MOUSEACTIVATE,
    WM_PAINT, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};

const SUB_H: i32 = 52;
const SUB_MIN_W: i32 = 180;
const SUB_MAX_W: i32 = 620;
const SUB_PAD_X: i32 = 26;
const SUB_FONT_PX: i32 = 24;
const SUB_MARGIN_BOTTOM: i32 = 64;
const SUB_FADE_DELAY_SECS: u64 = 6;

const ANIM_MS: u64 = 16;
const SPRING_K: f64 = 320.0;
const SPRING_D: f64 = 24.0;
const SPRING_M: f64 = 1.0;
const SPRING_DT: f64 = 0.016;
const SPRING_SETTLE: f64 = 0.25;
const FADE_FRAMES: u64 = 20;

const CLASS_NAME: &str = "SelahSubtitleOverlayWindow";
type RawHwnd = isize;

static HIDE_TOKEN: AtomicU64 = AtomicU64::new(0);
static FADE_TOKEN: AtomicU64 = AtomicU64::new(0);
static MORPH_TOKEN: AtomicU64 = AtomicU64::new(0);
static MORPH_DEBOUNCE_TOKEN: AtomicU64 = AtomicU64::new(0);
static OVERLAY_OPEN: AtomicBool = AtomicBool::new(false);
// True once the overlay hwnd is live; cleared on destroy. Allows callers to skip
// the CREATE_LOCK + WINDOW mutex when the common case (window exists) is true.
static HWND_READY: AtomicBool = AtomicBool::new(false);
// GDI font for the overlay — parameters never change so create once per process.
// Stored as isize because HGDIOBJ (*mut c_void) is not Send/Sync.
static CACHED_FONT_HANDLE: OnceLock<isize> = OnceLock::new();
// Set to true while the overlay thread is being spawned (before hwnd is written).
// Prevents double-spawning when ensure_overlay_window is called concurrently.
static CREATING: AtomicBool = AtomicBool::new(false);
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();
static LAST_PARTIAL_MS: AtomicU64 = AtomicU64::new(0);
const PARTIAL_MIN_INTERVAL_MS: u64 = 120;
const MORPH_DEBOUNCE_MS: u64 = 150;

#[derive(Default)]
struct OverlayWindow {
    hwnd: RawHwnd,
    width: i32,
    center_x: i32,
    top_y: i32,
    alpha: u8,
    text: String,
    dark: bool,
}

#[derive(Default)]
struct SharedState {
    event_listeners: Vec<tauri::EventId>,
}

static WINDOW: LazyLock<Mutex<OverlayWindow>> =
    LazyLock::new(|| Mutex::new(OverlayWindow::default()));
static SHARED: LazyLock<Mutex<SharedState>> = LazyLock::new(|| Mutex::new(SharedState::default()));
static CREATE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

// Per-thread solid-brush cache for the overlay Win32 thread.
// Keyed by COLORREF; invalidated (and old handle deleted) when bg color changes.
// Cleaned up in WM_DESTROY on the same thread.
thread_local! {
    static TL_BG_BRUSH: std::cell::Cell<(u32, usize)> = const { std::cell::Cell::new((u32::MAX, 0)) };
}

#[derive(Clone, Copy)]
struct Spring {
    pos: f64,
    vel: f64,
    target: f64,
}

impl Spring {
    fn new(pos: f64) -> Self {
        Self {
            pos,
            vel: 0.0,
            target: pos,
        }
    }

    fn set_target(&mut self, target: f64) {
        self.target = target;
    }

    fn tick(&mut self) -> bool {
        let dx = self.pos - self.target;
        let accel = (-SPRING_K * dx - SPRING_D * self.vel) / SPRING_M;
        self.vel += accel * SPRING_DT;
        self.pos += self.vel * SPRING_DT;
        dx.abs() > SPRING_SETTLE || self.vel.abs() > SPRING_SETTLE
    }
}

// Lightweight snapshot used by animation paths that don't need the text string.
#[derive(Clone, Copy)]
struct FrameSnapshot {
    width: i32,
    center_x: i32,
    top_y: i32,
    alpha: u8,
}

fn frame_snapshot() -> Option<FrameSnapshot> {
    let state = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
    if state.hwnd == 0 {
        None
    } else {
        Some(FrameSnapshot {
            width: state.width,
            center_x: state.center_x,
            top_y: state.top_y,
            alpha: state.alpha,
        })
    }
}

fn ease_out_quart(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(4)
}

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | ((g as u32) << 8) | ((b as u32) << 16)
}

fn wide_null(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

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

fn estimate_text_w(text: &str) -> i32 {
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

fn prefers_dark(app: &AppHandle) -> bool {
    let theme = app.state::<crate::ThemeState>();
    let mode = theme.0.lock().unwrap_or_else(|e| e.into_inner()).clone();
    match mode.as_str() {
        "light" => false,
        "dark" => true,
        // "system" (or anything unknown) follows the OS apps theme.
        _ => system_apps_use_dark_theme(),
    }
}

/// Reads HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize
/// `AppsUseLightTheme` (DWORD): 0 = dark, 1 = light. Defaults to dark when
/// the value is missing or unreadable, matching the previous behaviour.
fn system_apps_use_dark_theme() -> bool {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE,
        REG_DWORD,
    };

    let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0"
        .encode_utf16()
        .collect();
    let mut hkey: HKEY = std::ptr::null_mut();
    let open = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut hkey,
        )
    };
    if open != ERROR_SUCCESS {
        return true;
    }

    let value_name: Vec<u16> = "AppsUseLightTheme\0".encode_utf16().collect();
    let mut data: u32 = 0;
    let mut data_size: u32 = std::mem::size_of::<u32>() as u32;
    let mut data_type: u32 = 0;
    let result = unsafe {
        RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null_mut(),
            &mut data_type,
            &mut data as *mut u32 as *mut u8,
            &mut data_size,
        )
    };
    let _ = unsafe { RegCloseKey(hkey) };
    if result != ERROR_SUCCESS || data_type != REG_DWORD {
        return true;
    }
    data == 0
}

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

fn hwnd_from_raw(raw: RawHwnd) -> HWND {
    raw as HWND
}

fn window_snapshot() -> Option<OverlayWindow> {
    let state = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
    if state.hwnd == 0 {
        None
    } else {
        Some(OverlayWindow {
            hwnd: state.hwnd,
            width: state.width,
            center_x: state.center_x,
            top_y: state.top_y,
            alpha: state.alpha,
            text: state.text.clone(),
            dark: state.dark,
        })
    }
}

fn set_theme_mode(dark: bool) {
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

fn set_alpha(alpha: u8) {
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

fn apply_frame(width: i32, center_x: i32, top_y: i32) {
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

fn bring_main_window_to_front() {
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

unsafe extern "system" fn overlay_wndproc(
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
            HWND_READY.store(false, Ordering::Release);
            let mut state = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
            if state.hwnd == hwnd as RawHwnd {
                state.hwnd = 0;
                state.width = 0;
                state.alpha = 0;
                state.text.clear();
                OVERLAY_OPEN.store(false, Ordering::Relaxed);
            }
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
            state.dark = dark;
        }
        // hwnd is now visible to other threads; publish HWND_READY before clearing CREATING.
        HWND_READY.store(true, Ordering::Release);
        CREATING.store(false, Ordering::SeqCst);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}

fn ensure_overlay_window(app: &AppHandle) {
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

fn morph_to(target_w: i32) {
    let Some(snapshot) = frame_snapshot() else {
        return;
    };
    let token = MORPH_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        let mut spring = Spring::new(snapshot.width as f64);
        spring.set_target(target_w as f64);
        spring.vel = (target_w - snapshot.width) as f64 * 2.5;

        for _ in 0..90 {
            if MORPH_TOKEN.load(Ordering::Relaxed) != token {
                return;
            }
            if !spring.tick() {
                break;
            }
            apply_frame(spring.pos.round() as i32, snapshot.center_x, snapshot.top_y);
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
        apply_frame(target_w, snapshot.center_x, snapshot.top_y);
    });
}

// For partial updates: debounce the morph so rapid STT partials don't restart the
// spring animation on every word. Text is already displayed; only the width animation
// is delayed until the text stabilizes.
// For final/immediate: cancels any pending debounce and morphs right away.
fn trigger_morph(target_w: i32, debounce: bool) {
    let token = MORPH_DEBOUNCE_TOKEN
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    if debounce {
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(MORPH_DEBOUNCE_MS)).await;
            if MORPH_DEBOUNCE_TOKEN.load(Ordering::Relaxed) != token {
                return;
            }
            morph_to(target_w);
        });
    } else {
        morph_to(target_w);
    }
}

fn show_text(app: &AppHandle, text: String, is_final: bool) {
    HIDE_TOKEN.fetch_add(1, Ordering::Relaxed);

    if !OVERLAY_OPEN.load(Ordering::Relaxed) {
        return;
    }

    ensure_overlay_window(app);

    let target_w = estimate_text_w(&text);
    // Extract hwnd and update text while holding the lock, then release the lock
    // before making cross-thread Win32 calls (SendMessage-based calls while holding
    // a mutex shared with the overlay thread's WndProc would deadlock).
    let hwnd = {
        let mut state = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
        if state.hwnd == 0 {
            return;
        }
        state.text = text;
        state.hwnd
    };
    let hwnd = hwnd_from_raw(hwnd);
    unsafe {
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = InvalidateRect(hwnd, null(), 1);
    }

    // Partials are debounced: the spring only fires after the text has been stable
    // for MORPH_DEBOUNCE_MS, preventing jitter from rapid sequential STT updates.
    // Final transcripts trigger immediately and also cancel any pending debounce.
    trigger_morph(target_w, !is_final);

    let start_alpha = frame_snapshot().map(|s| s.alpha).unwrap_or(0);
    let fade_tok = FADE_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        if start_alpha >= 250 {
            return;
        }
        for i in 0..=FADE_FRAMES {
            if FADE_TOKEN.load(Ordering::Relaxed) != fade_tok {
                return;
            }
            let alpha = start_alpha as f64
                + (255.0 - start_alpha as f64) * ease_out_quart(i as f64 / FADE_FRAMES as f64);
            set_alpha(alpha.round().clamp(0.0, 255.0) as u8);
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });

    if is_final {
        schedule_fade_out(SUB_FADE_DELAY_SECS);
    }
}

fn schedule_fade_out(delay_secs: u64) {
    let token = HIDE_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(delay_secs)).await;
        if HIDE_TOKEN.load(Ordering::Relaxed) != token {
            return;
        }
        let Some(snapshot) = frame_snapshot() else {
            return;
        };

        trigger_morph(SUB_MIN_W, false);

        let fade_tok = FADE_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        for i in (0..=FADE_FRAMES).rev() {
            if FADE_TOKEN.load(Ordering::Relaxed) != fade_tok {
                return;
            }
            let alpha = (255.0 * ease_out_quart(i as f64 / FADE_FRAMES as f64))
                .round()
                .clamp(0.0, 255.0) as u8;
            set_alpha(alpha);
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
        set_alpha(0);
        apply_frame(SUB_MIN_W, snapshot.center_x, snapshot.top_y);
    });
}

pub fn setup(app: &AppHandle) {
    let _ = APP_HANDLE.set(app.clone());

    let app_state = app.clone();
    let lid_state = app.listen("live-session-updated", move |event| {
        if !OVERLAY_OPEN.load(Ordering::Relaxed) {
            return;
        }
        let payload = serde_json::from_str::<Value>(event.payload()).unwrap_or_default();
        let active = payload
            .get("active")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !active {
            schedule_fade_out(SUB_FADE_DELAY_SECS);
        } else {
            ensure_overlay_window(&app_state);
        }
    });

    let app_line = app.clone();
    let lid_line = app.listen("live-line-appended", move |event| {
        if !OVERLAY_OPEN.load(Ordering::Relaxed) {
            return;
        }
        let payload = serde_json::from_str::<Value>(event.payload()).unwrap_or_default();
        let text = payload
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_owned();
        if text.trim().is_empty() {
            return;
        }
        show_text(&app_line, text, true);
    });

    let app_partial = app.clone();
    let lid_partial = app.listen("stt-partial", move |event| {
        if !OVERLAY_OPEN.load(Ordering::Relaxed) {
            return;
        }
        let payload = serde_json::from_str::<Value>(event.payload()).unwrap_or_default();
        if payload.get("caller").and_then(|c| c.as_str()) != Some("live") {
            return;
        }
        let text = payload
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_owned();
        if text.trim().is_empty() {
            return;
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let last = LAST_PARTIAL_MS.load(Ordering::Relaxed);
        if now_ms.saturating_sub(last) < PARTIAL_MIN_INTERVAL_MS {
            return;
        }
        LAST_PARTIAL_MS.store(now_ms, Ordering::Relaxed);
        show_text(&app_partial, text, false);
    });

    let app_theme = app.clone();
    let lid_theme = app.listen("app-theme-changed", move |_event| {
        set_theme_mode(prefers_dark(&app_theme));
    });

    SHARED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .event_listeners = vec![lid_state, lid_line, lid_partial, lid_theme];
}

pub fn open_overlay(app: &AppHandle) -> Result<(), String> {
    OVERLAY_OPEN.store(true, Ordering::Relaxed);
    set_theme_mode(prefers_dark(app));
    ensure_overlay_window(app);
    set_alpha(frame_snapshot().map(|s| s.alpha).unwrap_or(0));
    Ok(())
}

pub fn close_overlay(_app: &AppHandle) -> Result<(), String> {
    OVERLAY_OPEN.store(false, Ordering::Relaxed);
    HWND_READY.store(false, Ordering::Relaxed);
    HIDE_TOKEN.fetch_add(1, Ordering::Relaxed);
    FADE_TOKEN.fetch_add(1, Ordering::Relaxed);
    MORPH_TOKEN.fetch_add(1, Ordering::Relaxed);
    MORPH_DEBOUNCE_TOKEN.fetch_add(1, Ordering::Relaxed);

    let closing_hwnd = {
        let mut state = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
        if state.hwnd == 0 {
            None
        } else {
            let hwnd = state.hwnd;
            state.hwnd = 0;
            state.width = 0;
            state.alpha = 0;
            state.text.clear();
            Some(hwnd)
        }
    };

    if let Some(hwnd) = closing_hwnd {
        let hwnd = hwnd_from_raw(hwnd);
        unsafe {
            ShowWindow(hwnd, SW_HIDE);
            let _ = PostMessageW(hwnd, WM_CLOSE, 0, 0);
        }
    }
    Ok(())
}

pub fn is_open() -> bool {
    OVERLAY_OPEN.load(Ordering::Relaxed) && HWND_READY.load(Ordering::Relaxed)
}
