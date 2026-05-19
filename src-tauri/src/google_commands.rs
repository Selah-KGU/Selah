use tauri::{Emitter, Manager, State};

use crate::google_calendar::{CalendarSyncEntry, GoogleCalConfig, GoogleCalStatus};
use crate::GCalState;

#[tauri::command]
pub async fn gcal_check_session(state: State<'_, GCalState>) -> Result<GoogleCalStatus, String> {
    let gcal = state.client.lock().await;
    Ok(gcal.status())
}

#[tauri::command]
pub async fn gcal_get_config(state: State<'_, GCalState>) -> Result<GoogleCalConfig, String> {
    let gcal = state.client.lock().await;
    Ok(gcal.config.clone())
}

#[tauri::command]
pub async fn gcal_save_config(
    state: State<'_, GCalState>,
    config: GoogleCalConfig,
) -> Result<(), String> {
    let mut gcal = state.client.lock().await;
    // Empty fields mean "use built-in default" — persist the user's choice
    // (empty on disk) but keep the resolved defaults in memory so OAuth works
    // immediately without a restart.
    crate::google_calendar::save_config(&config)?;
    gcal.config = crate::google_calendar::resolve_with_defaults(config);
    Ok(())
}

#[tauri::command]
pub async fn gcal_open_login(
    app: tauri::AppHandle,
    state: State<'_, GCalState>,
) -> Result<(), String> {
    log::info!("Opening Google Calendar login via system browser");

    // Bind a local TCP listener on a random available port
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("ローカルサーバー起動失敗: {}", e))?;
    let port = std_listener
        .local_addr()
        .map_err(|e| format!("ポート取得失敗: {}", e))?
        .port();

    let auth_url = {
        let mut gcal = state.client.lock().await;
        gcal.auth_url(port)?
    };
    let app_clone = app.clone();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    tokio::spawn(async move {
        // Convert to async listener (no polling loop, proper event-driven accept)
        if let Err(e) = std_listener.set_nonblocking(true) {
            let _ = ready_tx.send(Err(format!(
                "ローカルサーバー初期化失敗(set_nonblocking): {}",
                e
            )));
            return;
        }
        let listener = match tokio::net::TcpListener::from_std(std_listener) {
            Ok(l) => l,
            Err(e) => {
                let _ = ready_tx.send(Err(format!("ローカルサーバー初期化失敗(from_std): {}", e)));
                log::error!("Failed to create async TCP listener: {}", e);
                return;
            }
        };
        let _ = ready_tx.send(Ok(()));

        let code = tokio::time::timeout(std::time::Duration::from_secs(300), async {
            loop {
                let (stream, _) = listener
                    .accept()
                    .await
                    .map_err(|e| (format!("接続受信失敗: {}", e), None))?;

                let std_stream = stream.into_std().map_err(|e| {
                    (
                        format!("stream変換失敗: {}", e),
                        None::<std::net::TcpStream>,
                    )
                })?;

                match parse_oauth_callback(std_stream) {
                    Ok(ok) => break Ok(ok),
                    Err((e, Some(s))) if e == "oauth_code_missing" => {
                        // Browsers may issue an extra probe request (e.g. /favicon.ico) before the OAuth callback.
                        // Keep listener alive and return a tiny waiting page instead of failing the flow.
                        send_oauth_waiting_response(s);
                        continue;
                    }
                    Err(other) => break Err(other),
                }
            }
        })
        .await;

        match code {
            Ok(Ok((auth_code, stream))) => {
                let app_state = app_clone.state::<GCalState>();
                let mut gcal = app_state.client.lock().await;
                match gcal.exchange_code(&auth_code).await {
                    Ok(()) => {
                        log::info!("Google Calendar login successful");
                        send_oauth_response(stream, true, None);
                        let _ = app_clone.emit("gcal-login-success", ());
                    }
                    Err(e) => {
                        log::error!("Google Calendar login failed: {}", e);
                        send_oauth_response(stream, false, Some(&e));
                        let _ = app_clone.emit("gcal-login-error", &e);
                    }
                }
            }
            Ok(Err((e, stream))) => {
                log::error!("Google OAuth callback error: {}", e);
                if let Some(s) = stream {
                    send_oauth_response(s, false, Some(&e));
                }
                let _ = app_clone.emit("gcal-login-error", &e);
            }
            Err(_) => {
                log::warn!("Google Calendar login timed out (5 min)");
            }
        }
    });

    match tokio::time::timeout(std::time::Duration::from_secs(3), ready_rx).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(e))) => return Err(e),
        Ok(Err(_)) => return Err("ローカルサーバー初期化失敗: ready channel closed".into()),
        Err(_) => return Err("ローカルサーバー起動タイムアウト".into()),
    }

    log::info!(
        "OAuth listener ready on port {}, opening browser to: {}",
        port,
        crate::client::safe_truncate(&auth_url, 200)
    );

    // Open system browser via sandbox-safe opener plugin after listener is ready
    use tauri_plugin_opener::OpenerExt;
    if let Err(e) = app.opener().open_url(&auth_url, None::<&str>) {
        log::warn!(
            "opener plugin failed to open URL: {} — trying OS fallback",
            e
        );
        if let Err(fallback_err) = open_url_os_fallback(&auth_url) {
            log::error!("OS fallback also failed: {}", fallback_err);
            return Err(format!(
                "ブラウザを開けませんでした: {} (fallback: {})",
                e, fallback_err
            ));
        }
    }

    log::info!("Browser launch requested for Google Calendar OAuth");
    Ok(())
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
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .map_err(|e| format!("`start` spawn failed: {}", e))?;
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

