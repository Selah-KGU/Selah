#![cfg(target_os = "windows")]

//! Windows ネイティブ Agent ショートカット浮窗
//!
//! macOS の macos_native_agent に相当する Windows 版実装。
//! グローバルショートカット (Ctrl+Space 等) を長押しすると STT が起動し、
//! 音声テキストを Agent に送信して浮窗に結果を表示します。
//!
//! ショートカット操作:
//!   - 押し続け (140ms 以上): 音声入力開始 (Listening)
//!   - 離す: 音声送信 → Agent へ (Processing → Result)
//!   - Listening 中に再押し: 手動で入力終了 (Released 非対応環境向け fallback)

use crate::agent;
use crate::commands::NativeAgentConfig;
use crate::db::Database;
use crate::stt;
use rand::RngCore;
use serde_json::Value;
use std::mem::size_of;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Listener, Manager};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreatePen, CreateRoundRectRgn, CreateSolidBrush, DeleteObject,
    DrawTextW, Ellipse, EndPaint, GetDC, GetStockObject, InvalidateRect, ReleaseDC, RoundRect,
    SelectObject, SetBkMode, SetTextColor, SetWindowRgn, UpdateWindow, DEFAULT_CHARSET,
    DEFAULT_PITCH, DEFAULT_QUALITY, DT_CALCRECT, DT_CENTER, DT_END_ELLIPSIS, DT_NOPREFIX,
    DT_SINGLELINE, DT_TOP, DT_VCENTER, DT_WORDBREAK, FF_DONTCARE, FW_BOLD, FW_NORMAL, HGDIOBJ,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetSystemMetrics, LoadCursorW, PostMessageW, PostQuitMessage, RegisterClassExW,
    SetLayeredWindowAttributes, SetWindowPos, SetWindowsHookExW, ShowWindow, SystemParametersInfoW,
    TranslateMessage, UnhookWindowsHookEx, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, HHOOK, IDC_ARROW,
    KBDLLHOOKSTRUCT, LWA_ALPHA, MA_NOACTIVATE, MSG, SM_CXSCREEN, SM_CYSCREEN, SPI_GETWORKAREA,
    SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNOACTIVATE, WH_KEYBOARD_LL,
    WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONUP, WM_MOUSEACTIVATE,
    WM_PAINT, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_USER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

// ─ Window class ───────────────────────────────────────────────────────────────
const CLASS_NAME: &str = "SelahAgentOverlayWindow";
type RawHwnd = isize;

// ─ Dimensions ─────────────────────────────────────────────────────────────────
const LISTEN_W: i32 = 540;
const LISTEN_H: i32 = 76;
const PROCESS_W: i32 = 124;
const PROCESS_H: i32 = 52;
const RESULT_W: i32 = 540;
const RESULT_MIN_H: i32 = 100;
const RESULT_MAX_H: i32 = 360;
const NOTICE_W: i32 = 460;
const NOTICE_H: i32 = 60;
const CORNER_RADIUS: i32 = 22;
const TOP_MARGIN: i32 = 16;
const PAD_X: i32 = 28;
const LISTEN_INDICATOR_SIZE: i32 = 8;
const DOT_SIZE: i32 = 7;
const DOT_GAP: i32 = 10;
const RESULT_PAD_X: i32 = 24;
const RESULT_PAD_Y: i32 = 18;

// ─ Font sizes ─────────────────────────────────────────────────────────────────
const LISTEN_FONT_PX: i32 = 18;
const RESULT_FONT_PX: i32 = 14;
const NOTICE_FONT_PX: i32 = 14;

// ─ Timing ─────────────────────────────────────────────────────────────────────
const RESULT_AUTO_CLOSE_SECS: u64 = 14;
const NOTICE_AUTO_CLOSE_MS: u64 = 1800;
const DOTS_PERIOD_MS: u64 = 420;
const ANIM_MS: u64 = 16;
const FADE_FRAMES: u64 = 14;

// ─ Spring ─────────────────────────────────────────────────────────────────────
const SPRING_K: f64 = 260.0;
const SPRING_D: f64 = 33.0;
const SPRING_M: f64 = 1.0;
const SPRING_DT: f64 = 0.016;
const SPRING_SETTLE: f64 = 0.25;

// ─ Mode identifiers ───────────────────────────────────────────────────────────
const MODE_NONE: i32 = -1;
const MODE_LISTENING: i32 = 0;
const MODE_PROCESSING: i32 = 1;
const MODE_RESULT: i32 = 2;
const MODE_NOTICE: i32 = 3;

const MUTED_TEXT: &str = "話してください";
const NULL_PEN_STOCK: i32 = 8; // GetStockObject(NULL_PEN)

// ─ Custom window message ──────────────────────────────────────────────────────
const WM_AGENT_SHORTCUT_PRESS: u32 = WM_USER + 50;
const WM_AGENT_SHORTCUT_RELEASE: u32 = WM_USER + 51;

// ─ Atomic state ───────────────────────────────────────────────────────────────
static HWND_READY: AtomicBool = AtomicBool::new(false);
static CREATING: AtomicBool = AtomicBool::new(false);
static CURRENT_MODE: AtomicI32 = AtomicI32::new(MODE_NONE);
static MORPH_TOKEN: AtomicU64 = AtomicU64::new(0);
static FADE_TOKEN: AtomicU64 = AtomicU64::new(0);
static DOTS_TOKEN: AtomicU64 = AtomicU64::new(0);
static DOTS_ACTIVE: AtomicI32 = AtomicI32::new(-1);
static SHORTCUT_ARM_TOKEN: AtomicU64 = AtomicU64::new(0); // reset token for stop_listening
static AUTO_CLOSE_TOKEN: AtomicU64 = AtomicU64::new(0);

