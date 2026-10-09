use tauri::Manager;

use super::app_state::{GCalState, MailState};
#[cfg(target_os = "macos")]
use super::macos_fullscreen_exit;
use super::stt;
#[cfg(target_os = "macos")]
use super::tray;
#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSApp, NSWindow};

#[cfg(unix)]
pub(crate) fn protect_log_storage(
    app: &tauri::AppHandle,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;

    let dir = app.path().app_log_dir()?;
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;

    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.is_file() {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
    }

    let log_path = dir.join("kwic.log");
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    std::fs::set_permissions(log_path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn protect_log_storage(
    _app: &tauri::AppHandle,
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

pub(crate) fn is_widget_launch_url(url: &url::Url) -> bool {
    url.scheme().eq_ignore_ascii_case("selah")
}

pub(crate) fn run_event_panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

fn persist_sessions_before_exit(app: &tauri::AppHandle) {
    if let Err(error) = crate::session_coordinator::SESSIONS
        .checkpoint_and_commit(&crate::session_coordinator::signout_marker())
    {
        log::warn!("Exit session checkpoint failed: {error}");
    }
    let mail = app.state::<MailState>();
    match mail.client.try_lock() {
        Ok(m) => {
            if let Err(error) = m.save_token() {
                log::warn!("Exit token persistence failed: {error}");
            }
        }
        Err(_) => log::warn!("Exit: Mail mutex held, token not saved"),
    };
    let gcal = app.state::<GCalState>();
    match gcal.client.try_lock() {
        Ok(g) => {
            if let Err(error) = g.save_token() {
                log::warn!("Exit token persistence failed: {error}");
            }
        }
        Err(_) => log::warn!("Exit: GCal mutex held, token not saved"),
    };
}

fn defer_fullscreen_quit(
    app: &tauri::AppHandle,
    api: &tauri::ExitRequestApi,
    code: Option<i32>,
) -> bool {
    #[cfg(target_os = "macos")]
    {
        if macos_fullscreen_exit::defer_programmed_exit(app, code) {
            api.prevent_exit();
            return true;
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, api, code);
    }
    false
}

pub(crate) fn handle_run_event(app: &tauri::AppHandle, event: tauri::RunEvent) {
    match event {
        tauri::RunEvent::ExitRequested { api, code, .. } => {
            if defer_fullscreen_quit(app, &api, code) {
                return;
            }
            if super::app_shutdown::defer_exit(app, &api, code) {
                return;
            }
        }
        tauri::RunEvent::Exit => {
            if let Err(error) = stt::stt_shutdown_for_exit(std::time::Duration::from_millis(500)) {
                log::warn!("Exit: speech cleanup failed: {error}");
            }
            persist_sessions_before_exit(app);
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen {
            has_visible_windows,
            ..
        } => {
            reopen_windows_from_dock(app, has_visible_windows);
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Opened { urls } => {
            if urls.iter().any(is_widget_launch_url) {
                log::info!("widget URL opened; presenting main window");
                present_main_window(app);
            }
        }
        _ => {}
    }
}

pub(crate) fn attach_main_window_close_handler(
    window: &tauri::WebviewWindow,
    app_handle: tauri::AppHandle,
) {
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            if let Some(main) = app_handle.get_webview_window("main") {
                let _ = main.hide();
            }
        }
        tauri::WindowEvent::Focused(true) => {
            let app = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                crate::background_refresh::refresh_on_window_focus(&app).await;
            });
        }
        _ => {}
    });
}

/// Close hides the main window instead of destroying it. tao answers
/// applicationShouldHandleReopen with hasVisibleWindows, so AppKit will not
/// restore that window on a Dock click unless we show it ourselves.
/// macOS 14+ also ignores activateIgnoringOtherApps, so a visible window that
/// is not front still has to be ordered forward here.
#[cfg(target_os = "macos")]
fn reopen_windows_from_dock(app: &tauri::AppHandle, has_visible_windows: bool) {
    let any_window_on_screen = app.windows().values().any(window_is_on_screen);
    if !should_raise_on_reopen(has_visible_windows, any_window_on_screen, app_is_active()) {
        return;
    }
    if any_window_on_screen {
        raise_on_screen_windows(app);
        return;
    }
    present_main_window(app);
}

