use serde::{Deserialize, Serialize};
use reqwest::Client;
use std::path::PathBuf;

use crate::config;

pub const DEFAULT_CLIENT_ID_STR: &str = "9e5f94bc-e8a4-4e73-b8be-63364c29d753";
const MS_REDIRECT_URI: &str = "http://localhost";
const MS_SCOPES: &str = "Mail.ReadWrite offline_access";

const TOKEN_FILE: &str = "ms_mail_token.json";
const MAIL_CONFIG_FILE: &str = "ms_mail_config.json";

/// User-configurable mail settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct MailConfig {
    /// Azure AD Application (client) ID. Empty = use default.
    pub client_id: String,
}


impl MailConfig {
    /// Returns the effective client_id (user-configured or default)
    pub fn effective_client_id(&self) -> &str {
        if self.client_id.trim().is_empty() {
            DEFAULT_CLIENT_ID_STR
        } else {
            self.client_id.trim()
        }
    }
}

fn config_path() -> PathBuf {
    crate::client::data_dir().join(MAIL_CONFIG_FILE)
}

pub fn load_config() -> MailConfig {
    let path = config_path();
    if path.exists() {
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str(&data) {
                return cfg;
            }
        }
    }
    MailConfig::default()
}

pub fn save_config(config: &MailConfig) -> Result<(), String> {
    let path = config_path();
    let data = serde_json::to_string_pretty(config)
        .map_err(|e| format!("JSON serialization error: {}", e))?;
    std::fs::write(&path, &data)
        .map_err(|e| format!("Failed to write mail config: {}", e))?;
    Ok(())
}

/// Persisted token data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenData {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix timestamp (seconds) when access_token expires
    pub expires_at: i64,
}

/// A single mail message from Graph API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailMessage {
    pub id: String,
    pub subject: Option<String>,
    pub body_preview: Option<String>,
    pub from: Option<MailAddress>,
    pub received_date_time: Option<String>,
    pub is_read: Option<bool>,
    pub has_attachments: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailAddress {
    pub email_address: EmailAddress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailAddress {
    pub name: Option<String>,
    pub address: Option<String>,
}

/// A mail attachment entry (metadata only, no content bytes)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailAttachment {
    pub id: String,
    pub name: Option<String>,
    pub content_type: Option<String>,
    pub size: Option<i64>,
}

/// Full mail body for detail view
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailDetail {
    pub id: String,
    pub subject: Option<String>,
    pub body: Option<MailBody>,
    pub from: Option<MailAddress>,
    pub received_date_time: Option<String>,
    pub is_read: Option<bool>,
    pub has_attachments: Option<bool>,
    pub to_recipients: Option<Vec<MailAddress>>,
    pub cc_recipients: Option<Vec<MailAddress>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailBody {
    pub content_type: Option<String>,
    pub content: Option<String>,
}

/// Graph API list response wrapper
#[derive(Debug, Deserialize)]
pub(crate) struct GraphListResponse<T> {
    pub value: Vec<T>,
}

/// User profile from Graph API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailProfile {
    pub display_name: Option<String>,
    pub mail: Option<String>,
    pub user_principal_name: Option<String>,
}

fn token_path() -> PathBuf {
    crate::client::data_dir().join(TOKEN_FILE)
}

pub struct MailClient {
    http: Client,
    pub token: Option<TokenData>,
    pub config: MailConfig,
}

/// Validate a Graph API message ID (alphanumeric, hyphens, underscores, equals, dots).
pub(crate) fn validate_message_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 200 || !id.chars().all(|c| c.is_ascii_alphanumeric() || "-_=.".contains(c)) {
        return Err("無効なメッセージIDです".into());
    }
    Ok(())
}

/// Validate a Graph API attachment ID.
fn validate_attachment_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 600 || !id.chars().all(|c| c.is_ascii_alphanumeric() || "-_=.+/".contains(c)) {
        return Err("無効な添付ファイルIDです".into());
    }
    Ok(())
}

impl MailClient {
    pub fn new() -> Self {
        let http = Client::builder()
            .user_agent(crate::client::USER_AGENT)
            .build()
            .expect("failed to build mail HTTP client");
        Self { http, token: None, config: load_config() }
    }