// ─ LL keyboard hook state ─────────────────────────────────────────────────────
// Shortcut as modifier bitmask + VK code (0 = disabled).
// Written by apply_config; read lock-free from the hook callback.
static HOOK_VK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static HOOK_MODS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
// HHOOK handle (isize). Written/read on the overlay thread only.
static HOOK_HANDLE: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
// Direct HWND copy for PostMessageW from the hook callback (no lock needed).
static OVERLAY_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
// Modifier bitmask bits
const MOD_BIT_CTRL: u32 = 1;
const MOD_BIT_SHIFT: u32 = 2;
const MOD_BIT_ALT: u32 = 4;

// ─ Process-global font handles (created once, never freed) ────────────────────
static CACHED_LISTEN_FONT: OnceLock<isize> = OnceLock::new();
static CACHED_RESULT_FONT: OnceLock<isize> = OnceLock::new();
static CACHED_NOTICE_FONT: OnceLock<isize> = OnceLock::new();

static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

// ─ Window render state ────────────────────────────────────────────────────────
struct OverlayWindow {
    hwnd: RawHwnd,
    width: i32,
    height: i32,
    center_x: i32,
    top_y: i32,
    alpha: u8,
    text: String,
    dark: bool,
}

impl Default for OverlayWindow {
    fn default() -> Self {
        Self {
            hwnd: 0,
            width: LISTEN_W,
            height: LISTEN_H,
            center_x: 0,
            top_y: 0,
            alpha: 0,
            text: String::new(),
            dark: true,
        }
    }
}

// ─ Agent / STT logic state ────────────────────────────────────────────────────
#[derive(Default)]
struct AgentState {
    stop_requested: bool,
    finals_accumulated: String,
    current_speech: String,
    agent_listener: Option<tauri::EventId>,
    result_accumulated: String,
    event_listeners: Vec<tauri::EventId>,
}

static WINDOW: LazyLock<Mutex<OverlayWindow>> =
    LazyLock::new(|| Mutex::new(OverlayWindow::default()));
static AGENT: LazyLock<Mutex<AgentState>> =
    LazyLock::new(|| Mutex::new(AgentState::default()));
static CREATE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

// Thread-local background brush cache (overlay Win32 thread only).
thread_local! {
    static TL_BG_BRUSH: std::cell::Cell<(u32, usize)> =
        const { std::cell::Cell::new((u32::MAX, 0)) };
}

// ─ Utility ───────────────────────────────────────────────────────────────────
#[derive(Clone, Copy)]
struct Spring {
    pos: f64,
    vel: f64,
    target: f64,
}

impl Spring {
    fn new(pos: f64) -> Self {
        Self { pos, vel: 0.0, target: pos }
    }

    fn set_target(&mut self, t: f64) {
        self.target = t;
    }

    fn tick(&mut self) -> bool {
        let dx = self.pos - self.target;
        let accel = (-SPRING_K * dx - SPRING_D * self.vel) / SPRING_M;
        self.vel += accel * SPRING_DT;
        self.pos += self.vel * SPRING_DT;
        dx.abs() > SPRING_SETTLE || self.vel.abs() > SPRING_SETTLE
    }
}

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | ((g as u32) << 8) | ((b as u32) << 16)
}

fn wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn hwnd_from_raw(r: RawHwnd) -> HWND {
    r as HWND
}

fn ease_out_quart(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(4)
}

fn uuid_v4() -> String {
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-\
         {:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15],
    )
}

// ─ Font helpers ───────────────────────────────────────────────────────────────
fn make_font(px: i32, bold: bool) -> HGDIOBJ {
    let name = wide_null("Segoe UI");
    let weight = if bold { FW_BOLD as i32 } else { FW_NORMAL as i32 };
    unsafe {
        CreateFontW(
            -px, 0, 0, 0, weight, 0, 0, 0,
            DEFAULT_CHARSET as u32, OUT_DEFAULT_PRECIS as u32,
            0, DEFAULT_QUALITY as u32,
            (DEFAULT_PITCH | FF_DONTCARE) as u32,
            name.as_ptr(),
        ) as HGDIOBJ
    }
}

fn get_listen_font() -> HGDIOBJ {
    *CACHED_LISTEN_FONT.get_or_init(|| make_font(LISTEN_FONT_PX, true) as isize) as HGDIOBJ
}

fn get_result_font() -> HGDIOBJ {
    *CACHED_RESULT_FONT.get_or_init(|| make_font(RESULT_FONT_PX, false) as isize) as HGDIOBJ
}

fn get_notice_font() -> HGDIOBJ {
    *CACHED_NOTICE_FONT.get_or_init(|| make_font(NOTICE_FONT_PX, false) as isize) as HGDIOBJ
}

// ─ Thread-local background brush ─────────────────────────────────────────────
unsafe fn get_bg_brush(color: u32) -> HGDIOBJ {
    TL_BG_BRUSH.with(|cell| {
        let (cached_color, cached_handle) = cell.get();
        if cached_color == color && cached_handle != 0 {
            return cached_handle as HGDIOBJ;
        }
        if cached_handle != 0 {
            DeleteObject(cached_handle as HGDIOBJ);
        }
        let h = CreateSolidBrush(color) as usize;
        cell.set((color, h));
        h as HGDIOBJ
    })
}

