use sha2::{Digest, Sha256};

use super::{OAuthLoginAttempt, TokenData, GOOGLE_AUTH_URL, GOOGLE_TOKEN_URL, SCOPES};

fn generate_pkce() -> (String, String) {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let verifier: String = (0..64)
        .map(|_| {
            let idx = rng.gen_range(0..66);
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~"[idx] as char
        })
        .collect();
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    let challenge = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, hash);
    (verifier, challenge)
}

impl super::GoogleCalendarClient {
    /// Start a loopback login without storing PKCE on the shared client.
    /// Each attempt carries its own verifier, redirect URI, and state so a
    /// second login cannot clobber an in-flight callback.
    pub fn begin_login(&self, port: u16) -> Result<OAuthLoginAttempt, String> {
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
        Ok(OAuthLoginAttempt {
            url,
            verifier,
            redirect_uri,
            state,
        })
    }

    pub async fn exchange_code(
        &mut self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
    ) -> Result<(), String> {
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
            .post(GOOGLE_TOKEN_URL)
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
            let err = body["error_description"]
                .as_str()
                .or(body["error"].as_str())
                .unwrap_or("unknown error");
            return Err(format!("Google認証エラー: {}", err));
        }

        self.token = Some(TokenData {
            access_token: body["access_token"]
                .as_str()
                .ok_or("access_token missing")?
                .into(),
            refresh_token: body["refresh_token"]
                .as_str()
                .ok_or("refresh_token missing")?
                .into(),
            expires_at: chrono::Utc::now().timestamp()
                + body["expires_in"].as_i64().unwrap_or(3600),
        });
        self.save_token();
        log::info!("Google Calendar token obtained");
        Ok(())
    }

    pub async fn refresh_token(&mut self) -> Result<(), String> {
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
            .post(GOOGLE_TOKEN_URL)
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
            self.clear_token();
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
            expires_at: chrono::Utc::now().timestamp()
                + body["expires_in"].as_i64().unwrap_or(3600),
        });
        self.save_token();
        Ok(())
    }

    pub(super) async fn ensure_token(&mut self) -> Result<String, String> {
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
