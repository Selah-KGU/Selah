use super::*;
use crate::oauth_lifecycle::{Attempt, Lifecycle};
use serde::{Deserialize, Serialize};
use std::sync::{LazyLock, Mutex, OnceLock};
use tauri::{Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const STORE_KEY: &str = "chatgpt_accounts";
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static LOGIN: LazyLock<Mutex<Lifecycle>> = LazyLock::new(|| Mutex::new(Lifecycle::default()));
static VERSION: LazyLock<tokio::sync::watch::Sender<u64>> =
    LazyLock::new(|| tokio::sync::watch::channel(0).0);
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();

pub(crate) fn init(app: &tauri::AppHandle) {
    let _ = APP.set(app.clone());
}

// One atomic vault record keeps rotating tokens and account mappings together.
#[derive(Default, Serialize, Deserialize)]
pub(super) struct Store {
    host_id: String,
    active: Option<String>,
    accounts: Vec<Account>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Account {
    client_id: String,
    subject: String,
    email: String,
    access_token: String,
    refresh_token: String,
    id_token: String,
    expires_at: i64,
    scopes: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct AccountStatus {
    client_id: String,
    email: String,
    active: bool,
    connected: bool,
    plan_enabled: bool,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Model {
    pub slug: String,
    pub display_name: String,
}

#[derive(Deserialize)]
struct Tokens {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    token_type: String,
    expires_in: i64,
    #[serde(default)]
    scope: Option<String>,
}

impl Account {
    fn connected(&self) -> bool {
        !self.access_token.is_empty()
            && (self.expires_at > chrono::Utc::now().timestamp() || !self.refresh_token.is_empty())
    }
    fn plan_enabled(&self) -> bool {
        self.scopes.iter().any(|s| s == PLAN_SCOPE)
    }
    fn apply(&mut self, tokens: Tokens, initial: bool) -> Result<(), String> {
        if !tokens.token_type.eq_ignore_ascii_case("bearer")
            || tokens.access_token.is_empty()
            || tokens.expires_in <= 0
            || tokens.expires_in > 31_536_000
        {
            return Err("ChatGPT のトークン応答が正しくありません".into());
        }
        let scopes = tokens
            .scope
            .map(|s| s.split_whitespace().map(str::to_owned).collect())
            .unwrap_or_else(|| {
                if initial {
                    Vec::new()
                } else {
                    self.scopes.clone()
                }
            });
        self.access_token = tokens.access_token;
        if initial {
            self.refresh_token.clear();
            self.id_token.clear();
        }
        if let Some(refresh) = tokens.refresh_token {
            self.refresh_token = refresh;
        }
        if let Some(id) = tokens.id_token {
            self.id_token = id;
        }
        self.expires_at = chrono::Utc::now().timestamp() + tokens.expires_in;
        self.scopes = scopes;
        Ok(())
    }
    fn clear_tokens(&mut self) {
        self.access_token.clear();
        self.refresh_token.clear();
        self.id_token.clear();
        self.expires_at = 0;
        self.scopes.clear();
    }
}

fn read() -> Result<Store, String> {
    crate::keychain::get_secret(STORE_KEY)
        .map_err(|e| e.to_string())?
        .map(|s| {
            serde_json::from_str(&s)
                .map_err(|_| "ChatGPT の保存済みアカウントを読み込めません".into())
        })
        .unwrap_or_else(|| Ok(Store::default()))
}
async fn load() -> Result<Store, String> {
    crate::background_ipc::run("ChatGPT アカウント読み込み失敗", read).await
}
async fn save(store: Store) -> Result<(), String> {
    crate::background_ipc::run("ChatGPT アカウント保存失敗", move || {
        let json = serde_json::to_string(&store).map_err(|_| "ChatGPT アカウント変換失敗")?;
        crate::keychain::set_secret(STORE_KEY, &json)
    })
    .await
}

async fn save_attempt(store: Store, attempt: Attempt) -> Result<(), String> {
    crate::background_ipc::run("ChatGPT アカウント保存失敗", move || {
        // Cancellation and publication share one critical section so a late
        // callback can never reconnect an account after logout/cancel.
        let lifecycle = LOGIN.lock().unwrap_or_else(|e| e.into_inner());
        lifecycle.ensure(&attempt)?;
        let json = serde_json::to_string(&store).map_err(|_| "ChatGPT アカウント変換失敗")?;
        crate::keychain::set_secret(STORE_KEY, &json)
    })
    .await
}

pub(crate) fn ensure_connected() -> Result<(), String> {
    let store = read()?;
    if store
        .accounts
        .iter()
        .any(|a| Some(&a.client_id) == store.active.as_ref() && a.connected() && a.plan_enabled())
    {
        Ok(())
    } else {
        Err("AI 設定から ChatGPT にログインし、プランの利用を許可してください。".into())
    }
}

pub(super) fn session() -> tokio::sync::watch::Receiver<u64> {
    VERSION.subscribe()
}
fn changed(app: &tauri::AppHandle) {
    VERSION.send_modify(|v| *v = v.wrapping_add(1));
    publish_availability(app);
}
fn publish_availability(app: &tauri::AppHandle) {
    let _ = app.emit("ai-config-changed", ());
    if let Some(state) = app.try_state::<crate::live::LiveState>() {
        state.notify_flush_driver();
    }
}
fn check(attempt: &Attempt) -> Result<(), String> {
    LOGIN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .ensure(attempt)
}

#[tauri::command]
pub(crate) async fn chatgpt_accounts() -> Result<Vec<AccountStatus>, String> {
    let _guard = SERIAL.lock().await;
    let store = load().await?;
    Ok(store
        .accounts
        .iter()
        .map(|a| AccountStatus {
            client_id: a.client_id.clone(),
            email: a.email.clone(),
            active: Some(&a.client_id) == store.active.as_ref(),
            connected: a.connected(),
            plan_enabled: a.plan_enabled(),
        })
        .collect())
}

#[tauri::command]
pub(crate) async fn chatgpt_cancel_login() -> Result<(), String> {
    crate::background_ipc::run("ChatGPT ログインの中断に失敗しました", || {
        LOGIN.lock().unwrap_or_else(|e| e.into_inner()).invalidate();
        Ok(())
    })
    .await
}

#[tauri::command]
pub(crate) async fn chatgpt_select_account(
    app: tauri::AppHandle,
    client_id: String,
) -> Result<(), String> {
    chatgpt_cancel_login().await?;
    let _guard = SERIAL.lock().await;
    let mut store = load().await?;
    if !store
        .accounts
        .iter()
        .any(|a| a.client_id == client_id && a.connected() && a.plan_enabled())
    {
        return Err("このアカウントで再度ログインしてください".into());
    }
    store.active = Some(client_id);
    save(store).await?;
    changed(&app);
    Ok(())
}

fn authorize_url(
    host: &str,
    account: Option<&Account>,
    redirect: &str,
    state: &str,
    nonce: &str,
    challenge: &str,
) -> url::Url {
    let mut url = url::Url::parse("https://auth.openai.com/api/accounts/authorize").unwrap();
    {
        let mut q = url.query_pairs_mut();
        q.append_pair(
            "client_id",
            account
                .map(|a| a.client_id.as_str())
                .unwrap_or("dynamic_agent_client"),
        )
        .append_pair("ext_agent_host_id", host)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", redirect)
        .append_pair(
            "scope",
            "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct",
        )
        .append_pair("resource", RESOURCE)
        .append_pair("state", state)
        .append_pair("nonce", nonce)
        .append_pair("code_challenge_method", "S256")
        .append_pair("code_challenge", challenge);
        if let Some(a) = account {
            if !a.access_token.is_empty() && !a.plan_enabled() {
                q.append_pair("prompt", "consent");
            }
            if !a.id_token.is_empty() {
                q.append_pair("id_token_hint", &a.id_token);
            }
            if !a.email.is_empty() {
                q.append_pair("login_hint", &a.email);
            }
        } else {
            q.append_pair("agent_name_hint", "Selah");
        }
    }
    url
}

#[tauri::command]
pub(crate) async fn chatgpt_sign_in(
    app: tauri::AppHandle,
    client_id: Option<String>,
) -> Result<(), String> {
    let attempt = LOGIN.lock().unwrap_or_else(|e| e.into_inner()).begin();
    let (host, previous) = {
        let _guard = SERIAL.lock().await;
        let mut store = load().await?;
        let previous = match client_id {
            Some(id) => Some(
                store
                    .accounts
                    .iter()
                    .find(|a| a.client_id == id)
                    .cloned()
                    .ok_or("保存済みの ChatGPT アカウントが見つかりません")?,
            ),
            None => None,
        };
        if store.host_id.is_empty() {
            store.host_id = format!("urn:uuid:{}", uuid::Uuid::new_v4());
        }
        let host = store.host_id.clone();
        check(&attempt)?;
        save(store).await?;
        (host, previous)
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| "ChatGPT ログイン用ポートを開けません")?;
    let redirect = format!(
        "http://127.0.0.1:{}/auth/callback",
        listener.local_addr().map_err(|_| "ポート取得失敗")?.port()
    );
    let state = uuid::Uuid::new_v4().to_string();
    let nonce = uuid::Uuid::new_v4().to_string();
    let (verifier, challenge) = crate::oauth_lifecycle::generate_pkce();
    let url = authorize_url(
        &host,
        previous.as_ref(),
        &redirect,
        &state,
        &nonce,
        &challenge,
    );
    check(&attempt)?;
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|_| "ブラウザを開けませんでした")?;
    let (code, issued) = callback(
        listener,
        &state,
        previous.as_ref().map(|a| a.client_id.as_str()),
        &attempt,
    )
    .await?;
    // Retain an issued registration even if exchange/validation fails. Retrying
    // must reuse this client rather than create another workspace connection.
    {
        let _guard = SERIAL.lock().await;
        check(&attempt)?;
        let mut store = load().await?;
        if !store.accounts.iter().any(|a| a.client_id == issued) {
            store.accounts.push(Account {
                client_id: issued.clone(),
                ..Default::default()
            });
            save(store).await?;
        }
    }
    let tokens = exchange(&[
        ("grant_type", "authorization_code"),
        ("client_id", &issued),
        ("code", &code),
        ("code_verifier", &verifier),
        ("redirect_uri", &redirect),
        ("resource", RESOURCE),
    ])
    .await
    .map_err(|e| e.message)?;
    check(&attempt)?;
    let identity = validate_identity(
        tokens
            .id_token
            .as_deref()
            .ok_or("ChatGPT の ID トークンがありません")?,
        &issued,
        &nonce,
    )
    .await?;
    if previous
        .as_ref()
        .is_some_and(|a| !a.subject.is_empty() && a.subject != identity.sub)
    {
        return Err("選択した ChatGPT アカウントと認証結果が一致しません".into());
    }
    let mut account = previous.unwrap_or_default();
    account.client_id = issued.clone();
    account.subject = identity.sub;
    account.email = identity.email.unwrap_or_default();
    account.apply(tokens, true)?;
    let plan_enabled = account.plan_enabled();
    {
        let _guard = SERIAL.lock().await;
        check(&attempt)?;
        let mut store = load().await?;
        store.accounts.retain(|a| a.client_id != issued);
        store.accounts.push(account);
        if plan_enabled {
            store.active = Some(issued);
        }
        check(&attempt)?;
        save_attempt(store, attempt.clone()).await?;
        changed(&app);
    }
    if plan_enabled {
        Ok(())
    } else {
        Err("ログインしましたがプランの利用が許可されていません。ChatGPT の「使用状況」で Selah を許可してください。".into())
    }
}

struct TokenFailure {
    message: String,
    unusable: bool,
}
impl From<&str> for TokenFailure {
    fn from(message: &str) -> Self {
        Self {
            message: message.into(),
            unusable: false,
        }
    }
}

fn unusable_refresh(status: reqwest::StatusCode, body: &serde_json::Value) -> bool {
    let code = body
        .get("error")
        .and_then(|v| v.as_str())
        .or_else(|| body.pointer("/error/code").and_then(|v| v.as_str()))
        .unwrap_or("");
    status.is_client_error()
        && status != reqwest::StatusCode::TOO_MANY_REQUESTS
        && matches!(
            code,
            "invalid_grant"
                | "invalid_refresh_token"
                | "token_expired"
                | "refresh_token_expired"
                | "refresh_token_invalidated"
                | "refresh_token_reused"
        )
}

async fn exchange(fields: &[(&str, &str)]) -> Result<Tokens, TokenFailure> {
    let response = HTTP
        .post(TOKEN_URL)
        .timeout(std::time::Duration::from_secs(30))
        .form(fields)
        .send()
        .await
        .map_err(|_| "ChatGPT 認証サーバーに接続できません")?;
    let status = response.status();
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|_| "ChatGPT 認証応答を読み込めません")?;
    if !status.is_success() {
        return Err(TokenFailure {
            message: request_error(status, &body),
            unusable: unusable_refresh(status, &body),
        });
    }
    serde_json::from_value(body).map_err(|_| "ChatGPT 認証応答が正しくありません".into())
}

#[derive(Deserialize)]
struct Identity {
    sub: String,
    iat: i64,
    email: Option<String>,
    nonce: Option<String>,
}

async fn validate_identity(token: &str, client_id: &str, nonce: &str) -> Result<Identity, String> {
    use jsonwebtoken::{decode_header, jwk::JwkSet, Algorithm, DecodingKey};
    let invalid = || "ChatGPT の本人確認に失敗しました。再度ログインしてください。".to_string();
    let header = decode_header(token).map_err(|_| invalid())?;
    if !matches!(header.alg, Algorithm::RS256 | Algorithm::ES256) {
        return Err(invalid());
    }
    let keys: JwkSet = HTTP
        .get("https://auth.openai.com/.well-known/jwks.json")
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| invalid())?
        .error_for_status()
        .map_err(|_| invalid())?
        .json()
        .await
        .map_err(|_| invalid())?;
    let key = keys
        .find(header.kid.as_deref().ok_or_else(invalid)?)
        .ok_or_else(invalid)?;
    let key = DecodingKey::from_jwk(key).map_err(|_| invalid())?;
    verify_identity(token, client_id, nonce, &key, header.alg)
}

fn verify_identity(
    token: &str,
    client_id: &str,
    nonce: &str,
    key: &jsonwebtoken::DecodingKey,
    algorithm: jsonwebtoken::Algorithm,
) -> Result<Identity, String> {
    use jsonwebtoken::{decode, Algorithm, Validation};
    let invalid = || "ChatGPT の本人確認に失敗しました。再度ログインしてください。".to_string();
    if !matches!(algorithm, Algorithm::RS256 | Algorithm::ES256) {
        return Err(invalid());
    }
    let mut validation = Validation::new(algorithm);
    validation.set_audience(&[client_id]);
    validation.set_issuer(&[ISSUER]);
    validation.set_required_spec_claims(&["sub", "iss", "aud", "exp", "iat"]);
    validation.leeway = 5;
    let identity = decode::<Identity>(token, key, &validation)
        .map_err(|_| invalid())?
        .claims;
    if identity.sub.is_empty()
        || identity.nonce.as_deref() != Some(nonce)
        || identity.iat <= 0
        || identity.iat > chrono::Utc::now().timestamp() + 5
    {
        return Err(invalid());
    }
    Ok(identity)
}

fn parse_callback(
    target: &str,
    state: &str,
    previous: Option<&str>,
) -> Result<Option<(String, String)>, String> {
    let url = url::Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|_| "認証コールバックが正しくありません")?;
    if url.path() != "/auth/callback" {
        return Ok(None);
    }
    let mut params = std::collections::HashMap::new();
    for (key, value) in url.query_pairs() {
        if params.insert(key.to_string(), value.to_string()).is_some() {
            return Ok(None);
        }
    }
    if params.get("state").map(String::as_str) != Some(state) {
        return Ok(None);
    }
    if params.get("iss").is_some_and(|v| v != ISSUER) {
        return Err("認証サーバーが一致しません".into());
    }
    if params.contains_key("error") {
        return Err("ChatGPT ログインがキャンセルされたか、利用が許可されませんでした".into());
    }
    let code = params
        .get("code")
        .filter(|v| !v.is_empty())
        .ok_or("認証コードがありません")?;
    let issued = match (previous, params.get("client_id")) {
        (Some(expected), Some(actual)) if expected != actual => {
            return Err("ChatGPT の登録が一致しません".into())
        }
        (Some(expected), _) => expected.to_string(),
        (None, Some(actual)) => actual.clone(),
        _ => return Err("ChatGPT の登録 ID がありません。もう一度ログインしてください".into()),
    };
    if issued.is_empty()
        || issued == "dynamic_agent_client"
        || issued.len() > 256
        || !issued
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return Err("ChatGPT の登録 ID が正しくありません".into());
    }
    Ok(Some((code.clone(), issued)))
}