// ─ System helpers ─────────────────────────────────────────────────────────────
fn work_area() -> RECT {
    let mut r = RECT::default();
    unsafe {
        if SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut r as *mut RECT).cast(), 0) != 0 {
            return r;
        }
    }
    RECT {
        left: 0,
        top: 0,
        right: unsafe { GetSystemMetrics(SM_CXSCREEN) },
        bottom: unsafe { GetSystemMetrics(SM_CYSCREEN) },
    }
}

fn prefers_dark(app: &AppHandle) -> bool {
    let theme = app.state::<crate::ThemeState>();
    let mode = theme.0.lock().unwrap_or_else(|e| e.into_inner()).clone();
    match mode.as_str() {
        "light" => false,
        "dark" => true,
        _ => system_apps_use_dark_theme(),
    }
}

fn system_apps_use_dark_theme() -> bool {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_QUERY_VALUE, REG_DWORD,
    };
    let subkey: Vec<u16> =
        "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0"
            .encode_utf16()
            .collect();
    let mut hkey: HKEY = null_mut();
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_QUERY_VALUE, &mut hkey) }
        != ERROR_SUCCESS
    {
        return true;
    }
    let vname: Vec<u16> = "AppsUseLightTheme\0".encode_utf16().collect();
    let mut data: u32 = 0;
    let mut data_size = std::mem::size_of::<u32>() as u32;
    let mut data_type: u32 = 0;
    let res = unsafe {
        RegQueryValueExW(
            hkey, vname.as_ptr(), null_mut(), &mut data_type,
            &mut data as *mut u32 as *mut u8, &mut data_size,
        )
    };
    let _ = unsafe { RegCloseKey(hkey) };
    if res != ERROR_SUCCESS || data_type != REG_DWORD {
        return true;
    }
    data == 0
}

// ─ Window state helpers ───────────────────────────────────────────────────────
#[derive(Clone)]
struct OverlaySnapshot {
    width: i32,
    height: i32,
    text: String,
    dark: bool,
}

fn window_snapshot() -> Option<OverlaySnapshot> {
    let s = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
    if s.hwnd == 0 {
        return None;
    }
    Some(OverlaySnapshot {
        width: s.width,
        height: s.height,
        text: s.text.clone(),
        dark: s.dark,
    })
}

#[derive(Clone, Copy)]
struct FrameSnapshot {
    width: i32,
    height: i32,
    center_x: i32,
    top_y: i32,
    alpha: u8,
}

