//! Windows リアルタイム字幕浮窗 — ネイティブカプセル版
//!
//! Live 録課モジュールが発行する `live-session-updated` / `live-line-appended`
//! / `stt-partial` を監聴し、最新の転写テキストを画面下部のネイティブ浮窗に
//! 表示します。macOS 版と同様、STT / Agent 本体とは分離された補助 UI です。

#![cfg(target_os = "windows")]

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{LazyLock, Mutex, OnceLock};
use tauri::AppHandle;

#[path = "windows_subtitle_overlay/caption.rs"]
mod caption;
#[path = "windows_subtitle_overlay/events.rs"]
mod events;
#[path = "windows_subtitle_overlay/paint.rs"]
mod paint;
#[path = "windows_subtitle_overlay/theme.rs"]
mod theme;
#[path = "windows_subtitle_overlay/window.rs"]
mod window;

pub(in crate::windows_subtitle_overlay) use caption::*;
pub(in crate::windows_subtitle_overlay) use paint::*;
pub(in crate::windows_subtitle_overlay) use theme::*;
pub(in crate::windows_subtitle_overlay) use window::*;

pub use events::{close_overlay, is_open, open_overlay, setup};

// Win32 stock-object identifier for the per-DC custom-color pen; no allocation needed.
// Set the actual color via SetDCPenColor after selecting it into the DC.
const DC_PEN_STOCK: i32 = 19;

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

static LAST_CAPTION_SEQ: AtomicU64 = AtomicU64::new(0);

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn destroying_stale_window_does_not_clear_replacement_state() {
        {
            let mut state = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
            state.hwnd = 200;
            state.width = SUB_MIN_W;
        }
        OVERLAY_OPEN.store(true, Ordering::Relaxed);
        HWND_READY.store(true, Ordering::Release);

        assert!(!clear_destroyed_window(100));
        assert!(HWND_READY.load(Ordering::Acquire));
        assert!(OVERLAY_OPEN.load(Ordering::Relaxed));
        assert_eq!(WINDOW.lock().unwrap_or_else(|e| e.into_inner()).hwnd, 200);

        assert!(clear_destroyed_window(200));
        assert!(!HWND_READY.load(Ordering::Acquire));
        assert!(!OVERLAY_OPEN.load(Ordering::Relaxed));
    }
}
