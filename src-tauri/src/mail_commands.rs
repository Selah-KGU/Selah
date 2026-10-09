use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};

use crate::config;
use crate::mail::{self, MailAttachment, MailConfig, MailDetail, MailMessage, MailProfile};
use crate::MailState;

/// Decode JWT payload without signature verification (we trust Microsoft's token)
fn decode_jwt_claims(token: &str) -> Option<serde_json::Value> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    // JWT payload is base64url-encoded
    let padded = match parts[1].len() % 4 {
        2 => format!("{}==", parts[1]),
        3 => format!("{}=", parts[1]),
        _ => parts[1].to_string(),
    };
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, parts[1])
        .or_else(|_| base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &padded))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Mail session status returned to frontend
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct MailSessionStatus {
    pub generation: u64,
    pub connection_id: Option<String>,
    pub authenticated: bool,
    pub email: String,
    pub display_name: String,
}

/// Check if mail is authenticated
#[tauri::command]
pub async fn mail_check_session(state: State<'_, MailState>) -> Result<MailSessionStatus, String> {
    let mail = state.client.lock().await;
    let authenticated = mail.is_authenticated();
    let mut email = String::new();
    let mut display_name = String::new();
    if authenticated {
        if let Some(token) = &mail.token {
            if let Some(claims) = decode_jwt_claims(&token.access_token) {
                // Microsoft JWT: "upn" or "unique_name" for email, "name" for display name
                email = claims
                    .get("upn")
                    .or_else(|| claims.get("unique_name"))
                    .or_else(|| claims.get("preferred_username"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                display_name = claims
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
            }
        }
    }
    Ok(MailSessionStatus {
        generation: mail.lifecycle.revision(),
        connection_id: mail.connection_id().map(str::to_owned),
        authenticated,
        email,
        display_name,
    })
}

/// Open Microsoft OAuth login window
#[tauri::command]
pub async fn mail_open_login(
    app: tauri::AppHandle,
    state: State<'_, MailState>,
) -> Result<(), String> {
    log::info!("Opening Microsoft mail login webview");

    for (label, existing) in app.webview_windows() {
        if label.starts_with("mail-login") {
            let _ = existing.close();
        }
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<String, String>>(1);
    state.cancellation.cancel();
    let attempt = {
        let mut mail = state.client.lock().await;
        mail.begin_login()
    };
    let expected_state = attempt.state.clone();
    let window_label = format!("mail-login-{}", attempt.lifetime.generation);
    let parsed_url = attempt.url.clone();

    let _win = tauri::WebviewWindowBuilder::new(
        &app,
        &window_label,
        tauri::WebviewUrl::External(parsed_url),
    )
    .title("Microsoft 365 - サインイン")
    .inner_size(480.0, 700.0)
    .resizable(true)
    .on_navigation(move |url| {
        if let Some(result) = mail::oauth::callback(url, &expected_state) {
            let _ = tx.try_send(result);
            return false;
        }
        // Never navigate the embedded view to a rejected localhost callback.
        url.host_str() != Some("localhost")
    })
    .build()
    .map_err(|e| format!("メールログインウィンドウ作成失敗: {}", e))?;

    let app_clone = app.clone();
    tokio::spawn(async move {
        let received = tokio::time::timeout(std::time::Duration::from_secs(300), rx.recv())
            .await
            .ok()
            .flatten();
        match received {
            Some(callback) => {
                let exchanged = match callback {
                    Ok(code) => attempt.exchange_code(&code).await,
                    Err(error) => Err(error),
                };
                let app_state = app_clone.state::<MailState>();
                let mut mail = app_state.client.lock().await;
                if mail.lifecycle.ensure(&attempt.lifetime).is_err()
                    || attempt.http.ensure_current().is_err()
                {
                    if let Some(win) = app_clone.get_webview_window(&window_label) {
                        let _ = win.close();
                    }
                    return;
                }
                match exchanged.and_then(|token| mail.accept_login(&attempt, token)) {
                    Ok(()) => {
                        let _ = app_clone.emit(
                            "mail-session-changed",
                            serde_json::json!({"generation":mail.lifecycle.revision()}),
                        );
                        log::info!("Microsoft mail login successful");
                        // Fetch profile to get email/name
                        let profile = mail.fetch_profile().await.ok();
                        if attempt.http.ensure_current().is_err() {
                            if let Some(win) = app_clone.get_webview_window(&window_label) {
                                let _ = win.close();
                            }
                            return;
                        }
                        let email = profile
                            .as_ref()
                            .and_then(|p| p.mail.clone().or(p.user_principal_name.clone()))
                            .unwrap_or_default();
                        let display_name = profile
                            .as_ref()
                            .and_then(|p| p.display_name.clone())
                            .unwrap_or_default();
                        let _ = app_clone.emit(
                            "mail-login-success",
                            serde_json::json!({
                                "email": email,
                                "displayName": display_name,
                                "generation": mail.lifecycle.revision(),
                            }),
                        );
                        drop(mail);
                        if let Err(e) =
                            crate::notifier::notification_sync_now(app_clone.clone()).await
                        {
                            log::warn!("notification sync after mail login failed: {}", e);
                        }
                        if let Err(e) =
                            crate::background_refresh::refresh_backend_data_now(&app_clone).await
                        {
                            log::warn!("background refresh after mail login failed: {}", e);
                        }
                    }
                    Err(e) => {
                        log::error!("Microsoft mail login failed: {}", e);
                        let _ = app_clone.emit("mail-login-error", &e);
                    }
                }
                if let Some(win) = app_clone.get_webview_window(&window_label) {
                    let _ = win.close();
                }
            }
            None => {
                log::info!("Microsoft mail login cancelled");
                if let Some(win) = app_clone.get_webview_window(&window_label) {
                    let _ = win.close();
                }
            }
        }
    });

    Ok(())
}

/// Logout from Microsoft mail
#[tauri::command]
pub async fn mail_logout(app: tauri::AppHandle, state: State<'_, MailState>) -> Result<(), String> {
    state.cancellation.cancel();
    let mut mail = state.client.lock().await;
    let result = mail.clear_token();
    let _ = app.emit(
        "mail-session-changed",
        serde_json::json!({"generation":mail.lifecycle.revision()}),
    );
    for (label, win) in app.webview_windows() {
        if label.starts_with("mail-login") {
            let _ = win.close();
        }
    }
    result?;
    log::info!("Microsoft mail logged out");
    Ok(())
}

/// Fetch user's mail profile
#[tauri::command]
pub async fn mail_fetch_profile(
    state: State<'_, MailState>,
    db: crate::db::AccountDb,
) -> Result<MailProfile, String> {
    // Phase 1: short lock – auth check + token preparation
    let prep = {
        let mut mail = state.client.lock().await;
        let connection = mail
            .connection_id()
            .ok_or(config::MAIL_AUTH_REQUIRED_MSG)?
            .to_owned();
        db.ensure_mail_connection(&connection)?;
        let (http, token) = mail.prepare_http().await?;
        (http, token, connection)
    };
    let (http, token, connection) = prep;

    // Phase 2: lock-free network I/O
    let url = format!(
        "{}/me?$select=displayName,mail,userPrincipalName",
        config::GRAPH_BASE
    );
    let response = mail::graph_get_lockfree(&http, &url, &token).await;
    let mut mail = state.client.lock().await;
    mail.ensure_connection(&connection)?;
    let body = match response {
        Ok(body) => body,
        Err((_, true)) => {
            // 401: re-lock and retry with full auth refresh
            match mail.fetch_profile().await {
                Ok(data) => {
                    if let Ok(json) = serde_json::to_string(&data) {
                        let _ = db.save_data_cache("mail_profile", &json);
                    }
                    return Ok(data);
                }
                Err(e) => {
                    mail.ensure_connection(&connection)?;
                    if let Ok(Some((json, _))) = db.get_data_cache("mail_profile") {
                        if let Ok(cached) = serde_json::from_str(&json) {
                            log::info!("mail_profile: cache fallback ({})", e);
                            return Ok(cached);
                        }
                    }
                    return Err(e);
                }
            }
        }
        Err((msg, false)) => {
            if let Ok(Some((json, _))) = db.get_data_cache("mail_profile") {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("mail_profile: cache fallback ({})", msg);
                    return Ok(cached);
                }
            }
            return Err(msg);
        }
    };

    let data: MailProfile =
        serde_json::from_value(body).map_err(|e| format!("プロフィール解析失敗: {}", e))?;
    if let Ok(json) = serde_json::to_string(&data) {
        let _ = db.save_data_cache("mail_profile", &json);
    }
    Ok(data)
}

/// Fetch inbox messages
#[tauri::command]
pub async fn mail_fetch_inbox(
    state: State<'_, MailState>,
    db: crate::db::AccountDb,
    top: Option<u32>,
    skip: Option<u32>,
) -> Result<Vec<MailMessage>, String> {
    fetch_inbox_impl(&state, &db, top.unwrap_or(20), skip.unwrap_or(0)).await
}

/// Agent-accessible variant: resolves state from the AppHandle.
pub async fn fetch_inbox_internal(
    app: &tauri::AppHandle,
    top: u32,
    skip: u32,
) -> Result<Vec<MailMessage>, String> {
    use tauri::Manager;
    let state = app.state::<MailState>();
    let db = app.state::<crate::db::Database>().scope();
    fetch_inbox_impl(&state, &db, top, skip).await
}

async fn fetch_inbox_impl(
    state: &MailState,
    db: &crate::db::Database,
    top_val: u32,
    skip_val: u32,
) -> Result<Vec<MailMessage>, String> {
    // Phase 1: short lock
    let prep = {
        let mut mail = state.client.lock().await;
        let connection = mail
            .connection_id()
            .ok_or(config::MAIL_AUTH_REQUIRED_MSG)?
            .to_owned();
        db.ensure_mail_connection(&connection)?;
        let (http, token) = mail.prepare_http().await?;
        (http, token, connection)
    };
    let (http, token, connection) = prep;

    // Phase 2: lock-free network I/O
    let url = format!(
        "{}/me/mailFolders/inbox/messages?$top={}&$skip={}&$orderby=receivedDateTime desc&$select=id,subject,bodyPreview,body,from,receivedDateTime,isRead,hasAttachments",
        config::GRAPH_BASE, top_val, skip_val,
    );
    let response = mail::graph_get_lockfree_with_headers(
        &http,
        &url,
        &token,
        &[("Prefer", "outlook.body-content-type=\"text\"")],
    )
    .await;
    let mut mail = state.client.lock().await;
    mail.ensure_connection(&connection)?;
    let body = match response {
        Ok(body) => body,
        Err((_, true)) => match mail.fetch_inbox(top_val, skip_val).await {
            Ok(data) => {
                if skip_val == 0 {
                    if let Ok(json) = serde_json::to_string(&data) {
                        let _ = db.save_data_cache("mail_inbox", &json);
                    }
                }
                return Ok(data);
            }
            Err(e) => {
                mail.ensure_connection(&connection)?;
                if skip_val == 0 {
                    if let Ok(Some((json, _))) = db.get_data_cache("mail_inbox") {
                        if let Ok(cached) = serde_json::from_str(&json) {
                            log::info!("mail_inbox: cache fallback ({})", e);
                            return Ok(cached);
                        }
                    }
                }
                return Err(e);
            }
        },
        Err((msg, false)) => {
            if skip_val == 0 {
                if let Ok(Some((json, _))) = db.get_data_cache("mail_inbox") {
                    if let Ok(cached) = serde_json::from_str(&json) {
                        log::info!("mail_inbox: cache fallback ({})", msg);
                        return Ok(cached);
                    }
                }
            }
            return Err(msg);
        }
    };

    let resp: mail::GraphListResponse<MailMessage> =
        serde_json::from_value(body).map_err(|e| format!("メール解析失敗: {}", e))?;
    if skip_val == 0 {
        if let Ok(json) = serde_json::to_string(&resp.value) {
            let _ = db.save_data_cache("mail_inbox", &json);
        }
    }
    Ok(resp.value)
}

/// Fetch a single message detail
#[tauri::command]
pub async fn mail_fetch_message(
    state: State<'_, MailState>,
    db: crate::db::AccountDb,
    message_id: String,
) -> Result<MailDetail, String> {
    fetch_message_impl(&state, &db, &message_id).await
}

/// Agent-accessible variant: resolves state from the AppHandle.
pub async fn fetch_message_internal(
    app: &tauri::AppHandle,
    message_id: &str,
) -> Result<MailDetail, String> {
    use tauri::Manager;
    let state = app.state::<MailState>();
    let db = app.state::<crate::db::Database>().scope();
    fetch_message_impl(&state, &db, message_id).await
}

fn common_ascii_suffix_len(a: &str, b: &str) -> usize {
    a.as_bytes()
        .iter()
        .rev()
        .zip(b.as_bytes().iter().rev())
        .take_while(|(left, right)| left == right)
        .count()
}

fn has_matching_id_prefix(partial: &str, full: &str) -> bool {
    let prefix_len = partial.len().min(24);
    prefix_len >= 8 && full.starts_with(&partial[..prefix_len])
}

fn resolve_cached_message_id(db: &crate::db::Database, message_id: &str) -> String {
    let trimmed = message_id.trim();
    let Ok(Some((json, _))) = db.get_data_cache("mail_inbox") else {
        return trimmed.to_string();
    };
    let Ok(messages) = serde_json::from_str::<Vec<MailMessage>>(&json) else {
        return trimmed.to_string();
    };
    if messages.iter().any(|message| message.id == trimmed) {
        return trimmed.to_string();
    }

    let mut best: Option<(&str, usize)> = None;
    let mut tied = false;
    for message in &messages {
        if !has_matching_id_prefix(trimmed, &message.id) {
            continue;
        }
        let score = common_ascii_suffix_len(trimmed, &message.id);
        if score < 12 {
            continue;
        }
        match best {
            Some((_, best_score)) if score == best_score => tied = true,
            Some((_, best_score)) if score <= best_score => {}
            _ => {
                best = Some((message.id.as_str(), score));
                tied = false;
            }
        }
    }

    if !tied {
        if let Some((id, score)) = best {
            log::info!(
                "mail_fetch_message: repaired cached message id by suffix match ({} chars)",
                score
            );
            return id.to_string();
        }
    }
    trimmed.to_string()
}

async fn fetch_message_impl(
    state: &MailState,
    db: &crate::db::Database,
    message_id: &str,
) -> Result<MailDetail, String> {
    let (http, token, connection, message_id, cache_key) = {
        let mut mail = state.client.lock().await;
        let connection = mail
            .connection_id()
            .ok_or(config::MAIL_AUTH_REQUIRED_MSG)?
            .to_owned();
        db.ensure_mail_connection(&connection)?;
        let message_id = resolve_cached_message_id(db, message_id);
        mail::validate_message_id(&message_id)?;
        let cache_key = format!("mail_msg:{}", message_id);
        if let Err(e) = mail.mark_as_read(&message_id).await {
            log::warn!("Failed to mark message as read: {e}");
        }
        let (http, token) = mail.prepare_http().await?;
        (http, token, connection, message_id, cache_key)
    };

    // Phase 2: lock-free fetch message (the heavier GET)
    let encoded_message_id = urlencoding::encode(&message_id);
    let url = format!(
        "{}/me/messages/{}?$select=id,subject,body,from,receivedDateTime,isRead,hasAttachments,toRecipients,ccRecipients",
        config::GRAPH_BASE, encoded_message_id,
    );
    let response = mail::graph_get_lockfree(&http, &url, &token).await;
    let mut mail = state.client.lock().await;
    mail.ensure_connection(&connection)?;
    let body = match response {
        Ok(body) => body,
        Err((_, true)) => match mail.fetch_message(&message_id).await {
            Ok(data) => {
                if let Ok(json) = serde_json::to_string(&data) {
                    let _ = db.save_data_cache(&cache_key, &json);
                }
                return Ok(data);
            }
            Err(e) => {
                mail.ensure_connection(&connection)?;
                if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                    if let Ok(cached) = serde_json::from_str(&json) {
                        log::info!("{}: cache fallback ({})", cache_key, e);
                        return Ok(cached);
                    }
                }
                return Err(e);
            }
        },
        Err((msg, false)) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("{}: cache fallback ({})", cache_key, msg);
                    return Ok(cached);
                }
            }
            return Err(msg);
        }
    };

    let data: MailDetail =
        serde_json::from_value(body).map_err(|e| format!("メール詳細解析失敗: {}", e))?;
    if let Ok(json) = serde_json::to_string(&data) {
        let _ = db.save_data_cache(&cache_key, &json);
    }
    Ok(data)
}