fn frame_snapshot() -> Option<FrameSnapshot> {
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

fn set_alpha(alpha: u8) {
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

fn apply_frame(width: i32, height: i32, center_x: i32, top_y: i32) {
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
            hwnd, null_mut(), x, top_y, width, height,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = InvalidateRect(hwnd, null(), 1);
    }
}

fn update_text_content(text: String) {
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

fn set_theme_dark(dark: bool) {
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

// ─ Result height estimation ───────────────────────────────────────────────────
fn estimate_result_height(text: &str) -> i32 {
    let text_w = RESULT_W - RESULT_PAD_X * 2;
    let wide = wide_null(text);
    unsafe {
        let hdc = GetDC(null_mut());
        if !hdc.is_null() {
            let font = get_result_font();
            let old_font = SelectObject(hdc, font);
            let mut rect = RECT { left: 0, top: 0, right: text_w, bottom: 0 };
            DrawTextW(
                hdc, wide.as_ptr(), -1, &mut rect,
                DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
            );
            SelectObject(hdc, old_font);
            ReleaseDC(null_mut(), hdc);
            return (rect.bottom + RESULT_PAD_Y * 2).clamp(RESULT_MIN_H, RESULT_MAX_H);
        }
    }
    RESULT_MIN_H
}

// ─ WndProc ────────────────────────────────────────────────────────────────────
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
            // Any click: bring main window forward and dismiss the overlay.
            if CURRENT_MODE.load(Ordering::Relaxed) == MODE_RESULT {
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
        WM_AGENT_SHORTCUT_PRESS => {
            // Spawn async so WndProc returns immediately (<1 ms).
            // Blocking here would freeze the overlay thread and risk the
            // LL hook 300 ms system timeout.
            if let Some(app) = APP_HANDLE.get() {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    handle_shortcut_press(app);
                });
            }
            0
        }
        WM_AGENT_SHORTCUT_RELEASE => {
            tauri::async_runtime::spawn(async move {
                handle_shortcut_release();
            });
            0
        }
        WM_CLOSE => {
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
            HWND_READY.store(false, Ordering::Release);
            {
                let mut s = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
                if s.hwnd == hwnd as RawHwnd {
                    s.hwnd = 0;
                    s.alpha = 0;
                    s.text.clear();
                }
            }
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ─ Paint ──────────────────────────────────────────────────────────────────────
unsafe fn paint_overlay(hwnd: HWND) {
    let Some(state) = window_snapshot() else { return };
    let mode = CURRENT_MODE.load(Ordering::Relaxed);

    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    if hdc.is_null() {
        return;
    }

    // ── Colors ────────────────────────────────────────────────────────────────
    let bg = if state.dark { rgb(22, 20, 28) } else { rgb(250, 248, 253) };
    // Border: pre-blended purple on bg (≈28% opacity purple over bg)
    let border = if state.dark { rgb(63, 51, 85) } else { rgb(227, 209, 245) };
    let text_col = if state.dark { rgb(246, 246, 250) } else { rgb(30, 24, 54) };
    let muted_col = if state.dark { rgb(140, 136, 165) } else { rgb(150, 140, 170) };

    // ── Background + thin border via RoundRect ────────────────────────────────
    let brush = get_bg_brush(bg);
    let pen = CreatePen(PS_SOLID as i32, 1, border);
    let old_brush = SelectObject(hdc, brush);
    let old_pen = SelectObject(hdc, pen as HGDIOBJ);
    let r = CORNER_RADIUS * 2;
    RoundRect(hdc, 0, 0, state.width, state.height, r, r);

    // ── Mode-specific content ─────────────────────────────────────────────────
    match mode {
        MODE_LISTENING => {
            // Small indicator dot on the left
            let dot_col = if state.dark { rgb(255, 120, 158) } else { rgb(228, 78, 132) };
            let dot_brush = CreateSolidBrush(dot_col);
            let null_pen = GetStockObject(NULL_PEN_STOCK);
            let ob = SelectObject(hdc, dot_brush as HGDIOBJ);
            let op = SelectObject(hdc, null_pen);
            let dx = PAD_X - 4;
            let dy = (state.height - LISTEN_INDICATOR_SIZE) / 2;
            Ellipse(hdc, dx, dy, dx + LISTEN_INDICATOR_SIZE, dy + LISTEN_INDICATOR_SIZE);
            SelectObject(hdc, op);
            SelectObject(hdc, ob);
            DeleteObject(dot_brush as HGDIOBJ);

            // Text — muted placeholder or transcribed speech
            let font = get_listen_font();
            let of = SelectObject(hdc, font);
            SetBkMode(hdc, TRANSPARENT as i32);
            let display = if state.text.is_empty() {
                MUTED_TEXT.to_string()
            } else {
                state.text.clone()
            };
            let is_muted = display == MUTED_TEXT;
            SetTextColor(hdc, if is_muted { muted_col } else { text_col });
            let wide = wide_null(&display);
            let gap = LISTEN_INDICATOR_SIZE + 8;
            let mut rect = RECT {
                left: PAD_X + gap,
                top: 0,
                right: state.width - PAD_X,
                bottom: state.height,
            };
            DrawTextW(
                hdc, wide.as_ptr(), -1, &mut rect,
                DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            SelectObject(hdc, of);
        }

        MODE_PROCESSING => {
            // Three sequential dots
            let dot_active = DOTS_ACTIVE.load(Ordering::Relaxed);
            let active_col = if state.dark { rgb(200, 154, 248) } else { rgb(150, 84, 220) };
            let dim_col = if state.dark { rgb(80, 64, 108) } else { rgb(210, 200, 225) };

            let total_w = DOT_SIZE * 3 + DOT_GAP * 2;
            let sx = (state.width - total_w) / 2;
            let sy = (state.height - DOT_SIZE) / 2;
            let null_pen = GetStockObject(NULL_PEN_STOCK);
            let op = SelectObject(hdc, null_pen);
            for i in 0i32..3 {
                let col = if dot_active == i { active_col } else { dim_col };
                let db = CreateSolidBrush(col);
                let ob = SelectObject(hdc, db as HGDIOBJ);
                let x = sx + i * (DOT_SIZE + DOT_GAP);
                Ellipse(hdc, x, sy, x + DOT_SIZE, sy + DOT_SIZE);
                SelectObject(hdc, ob);
                DeleteObject(db as HGDIOBJ);
            }
            SelectObject(hdc, op);
        }

        MODE_RESULT => {
            let font = get_result_font();
            let of = SelectObject(hdc, font);
            SetBkMode(hdc, TRANSPARENT as i32);
            SetTextColor(hdc, text_col);
            let wide = wide_null(&state.text);
            let mut rect = RECT {
                left: RESULT_PAD_X,
                top: RESULT_PAD_Y,
                right: state.width - RESULT_PAD_X,
                bottom: state.height - RESULT_PAD_Y,
            };
            DrawTextW(
                hdc, wide.as_ptr(), -1, &mut rect,
                DT_WORDBREAK | DT_TOP | DT_NOPREFIX | DT_END_ELLIPSIS,
            );
            SelectObject(hdc, of);
        }

        MODE_NOTICE => {
            let font = get_notice_font();
            let of = SelectObject(hdc, font);
            SetBkMode(hdc, TRANSPARENT as i32);
            SetTextColor(hdc, text_col);
            let wide = wide_null(&state.text);
            let mut rect = RECT {
                left: PAD_X,
                top: 0,
                right: state.width - PAD_X,
                bottom: state.height,
            };
            DrawTextW(
                hdc, wide.as_ptr(), -1, &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            SelectObject(hdc, of);
        }

        _ => {}
    }

    SelectObject(hdc, old_pen);
    SelectObject(hdc, old_brush);
    DeleteObject(pen as HGDIOBJ);
    // brush is cached via get_bg_brush — do not delete here

    EndPaint(hwnd, &ps);
}

// ─ Window creation ────────────────────────────────────────────────────────────
fn spawn_overlay_thread(app: &AppHandle) {
    let dark = prefers_dark(app);
    std::thread::spawn(move || unsafe {
        let class_name = wide_null(CLASS_NAME);
        let title = wide_null("Selah Agent Overlay");
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
        let center_x = (work.left + work.right) / 2;
        let top_y = work.top + TOP_MARGIN;
        let x = center_x - LISTEN_W / 2;

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE,
            class_name.as_ptr(), title.as_ptr(), WS_POPUP,
            x, top_y, LISTEN_W, LISTEN_H,
            null_mut(), null_mut(), hinstance, null(),
        );

        if hwnd.is_null() {
            log::error!(
                "agent overlay: CreateWindowExW failed: {}",
                std::io::Error::last_os_error()
            );
            CREATING.store(false, Ordering::SeqCst);
            return;
        }

        let r = CORNER_RADIUS * 2;
        let rgn = CreateRoundRectRgn(0, 0, LISTEN_W + 1, LISTEN_H + 1, r, r);
        if !rgn.is_null() {
            if SetWindowRgn(hwnd, rgn, 1) == 0 {
                let _ = DeleteObject(rgn as _);
            }
        }
        let _ = SetLayeredWindowAttributes(hwnd, 0, 0, LWA_ALPHA);
        ShowWindow(hwnd, SW_HIDE);
        UpdateWindow(hwnd);

        {
            let mut s = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
            s.hwnd = hwnd as RawHwnd;
            s.width = LISTEN_W;
            s.height = LISTEN_H;
            s.center_x = center_x;
            s.top_y = top_y;
            s.alpha = 0;
            s.text.clear();
            s.dark = dark;
        }
        OVERLAY_HWND.store(hwnd as isize, Ordering::Relaxed);
        HWND_READY.store(true, Ordering::Release);
        CREATING.store(false, Ordering::SeqCst);

        // Install the LL keyboard hook on this thread so it runs inside our
        // GetMessage loop — bypasses IME/TSF which intercepts RegisterHotKey.
        install_ll_hook();

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}

fn ensure_overlay_window(app: &AppHandle) {
    if HWND_READY.load(Ordering::Acquire) {
        return;
    }
    let _lock = CREATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if HWND_READY.load(Ordering::Relaxed) || CREATING.load(Ordering::SeqCst) {
        return;
    }
    CREATING.store(true, Ordering::SeqCst);
    spawn_overlay_thread(app);
}

// ─ Animation ─────────────────────────────────────────────────────────────────
fn morph_to(target_w: i32, target_h: i32) {
    let Some(snap) = frame_snapshot() else { return };
    let token = MORPH_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        let mut sw = Spring::new(snap.width as f64);
        sw.set_target(target_w as f64);
        let mut sh = Spring::new(snap.height as f64);
        sh.set_target(target_h as f64);

        for _ in 0..90 {
            if MORPH_TOKEN.load(Ordering::Relaxed) != token {
                return;
            }
            let mw = sw.tick();
            let mh = sh.tick();
            apply_frame(
                sw.pos.round() as i32,
                sh.pos.round() as i32,
                snap.center_x,
                snap.top_y,
            );
            if !mw && !mh {
                break;
            }
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
        if MORPH_TOKEN.load(Ordering::Relaxed) == token {
            apply_frame(target_w, target_h, snap.center_x, snap.top_y);
        }
    });
}

fn fade_in() {
    let start = frame_snapshot().map(|s| s.alpha).unwrap_or(0);
    if start >= 250 {
        return;
    }
    let token = FADE_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        for i in 0..=FADE_FRAMES {
            if FADE_TOKEN.load(Ordering::Relaxed) != token {
                return;
            }
            let a = start as f64
                + (255.0 - start as f64) * ease_out_quart(i as f64 / FADE_FRAMES as f64);
            set_alpha(a.round().clamp(0.0, 255.0) as u8);
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
    });
}

fn fade_out_then_hide() {
    let token = FADE_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        for i in (0..=FADE_FRAMES).rev() {
            if FADE_TOKEN.load(Ordering::Relaxed) != token {
                return;
            }
            let a = (255.0 * ease_out_quart(i as f64 / FADE_FRAMES as f64))
                .round()
                .clamp(0.0, 255.0) as u8;
            set_alpha(a);
            tokio::time::sleep(Duration::from_millis(ANIM_MS)).await;
        }
        if FADE_TOKEN.load(Ordering::Relaxed) == token {
            hide_panel();
        }
    });
}

// Just hide the window — never destroy it while the shortcut is enabled,
// so the LL keyboard hook (which lives on the overlay thread) stays installed.
fn hide_panel() {
    let hwnd = WINDOW.lock().unwrap_or_else(|e| e.into_inner()).hwnd;
    if hwnd == 0 {
        return;
    }
    unsafe {
        ShowWindow(hwnd_from_raw(hwnd), SW_HIDE);
    }
    // HWND_READY stays true — the window still exists.
}

// Called only when the feature is explicitly disabled (apply_config enabled=false).
fn force_destroy_panel() {
    let hwnd = {
        let mut s = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
        let h = s.hwnd;
        s.hwnd = 0;
        s.alpha = 0;
        s.text.clear();
        h
    };
    if hwnd == 0 {
        return;
    }
    HWND_READY.store(false, Ordering::Relaxed);
    unsafe {
        ShowWindow(hwnd_from_raw(hwnd), SW_HIDE);
        let _ = PostMessageW(hwnd_from_raw(hwnd), WM_CLOSE, 0, 0);
    }
}

fn start_dots_animation() {
    DOTS_ACTIVE.store(0, Ordering::Relaxed);
    let token = DOTS_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        let mut phase: i32 = 0;
        loop {
            if DOTS_TOKEN.load(Ordering::Relaxed) != token {
                break;
            }
            if CURRENT_MODE.load(Ordering::Relaxed) != MODE_PROCESSING {
                break;
            }
            DOTS_ACTIVE.store(phase % 3, Ordering::Relaxed);
            let hwnd = WINDOW.lock().unwrap_or_else(|e| e.into_inner()).hwnd;
            if hwnd != 0 {
                unsafe {
                    let _ = InvalidateRect(hwnd_from_raw(hwnd), null(), 1);
                }
            }
            phase = phase.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(DOTS_PERIOD_MS)).await;
        }
        DOTS_ACTIVE.store(-1, Ordering::Relaxed);
    });
}

