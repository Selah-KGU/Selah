use crate::oauth_lifecycle::generate_pkce;

use super::{OAuthLoginAttempt, TokenData, GOOGLE_AUTH_URL, GOOGLE_TOKEN_URL, SCOPES};

impl super::GoogleCalendarClient {
    /// Start a loopback login without storing PKCE on the shared client.
    /// Each attempt carries its own verifier, redirect URI, and state so a
    /// second login cannot clobber an in-flight callback.
    pub fn begin_login(&mut self, port: u16) -> Result<OAuthLoginAttempt, String> {
        self.ensure_config()?;
        if self.config.client_id.trim().is_empty() {
            return Err("Google Client IDが未設定です。設定画面で入力してください。".into());
        }
        let (verifier, challenge) = generate_pkce();
        let redirect_uri = format!("http://127.0.0.1:{}", port);
        let state = uuid::Uuid::new_v4().to_string();
        let url = format!(
            "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent&code_challenge={}&code_challenge_method=S256&state={}",
            GOOGLE_AUTH_URL,
            urlencoding::encode(self.config.client_id.trim()),
            urlencoding::encode(&redirect_uri),
            urlencoding::encode(SCOPES),
            urlencoding::encode(&challenge),
            urlencoding::encode(&state),
        );
        self.http.renew();
        let lifetime = self.lifecycle.begin();
        Ok(OAuthLoginAttempt {
            http: self.http.clone(),
            config: self.config.clone(),
            lifetime,
            url,
            verifier,
            redirect_uri,
            state,
        })
    }

    pub(crate) fn accept_login(
        &mut self,
        attempt: &OAuthLoginAttempt,
        token: TokenData,
    ) -> Result<(), String> {
        self.lifecycle.ensure(&attempt.lifetime)?;
        attempt.http.ensure_current()?;
        self.new_login = true;
        self.logout_requested = false;
        self.token = Some(token);
        self.lifecycle.published();
        if let Err(error) = self.save_token() {
            log::warn!("Token persistence pending: {error}");
        }
        // Require new consent, but retain event IDs until the selected calendar
        // is checked again. Reauthorizing the same account must not duplicate events.
        self.sync_state.auto_sync_binding = None;
        super::config::save_sync_state(&self.sync_state)?;
        log::info!("Google Calendar token obtained");
        Ok(())
    }

    pub async fn refresh_token(&mut self) -> Result<(), String> {
        self.ensure_config()?;
        let refresh = self
            .token
            .as_ref()
            .map(|t| t.refresh_token.clone())
            .ok_or("リフレッシュトークンがありません")?;
        let client_id = self.config.client_id.trim().to_string();
        let client_secret = self.config.client_secret.trim().to_string();

        let mut params = vec![
            ("client_id", client_id.as_str()),
            ("refresh_token", refresh.as_str()),
            ("grant_type", "refresh_token"),
        ];
        if !client_secret.is_empty() {
            params.push(("client_secret", client_secret.as_str()));
        }

        let resp = self
            .http
            .send(self.http.client.post(GOOGLE_TOKEN_URL).form(&params))
            .await
            .map_err(|e| format!("トークン更新失敗: {}", e))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .map_err(|e| format!("レスポンス解析失敗: {}", e))?;
        if !status.is_success() {
            if crate::oauth_lifecycle::refresh_grant_rejected(status, &body) {
                self.clear_token()?;
            }
            let err = body["error_description"]
                .as_str()
                .or(body["error"].as_str())
                .unwrap_or("unknown error");
            return Err(format!("トークン更新失敗: {}", err));
        }

        self.token = Some(TokenData {
            access_token: body["access_token"]
                .as_str()
                .ok_or("access_token missing")?
                .into(),
            refresh_token: body["refresh_token"].as_str().unwrap_or(&refresh).into(),
            connection_id: self
                .token
                .as_ref()
                .map(|token| token.connection_id.clone())
                .unwrap_or_default(),
            expires_at: chrono::Utc::now().timestamp()
                + body["expires_in"].as_i64().unwrap_or(3600),
        });
        if let Err(error) = self.save_token() {
            log::warn!("Token persistence pending: {error}");
        }
        Ok(())
    }

