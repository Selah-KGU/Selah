use crate::oauth_http::Http;
use std::path::PathBuf;

use crate::config;

use super::config::load_config;
use super::types::TokenData;

pub(super) const MS_REDIRECT_URI: &str = "http://localhost";
pub(super) const MS_SCOPES: &str = "Mail.ReadWrite offline_access";

const TOKEN_FILE: &str = "ms_mail_token.json";

fn token_path() -> PathBuf {
    crate::client::data_dir().join(TOKEN_FILE)
}

impl super::MailClient {
    pub fn new() -> Self {
        let http = Http::new();
        Self {
            http,
            lifecycle: Default::default(),
            new_login: false,
            logout_requested: false,
            token: None,
            config: load_config(),
        }
    }

    /// Try to load saved token — keychain first, then migrate from legacy JSON file
    pub fn try_restore_token(&mut self) -> Result<bool, crate::keychain::StoreError> {
        if self.logout_requested {
            crate::keychain::tokens::revoke("ms_mail_token", &token_path())?;
            return Ok(false);
        }
        if let Some(mut token) =
            crate::keychain::tokens::restore::<TokenData>("ms_mail_token", &token_path())?
        {
            if token.connection_id.is_empty() {
                token.connection_id = uuid::Uuid::new_v4().to_string();
            }
            super::cache::set_owner(Some(token.connection_id.clone()));
            self.token = Some(token);
            self.lifecycle.published();
            self.save_token()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn save_token(&self) -> Result<(), crate::keychain::StoreError> {
        if self.logout_requested {
            return crate::keychain::tokens::revoke("ms_mail_token", &token_path());
        }
        crate::keychain::tokens::save(
            "ms_mail_token",
            &token_path(),
            self.token.as_ref(),
            self.new_login,
        )
    }

    pub(crate) fn cancellation(&self) -> crate::oauth_http::Cancellation {
        self.http.cancellation()
    }

    pub(crate) fn cancel_requests(&mut self) {
        self.lifecycle.invalidate();
        self.http.cancellation().cancel();
        self.http.renew();
    }

    pub(crate) fn retire(&mut self) {
        self.cancel_requests();
        self.token = None;
        self.new_login = false;
        self.logout_requested = false;
        super::cache::set_owner(None);
    }

    pub fn clear_token(&mut self) -> Result<(), crate::keychain::StoreError> {
        self.retire();
        self.logout_requested = true;
        crate::keychain::tokens::revoke("ms_mail_token", &token_path())
    }

    pub(crate) fn connection_id(&self) -> Option<&str> {
        self.token.as_ref().map(|t| t.connection_id.as_str())
    }
    pub(crate) fn ensure_connection(&self, connection: &str) -> Result<(), String> {
        if self.connection_id() == Some(connection) {
            Ok(())
        } else {
            Err("Mail connection changed; discard the previous request".into())
        }
    }

    pub fn is_authenticated(&self) -> bool {
        self.token.is_some()
    }

    pub(crate) fn begin_login(&mut self) -> super::oauth::LoginAttempt {
        self.http.renew();
        super::oauth::LoginAttempt::new(
            self.lifecycle.begin(),
            self.http.clone(),
            self.config.effective_client_id().to_owned(),
        )
    }

    pub(crate) fn accept_login(
        &mut self,
        attempt: &super::oauth::LoginAttempt,
        token: TokenData,
    ) -> Result<(), String> {
        self.lifecycle.ensure(&attempt.lifetime)?;
        attempt.http.ensure_current()?;
        self.new_login = true;
        self.logout_requested = false;
        super::cache::set_owner(Some(token.connection_id.clone()));
        self.token = Some(token);
        self.lifecycle.published();
        if let Err(error) = self.save_token() {
            log::warn!("Token persistence pending: {error}");
        }
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
            .send(
                self.http
                    .client
                    .post(format!("{}/token", config::MS_AUTHORITY))
                    .form(&params),
            )
            .await
            .map_err(|e| format!("トークン更新失敗: {}", e))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .map_err(|e| format!("レスポンス解析失敗: {}", e))?;

        if !status.is_success() {
            let err_desc = body["error_description"]
                .as_str()
                .unwrap_or("unknown error");
            if crate::oauth_lifecycle::refresh_grant_rejected(status, &body) {
                self.clear_token()?;
            }
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
            connection_id: self.connection_id().unwrap_or_default().to_owned(),
            access_token,
            refresh_token,
            expires_at,
        });
        if let Err(error) = self.save_token() {
            log::warn!("Token persistence pending: {error}");
        }
        log::info!("Microsoft mail token refreshed");
        Ok(())
    }

    /// Ensure we have a valid (non-expired) access token, refreshing if needed
    pub(in crate::mail) async fn ensure_token(&mut self) -> Result<String, String> {
        self.http.ensure_current()?;
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
    pub(crate) async fn prepare_http(&mut self) -> Result<(Http, String), String> {
        let token = self.ensure_token().await?;
        Ok((self.http.clone(), token))
    }
}

impl super::oauth::LoginAttempt {
    /// Exchange authorization code for tokens
    pub async fn exchange_code(&self, code: &str) -> Result<TokenData, String> {
        self.exchange_code_at(code, &format!("{}/token", config::MS_AUTHORITY))
            .await
    }

    async fn exchange_code_at(&self, code: &str, endpoint: &str) -> Result<TokenData, String> {
        let client_id = self.client_id.clone();
        let params = [
            ("client_id", client_id.as_str()),
            ("code", code),
            ("redirect_uri", MS_REDIRECT_URI),
            ("grant_type", "authorization_code"),
            ("scope", MS_SCOPES),
            ("code_verifier", self.verifier.as_str()),
        ];

        let resp = self
            .http
            .send(self.http.client.post(endpoint).form(&params))
            .await
            .map_err(|e| format!("トークン交換失敗: {}", e))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
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

        Ok(TokenData {
            connection_id: uuid::Uuid::new_v4().to_string(),
            access_token,
            refresh_token,
            expires_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> crate::mail::MailClient {
        crate::mail::MailClient {
            http: Http::new(),
            lifecycle: Default::default(),
            new_login: false,
            logout_requested: false,
            token: None,
            config: crate::mail::MailConfig {
                client_id: "test-client".into(),
            },
        }
    }

    #[tokio::test]
    async fn token_request_uses_attempt_verifier_and_cancelled_result_cannot_publish() {
        let mut client = client();
        let attempt = client.begin_login();
        let (url, server) = crate::oauth_http::tests::token_server().await;
        let token = attempt.exchange_code_at("test-code", &url).await.unwrap();
        let form = server.await.unwrap();
        assert_eq!(form["code_verifier"], attempt.verifier);
        assert_eq!(form["client_id"], "test-client");
        assert_eq!(form["code"], "test-code");
        assert_eq!(form["redirect_uri"], MS_REDIRECT_URI);
        assert!(client.token.is_none()); // Network completion alone cannot publish.
        client.cancellation().cancel(); // Logout started before it obtains the mutex.
        assert!(client.accept_login(&attempt, token.clone()).is_err());
        let _new_attempt = client.begin_login();
        assert!(client.accept_login(&attempt, token).is_err());
        assert!(client.token.is_none());
        assert!(!client.new_login);
    }
}