fn stop_dots_animation() {
    DOTS_TOKEN.fetch_add(1, Ordering::Relaxed);
    DOTS_ACTIVE.store(-1, Ordering::Relaxed);
}

fn schedule_auto_close(delay: Duration) {
    let token = AUTO_CLOSE_TOKEN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        if AUTO_CLOSE_TOKEN.load(Ordering::Relaxed) == token {
            if let Some(app) = APP_HANDLE.get() {
                close_panel(app, false);
            }
        }
    });
}

fn cancel_auto_close() {
    AUTO_CLOSE_TOKEN.fetch_add(1, Ordering::Relaxed);
}

// ─ Navigation ─────────────────────────────────────────────────────────────────
fn bring_main_window_to_front() {
    let Some(app) = APP_HANDLE.get() else { return };
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
    let _ = app.emit("tray-open-tab", "agent");
}

// ─ Mode transitions ───────────────────────────────────────────────────────────

// Ensure the overlay window exists and is faded in.
// Waits up to ~150 ms for the Win32 thread to initialize on first use.
fn ensure_panel(app: &AppHandle) {
    ensure_overlay_window(app);
    for _ in 0..30 {
        if HWND_READY.load(Ordering::Acquire) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    fade_in();
    let hwnd = WINDOW.lock().unwrap_or_else(|e| e.into_inner()).hwnd;
    if hwnd != 0 {
        unsafe {
            ShowWindow(hwnd_from_raw(hwnd), SW_SHOWNOACTIVATE);
        }
    }
}

fn listening_display_text(ag: &AgentState) -> String {
    let mut out = ag.finals_accumulated.clone();
    let partial = ag.current_speech.trim();
    if !partial.is_empty() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(partial);
    }
    out
}