    pub(super) async fn ensure_token(&mut self) -> Result<String, String> {
        self.http.ensure_current()?;
        if self.token.is_none() {
            return Err("Google Calendarにログインしてください".into());
        }
        let needs_refresh = self
            .token
            .as_ref()
            .map(|t| chrono::Utc::now().timestamp() >= t.expires_at - 60)
            .unwrap_or(true);
        if needs_refresh {
            self.refresh_token().await?;
        }
        Ok(self
            .token
            .as_ref()
            .ok_or("token lost after refresh")?
            .access_token
            .clone())
    }
}

impl OAuthLoginAttempt {
    pub async fn exchange_code(&self, code: &str) -> Result<TokenData, String> {
        self.exchange_code_at(code, GOOGLE_TOKEN_URL).await
    }

    async fn exchange_code_at(&self, code: &str, endpoint: &str) -> Result<TokenData, String> {
        let verifier = self.verifier.as_str();
        let redirect_uri = self.redirect_uri.as_str();
        if verifier.is_empty() {
            return Err("PKCE verifier missing. Please retry login.".into());
        }
        if redirect_uri.is_empty() {
            return Err("Redirect URI missing. Please retry login.".into());
        }
        let client_id = self.config.client_id.trim().to_string();
        let client_secret = self.config.client_secret.trim().to_string();

        let mut params = vec![
            ("client_id", client_id.as_str()),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier),
        ];
        if !client_secret.is_empty() {
            params.push(("client_secret", client_secret.as_str()));
        }

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
            let err = body["error_description"]
                .as_str()
                .or(body["error"].as_str())
                .unwrap_or("unknown error");
            return Err(format!("Google認証エラー: {}", err));
        }

        let token = TokenData {
            access_token: body["access_token"]
                .as_str()
                .ok_or("access_token missing")?
                .into(),
            refresh_token: body["refresh_token"]
                .as_str()
                .ok_or("refresh_token missing")?
                .into(),
            connection_id: uuid::Uuid::new_v4().to_string(),
            expires_at: chrono::Utc::now().timestamp()
                + body["expires_in"].as_i64().unwrap_or(3600),
        };
        Ok(token)
    }
}

#[cfg(test)]
mod tests {

    #[tokio::test]
    async fn exchange_captures_config_and_rejects_result_after_disconnect_or_new_login() {
        let mut client = crate::google_calendar::GoogleCalendarClient {
            http: crate::oauth_http::Http::new(),
            lifecycle: Default::default(),
            new_login: false,
            logout_requested: false,
            token: None,
            config: crate::google_calendar::GoogleCalConfig {
                client_id: "test-client".into(),
                client_secret: "test-secret".into(),
            },
            config_error: None,
            sync_state: Default::default(),
        };
        let attempt = client.begin_login(1234).unwrap();
        client.config.client_id = "changed-client".into();
        let (url, server) = crate::oauth_http::tests::token_server().await;
        let token = attempt.exchange_code_at("test-code", &url).await.unwrap();
        let form = server.await.unwrap();
        assert_eq!(form["client_id"], "test-client");
        assert_eq!(form["client_secret"], "test-secret");
        assert_eq!(form["code_verifier"], attempt.verifier);
        assert_eq!(form["redirect_uri"], attempt.redirect_uri);
        assert!(client.token.is_none());
        client.cancellation().cancel();
        assert!(client.accept_login(&attempt, token.clone()).is_err());
        client.sync_state.calendar_id = "calendar".into();
        client
            .sync_state
            .event_map
            .insert("2026-10-05-1".into(), "event".into());
        assert!(client
            .sync_to_calendar(Vec::new(), "2026-10-05".into(), "calendar".into())
            .await
            .is_err());
        assert!(client.clear_calendar(false).await.is_err());
        assert_eq!(
            client
                .sync_state
                .event_map
                .get("2026-10-05-1")
                .map(String::as_str),
            Some("event")
        );
        let _new = client.begin_login(1235).unwrap();
        assert!(client.accept_login(&attempt, token).is_err());
        assert!(client.token.is_none());
        assert!(!client.new_login);
    }
}
