use tauri::Manager;

use super::app_state::{GCalState, KgcState, KwicState, LunaState, MailState};
#[cfg(target_os = "macos")]
use super::macos_fullscreen_exit;
use super::stt;
#[cfg(target_os = "macos")]
use super::tray;

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
    // Persist all session cookies on exit so they survive restarts.
    // Use try_lock to avoid deadlock if another task holds the lock.
    let kgc = app.state::<KgcState>();
    match kgc.client.try_lock() {
        Ok(c) => c.save_session(),
        Err(_) => log::warn!("Exit: KGC mutex held, session not saved"),
    };
    let luna = app.state::<LunaState>();
    match luna.client.try_lock() {
        Ok(l) => l.save_session(),
        Err(_) => log::warn!("Exit: Luna mutex held, session not saved"),
    };
    let kwic = app.state::<KwicState>();
    match kwic.client.try_lock() {
        Ok(k) => k.save_session(),
        Err(_) => log::warn!("Exit: KWIC mutex held, session not saved"),
    };
    let mail = app.state::<MailState>();
    match mail.client.try_lock() {
        Ok(m) => m.save_token(),
        Err(_) => log::warn!("Exit: Mail mutex held, token not saved"),
    };
    let gcal = app.state::<GCalState>();
    match gcal.client.try_lock() {
        Ok(g) => g.save_token(),
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
            stt::stt_shutdown_for_exit(std::time::Duration::from_millis(1500));
        }
        tauri::RunEvent::Exit => {
            stt::stt_shutdown_for_exit(std::time::Duration::from_millis(500));
            persist_sessions_before_exit(app);
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen {
            has_visible_windows,
            ..
        } => {
            reopen_windows_from_dock(app, has_visible_windows);
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
#[cfg(target_os = "macos")]
fn reopen_windows_from_dock(app: &tauri::AppHandle, has_visible_windows: bool) {
    let any_window_on_screen = app.windows().values().any(window_is_on_screen);
    if !should_restore_on_dock_click(has_visible_windows, any_window_on_screen) {
        return;
    }

    // Cmd+H hides the process; unhide before ordering a window front.
    let _ = app.show();
    if app.get_webview_window("main").is_none() {
        if let Err(err) = recreate_main_window(app) {
            log::warn!("failed to recreate main window from Dock: {err}");
        }
    }
    if app.get_webview_window("main").is_some() {
        present_main_window(app);
        return;
    }
    if let Some(window) = app.get_window("document-tabs") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg(target_os = "macos")]
fn present_main_window(app: &tauri::AppHandle) {
    let _ = tray::show_main_window_with_tab(app, None);
    // AppKit can order the window back out after applicationShouldHandleReopen
    // returns. Show again on the next main-thread turn so the Dock click sticks.
    let deferred = app.clone();
    let _ = app.run_on_main_thread(move || {
        let _ = tray::show_main_window_with_tab(&deferred, None);
    });
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
}