fn consume_all_speech(ag: &mut AgentState) -> String {
    let partial = ag.current_speech.trim().to_string();
    let mut out = std::mem::take(&mut ag.finals_accumulated);
    ag.current_speech.clear();
    if !partial.is_empty() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&partial);
    }
    out.trim().to_string()
}

fn transition_to_listening(app: &AppHandle, initial_text: Option<&str>) {
    cancel_auto_close();
    stop_dots_animation();
    CURRENT_MODE.store(MODE_LISTENING, Ordering::Relaxed);
    ensure_panel(app);
    morph_to(LISTEN_W, LISTEN_H);
    let display = if let Some(t) = initial_text {
        t.to_string()
    } else {
        let ag = AGENT.lock().unwrap_or_else(|e| e.into_inner());
        let s = listening_display_text(&ag);
        if s.trim().is_empty() { MUTED_TEXT.to_string() } else { s }
    };
    update_text_content(display);
}

fn transition_to_processing(app: &AppHandle) {
    cancel_auto_close();
    CURRENT_MODE.store(MODE_PROCESSING, Ordering::Relaxed);
    {
        let mut ag = AGENT.lock().unwrap_or_else(|e| e.into_inner());
        ag.finals_accumulated.clear();
        ag.current_speech.clear();
    }
    ensure_panel(app);
    morph_to(PROCESS_W, PROCESS_H);
    update_text_content(String::new());
    start_dots_animation();
}

fn transition_to_result(app: &AppHandle, text: &str) {
    cancel_auto_close();
    stop_dots_animation();
    CURRENT_MODE.store(MODE_RESULT, Ordering::Relaxed);
    let target_h = estimate_result_height(text);
    ensure_panel(app);
    morph_to(RESULT_W, target_h);
    update_text_content(text.to_string());
    schedule_auto_close(Duration::from_secs(RESULT_AUTO_CLOSE_SECS));
}

fn transition_to_notice(app: &AppHandle, message: &str) {
    cancel_auto_close();
    stop_dots_animation();
    CURRENT_MODE.store(MODE_NOTICE, Ordering::Relaxed);
    ensure_panel(app);
    morph_to(NOTICE_W, NOTICE_H);
    update_text_content(message.to_string());
    schedule_auto_close(Duration::from_millis(NOTICE_AUTO_CLOSE_MS));
}

pub fn close_panel(app: &AppHandle, immediate: bool) {
    cancel_auto_close();
    stop_dots_animation();
    CURRENT_MODE.store(MODE_NONE, Ordering::Relaxed);
    clear_agent_listener(app);
    {
        let mut ag = AGENT.lock().unwrap_or_else(|e| e.into_inner());
        ag.stop_requested = false;
        ag.finals_accumulated.clear();
        ag.current_speech.clear();
        ag.result_accumulated.clear();
    }
    if immediate {
        hide_panel();
    } else {
        fade_out_then_hide();
    }
}

