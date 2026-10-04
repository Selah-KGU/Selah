//! Shared HTTP client and non-streaming retry for remote providers.

use std::sync::LazyLock;

const NON_STREAMING_ATTEMPTS: usize = 2;
// ─────────────────────── Remote: non-streaming (plan) ───────────────────────

/// HTTP client shared with `ai.rs`.
pub fn http_client() -> &'static reqwest::Client {
    // Reuse the same LazyLock-based client from ai.rs.
    // We access it by calling a non-streaming chat completion.
    // For decoupling, we build our own minimal client.
    static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
        reqwest::Client::builder()
            // Planning calls can return large JSON payloads. A short whole-request
            // timeout can expire while a healthy provider is still sending them.
            .timeout(std::time::Duration::from_secs(300))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("failed to build HTTP client")
    });
    &CLIENT
}

pub async fn send_non_streaming_request(
    request: reqwest::RequestBuilder,
    provider: &str,
) -> Result<(reqwest::StatusCode, String), String> {
    for attempt in 1..=NON_STREAMING_ATTEMPTS {
        let request = request
            .try_clone()
            .ok_or_else(|| "AIリクエストを再試行用に複製できませんでした".to_string())?;
        let resp = match request.send().await {
            Ok(resp) => resp,
            Err(error) if attempt < NON_STREAMING_ATTEMPTS => {
                log::warn!(
                    "plan({}): request transport failed on attempt {}/{}; retrying: {}",
                    provider,
                    attempt,
                    NON_STREAMING_ATTEMPTS,
                    error
                );
                tokio::time::sleep(std::time::Duration::from_millis(750)).await;
                continue;
            }
            Err(error) => return Err(format!("リクエスト失敗: {}", error)),
        };
        let status = resp.status();
        match resp.text().await {
            Ok(text) => return Ok((status, text)),
            Err(error) => {
                return Err(format!(
                    "AI応答の受信が途中で中断されました。重複生成を避けるため自動再試行しません: {}",
                    error
                ));
            }
        }
    }
    Err("AI応答を受信できませんでした".to_string())
}
