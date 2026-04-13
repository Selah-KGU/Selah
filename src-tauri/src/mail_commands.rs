use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};

use crate::config;
use crate::mail::{self, MailMessage, MailDetail, MailProfile, MailConfig, MailAttachment};
use crate::AppState;

/// Decode JWT payload without signature verification (we trust Microsoft's token)
fn decode_jwt_claims(token: &str) -> Option<serde_json::Value> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 { return None; }
    // JWT payload is base64url-encoded
    let padded = match parts[1].len() % 4 {
        2 => format!("{}==", parts[1]),
        3 => format!("{}=", parts[1]),
        _ => parts[1].to_string(),
    };
    let bytes = base64::Engine::decode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        parts[1],
    ).or_else(|_| base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &padded,
    )).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Mail session status returned to frontend
#[derive(Debug, Serialize, Deserialize)]
pub struct MailSessionStatus {
    pub authenticated: bool,
    pub email: String,
    pub display_name: String,
}

/// Check if mail is authenticated
#[tauri::command]
pub async fn mail_check_session(state: State<'_, AppState>) -> Result<MailSessionStatus, String> {
    let mail = state.mail.lock().await;
    let authenticated = mail.is_authenticated();
    let mut email = String::new();
    let mut display_name = String::new();
    if authenticated {
        if let Some(token) = &mail.token {
            if let Some(claims) = decode_jwt_claims(&token.access_token) {
                // Microsoft JWT: "upn" or "unique_name" for email, "name" for display name
                email = claims.get("upn")
                    .or_else(|| claims.get("unique_name"))
                    .or_else(|| claims.get("preferred_username"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                display_name = claims.get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
            }
        }
    }
    Ok(MailSessionStatus {
        authenticated,
        email,
        display_name,
    })
}

/// Open Microsoft OAuth login window
#[tauri::command]
pub async fn mail_open_login(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    log::info!("Opening Microsoft mail login webview");

    if let Some(existing) = app.get_webview_window("mail-login") {
        let _ = existing.close();
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(1);

    let auth_url = {
        let mail = state.mail.lock().await;
        mail.auth_url()
    };
    let parsed_url: url::Url = auth_url.parse()
        .map_err(|e| format!("URL parse error: {}", e))?;

    let _win = tauri::WebviewWindowBuilder::new(
        &app,
        "mail-login",
        tauri::WebviewUrl::External(parsed_url),
    )
    .title("Microsoft 365 - サインイン")
    .inner_size(480.0, 700.0)
    .resizable(true)
    .on_navigation(move |url| {
        // Intercept redirect to http://localhost?code=XXXXX
        if url.host_str() == Some("localhost") {
            let pairs: std::collections::HashMap<String, String> =
                url.query_pairs().into_owned().collect();
            if let Some(code) = pairs.get("code") {
                log::info!("Intercepted Microsoft OAuth code (len={})", code.len());
                let _ = tx.try_send(code.clone());
            } else if let Some(error) = pairs.get("error") {
                log::error!("Microsoft OAuth error: {} - {}", error, pairs.get("error_description").unwrap_or(&String::new()));
            }
            return false; // Block navigation to localhost
        }
        true
    })
    .build()
    .map_err(|e| format!("メールログインウィンドウ作成失敗: {}", e))?;

    let app_clone = app.clone();
    tokio::spawn(async move {
        match rx.recv().await {
            Some(code) => {
                let app_state = app_clone.state::<AppState>();
                let mut mail = app_state.mail.lock().await;
                match mail.exchange_code(&code).await {
                    Ok(()) => {
                        log::info!("Microsoft mail login successful");
                        // Fetch profile to get email/name
                        let profile = mail.fetch_profile().await.ok();
                        let email = profile.as_ref()
                            .and_then(|p| p.mail.clone().or(p.user_principal_name.clone()))
                            .unwrap_or_default();
                        let display_name = profile.as_ref()
                            .and_then(|p| p.display_name.clone())
                            .unwrap_or_default();
                        let _ = app_clone.emit("mail-login-success", serde_json::json!({
                            "email": email,
                            "displayName": display_name,
                        }));
                    }
                    Err(e) => {
                        log::error!("Microsoft mail login failed: {}", e);
                        let _ = app_clone.emit("mail-login-error", &e);
                    }
                }
                if let Some(win) = app_clone.get_webview_window("mail-login") {
                    let _ = win.close();
                }
            }
            None => {
                log::info!("Microsoft mail login cancelled");
            }
        }
    });

    Ok(())
}

/// Logout from Microsoft mail
#[tauri::command]
pub async fn mail_logout(state: State<'_, AppState>) -> Result<(), String> {
    let mut mail = state.mail.lock().await;
    mail.clear_token();
    log::info!("Microsoft mail logged out");
    Ok(())
}

/// Fetch user's mail profile
#[tauri::command]
pub async fn mail_fetch_profile(
    state: State<'_, AppState>,
    db: State<'_, crate::db::Database>,
) -> Result<MailProfile, String> {
    // Phase 1: short lock – auth check + token preparation
    let prep = {
        let mut mail = state.mail.lock().await;
        if !mail.is_authenticated() {
            if let Ok(Some((json, _))) = db.get_data_cache("mail_profile") {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("mail_profile: cache fallback (not authenticated)");
                    return Ok(cached);
                }
            }
            return Err(config::MAIL_AUTH_REQUIRED_MSG.into());
        }
        mail.prepare_http().await
    };
    let (http, token) = prep?;

    // Phase 2: lock-free network I/O
    let url = format!("{}/me?$select=displayName,mail,userPrincipalName", config::GRAPH_BASE);
    let body = match mail::graph_get_lockfree(&http, &url, &token).await {
        Ok(body) => body,
        Err((_, true)) => {
            // 401: re-lock and retry with full auth refresh
            let mut mail = state.mail.lock().await;
            match mail.fetch_profile().await {
                Ok(data) => {
                    if let Ok(json) = serde_json::to_string(&data) {
                        let _ = db.save_data_cache("mail_profile", &json);
                    }
                    return Ok(data);
                }
                Err(e) => {
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

    let data: MailProfile = serde_json::from_value(body)
        .map_err(|e| format!("プロフィール解析失敗: {}", e))?;
    if let Ok(json) = serde_json::to_string(&data) {
        let _ = db.save_data_cache("mail_profile", &json);
    }
    Ok(data)
}

/// Fetch inbox messages
#[tauri::command]
pub async fn mail_fetch_inbox(
    state: State<'_, AppState>,
    db: State<'_, crate::db::Database>,
    top: Option<u32>,
    skip: Option<u32>,
) -> Result<Vec<MailMessage>, String> {
    let top_val = top.unwrap_or(20);
    let skip_val = skip.unwrap_or(0);

    // Phase 1: short lock
    let prep = {
        let mut mail = state.mail.lock().await;
        if !mail.is_authenticated() {
            if let Ok(Some((json, _))) = db.get_data_cache("mail_inbox") {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("mail_inbox: cache fallback (not authenticated)");
                    return Ok(cached);
                }
            }
            return Err(config::MAIL_AUTH_REQUIRED_MSG.into());
        }
        mail.prepare_http().await
    };
    let (http, token) = prep?;

    // Phase 2: lock-free network I/O
    let url = format!(
        "{}/me/mailFolders/inbox/messages?$top={}&$skip={}&$orderby=receivedDateTime desc&$select=id,subject,bodyPreview,from,receivedDateTime,isRead,hasAttachments",
        config::GRAPH_BASE, top_val, skip_val,
    );
    let body = match mail::graph_get_lockfree(&http, &url, &token).await {
        Ok(body) => body,
        Err((_, true)) => {
            let mut mail = state.mail.lock().await;
            match mail.fetch_inbox(top_val, skip_val).await {
                Ok(data) => {
                    if skip_val == 0 {
                        if let Ok(json) = serde_json::to_string(&data) {
                            let _ = db.save_data_cache("mail_inbox", &json);
                        }
                    }
                    return Ok(data);
                }
                Err(e) => {
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
            }
        }
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

    let resp: mail::GraphListResponse<MailMessage> = serde_json::from_value(body)
        .map_err(|e| format!("メール解析失敗: {}", e))?;
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
    state: State<'_, AppState>,
    db: State<'_, crate::db::Database>,
    message_id: String,
) -> Result<MailDetail, String> {
    mail::validate_message_id(&message_id)?;
    let cache_key = format!("mail_msg:{}", message_id);

    // Phase 1: short lock – auth check, mark_as_read (fast PATCH), prepare token
    let prep = {
        let mut mail = state.mail.lock().await;
        if !mail.is_authenticated() {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("{}: cache fallback (not authenticated)", cache_key);
                    return Ok(cached);
                }
            }
            return Err(config::MAIL_AUTH_REQUIRED_MSG.into());
        }
        // Mark as read while we hold the lock (best-effort, fast PATCH)
        if let Err(e) = mail.mark_as_read(&message_id).await {
            log::warn!("Failed to mark message {} as read: {}", message_id, e);
        }
        mail.prepare_http().await
    };
    let (http, token) = prep?;

    // Phase 2: lock-free fetch message (the heavier GET)
    let url = format!(
        "{}/me/messages/{}?$select=id,subject,body,from,receivedDateTime,isRead,hasAttachments,toRecipients,ccRecipients",
        config::GRAPH_BASE, message_id,
    );
    let body = match mail::graph_get_lockfree(&http, &url, &token).await {
        Ok(body) => body,
        Err((_, true)) => {
            let mut mail = state.mail.lock().await;
            match mail.fetch_message(&message_id).await {
                Ok(data) => {
                    if let Ok(json) = serde_json::to_string(&data) {
                        let _ = db.save_data_cache(&cache_key, &json);
                    }
                    return Ok(data);
                }
                Err(e) => {
                    if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                        if let Ok(cached) = serde_json::from_str(&json) {
                            log::info!("{}: cache fallback ({})", cache_key, e);
                            return Ok(cached);
                        }
                    }
                    return Err(e);
                }
            }
        }
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

    let data: MailDetail = serde_json::from_value(body)
        .map_err(|e| format!("メール詳細解析失敗: {}", e))?;
    if let Ok(json) = serde_json::to_string(&data) {
        let _ = db.save_data_cache(&cache_key, &json);
    }
    Ok(data)
}

/// Get mail config
#[tauri::command]
pub async fn mail_get_config(state: State<'_, AppState>) -> Result<MailConfig, String> {
    let mail = state.mail.lock().await;
    Ok(mail.config.clone())
}

/// Save mail config (client_id). Clears existing token if client_id changed.
#[tauri::command]
pub async fn mail_save_config(
    state: State<'_, AppState>,
    config: MailConfig,
) -> Result<(), String> {
    let mut mail = state.mail.lock().await;
    let old_id = mail.config.effective_client_id().to_string();
    let new_id = config.effective_client_id().to_string();
    // If client_id changed, invalidate existing token (it was issued for the old app)
    if old_id != new_id && mail.is_authenticated() {
        log::info!("Mail client_id changed, clearing old token");
        mail.clear_token();
    }
    mail.config = config.clone();
    crate::mail::save_config(&config)?;
    log::info!("Mail config saved (client_id: {})", if new_id == crate::mail::DEFAULT_CLIENT_ID_STR { "default" } else { &new_id });
    Ok(())
}

/// Fetch attachment metadata list for a message
#[tauri::command]
pub async fn mail_fetch_attachments(
    state: State<'_, AppState>,
    message_id: String,
) -> Result<Vec<MailAttachment>, String> {
    let mut mail = state.mail.lock().await;
    if !mail.is_authenticated() {
        return Err(config::MAIL_AUTH_REQUIRED_MSG.into());
    }
    mail.fetch_attachments(&message_id).await
}

/// Download a single attachment to the Downloads folder and open it
#[tauri::command]
pub async fn mail_download_attachment(
    state: State<'_, AppState>,
    message_id: String,
    attachment_id: String,
    file_name: String,
) -> Result<String, String> {
    let mut mail = state.mail.lock().await;
    if !mail.is_authenticated() {
        return Err(config::MAIL_AUTH_REQUIRED_MSG.into());
    }
    mail.download_attachment(&message_id, &attachment_id, &file_name).await
}