async fn callback(
    listener: tokio::net::TcpListener,
    state: &str,
    previous: Option<&str>,
    attempt: &Attempt,
) -> Result<(String, String), String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(600);
    loop {
        check(attempt)?;
        if tokio::time::Instant::now() >= deadline {
            return Err("ChatGPT ログインがタイムアウトしました".into());
        }
        let accepted =
            tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept()).await;
        let (mut stream, _) = match accepted {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => return Err("認証接続の受信に失敗しました".into()),
            Err(_) => continue,
        };
        let headers = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let mut bytes = Vec::new();
            let mut buf = [0; 1024];
            while bytes.len() < 16 * 1024 {
                let n = stream.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buf[..n]);
                if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                    return Ok::<_, std::io::Error>(bytes);
                }
            }
            Err(std::io::Error::other("incomplete headers"))
        })
        .await;
        let Ok(Ok(headers)) = headers else { continue };
        check(attempt)?;
        let request = String::from_utf8_lossy(&headers);
        let mut parts = request
            .lines()
            .next()
            .unwrap_or_default()
            .split_whitespace();
        if parts.next() != Some("GET") {
            continue;
        }
        let result = parse_callback(parts.next().unwrap_or_default(), state, previous);
        let (status, message) = match &result {
            Ok(Some(_)) => (
                "200 OK",
                "ChatGPT から戻りました。Selah で接続結果を確認し、このタブを閉じてください。",
            ),
            Ok(None) => (
                "400 Bad Request",
                "Selah で開始した認証をブラウザで完了してください。",
            ),
            Err(_) => (
                "400 Bad Request",
                "認証を完了できませんでした。Selah に戻って再度お試しください。",
            ),
        };
        let body = format!(
            "<!doctype html><meta charset=utf-8><title>Selah · ChatGPT</title><p>{message}</p>"
        );
        let reply = format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}", body.len());
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            stream.write_all(reply.as_bytes()),
        )
        .await;
        if let Some(value) = result? {
            return Ok(value);
        }
    }
}