/// Parse the OAuth callback request, extract code, but don't send response yet.
/// Returns (code, stream) on success, or (error, optionally stream) on failure.
fn parse_oauth_callback(
    mut stream: std::net::TcpStream,
) -> Result<(String, std::net::TcpStream), (String, Option<std::net::TcpStream>)> {
    use std::io::Read;

    let mut buf = [0u8; 4096];
    let n = stream
        .read(&mut buf)
        .map_err(|e| (format!("リクエスト読み取り失敗: {}", e), None))?;
    let request = String::from_utf8_lossy(&buf[..n]);

    let first_line = request.lines().next().unwrap_or("");
    let path = first_line.split_whitespace().nth(1).unwrap_or("/");

    let query = path.split('?').nth(1).unwrap_or("");
    let params: std::collections::HashMap<String, String> = query
        .split('&')
        .filter_map(|pair| {
            let mut kv = pair.splitn(2, '=');
            Some((
                urlencoding::decode(kv.next()?).ok()?.into_owned(),
                urlencoding::decode(kv.next()?).ok()?.into_owned(),
            ))
        })
        .collect();

    if let Some(code) = params.get("code") {
        Ok((code.clone(), stream))
    } else if let Some(err) = params.get("error") {
        Err((err.clone(), Some(stream)))
    } else {
        Err(("oauth_code_missing".into(), Some(stream)))
    }
}

fn send_oauth_waiting_response(mut stream: std::net::TcpStream) {
    use std::io::Write;

    let body = r#"<!DOCTYPE html><html><head><meta charset=\"utf-8\"><style>
body{font-family:-apple-system,system-ui,sans-serif;display:flex;justify-content:center;align-items:center;height:100vh;margin:0;background:#f5f5f7;color:#1d1d1f}
.card{text-align:center;padding:30px;border-radius:14px;background:#fff;box-shadow:0 2px 12px rgba(0,0,0,.08)}
h1{font-size:18px;margin:0 0 8px}p{font-size:13px;color:#86868b;margin:0}
</style></head><body><div class=\"card\"><h1>認証を待機中...</h1><p>このタブはそのままにしてください。</p></div></body></html>"#;

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body,
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// Send the final HTML response to the browser after token exchange.
fn send_oauth_response(mut stream: std::net::TcpStream, success: bool, error: Option<&str>) {
    use std::io::Write;

    let (status, body) = if success {
        ("200 OK", r#"<!DOCTYPE html><html><head><meta charset="utf-8"><style>
body{font-family:-apple-system,system-ui,sans-serif;display:flex;justify-content:center;align-items:center;height:100vh;margin:0;background:#f5f5f7;color:#1d1d1f}
.card{text-align:center;padding:40px;border-radius:16px;background:#fff;box-shadow:0 2px 12px rgba(0,0,0,.08)}
h1{font-size:20px;margin:0 0 8px}p{font-size:14px;color:#86868b;margin:0}
</style></head><body><div class="card"><h1>Google Calendar 認証完了</h1><p>このタブを閉じてください。</p></div></body></html>"#.to_string())
    } else {
        let escaped_error = error
            .unwrap_or("不明なエラー")
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;");
        (
            "400 Bad Request",
            format!(
                r#"<!DOCTYPE html><html><head><meta charset="utf-8"><style>
body{{font-family:-apple-system,system-ui,sans-serif;display:flex;justify-content:center;align-items:center;height:100vh;margin:0;background:#f5f5f7;color:#1d1d1f}}
.card{{text-align:center;padding:40px;border-radius:16px;background:#fff;box-shadow:0 2px 12px rgba(0,0,0,.08)}}
h1{{font-size:20px;margin:0 0 8px;color:#ff3b30}}p{{font-size:14px;color:#86868b;margin:0}}
</style></head><body><div class="card"><h1>認証エラー</h1><p>{}</p></div></body></html>"#,
                escaped_error
            ),
        )
    };

    let response = format!(
        "HTTP/1.1 {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        body.len(),
        body,
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

#[tauri::command]
pub async fn gcal_disconnect(state: State<'_, GCalState>) -> Result<(), String> {
    let mut gcal = state.client.lock().await;
    gcal.disconnect();
    log::info!("Google Calendar disconnected");
    Ok(())
}

/// Sync this week's timetable to Google Calendar
#[tauri::command]
pub async fn gcal_sync_timetable(
    state: State<'_, GCalState>,
    entries: Vec<CalendarSyncEntry>,
    week_label: String,
) -> Result<String, String> {
    let mut gcal = state.client.lock().await;
    gcal.sync_timetable(entries, week_label).await
}

#[tauri::command]
pub async fn gcal_clear_calendar(
    state: State<'_, GCalState>,
    delete_calendar: bool,
) -> Result<String, String> {
    let mut gcal = state.client.lock().await;
    gcal.clear_calendar(delete_calendar).await
}
