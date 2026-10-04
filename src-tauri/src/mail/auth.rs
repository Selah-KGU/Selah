use reqwest::Client;
use std::path::PathBuf;

use crate::config;

use super::config::load_config;
use super::types::TokenData;

const MS_REDIRECT_URI: &str = "http://localhost";
const MS_SCOPES: &str = "Mail.ReadWrite offline_access";

const TOKEN_FILE: &str = "ms_mail_token.json";

fn token_path() -> PathBuf {
    crate::client::data_dir().join(TOKEN_FILE)
}

impl super::MailClient {
    pub fn new() -> Self {
        let http = Client::builder()
            .user_agent(crate::client::USER_AGENT)
            .build()
            .expect("failed to build mail HTTP client");
        Self {
            http,
            token: None,
            config: load_config(),
        }
    }

    /// Try to load saved token — keychain first, then migrate from legacy JSON file
    pub fn try_restore_token(&mut self) {
        // Prefer keychain
        if let Some(json) = crate::keychain::get_secret("ms_mail_token") {
            if let Ok(token) = serde_json::from_str::<TokenData>(&json) {
                log::info!("Restored Microsoft mail token from keychain");
                self.token = Some(token);
                return;
            }
        }
        // Legacy file migration
        let path = token_path();
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(token) = serde_json::from_str::<TokenData>(&data) {
                log::info!("Migrating Microsoft mail token from file to keychain");
                self.token = Some(token);
                self.save_token(); // persist into keychain
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    pub fn save_token(&self) {
        if let Some(ref token) = self.token {
            if let Ok(json) = serde_json::to_string(token) {
                if let Err(e) = crate::keychain::set_secret("ms_mail_token", &json) {
                    log::warn!("Failed to save mail token to keychain: {}", e);
                }
            }
        }
    }

    pub fn clear_token(&mut self) {
        self.token = None;
        crate::keychain::delete_secret("ms_mail_token");
        let _ = std::fs::remove_file(token_path()); // clean up legacy file too
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

        let resp = self
            .http
            .post(format!("{}/token", config::MS_AUTHORITY))
            .form(&params)
            .send()
            .await
            .map_err(|e| format!("トークン交換失敗: {}", e))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("レスポンス解析失敗: {}", e))?;

        if !status.is_success() {
            let err_desc = body["error_description"]
                .as_str()
                .unwrap_or("unknown error");
            return Err(format!("認証エラー: {}", err_desc));
        }

        let access_token = body["access_token"]
            .as_str()
            .ok_or("access_token missing")?
            .to_string();
        let refresh_token = body["refresh_token"]
            .as_str()
            .ok_or("refresh_token missing")?
            .to_string();
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
        let refresh = self
            .token
            .as_ref()
            .map(|t| t.refresh_token.clone())
            .ok_or("リフレッシュトークンがありません")?;

        let client_id = self.config.effective_client_id().to_string();
        let params = [
            ("client_id", client_id.as_str()),
            ("refresh_token", refresh.as_str()),
            ("grant_type", "refresh_token"),
            ("scope", MS_SCOPES),
        ];

        let resp = self
            .http
            .post(format!("{}/token", config::MS_AUTHORITY))
            .form(&params)
            .send()
            .await
            .map_err(|e| format!("トークン更新失敗: {}", e))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("レスポンス解析失敗: {}", e))?;

        if !status.is_success() {
            let err_desc = body["error_description"]
                .as_str()
                .unwrap_or("unknown error");
            self.clear_token();
            return Err(format!("トークン更新失敗: {}", err_desc));
        }

        let access_token = body["access_token"]
            .as_str()
            .ok_or("access_token missing")?
            .to_string();
        let refresh_token = body["refresh_token"]
            .as_str()
            .unwrap_or(&refresh)
            .to_string();
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
    pub(in crate::mail) async fn ensure_token(&mut self) -> Result<String, String> {
        let token = self.token.as_ref().ok_or(config::MAIL_AUTH_REQUIRED_MSG)?;
        let now = chrono::Utc::now().timestamp();
        if now >= token.expires_at - 60 {
            // Token expired or about to expire, refresh
            self.refresh_token().await?;
        }
        Ok(self
            .token
            .as_ref()
            .ok_or("token lost after refresh")?
            .access_token
            .clone())
    }

    /// Prepare an HTTP client + valid access token for lock-free network I/O.
    /// Callers should: lock -> prepare_http() -> unlock -> use (http, token) for requests.
    pub async fn prepare_http(&mut self) -> Result<(Client, String), String> {
        let token = self.ensure_token().await?;
        Ok((self.http.clone(), token))
    }
}
