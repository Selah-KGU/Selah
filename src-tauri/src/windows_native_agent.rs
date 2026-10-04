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

#[allow(unused_imports)]
use crate::agent;
#[allow(unused_imports)]
use crate::commands::NativeAgentConfig;
#[allow(unused_imports)]
use crate::db::Database;
#[allow(unused_imports)]
use crate::stt;
#[allow(unused_imports)]
use rand::RngCore;
#[allow(unused_imports)]
use serde_json::Value;
#[allow(unused_imports)]
use std::mem::size_of;
#[allow(unused_imports)]
use std::ptr::{null, null_mut};
#[allow(unused_imports)]
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
#[allow(unused_imports)]
use std::sync::{LazyLock, Mutex, OnceLock};
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use tauri::{AppHandle, Listener, Manager};
#[allow(unused_imports)]
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
#[allow(unused_imports)]
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreatePen, CreateRoundRectRgn, CreateSolidBrush, DeleteObject,
    DrawTextW, Ellipse, EndPaint, GetDC, GetStockObject, InvalidateRect, ReleaseDC, RoundRect,
    SelectObject, SetBkMode, SetTextColor, SetWindowRgn, UpdateWindow, DEFAULT_CHARSET,
    DEFAULT_PITCH, DEFAULT_QUALITY, DT_CALCRECT, DT_CENTER, DT_END_ELLIPSIS, DT_NOPREFIX,
    DT_SINGLELINE, DT_TOP, DT_VCENTER, DT_WORDBREAK, FF_DONTCARE, FW_BOLD, FW_NORMAL, HGDIOBJ,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, TRANSPARENT,
};
#[allow(unused_imports)]
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
#[allow(unused_imports)]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT};
#[allow(unused_imports)]
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

#[path = "windows_native_agent/bootstrap.rs"]
mod bootstrap;
#[path = "windows_native_agent/overlay.rs"]
mod overlay;
#[path = "windows_native_agent/paint.rs"]
mod paint;
#[path = "windows_native_agent/session.rs"]
mod session;
#[path = "windows_native_agent/shortcut.rs"]
mod shortcut;
#[path = "windows_native_agent/util.rs"]
mod util;
#[path = "windows_native_agent/window.rs"]
mod window;

pub use bootstrap::{apply_config, setup};
pub(in crate::windows_native_agent) use bootstrap::{install_ll_hook, uninstall_ll_hook};
pub(in crate::windows_native_agent) use overlay::*;
pub(in crate::windows_native_agent) use paint::*;
pub use session::close_panel;
pub(in crate::windows_native_agent) use session::*;
pub(in crate::windows_native_agent) use shortcut::*;
pub(in crate::windows_native_agent) use util::{
    ease_out_quart, get_bg_brush, get_listen_font, get_notice_font, get_result_font, hwnd_from_raw,
    prefers_dark, rgb, uuid_v4, wide_null, work_area,
};
pub(in crate::windows_native_agent) use window::{
    apply_frame, frame_snapshot, overlay_wndproc, set_alpha, set_theme_dark, update_text_content,
    window_snapshot,
};

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
static DESTROYING: AtomicBool = AtomicBool::new(false);
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
static AGENT: LazyLock<Mutex<AgentState>> = LazyLock::new(|| Mutex::new(AgentState::default()));
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
        Self {
            pos,
            vel: 0.0,
            target: pos,
        }
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

#[derive(Clone)]
struct OverlaySnapshot {
    width: i32,
    height: i32,
    text: String,
    dark: bool,
}

#[derive(Clone, Copy)]
struct FrameSnapshot {
    width: i32,
    height: i32,
    center_x: i32,
    top_y: i32,
    alpha: u8,
}
