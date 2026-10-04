//! macOS leaves a black fullscreen Space if the process dies while a window
//! still owns that Space.
//!
//! Cmd+Q and Dock Quit call -[NSApplication terminate:]. tao does not
//! implement applicationShouldTerminate:, so AppKit tears the process down
//! immediately and WindowServer never finishes leaving the Space. Tray quit
//! goes through app.exit() and then process::exit with the same result.
//!
//! Hold the quit until NSWindowDidExitFullScreenNotification, then allow
//! termination. The run loop must keep spinning during the wait; blocking
//! here prevents the Space animation from completing.

use std::cell::RefCell;
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use objc2::encode::{Encode, EncodeArguments};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, MethodImplementation, NSObject, Sel};
use objc2::{msg_send, sel, ClassType, MainThreadMarker};
use objc2_app_kit::{
    NSApp, NSApplicationTerminateReply, NSWindow, NSWindowDidExitFullScreenNotification,
    NSWindowStyleMask,
};
use objc2_foundation::{NSArray, NSNotificationCenter};
use tauri::AppHandle;

const EXIT_TIMEOUT: Duration = Duration::from_secs(3);
const EXIT_SETTLE: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IncomingQuit {
    Terminate,
    ProgrammedExit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuitReply {
    Proceed,
    Defer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct PendingFlags {
    terminate: bool,
    programmed_exit: bool,
}

struct PendingQuit {
    flags: PendingFlags,
    exit: Option<(AppHandle, i32)>,
}

static PENDING: Mutex<Option<PendingQuit>> = Mutex::new(None);
static ALLOW_EXIT: AtomicBool = AtomicBool::new(false);
static EXIT_STARTED: AtomicBool = AtomicBool::new(false);
static FINISH_SCHEDULED: AtomicBool = AtomicBool::new(false);
static APP: Mutex<Option<AppHandle>> = Mutex::new(None);

thread_local! {
    static TOGGLED: RefCell<Vec<Retained<NSWindow>>> = RefCell::new(Vec::new());
    static OBSERVER: RefCell<Option<Retained<NSObject>>> = RefCell::new(None);
}

pub(crate) fn install(app: &AppHandle) {
    if let Ok(mut slot) = APP.lock() {
        *slot = Some(app.clone());
    }
    install_terminate_hook();
    install_exit_observer();
}

/// Returns true when this exit must be delayed. Caller should prevent_exit().
pub(crate) fn defer_programmed_exit(app: &AppHandle, code: Option<i32>) -> bool {
    // prevent_exit is ignored for restart, so delaying here cannot help.
    if code == Some(tauri::RESTART_EXIT_CODE) {
        return false;
    }
    let fullscreen = any_native_fullscreen();
    let mut started = false;
    let reply = with_pending(|pending, allow_exit| {
        let is_new = pending.is_none() && fullscreen;
        let reply = note_quit(
            pending,
            allow_exit,
            fullscreen,
            IncomingQuit::ProgrammedExit,
        );
        if reply == QuitReply::Defer {
            if let Some(state) = pending.as_mut() {
                state.exit = Some((app.clone(), code.unwrap_or(0)));
            }
            started = is_new;
        }
        reply
    });
    if started {
        begin_fullscreen_exit();
    }
    reply == QuitReply::Defer
}

fn install_terminate_hook() {
    let Some(mtm) = MainThreadMarker::new() else {
        log::error!("fullscreen quit guard must be installed on the main thread");
        return;
    };
    let app = NSApp(mtm);
    let Some(delegate) = app.delegate() else {
        log::error!("NSApp has no delegate; fullscreen quit guard was not installed");
        return;
    };
    let class = unsafe {
        let class_ptr: *const AnyClass = msg_send![&*delegate, class];
        class_ptr.as_ref()
    };
    let Some(class) = class else {
        log::error!("NSApp delegate has no class; fullscreen quit guard was not installed");
        return;
    };
    let types = terminate_method_types();
    let terminate: extern "C-unwind" fn(
        *mut AnyObject,
        Sel,
        *mut AnyObject,
    ) -> NSApplicationTerminateReply = application_should_terminate;
    let imp = MethodImplementation::__imp(terminate);
    let added = unsafe {
        objc2::ffi::class_addMethod(
            std::ptr::from_ref(class).cast_mut(),
            sel!(applicationShouldTerminate:),
            imp,
            types.as_ptr(),
        )
    };
    if !added.as_bool() {
        log::error!("failed to install applicationShouldTerminate:; fullscreen Cmd+Q may stick");
    }
}

fn terminate_method_types() -> CString {
    let ret = <NSApplicationTerminateReply as Encode>::ENCODING;
    let args = <(*mut AnyObject,) as EncodeArguments>::ENCODINGS;
    let mut types = format!(
        "{ret}{}{}",
        <*mut AnyObject as Encode>::ENCODING,
        <Sel as Encode>::ENCODING
    );
    for encoding in args {
        use std::fmt::Write;
        let _ = write!(&mut types, "{encoding}");
    }
    CString::new(types).expect("terminate method encoding")
}

extern "C-unwind" fn application_should_terminate(
    _this: *mut AnyObject,
    _cmd: Sel,
    _sender: *mut AnyObject,
) -> NSApplicationTerminateReply {
    match std::panic::catch_unwind(|| {
        let fullscreen = any_native_fullscreen();
        let mut started = false;
        let reply = with_pending(|pending, allow_exit| {
            let is_new = pending.is_none() && fullscreen;
            let reply = note_quit(pending, allow_exit, fullscreen, IncomingQuit::Terminate);
            if reply == QuitReply::Defer && is_new {
                started = true;
            }
            reply
        });
        if started {
            begin_fullscreen_exit();
        }
        match reply {
            QuitReply::Defer => NSApplicationTerminateReply::TerminateLater,
            QuitReply::Proceed => NSApplicationTerminateReply::TerminateNow,
        }
    }) {
        Ok(reply) => reply,
        Err(_) => {
            log::error!("applicationShouldTerminate panicked; allowing quit");
            NSApplicationTerminateReply::TerminateNow
        }
    }
}

fn install_exit_observer() {
    if MainThreadMarker::new().is_none() {
        return;
    }
    let class = observer_class();
    let observer = unsafe {
        let allocated: *mut NSObject = msg_send![class, alloc];
        let initialized: *mut NSObject = msg_send![allocated, init];
        Retained::from_raw(initialized)
    };
    let Some(observer) = observer else {
        log::error!("failed to allocate fullscreen-exit observer");
        return;
    };
    let center = NSNotificationCenter::defaultCenter();
    let name = unsafe { NSWindowDidExitFullScreenNotification };
    unsafe {
        center.addObserver_selector_name_object(
            &observer,
            sel!(windowDidExitFullScreen:),
            Some(name),
            None,
        );
    }
    OBSERVER.with(|slot| {
        *slot.borrow_mut() = Some(observer);
    });
}

fn observer_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        if let Some(existing) = AnyClass::get(c"SelahFullscreenExitObserver") {
            return existing;
        }
        let mut builder =
            objc2::runtime::ClassBuilder::new(c"SelahFullscreenExitObserver", NSObject::class())
                .expect("SelahFullscreenExitObserver class name is unused");
        let exited: extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) =
            window_did_exit_fullscreen;
        unsafe {
            builder.add_method(sel!(windowDidExitFullScreen:), exited);
        }
        builder.register()
    })
}

