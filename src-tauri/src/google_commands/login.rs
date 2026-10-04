use super::oauth::{send_oauth_response, wait_for_oauth_callback};
use crate::GCalState;
use tauri::{Emitter, Manager, State};

#[tauri::command]
pub async fn gcal_open_login(
    app: tauri::AppHandle,
    state: State<'_, GCalState>,
) -> Result<(), String> {
    log::info!("Opening Google Calendar login via system browser");

    let std_listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("ローカルサーバー起動失敗: {}", e))?;
    let port = std_listener
        .local_addr()
        .map_err(|e| format!("ポート取得失敗: {}", e))?
        .port();

    let attempt = {
        let gcal = state.client.lock().await;
        gcal.begin_login(port)?
    };
    let auth_url = attempt.url.clone();
    let verifier = attempt.verifier;
    let redirect_uri = attempt.redirect_uri;
    let expected_state = attempt.state;
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancel_for_wait = cancel.clone();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<Result<(), String>>();

    let waiter = tokio::task::spawn_blocking(move || {
        if let Err(e) = std_listener.set_nonblocking(true) {
            let msg = format!("ローカルサーバー初期化失敗: {}", e);
            let _ = ready_tx.send(Err(msg.clone()));
            return Err((msg, None));
        }
        let _ = ready_tx.send(Ok(()));
        wait_for_oauth_callback(
            std_listener,
            &expected_state,
            std::time::Duration::from_secs(300),
            &cancel_for_wait,
        )
    });

    match tokio::time::timeout(std::time::Duration::from_secs(3), ready_rx).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(e))) => {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = waiter.await;
            return Err(e);
        }
        Ok(Err(_)) => {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = waiter.await;
            return Err("ローカルサーバー初期化失敗: ready channel closed".into());
        }
        Err(_) => {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = waiter.await;
            return Err("ローカルサーバー起動タイムアウト".into());
        }
    }

    log::info!(
        "OAuth listener ready on port {}, opening browser to: {}",
        port,
        crate::client::safe_truncate(&auth_url, 200)
    );

    // Open the system browser only after the listener is accepting. This command
    // stays pending until the code is exchanged, so the UI does not depend on an
    // event that can be missed.
    use tauri_plugin_opener::OpenerExt;
    if let Err(e) = app.opener().open_url(&auth_url, None::<&str>) {
        log::warn!(
            "opener plugin failed to open URL: {} — trying OS fallback",
            e
        );
        if let Err(fallback_err) = open_url_os_fallback(&auth_url) {
            log::error!("OS fallback also failed: {}", fallback_err);
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = waiter.await;
            return Err(format!(
                "ブラウザを開けませんでした: {} (fallback: {})",
                e, fallback_err
            ));
        }
    }
    log::info!("Browser launch requested for Google Calendar OAuth");

    let callback = waiter
        .await
        .map_err(|e| format!("認証コールバック待ちに失敗: {}", e))?;
    match callback {
        Ok((auth_code, stream)) => {
            let app_state = app.state::<GCalState>();
            let mut gcal = app_state.client.lock().await;
            match gcal
                .exchange_code(&auth_code, &verifier, &redirect_uri)
                .await
            {
                Ok(()) => {
                    log::info!("Google Calendar login successful");
                    send_oauth_response(&stream, true, None, None);
                    let _ = app.emit("gcal-login-success", ());
                    Ok(())
                }
                Err(e) => {
                    log::error!("Google Calendar login failed: {}", e);
                    send_oauth_response(&stream, false, Some(&e), None);
                    let _ = app.emit("gcal-login-error", &e);
                    Err(e)
                }
            }
        }
        Err((e, stream)) => {
            log::error!("Google OAuth callback error: {}", e);
            if let Some(stream) = stream.as_ref() {
                send_oauth_response(stream, false, Some(&e), None);
            }
            let _ = app.emit("gcal-login-error", &e);
            Err(e)
        }
    }
}

/// OS-level fallback for opening a URL when the Tauri opener plugin fails.
fn open_url_os_fallback(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("`open` spawn failed: {}", e))?;
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let operation: Vec<u16> = std::ffi::OsStr::new("open")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let target: Vec<u16> = std::ffi::OsStr::new(url)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if result as isize <= 32 {
            return Err(format!(
                "ShellExecuteW failed with code {}",
                result as isize
            ));
        }
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("`xdg-open` spawn failed: {}", e))?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err("No OS-level fallback for this platform".into())
}