/// Show the main window and ask AppKit to activate this process.
///
/// Call this from the widget or Dock callback. `set_focus` only calls the
/// deprecated activation API, which no longer raises a background window.
#[cfg(target_os = "macos")]
pub(crate) fn present_main_window(app: &tauri::AppHandle) {
    let _ = app.show();
    if app.get_webview_window("main").is_none() {
        if let Err(err) = recreate_main_window(app) {
            log::warn!("failed to recreate main window: {err}");
        }
    }
    if let Some(window) = app.get_webview_window("main") {
        raise_webview(&window);
        let _ = tray::show_main_window_with_tab(app, None);
    } else if let Some(window) = app.get_window("document-tabs") {
        raise_window(&window);
    }
    // Activate after the window exists. macOS 14+ drops activation that
    // happens before there is a window to order forward.
    activate_app();
    if let Some(window) = app.get_webview_window("main") {
        raise_webview(&window);
    }
    // AppKit can order the window back out after the callback returns.
    let deferred = app.clone();
    let _ = app.run_on_main_thread(move || {
        activate_app();
        if let Some(window) = deferred.get_webview_window("main") {
            raise_webview(&window);
        }
    });
}

#[cfg(target_os = "macos")]
fn raise_on_screen_windows(app: &tauri::AppHandle) {
    let _ = app.show();
    if let Some(main) = app.get_webview_window("main") {
        if main.is_visible().unwrap_or(false) && !main.is_minimized().unwrap_or(false) {
            raise_webview(&main);
            activate_app();
            raise_webview(&main);
            return;
        }
    }
    if let Some(window) = app
        .windows()
        .values()
        .find(|window| window_is_on_screen(window))
    {
        raise_window(window);
    }
    activate_app();
}

#[cfg(target_os = "macos")]
fn activate_app() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApp(mtm);
    app.unhide(None);
    app.activate();
}

#[cfg(target_os = "macos")]
fn app_is_active() -> bool {
    MainThreadMarker::new().is_some_and(|mtm| NSApp(mtm).isActive())
}

#[cfg(target_os = "macos")]
fn raise_webview(window: &tauri::WebviewWindow) {
    let _ = window.unminimize();
    let _ = window.show();
    if let Ok(ptr) = window.ns_window() {
        order_raw_window(ptr);
    }
}

#[cfg(target_os = "macos")]
fn raise_window(window: &tauri::Window) {
    let _ = window.unminimize();
    let _ = window.show();
    if let Ok(ptr) = window.ns_window() {
        order_raw_window(ptr);
    }
}

#[cfg(target_os = "macos")]
fn order_raw_window(ptr: *mut std::ffi::c_void) {
    if ptr.is_null() {
        return;
    }
    let window = unsafe { &*(ptr as *const NSWindow) };
    if window.isMiniaturized() {
        window.deminiaturize(None);
    }
    window.makeKeyAndOrderFront(None);
    if !app_is_active() {
        window.orderFrontRegardless();
    }
}

#[cfg(target_os = "macos")]
fn recreate_main_window(app: &tauri::AppHandle) -> Result<(), String> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == "main")
        .cloned()
        .or_else(|| app.config().app.windows.first().cloned())
        .ok_or_else(|| "main window config missing".to_string())?;
    let window = tauri::WebviewWindowBuilder::from_config(app, &config)
        .map_err(|err| err.to_string())?
        .build()
        .map_err(|err| err.to_string())?;
    attach_main_window_close_handler(&window, app.clone());
    Ok(())
}

#[cfg(target_os = "macos")]
fn window_is_on_screen(window: &tauri::Window) -> bool {
    window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn should_restore_on_dock_click(has_visible_windows: bool, any_window_on_screen: bool) -> bool {
    !has_visible_windows || !any_window_on_screen
}

#[cfg(target_os = "macos")]
fn should_raise_on_reopen(
    has_visible_windows: bool,
    any_window_on_screen: bool,
    app_is_active: bool,
) -> bool {
    !app_is_active || should_restore_on_dock_click(has_visible_windows, any_window_on_screen)
}

#[cfg(all(test, target_os = "macos"))]
mod dock_reopen_tests {
    use super::should_restore_on_dock_click;

    #[test]
    fn restores_when_no_window_is_on_screen() {
        assert!(should_restore_on_dock_click(false, false));
        assert!(should_restore_on_dock_click(true, false));
    }

    #[test]
    fn restores_when_appkit_reports_no_visible_windows() {
        assert!(should_restore_on_dock_click(false, true));
    }

    #[test]
    fn leaves_an_open_window_alone() {
        assert!(!should_restore_on_dock_click(true, true));
    }

    #[test]
    fn raises_a_background_window() {
        assert!(super::should_raise_on_reopen(true, true, false));
        assert!(!super::should_raise_on_reopen(true, true, true));
    }
}

#[cfg(test)]
mod widget_url_tests {
    use super::is_widget_launch_url;

    #[test]
    fn accepts_selah_open_urls() {
        let url = url::Url::parse("selah://open").unwrap();
        assert!(is_widget_launch_url(&url));
        let other = url::Url::parse("https://example.com").unwrap();
        assert!(!is_widget_launch_url(&other));
    }
}