pub(super) async fn access_token() -> Result<String, String> {
    // Once a rotating refresh starts, finish persisting it even when an
    // inference waiter is cancelled or its account is switched.
    tauri::async_runtime::spawn(access_token_serialized())
        .await
        .map_err(|_| "ChatGPT セッションの更新に失敗しました".to_string())?
}

async fn access_token_serialized() -> Result<String, String> {
    let _guard = SERIAL.lock().await;
    let mut store = load().await?;
    let account = store
        .accounts
        .iter_mut()
        .find(|a| Some(&a.client_id) == store.active.as_ref())
        .ok_or("AI 設定から ChatGPT にログインしてください")?;
    if !account.plan_enabled() {
        return Err("ChatGPT プランの利用を許可してください".into());
    }
    if account.expires_at <= chrono::Utc::now().timestamp() + 60 {
        if account.refresh_token.is_empty() {
            return Err("ChatGPT にもう一度ログインしてください".into());
        }
        let tokens = match exchange(&[
            ("grant_type", "refresh_token"),
            ("client_id", &account.client_id),
            ("refresh_token", &account.refresh_token),
            ("resource", RESOURCE),
        ])
        .await
        {
            Ok(tokens) => tokens,
            Err(failure) => {
                if failure.unusable {
                    account.clear_tokens();
                    save(store).await?;
                    if let Some(app) = APP.get() {
                        publish_availability(app);
                    }
                }
                return Err(failure.message);
            }
        };
        account.apply(tokens, false)?;
        let token = account.access_token.clone();
        let enabled = account.plan_enabled();
        save(store).await?;
        if !enabled {
            if let Some(app) = APP.get() {
                publish_availability(app);
            }
            return Err(
                "ChatGPT プランの利用が取り消されました。使用状況を確認してください".into(),
            );
        }
        return Ok(token);
    }
    Ok(account.access_token.clone())
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;

#[tauri::command]
pub(crate) async fn chatgpt_list_models() -> Result<Vec<Model>, String> {
    let mut session = session();
    tokio::select! {
        biased;
        _ = session.changed() => Err("ChatGPT アカウントが変更されました".into()),
        result = async {
            let token = access_token().await?;
            let response = HTTP.get(format!("{RESOURCE}/models")).bearer_auth(token)
                .timeout(std::time::Duration::from_secs(30)).send().await
                .map_err(|_| "ChatGPT のモデル一覧を取得できません")?;
            let status = response.status();
            let body: serde_json::Value = response.json().await.map_err(|_| "ChatGPT のモデル一覧を読み込めません")?;
            if !status.is_success() { return Err(request_error(status, &body)); }
            let models = body.get("models").and_then(|v| v.as_array()).ok_or("ChatGPT のモデル一覧が正しくありません")?;
            Ok(models.iter().filter(|v| v.get("visibility").and_then(|v| v.as_str()) == Some("list"))
                .filter_map(|v| serde_json::from_value::<Model>(v.clone()).ok()).collect())
        } => result,
    }
}

#[tauri::command]
pub(crate) async fn chatgpt_disconnect(
    app: tauri::AppHandle,
    client_id: String,
) -> Result<bool, String> {
    chatgpt_cancel_login().await?;
    // Interrupt inference before waiting for a pending token refresh.
    changed(&app);
    let _guard = SERIAL.lock().await;
    let mut store = load().await?;
    let account = store
        .accounts
        .iter_mut()
        .find(|a| a.client_id == client_id)
        .ok_or("アカウントが見つかりません")?;
    let mut revoked = account.refresh_token.is_empty();
    if !revoked {
        // Use the issuer's published endpoint, never an arbitrary callback URL.
        let discovery = HTTP
            .get(format!("{ISSUER}/.well-known/openid-configuration"))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;
        if let Ok(response) = discovery {
            if let Ok(body) = response.json::<serde_json::Value>().await {
                if body.get("issuer").and_then(|v| v.as_str()) == Some(ISSUER) {
                    if let Some(endpoint) = body.get("revocation_endpoint").and_then(|v| v.as_str())
                    {
                        if url::Url::parse(endpoint).is_ok_and(|u| {
                            u.scheme() == "https" && u.host_str() == Some("auth.openai.com")
                        }) {
                            for attempt in 0..2 {
                                match HTTP
                                    .post(endpoint)
                                    .timeout(std::time::Duration::from_secs(10))
                                    .form(&[
                                        ("token", account.refresh_token.as_str()),
                                        ("token_type_hint", "refresh_token"),
                                        ("client_id", account.client_id.as_str()),
                                    ])
                                    .send()
                                    .await
                                {
                                    Ok(r) if r.status() == reqwest::StatusCode::OK => {
                                        revoked = true;
                                        break;
                                    }
                                    Ok(r) if !r.status().is_server_error() => break,
                                    _ if attempt == 0 => {
                                        tokio::time::sleep(std::time::Duration::from_millis(500))
                                            .await
                                    }
                                    _ => break,
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    account.clear_tokens();
    if store.active.as_ref() == Some(&client_id) {
        store.active = None;
    }
    save(store).await?;
    changed(&app);
    Ok(revoked)
}