extern "C-unwind" fn window_did_exit_fullscreen(
    _this: *mut AnyObject,
    _cmd: Sel,
    _note: *mut AnyObject,
) {
    let _ = std::panic::catch_unwind(|| {
        if !quit_is_pending() || any_native_fullscreen() {
            return;
        }
        schedule_finish();
    });
}

fn begin_fullscreen_exit() {
    if EXIT_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    log::info!("leaving fullscreen before quit so the desktop Space can be restored");
    toggle_fullscreen_windows();
    let app = app_handle();
    thread::spawn(move || {
        let started = Instant::now();
        loop {
            thread::sleep(Duration::from_millis(100));
            if !quit_is_pending() {
                return;
            }
            if started.elapsed() < EXIT_TIMEOUT {
                continue;
            }
            log::warn!("fullscreen exit timed out; quitting anyway");
            let finished = app.as_ref().and_then(|app| {
                run_on_main(app, || {
                    finish_quit();
                })
            });
            if finished.is_none() {
                log::error!("could not reach the main thread to finish a deferred quit");
                // NSTerminateLater would otherwise leave Cmd+Q stuck forever.
                std::process::exit(0);
            }
            return;
        }
    });
}

fn schedule_finish() {
    if FINISH_SCHEDULED.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app_handle();
    thread::spawn(move || {
        thread::sleep(EXIT_SETTLE);
        let Some(app) = app else {
            return;
        };
        let _ = run_on_main(&app, finish_quit);
    });
}

fn toggle_fullscreen_windows() {
    let Some(windows) = app_windows() else {
        return;
    };
    TOGGLED.with(|slots| {
        let mut slots = slots.borrow_mut();
        for index in 0..windows.count() {
            let window = windows.objectAtIndex(index);
            if !window.styleMask().contains(NSWindowStyleMask::FullScreen) {
                continue;
            }
            window.toggleFullScreen(None);
            slots.push(window);
        }
    });
}

