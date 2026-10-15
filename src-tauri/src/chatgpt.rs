//! ChatGPT plan usage through the public Sign in with ChatGPT OAuth flow.
//! Credentials stay in the native secret vault; the webview receives status only.
mod auth;
mod inference;
#[cfg(test)]
mod tests;

pub(crate) use auth::*;
pub(crate) use inference::complete;

const ISSUER: &str = "https://auth.openai.com";
const RESOURCE: &str = "https://api.openai.com/v1";
const TOKEN_URL: &str = "https://auth.openai.com/api/accounts/oauth/token";
const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";

static HTTP: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent(crate::client::USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .expect("ChatGPT HTTP client")
});

fn request_error(status: reqwest::StatusCode, body: &serde_json::Value) -> String {
    let code = body
        .pointer("/error/code")
        .and_then(|v| v.as_str())
        .or_else(|| body.get("error").and_then(|v| v.as_str()))
        .or_else(|| body.get("code").and_then(|v| v.as_str()))
        .unwrap_or("");
    match code {
        "subscription_sharing_usage_limit_exceeded" => "ChatGPT の利用上限に達しました。ChatGPT の「使用状況」で上限を確認してください。".into(),
        "subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable" => "ChatGPT の利用状況を一時的に確認できません。しばらく待ってから再試行してください。".into(),
        "subscription_sharing_user_not_eligible" => "このアカウントまたはワークスペースでは ChatGPT プランを Selah で利用できません。対象プランと管理者の設定を確認してください。".into(),
        "invalid_grant" | "invalid_refresh_token" | "token_expired" | "refresh_token_expired"
        | "refresh_token_invalidated" | "refresh_token_reused" => "ChatGPT のセッションが失効しました。もう一度ログインしてください。".into(),
        _ if status == reqwest::StatusCode::UNAUTHORIZED => "ChatGPT の認証が失効しました。もう一度ログインしてください。".into(),
        _ if status == reqwest::StatusCode::FORBIDDEN => "ChatGPT プランへのアクセスが拒否されました。アカウントの権限を確認してください。".into(),
        _ if status == reqwest::StatusCode::TOO_MANY_REQUESTS => "ChatGPT のリクエスト上限に達しました。しばらく待ってから再試行してください。".into(),
        _ => format!("ChatGPT リクエストに失敗しました ({status})。接続とアカウントの権限を確認してください。"),
    }
}
