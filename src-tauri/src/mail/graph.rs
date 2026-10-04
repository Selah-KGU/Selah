use reqwest::Client;

use crate::config;

impl super::MailClient {
    /// GET request to Graph API with auto-refresh
    pub(in crate::mail) async fn graph_get(
        &mut self,
        url: &str,
    ) -> Result<serde_json::Value, String> {
        self.graph_get_with_headers(url, &[]).await
    }

    /// Like [`graph_get`] but allows extra request headers
    /// (e.g. `Prefer: outlook.body-content-type="text"`).
    pub(in crate::mail) async fn graph_get_with_headers(
        &mut self,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<serde_json::Value, String> {
        let access_token = self.ensure_token().await?;

        let mut req = self.http.get(url).bearer_auth(&access_token);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("Graph APIリクエスト失敗: {}", e))?;

        let status = resp.status();
        if status.as_u16() == 401 {
            // Token might have been revoked, try refresh once
            self.refresh_token().await?;
            let new_token = self
                .token
                .as_ref()
                .ok_or("token lost after refresh")?
                .access_token
                .clone();
            let mut req2 = self.http.get(url).bearer_auth(&new_token);
            for (k, v) in headers {
                req2 = req2.header(*k, *v);
            }
            let resp2 = req2
                .send()
                .await
                .map_err(|e| format!("Graph APIリクエスト失敗: {}", e))?;
            if !resp2.status().is_success() {
                self.clear_token();
                return Err(config::MAIL_SESSION_EXPIRED_MSG.into());
            }
            return resp2
                .json()
                .await
                .map_err(|e| format!("レスポンス解析失敗: {}", e));
        }

        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Graph APIエラー ({}): {}", status, body));
        }

        resp.json()
            .await
            .map_err(|e| format!("レスポンス解析失敗: {}", e))
    }

    /// GET request to Graph API returning raw bytes (for attachment downloads)
    pub(in crate::mail) async fn graph_get_bytes(&mut self, url: &str) -> Result<Vec<u8>, String> {
        let access_token = self.ensure_token().await?;
        let resp = self
            .http
            .get(url)
            .bearer_auth(&access_token)
            .send()
            .await
            .map_err(|e| format!("Graph APIリクエスト失敗: {}", e))?;
        let status = resp.status();
        if status.as_u16() == 401 {
            self.refresh_token().await?;
            let new_token = self
                .token
                .as_ref()
                .ok_or("token lost after refresh")?
                .access_token
                .clone();
            let resp2 = self
                .http
                .get(url)
                .bearer_auth(&new_token)
                .send()
                .await
                .map_err(|e| format!("Graph APIリクエスト失敗: {}", e))?;
            if !resp2.status().is_success() {
                self.clear_token();
                return Err(config::MAIL_SESSION_EXPIRED_MSG.into());
            }
            return resp2
                .bytes()
                .await
                .map(|b| b.to_vec())
                .map_err(|e| format!("レスポンス読み込み失敗: {}", e));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Graph APIエラー ({}): {}", status, body));
        }
        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| format!("レスポンス読み込み失敗: {}", e))
    }
}

/// Lock-free Graph API GET. Returns Err((msg, needs_reauth)).
/// On 401, returns Err with needs_reauth=true so callers can re-lock and retry.
pub async fn graph_get_lockfree(
    http: &Client,
    url: &str,
    token: &str,
) -> Result<serde_json::Value, (String, bool)> {
    graph_get_lockfree_with_headers(http, url, token, &[]).await
}

/// Same as [`graph_get_lockfree`] but allows passing extra request headers
/// (e.g. `Prefer: outlook.body-content-type="text"` to fetch plain-text bodies).
pub async fn graph_get_lockfree_with_headers(
    http: &Client,
    url: &str,
    token: &str,
    headers: &[(&str, &str)],
) -> Result<serde_json::Value, (String, bool)> {
    let mut req = http.get(url).bearer_auth(token);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let resp = req
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