/// Get mail config
#[tauri::command]
pub async fn mail_get_config(state: State<'_, MailState>) -> Result<MailConfig, String> {
    let mail = state.client.lock().await;
    Ok(mail.config.clone())
}

/// Save mail config (client_id). Clears existing token if client_id changed.
#[tauri::command]
pub async fn mail_save_config(
    app: tauri::AppHandle,
    state: State<'_, MailState>,
    config: MailConfig,
) -> Result<(), String> {
    state.cancellation.cancel();
    let mut mail = state.client.lock().await;
    mail.cancel_requests();
    let old_id = mail.config.effective_client_id().to_string();
    let new_id = config.effective_client_id().to_string();
    // If client_id changed, invalidate existing token (it was issued for the old app)
    if old_id != new_id {
        log::info!("Mail client_id changed, clearing old token");
        let result = mail.clear_token();
        let _ = app.emit(
            "mail-session-changed",
            serde_json::json!({"generation":mail.lifecycle.revision()}),
        );
        result?;
    }
    mail.config = config.clone();
    crate::mail::save_config(&config)?;
    log::info!(
        "Mail config saved (client_id: {})",
        if config.client_id.trim().is_empty() {
            "default"
        } else {
            &new_id
        }
    );
    Ok(())
}

/// Fetch attachment metadata list for a message
#[tauri::command]
pub async fn mail_fetch_attachments(
    state: State<'_, MailState>,
    db: crate::db::AccountDb,
    message_id: String,
) -> Result<Vec<MailAttachment>, String> {
    let mut mail = state.client.lock().await;
    db.ensure_mail_connection(mail.connection_id().ok_or(config::MAIL_AUTH_REQUIRED_MSG)?)?;
    mail.fetch_attachments(&message_id).await
}

/// Download a single attachment to the Downloads folder and open it
#[tauri::command]
pub async fn mail_download_attachment(
    state: State<'_, MailState>,
    db: crate::db::AccountDb,
    message_id: String,
    attachment_id: String,
    file_name: String,
) -> Result<String, String> {
    let mut mail = state.client.lock().await;
    db.ensure_mail_connection(mail.connection_id().ok_or(config::MAIL_AUTH_REQUIRED_MSG)?)?;
    mail.download_attachment(&message_id, &attachment_id, &file_name)
        .await
}
