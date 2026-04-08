use tauri::{Emitter, Manager, State};

use crate::google_calendar::{CalendarSyncEntry, GoogleCalConfig, GoogleCalStatus};
use crate::AppState;

#[tauri::command]
pub async fn gcal_check_session(state: State<'_, AppState>) -> Result<GoogleCalStatus, String> {
    let gcal = state.gcal.lock().await;
    Ok(gcal.status())
}

#[tauri::command]
pub async fn gcal_get_config(state: State<'_, AppState>) -> Result<GoogleCalConfig, String> {
    let gcal = state.gcal.lock().await;
    Ok(gcal.config.clone())
}

#[tauri::command]
pub async fn gcal_save_config(
    state: State<'_, AppState>,
    config: GoogleCalConfig,
) -> Result<(), String> {
    let mut gcal = state.gcal.lock().await;
    crate::google_calendar::save_config(&config)?;
    gcal.config = config;
    Ok(())
}

#[tauri::command]
pub async fn gcal_open_login(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    log::info!("Opening Google Calendar login via system browser");

    // Bind a local TCP listener on a random available port
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("ローカルサーバー起動失敗: {}", e))?;
    let port = listener.local_addr()
        .map_err(|e| format!("ポート取得失敗: {}", e))?.port();

    let auth_url = {
        let mut gcal = state.gcal.lock().await;
        gcal.auth_url(port)?
    };

    // Open system browser
    std::process::Command::new("open")
        .arg(&auth_url)
        .spawn()
        .map_err(|e| format!("ブラウザを開けませんでした: {}", e))?;

    let app_clone = app.clone();
    tokio::spawn(async move {
        // Accept one connection with a timeout
        listener.set_nonblocking(true)
            .unwrap_or_else(|e| log::warn!("set_nonblocking failed: {}", e));

        let code = tokio::time::timeout(std::time::Duration::from_secs(300), async {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        return parse_oauth_callback(stream);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                    Err(e) => {
                        return Err((format!("接続受信失敗: {}", e), None));
                    }
                }
            }
        }).await;

        match code {
            Ok(Ok((auth_code, stream))) => {
                let app_state = app_clone.state::<AppState>();
                let mut gcal = app_state.gcal.lock().await;
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
                if let Some(s) = stream { send_oauth_response(s, false, Some(&e)); }
                let _ = app_clone.emit("gcal-login-error", &e);
            }
            Err(_) => {
                log::warn!("Google Calendar login timed out (5 min)");
            }
        }
    });

    Ok(())
}

/// Parse the OAuth callback request, extract code, but don't send response yet.
/// Returns (code, stream) on success, or (error, optionally stream) on failure.
fn parse_oauth_callback(mut stream: std::net::TcpStream) -> Result<(String, std::net::TcpStream), (String, Option<std::net::TcpStream>)> {
    use std::io::Read;

    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf)
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
    } else {
        let err = params.get("error").cloned().unwrap_or_else(|| "不明なエラー".into());
        Err((err, Some(stream)))
    }
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
        ("400 Bad Request", format!(
            r#"<!DOCTYPE html><html><head><meta charset="utf-8"><style>
body{{font-family:-apple-system,system-ui,sans-serif;display:flex;justify-content:center;align-items:center;height:100vh;margin:0;background:#f5f5f7;color:#1d1d1f}}
.card{{text-align:center;padding:40px;border-radius:16px;background:#fff;box-shadow:0 2px 12px rgba(0,0,0,.08)}}
h1{{font-size:20px;margin:0 0 8px;color:#ff3b30}}p{{font-size:14px;color:#86868b;margin:0}}
</style></head><body><div class="card"><h1>認証エラー</h1><p>{}</p></div></body></html>"#,
            error.unwrap_or("不明なエラー")
        ))
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
pub async fn gcal_disconnect(state: State<'_, AppState>) -> Result<(), String> {
    let mut gcal = state.gcal.lock().await;
    gcal.disconnect();
    log::info!("Google Calendar disconnected");
    Ok(())
}

/// Sync this week's timetable to Google Calendar
#[tauri::command]
pub async fn gcal_sync_timetable(
    state: State<'_, AppState>,
    entries: Vec<CalendarSyncEntry>,
    week_label: String,
) -> Result<String, String> {
    let mut gcal = state.gcal.lock().await;
    gcal.sync_timetable(entries, week_label).await
}

#[tauri::command]
pub async fn gcal_clear_calendar(
    state: State<'_, AppState>,
    delete_calendar: bool,
) -> Result<String, String> {
    let mut gcal = state.gcal.lock().await;
    gcal.clear_calendar(delete_calendar).await
}