fn finish_quit() {
    ALLOW_EXIT.store(true, Ordering::SeqCst);
    let pending = lock_pending().take();
    let Some(pending) = pending else {
        return;
    };
    hide_toggled_windows();
    if pending.flags.terminate {
        if let Some(mtm) = MainThreadMarker::new() {
            NSApp(mtm).replyToApplicationShouldTerminate(true);
        }
    }
    if let Some((app, code)) = pending.exit {
        app.exit(code);
    }
}

fn hide_toggled_windows() {
    TOGGLED.with(|slots| {
        for window in slots.borrow_mut().drain(..) {
            // Hiding a window that still owns a fullscreen Space is what leaves
            // the desktop black. Only hide after AppKit has cleared the mask.
            if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
                continue;
            }
            window.orderOut(None);
        }
    });
}

fn any_native_fullscreen() -> bool {
    let Some(windows) = app_windows() else {
        return false;
    };
    (0..windows.count()).any(|index| {
        windows
            .objectAtIndex(index)
            .styleMask()
            .contains(NSWindowStyleMask::FullScreen)
    })
}

fn app_windows() -> Option<Retained<NSArray<NSWindow>>> {
    let mtm = MainThreadMarker::new()?;
    Some(NSApp(mtm).windows())
}

fn quit_is_pending() -> bool {
    lock_pending().is_some()
}

fn app_handle() -> Option<AppHandle> {
    APP.lock().ok().and_then(|slot| slot.clone())
}

fn run_on_main(app: &AppHandle, work: impl FnOnce() + Send + 'static) -> Option<()> {
    let (tx, rx) = mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
        let _ = tx.send(());
    })
    .ok()?;
    rx.recv_timeout(Duration::from_secs(2)).ok()
}

fn with_pending(update: impl FnOnce(&mut Option<PendingQuit>, bool) -> QuitReply) -> QuitReply {
    let allow_exit = ALLOW_EXIT.load(Ordering::SeqCst);
    update(&mut lock_pending(), allow_exit)
}

fn lock_pending() -> std::sync::MutexGuard<'static, Option<PendingQuit>> {
    PENDING.lock().unwrap_or_else(|err| err.into_inner())
}

fn note_quit(
    pending: &mut Option<PendingQuit>,
    allow_exit: bool,
    fullscreen: bool,
    incoming: IncomingQuit,
) -> QuitReply {
    if allow_exit {
        return QuitReply::Proceed;
    }
    if let Some(state) = pending.as_mut() {
        match incoming {
            IncomingQuit::Terminate => state.flags.terminate = true,
            IncomingQuit::ProgrammedExit => state.flags.programmed_exit = true,
        }
        return QuitReply::Defer;
    }
    if !fullscreen {
        return QuitReply::Proceed;
    }
    *pending = Some(PendingQuit {
        flags: match incoming {
            IncomingQuit::Terminate => PendingFlags {
                terminate: true,
                programmed_exit: false,
            },
            IncomingQuit::ProgrammedExit => PendingFlags {
                terminate: false,
                programmed_exit: true,
            },
        },
        exit: None,
    });
    QuitReply::Defer
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(flags: PendingFlags) -> Option<PendingQuit> {
        Some(PendingQuit { flags, exit: None })
    }

    #[test]
    fn quit_proceeds_when_not_fullscreen() {
        let mut state = None;
        assert_eq!(
            note_quit(&mut state, false, false, IncomingQuit::Terminate),
            QuitReply::Proceed
        );
        assert!(state.is_none());
    }

    #[test]
    fn fullscreen_quit_is_deferred_until_allowed() {
        let mut state = None;
        assert_eq!(
            note_quit(&mut state, false, true, IncomingQuit::Terminate),
            QuitReply::Defer
        );
        assert!(state.unwrap().flags.terminate);

        let mut state = pending(PendingFlags {
            terminate: true,
            programmed_exit: false,
        });
        assert_eq!(
            note_quit(&mut state, false, true, IncomingQuit::ProgrammedExit),
            QuitReply::Defer
        );
        let state = state.unwrap();
        assert!(state.flags.terminate && state.flags.programmed_exit);

        let mut state = pending(PendingFlags {
            terminate: true,
            programmed_exit: false,
        });
        assert_eq!(
            note_quit(&mut state, true, true, IncomingQuit::Terminate),
            QuitReply::Proceed
        );
    }
}