// ─ Shortcut handling ─────────────────────────────────────────────────────────
//
// Windows ショートカット操作モデル（長押し録音・離して送信）:
//   - キー押下 → 音声入力開始 (Listening)
//   - キー離す → 音声送信 → Agent へ (Processing → Result)
//
// キーリピートは MODE_LISTENING 中の press を無視することで自然に処理される。
// Called from WM_AGENT_SHORTCUT_PRESS on the overlay thread.
fn handle_shortcut_press(app: AppHandle) {
    log::info!("[agent] shortcut press, mode={}", CURRENT_MODE.load(Ordering::Relaxed));
    match CURRENT_MODE.load(Ordering::Relaxed) {
        MODE_LISTENING => {
            // Key is held down (repeat) or already recording — ignore.
        }
        MODE_PROCESSING => {
            // 処理中は無視
        }
        _ => {
            cancel_auto_close();
            start_agent_capture(app);
        }
    }
}

// Called from WM_AGENT_SHORTCUT_RELEASE on the overlay thread.
// Hold-to-talk: releasing the key finalizes the recording and submits.
fn handle_shortcut_release() {
    log::info!("[agent] shortcut release, mode={}", CURRENT_MODE.load(Ordering::Relaxed));
    if CURRENT_MODE.load(Ordering::Relaxed) == MODE_LISTENING {
        stop_listening();
    }
}

// Stop listening and signal STT to finalize.
fn stop_listening() {
    SHORTCUT_ARM_TOKEN.fetch_add(1, Ordering::Relaxed);
    if stt::stt_get_active_caller().as_deref() != Some("native_agent") {
        // No active native_agent STT session — close the overlay if it is stuck in LISTENING.
        if CURRENT_MODE.load(Ordering::Relaxed) == MODE_LISTENING {
            if let Some(app) = APP_HANDLE.get() {
                close_panel(app, false);
            }
        }
        return;
    }
    AGENT.lock().unwrap_or_else(|e| e.into_inner()).stop_requested = true;
    let _ = stt::stt_stop_stream();
}

fn start_agent_capture(app: AppHandle) {
    clear_agent_listener(&app);

    if stt::stt_is_running() {
        match stt::stt_get_active_caller().as_deref() {
            Some("native_agent") => return, // already capturing for this caller
            Some(_) => {
                transition_to_notice(&app, "ほかの音声入力が動作中です");
                return;
            }
            None => {}
        }
    }

    {
        let mut ag = AGENT.lock().unwrap_or_else(|e| e.into_inner());
        ag.stop_requested = false;
        ag.finals_accumulated.clear();
        ag.current_speech.clear();
        ag.result_accumulated.clear();
    }

    transition_to_listening(&app, Some(MUTED_TEXT));

    if let Err(err) = stt::stt_start_stream(app.clone(), "native_agent".to_string(), Some(false)) {
        transition_to_notice(&app, &err);
    }
}

// ─ Agent integration ─────────────────────────────────────────────────────────
fn submit_to_agent(app: AppHandle, text: String) {
    if text.trim().is_empty() {
        close_panel(&app, false);
        return;
    }

    transition_to_processing(&app);

    let db = app.state::<Database>();
    let conv_id = uuid_v4();
    let _ = db.agent_create_conversation(&conv_id, "Voice Shortcut");

    let cid = conv_id.clone();
    let app_listener = app.clone();
    let lid = app.listen(format!("agent_stream:{conv_id}"), move |event| {
        handle_agent_stream(&app_listener, &cid, event.payload());
    });
    AGENT.lock().unwrap_or_else(|e| e.into_inner()).agent_listener = Some(lid);

    tauri::async_runtime::spawn(async move {
        let _ = agent::agent_send(app, conv_id, text, Vec::new()).await;
    });
}

fn handle_agent_stream(app: &AppHandle, _conv_id: &str, payload: &str) {
    let parsed = serde_json::from_str::<Value>(payload).unwrap_or(Value::Null);
    let ev = parsed.get("type").and_then(|v| v.as_str()).unwrap_or_default();

    match ev {
        "token" => {
            let chunk = parsed.get("text").and_then(|v| v.as_str()).unwrap_or_default();
            if !chunk.is_empty() {
                AGENT.lock().unwrap_or_else(|e| e.into_inner()).result_accumulated.push_str(chunk);
            }
        }
        "error" => {
            let msg = parsed
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("エラーが発生しました");
            clear_agent_listener(app);
            AGENT.lock().unwrap_or_else(|e| e.into_inner()).result_accumulated = msg.to_string();
            transition_to_notice(app, msg);
        }
        "done" => {
            let final_text = AGENT
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .result_accumulated
                .trim()
                .to_string();
            clear_agent_listener(app);
            if final_text.is_empty() {
                transition_to_notice(app, "応答を取得できませんでした");
            } else {
                transition_to_result(app, &final_text);
            }
        }
        _ => {}
    }
}

fn clear_agent_listener(app: &AppHandle) {
    if let Some(id) = AGENT.lock().unwrap_or_else(|e| e.into_inner()).agent_listener.take() {
        app.unlisten(id);
    }
}