    /// Try to load saved token from disk
    pub fn try_restore_token(&mut self) {
        let path = token_path();
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(token) = serde_json::from_str::<TokenData>(&data) {
                log::info!("Restored Microsoft mail token from disk");
                self.token = Some(token);
            }
        }
    }

    pub fn save_token(&self) {
        if let Some(ref token) = self.token {
            let path = token_path();
            if let Ok(json) = serde_json::to_string_pretty(token) {
                if let Err(e) = std::fs::write(&path, json) {
                    log::warn!("Failed to save mail token: {}", e);
                } else {
                    #[cfg(unix)]
                    { let _ = std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600)); }
                }
            }
        }
    }

    pub fn clear_token(&mut self) {
        self.token = None;
        let _ = std::fs::remove_file(token_path());
    }

    pub fn is_authenticated(&self) -> bool {
        self.token.is_some()
    }

    /// Build the OAuth2 authorization URL for the webview
    pub fn auth_url(&self) -> String {
        format!(
            "{}/authorize?client_id={}&response_type=code&redirect_uri={}&scope={}&response_mode=query",
            config::MS_AUTHORITY,
            self.config.effective_client_id(),
            urlencoding::encode(MS_REDIRECT_URI),
            urlencoding::encode(MS_SCOPES),
        )
    }

    /// Exchange authorization code for tokens
    pub async fn exchange_code(&mut self, code: &str) -> Result<(), String> {
        let client_id = self.config.effective_client_id().to_string();
        let params = [
            ("client_id", client_id.as_str()),
            ("code", code),
            ("redirect_uri", MS_REDIRECT_URI),
            ("grant_type", "authorization_code"),
            ("scope", MS_SCOPES),
        ];

        let resp = self.http
            .post(format!("{}/token", config::MS_AUTHORITY))
            .form(&params)
            .send()
            .await
            .map_err(|e| format!("トークン交換失敗: {}", e))?;

        let status = resp.status();
        let body: serde_json::Value = resp.json().await
            .map_err(|e| format!("レスポンス解析失敗: {}", e))?;

        if !status.is_success() {
            let err_desc = body["error_description"].as_str().unwrap_or("unknown error");
            return Err(format!("認証エラー: {}", err_desc));
        }

        let access_token = body["access_token"].as_str()
            .ok_or("access_token missing")?.to_string();
        let refresh_token = body["refresh_token"].as_str()
            .ok_or("refresh_token missing")?.to_string();
        let expires_in = body["expires_in"].as_i64().unwrap_or(3600);
        let expires_at = chrono::Utc::now().timestamp() + expires_in;

        self.token = Some(TokenData {
            access_token,
            refresh_token,
            expires_at,
        });
        self.save_token();
        log::info!("Microsoft mail token obtained successfully");
        Ok(())
    }

    /// Refresh the access token using refresh_token
    pub async fn refresh_token(&mut self) -> Result<(), String> {
        let refresh = self.token.as_ref()
            .map(|t| t.refresh_token.clone())
            .ok_or("リフレッシュトークンがありません")?;

        let client_id = self.config.effective_client_id().to_string();
        let params = [
            ("client_id", client_id.as_str()),
            ("refresh_token", refresh.as_str()),
            ("grant_type", "refresh_token"),
            ("scope", MS_SCOPES),
        ];

        let resp = self.http
            .post(format!("{}/token", config::MS_AUTHORITY))
            .form(&params)
            .send()
            .await
            .map_err(|e| format!("トークン更新失敗: {}", e))?;

        let status = resp.status();
        let body: serde_json::Value = resp.json().await
            .map_err(|e| format!("レスポンス解析失敗: {}", e))?;

        if !status.is_success() {
            let err_desc = body["error_description"].as_str().unwrap_or("unknown error");
            self.clear_token();
            return Err(format!("トークン更新失敗: {}", err_desc));
        }

        let access_token = body["access_token"].as_str()
            .ok_or("access_token missing")?.to_string();
        let refresh_token = body["refresh_token"].as_str()
            .unwrap_or(&refresh).to_string();
        let expires_in = body["expires_in"].as_i64().unwrap_or(3600);
        let expires_at = chrono::Utc::now().timestamp() + expires_in;

        self.token = Some(TokenData {
            access_token,
            refresh_token,
            expires_at,
        });
        self.save_token();
        log::info!("Microsoft mail token refreshed");
        Ok(())
    }

    /// Ensure we have a valid (non-expired) access token, refreshing if needed
    async fn ensure_token(&mut self) -> Result<String, String> {
        let token = self.token.as_ref().ok_or(config::MAIL_AUTH_REQUIRED_MSG)?;
        let now = chrono::Utc::now().timestamp();
        if now >= token.expires_at - 60 {
            // Token expired or about to expire, refresh
            self.refresh_token().await?;
        }
        Ok(self.token.as_ref().ok_or("token lost after refresh")?.access_token.clone())
    }

    /// Prepare an HTTP client + valid access token for lock-free network I/O.
    /// Callers should: lock -> prepare_http() -> unlock -> use (http, token) for requests.
    pub async fn prepare_http(&mut self) -> Result<(Client, String), String> {
        let token = self.ensure_token().await?;
        Ok((self.http.clone(), token))
    }

    /// GET request to Graph API with auto-refresh
    async fn graph_get(&mut self, url: &str) -> Result<serde_json::Value, String> {
        let access_token = self.ensure_token().await?;

        let resp = self.http
            .get(url)
            .bearer_auth(&access_token)
            .send()
            .await
            .map_err(|e| format!("Graph APIリクエスト失敗: {}", e))?;

        let status = resp.status();
        if status.as_u16() == 401 {
            // Token might have been revoked, try refresh once
            self.refresh_token().await?;
            let new_token = self.token.as_ref().ok_or("token lost after refresh")?.access_token.clone();
            let resp2 = self.http
                .get(url)
                .bearer_auth(&new_token)
                .send()
                .await
                .map_err(|e| format!("Graph APIリクエスト失敗: {}", e))?;
            if !resp2.status().is_success() {
                self.clear_token();
                return Err(config::MAIL_SESSION_EXPIRED_MSG.into());
            }
            return resp2.json().await.map_err(|e| format!("レスポンス解析失敗: {}", e));
        }

        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Graph APIエラー ({}): {}", status, body));
        }

        resp.json().await.map_err(|e| format!("レスポンス解析失敗: {}", e))
    }

    /// Fetch user's mail profile
    pub async fn fetch_profile(&mut self) -> Result<MailProfile, String> {
        let body = self.graph_get(&format!("{}/me?$select=displayName,mail,userPrincipalName", config::GRAPH_BASE)).await?;
        serde_json::from_value(body).map_err(|e| format!("プロフィール解析失敗: {}", e))
    }

    /// Fetch inbox messages
    pub async fn fetch_inbox(&mut self, top: u32, skip: u32) -> Result<Vec<MailMessage>, String> {
        let url = format!(
            "{}/me/mailFolders/inbox/messages?$top={}&$skip={}&$orderby=receivedDateTime desc&$select=id,subject,bodyPreview,from,receivedDateTime,isRead,hasAttachments",
            config::GRAPH_BASE, top, skip,
        );
        let body = self.graph_get(&url).await?;
        let resp: GraphListResponse<MailMessage> = serde_json::from_value(body)
            .map_err(|e| format!("メール解析失敗: {}", e))?;
        Ok(resp.value)
    }

    /// Fetch a single message detail
    pub async fn fetch_message(&mut self, message_id: &str) -> Result<MailDetail, String> {
        validate_message_id(message_id)?;
        let url = format!(
            "{}/me/messages/{}?$select=id,subject,body,from,receivedDateTime,isRead,hasAttachments,toRecipients,ccRecipients",
            config::GRAPH_BASE, message_id,
        );
        let body = self.graph_get(&url).await?;
        serde_json::from_value(body).map_err(|e| format!("メール詳細解析失敗: {}", e))
    }

    /// Mark a message as read
    pub async fn mark_as_read(&mut self, message_id: &str) -> Result<(), String> {
        validate_message_id(message_id)?;
        let access_token = self.ensure_token().await?;
        let url = format!("{}/me/messages/{}", config::GRAPH_BASE, message_id);
        let body = serde_json::json!({"isRead": true});
        let resp = self.http
            .patch(&url)
            .bearer_auth(&access_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("既読設定失敗: {}", e))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            log::warn!("mark_as_read failed: HTTP {} - {}", status, body);
            return Err(format!("既読設定失敗: HTTP {}", status));
        }
        Ok(())
    }

    /// GET request to Graph API returning raw bytes (for attachment downloads)
    async fn graph_get_bytes(&mut self, url: &str) -> Result<Vec<u8>, String> {
        let access_token = self.ensure_token().await?;
        let resp = self.http
            .get(url)
            .bearer_auth(&access_token)
            .send()
            .await
            .map_err(|e| format!("Graph APIリクエスト失敗: {}", e))?;
        let status = resp.status();
        if status.as_u16() == 401 {
            self.refresh_token().await?;
            let new_token = self.token.as_ref().ok_or("token lost after refresh")?.access_token.clone();
            let resp2 = self.http
                .get(url)
                .bearer_auth(&new_token)
                .send()
                .await
                .map_err(|e| format!("Graph APIリクエスト失敗: {}", e))?;
            if !resp2.status().is_success() {
                self.clear_token();
                return Err(config::MAIL_SESSION_EXPIRED_MSG.into());
            }
            return resp2.bytes().await.map(|b| b.to_vec()).map_err(|e| format!("レスポンス読み込み失敗: {}", e));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Graph APIエラー ({}): {}", status, body));
        }
        resp.bytes().await.map(|b| b.to_vec()).map_err(|e| format!("レスポンス読み込み失敗: {}", e))
    }

    /// Fetch attachment metadata for a message (no content bytes)
    pub async fn fetch_attachments(&mut self, message_id: &str) -> Result<Vec<MailAttachment>, String> {
        validate_message_id(message_id)?;
        let url = format!(
            "{}/me/messages/{}/attachments?$select=id,name,contentType,size",
            config::GRAPH_BASE, message_id,
        );
        let body = self.graph_get(&url).await?;
        let resp: GraphListResponse<MailAttachment> = serde_json::from_value(body)
            .map_err(|e| format!("添付ファイル解析失敗: {}", e))?;
        Ok(resp.value)
    }

    /// Download a single attachment and save it to the Downloads folder.
    /// Returns the saved file path as a string.
    pub async fn download_attachment(
        &mut self,
        message_id: &str,
        attachment_id: &str,
        file_name: &str,
    ) -> Result<String, String> {
        validate_message_id(message_id)?;
        validate_attachment_id(attachment_id)?;

        // Sanitize file name: keep only the basename, replace dangerous chars
        let safe_name: String = std::path::Path::new(file_name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("attachment")
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || ".-_ ()[]".contains(c) { c } else { '_' })
            .collect();
        let safe_name = if safe_name.is_empty() { "attachment".to_string() } else { safe_name };

        let url = format!(
            "{}/me/messages/{}/attachments/{}/$value",
            config::GRAPH_BASE,
            message_id,
            urlencoding::encode(attachment_id),
        );
        let data = self.graph_get_bytes(&url).await?;

        let downloads_dir = dirs::download_dir()
            .or_else(|| dirs::home_dir())
            .unwrap_or_else(|| PathBuf::from("."));

        // Avoid overwriting: append a counter if file exists
        let mut dest = downloads_dir.join(&safe_name);
        if dest.exists() {
            let stem = std::path::Path::new(&safe_name)
                .file_stem().and_then(|s| s.to_str()).unwrap_or("attachment");
            let ext = std::path::Path::new(&safe_name)
                .extension().and_then(|s| s.to_str()).unwrap_or("");
            let mut i = 1u32;
            loop {
                let candidate = if ext.is_empty() {
                    format!("{} ({})", stem, i)
                } else {
                    format!("{} ({}).{}", stem, i, ext)
                };
                dest = downloads_dir.join(&candidate);
                if !dest.exists() { break; }
                i += 1;
            }
        }

        std::fs::write(&dest, &data)
            .map_err(|e| format!("ファイル保存失敗: {}", e))?;

        let path_str = dest.to_string_lossy().to_string();
        log::info!("Attachment saved to: {}", path_str);
        Ok(path_str)
    }
}

/// Lock-free Graph API GET. Returns Err((msg, needs_reauth)).
/// On 401, returns Err with needs_reauth=true so callers can re-lock and retry.
pub async fn graph_get_lockfree(
    http: &Client,
    url: &str,
    token: &str,
) -> Result<serde_json::Value, (String, bool)> {
    let resp = http
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| (format!("Graph APIリクエスト失敗: {}", e), false))?;

    let status = resp.status();
    if status.as_u16() == 401 {
        return Err((config::MAIL_SESSION_EXPIRED_MSG.into(), true));
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err((format!("Graph APIエラー ({}): {}", status, body), false));
    }
    resp.json()
        .await
        .map_err(|e| (format!("レスポンス解析失敗: {}", e), false))
}