// ─ Shortcut normalization ─────────────────────────────────────────────────────
fn normalize_shortcut(s: &str) -> String {
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
fn parse_shortcut_to_vk(s: &str) -> (u32, u32) {
    let mut mods: u32 = 0;
    let mut vk: u32 = 0;
    for token in s.split('+') {
        let t = token.trim().to_ascii_lowercase();
        match t.as_str() {
            "ctrl" | "control" => mods |= MOD_BIT_CTRL,
            "shift" => mods |= MOD_BIT_SHIFT,
            "alt" | "option" => mods |= MOD_BIT_ALT,
            // meta/super/cmd/win: not supported as mod here
            _ => vk = code_to_vk(&t),
        }
    }
    (mods, vk)
}

fn code_to_vk(code: &str) -> u32 {
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
        "lalt" | "altleft" => 0xA4,   // VK_LMENU
        "ralt" | "altright" => 0xA5,  // VK_RMENU
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
    CallNextHookEx(HOOK_HANDLE.load(Ordering::Relaxed) as HHOOK, code, wparam, lparam)
}

fn install_ll_hook() {
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

fn uninstall_ll_hook() {
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
        set_theme_dark(prefers_dark(&app_theme));
    });

    // stt-final: a VAD-finalized speech segment arrived
    let app_final = app.clone();
    let lid_final = app.listen("stt-final", move |event| {
        let payload = serde_json::from_str::<Value>(event.payload()).unwrap_or_default();
        if payload.get("caller").and_then(|c| c.as_str()) != Some("native_agent") {
            return;
        }
        if CURRENT_MODE.load(Ordering::Relaxed) == MODE_NONE {
            return;
        }
        let text = payload
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string();

        let (display, pending_submit) = {
            let mut ag = AGENT.lock().unwrap_or_else(|e| e.into_inner());
            let trimmed = text.trim().to_string();
            if !trimmed.is_empty() {
                if !ag.finals_accumulated.is_empty() {
                    ag.finals_accumulated.push(' ');
                }
                ag.finals_accumulated.push_str(&trimmed);
            }
            ag.current_speech.clear();
            let display = listening_display_text(&ag);
            let pending = if ag.stop_requested {
                ag.stop_requested = false;
                Some(consume_all_speech(&mut ag))
            } else {
                None
            };
            (display, pending)
        };

        if CURRENT_MODE.load(Ordering::Relaxed) == MODE_LISTENING && !display.is_empty() {
            update_text_content(display);
        }
        if let Some(t) = pending_submit {
            if !t.is_empty() {
                submit_to_agent(app_final.clone(), t);
            }
        }
    });

    // stt-partial: in-flight transcription while user is still speaking
    let lid_partial = app.listen("stt-partial", move |event| {
        let payload = serde_json::from_str::<Value>(event.payload()).unwrap_or_default();
        if payload.get("caller").and_then(|c| c.as_str()) != Some("native_agent") {
            return;
        }
        if CURRENT_MODE.load(Ordering::Relaxed) != MODE_LISTENING {
            return;
        }
        let text = payload
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string();
        if text.trim().is_empty() {
            return;
        }
        let display = {
            let mut ag = AGENT.lock().unwrap_or_else(|e| e.into_inner());
            ag.current_speech = text;
            listening_display_text(&ag)
        };
        update_text_content(display);
    });

    // stt-state: STT engine state changed (started / stopped)
    let app_state = app.clone();
    let lid_state = app.listen("stt-state", move |event| {
        let payload = serde_json::from_str::<Value>(event.payload()).unwrap_or_default();
        if payload.get("caller").and_then(|c| c.as_str()) != Some("native_agent") {
            return;
        }
        let state_name = payload.get("state").and_then(|t| t.as_str()).unwrap_or_default();
        let is_listening = matches!(state_name, "initializing" | "listening");

        if is_listening {
            transition_to_listening(&app_state, None);
            return;
        }

        let mode = CURRENT_MODE.load(Ordering::Relaxed);
        let pending = {
            let mut ag = AGENT.lock().unwrap_or_else(|e| e.into_inner());
            if mode == MODE_PROCESSING || mode == MODE_RESULT || mode == MODE_NOTICE {
                ag.finals_accumulated.clear();
                ag.current_speech.clear();
                None
            } else {
                ag.stop_requested = false;
                Some(consume_all_speech(&mut ag))
            }
        };
        if let Some(t) = pending {
            if t.is_empty() {
                close_panel(&app_state, false);
            } else {
                submit_to_agent(app_state.clone(), t);
            }
        }
    });

    // stt-error: microphone / engine failure (only for our own caller)
    let app_err = app.clone();
    let lid_err = app.listen("stt-error", move |event| {
        let payload = serde_json::from_str::<Value>(event.payload()).unwrap_or_default();
        if payload.get("caller").and_then(|c| c.as_str()) != Some("native_agent") {
            return;
        }
        {
            let mut ag = AGENT.lock().unwrap_or_else(|e| e.into_inner());
            ag.stop_requested = false;
            ag.finals_accumulated.clear();
            ag.current_speech.clear();
        }
        transition_to_notice(&app_err, "音声入力を開始できませんでした");
    });

    AGENT.lock().unwrap_or_else(|e| e.into_inner()).event_listeners =
        vec![lid_theme, lid_final, lid_partial, lid_state, lid_err];
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
        SHORTCUT_ARM_TOKEN.fetch_add(1, Ordering::Relaxed);
        if stt::stt_get_active_caller().as_deref() == Some("native_agent") {
            let _ = stt::stt_stop_stream();
        }
        force_destroy_panel();
    }

    Ok(())
}
