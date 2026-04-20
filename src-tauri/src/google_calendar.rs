use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::LazyLock;

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GCAL_API_BASE: &str = "https://www.googleapis.com/calendar/v3";
const SCOPES: &str = "https://www.googleapis.com/auth/calendar.app.created";
const TOKEN_FILE: &str = "google_calendar_token.json";
const SYNC_STATE_FILE: &str = "google_calendar_sync.json";
const CONFIG_FILE: &str = "google_calendar_config.json";
const CALENDAR_SUMMARY: &str = "Selah 時間割";

fn default_client_id() -> String {
    crate::embedded_keys::decode(&[
        0x40, 0x5D, 0x55, 0x57, 0x58, 0x1D, 0x5C, 0x46, 0x5D, 0x5B, 0x00, 0x44, 0x05, 0x0A, 0x5E,
        0x12, 0x14, 0x19, 0x59, 0x5B, 0x15, 0x5A, 0x47, 0x00, 0x0E, 0x59, 0x02, 0x5D, 0x5B, 0x57,
        0x01, 0x00, 0x01, 0x51, 0x5A, 0x14, 0x52, 0x10, 0x5D, 0x50, 0x58, 0x5D, 0x54, 0x1C, 0x57,
        0x03, 0x15, 0x1F, 0x4F, 0x0F, 0x42, 0x04, 0x10, 0x05, 0x06, 0x58, 0x41, 0x55, 0x40, 0x55,
        0x1C, 0x0B, 0x18, 0x04, 0x06, 0x59, 0x45, 0x14, 0x06, 0x0E,
    ])
}

fn default_client_secret() -> String {
    crate::embedded_keys::decode(&[
        0x34, 0x2A, 0x2F, 0x32, 0x38, 0x75, 0x46, 0x38, 0x0B, 0x2C, 0x1A, 0x59, 0x69, 0x7E, 0x5B,
        0x31, 0x0D, 0x5A, 0x0E, 0x12, 0x67, 0x28, 0x42, 0x0C, 0x56, 0x49, 0x73, 0x62, 0x5B, 0x51,
        0x43, 0x01, 0x03, 0x2E, 0x10,
    ])
}

/// Google Calendar OAuth settings.
/// Built-in credentials are used by default; users can override if needed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GoogleCalConfig {
    pub client_id: String,
    pub client_secret: String,
}

impl Default for GoogleCalConfig {
    fn default() -> Self {
        Self {
            client_id: default_client_id(),
            client_secret: default_client_secret(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenData {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}

/// Tracks which events we have synced.
/// event_map key: "YYYY-MM-DD-period" (e.g. "2026-04-07-3")
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncState {
    pub calendar_id: String,
    pub event_map: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarSyncEntry {
    pub day: String,
    pub period: i32,
    pub course_name: String,
    pub room: String,
    pub is_cancelled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleCalStatus {
    pub authenticated: bool,
    pub calendar_exists: bool,
    pub synced_events: usize,
}

fn token_path() -> PathBuf {
    crate::client::data_dir().join(TOKEN_FILE)
}
fn sync_state_path() -> PathBuf {
    crate::client::data_dir().join(SYNC_STATE_FILE)
}
fn config_path() -> PathBuf {
    crate::client::data_dir().join(CONFIG_FILE)
}

pub fn load_config() -> GoogleCalConfig {
    let path = config_path();
    let mut cfg: GoogleCalConfig = if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|d| serde_json::from_str(&d).ok())
            .unwrap_or_default()
    } else {
        GoogleCalConfig::default()
    };

    // Migration: move client_secret from JSON to keychain
    if !cfg.client_secret.is_empty() {
        if crate::keychain::set_secret("gcal_client_secret", &cfg.client_secret).is_ok() {
            let secret = std::mem::take(&mut cfg.client_secret);
            let _ = save_config_to_disk(&cfg);
            cfg.client_secret = secret;
        }
    } else if let Some(secret) = crate::keychain::get_secret("gcal_client_secret") {
        cfg.client_secret = secret;
    }

    cfg
}

pub fn save_config(config: &GoogleCalConfig) -> Result<(), String> {
    // Store client_secret in keychain, never on disk
    if !config.client_secret.is_empty() {
        crate::keychain::set_secret("gcal_client_secret", &config.client_secret)?;
    } else {
        crate::keychain::delete_secret("gcal_client_secret");
    }

    let mut disk_cfg = config.clone();
    disk_cfg.client_secret = String::new();
    save_config_to_disk(&disk_cfg)
}

fn save_config_to_disk(config: &GoogleCalConfig) -> Result<(), String> {
    let data =
        serde_json::to_string_pretty(config).map_err(|e| format!("設定の保存に失敗: {}", e))?;
    let path = config_path();
    std::fs::write(&path, &data).map_err(|e| format!("設定ファイルの書き込みに失敗: {}", e))?;
    #[cfg(unix)]
    {
        let _ =
            std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600));
    }
    Ok(())
}

fn load_sync_state() -> SyncState {
    let path = sync_state_path();
    if path.exists() {
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(state) = serde_json::from_str(&data) {
                return state;
            }
        }
    }
    SyncState::default()
}

fn save_sync_state(state: &SyncState) -> Result<(), String> {
    let data =
        serde_json::to_string_pretty(state).map_err(|e| format!("同期状態の保存に失敗: {}", e))?;
    std::fs::write(sync_state_path(), &data)
        .map_err(|e| format!("同期状態ファイルの書き込みに失敗: {}", e))?;
    Ok(())
}

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

/// Parse week_label like "2026/03/30(月)～2026/04/05(日)" to get Monday's date
static WEEK_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(\d{4})/(\d{2})/(\d{2})").expect("valid hardcoded regex"));

fn parse_week_start(week_label: &str) -> Result<chrono::NaiveDate, String> {
    let re = &*WEEK_RE;
    let caps = re
        .captures(week_label)
        .ok_or_else(|| format!("週ラベルを解析できません: {}", week_label))?;
    let y: i32 = caps[1].parse().map_err(|e| format!("year: {}", e))?;
    let m: u32 = caps[2].parse().map_err(|e| format!("month: {}", e))?;
    let d: u32 = caps[3].parse().map_err(|e| format!("day: {}", e))?;
    chrono::NaiveDate::from_ymd_opt(y, m, d).ok_or_else(|| format!("無効な日付: {}/{}/{}", y, m, d))
}

fn day_offset(day: &str) -> i64 {
    match day {
        "月" => 0,
        "火" => 1,
        "水" => 2,
        "木" => 3,
        "金" => 4,
        "土" => 5,
        _ => 0,
    }
}

pub struct GoogleCalendarClient {
    http: Client,
    pub token: Option<TokenData>,
    pub config: GoogleCalConfig,
    pub sync_state: SyncState,
    pkce_verifier: Option<String>,
    redirect_uri: Option<String>,
}

impl GoogleCalendarClient {
    pub fn new() -> Self {
        let http = Client::builder()
            .user_agent(crate::client::USER_AGENT)
            .build()
            .expect("failed to build Google Calendar HTTP client");
        Self {
            http,
            token: None,
            config: load_config(),
            sync_state: load_sync_state(),
            pkce_verifier: None,
            redirect_uri: None,
        }
    }

    pub fn try_restore_token(&mut self) {
        // Prefer keychain
        if let Some(json) = crate::keychain::get_secret("gcal_token") {
            if let Ok(token) = serde_json::from_str::<TokenData>(&json) {
                log::info!("Restored Google Calendar token from keychain");
                self.token = Some(token);
                return;
            }
        }
        // Legacy file migration
        let path = token_path();
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(token) = serde_json::from_str::<TokenData>(&data) {
                log::info!("Migrating Google Calendar token from file to keychain");
                self.token = Some(token);
                self.save_token();
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    pub fn save_token(&self) {
        if let Some(ref token) = self.token {
            if let Ok(json) = serde_json::to_string(token) {
                if let Err(e) = crate::keychain::set_secret("gcal_token", &json) {
                    log::warn!("Failed to save Google Calendar token to keychain: {}", e);
                }
            }
        }
    }

    pub fn clear_token(&mut self) {
        self.token = None;
        crate::keychain::delete_secret("gcal_token");
        let _ = std::fs::remove_file(token_path()); // clean up legacy file
    }

    pub fn is_authenticated(&self) -> bool {
        self.token.is_some()
    }

    pub fn auth_url(&mut self, port: u16) -> Result<String, String> {
        if self.config.client_id.trim().is_empty() {
            return Err("Google Client IDが未設定です。設定画面で入力してください。".into());
        }
        let (verifier, challenge) = generate_pkce();
        self.pkce_verifier = Some(verifier);
        let redirect_uri = format!("http://127.0.0.1:{}", port);
        self.redirect_uri = Some(redirect_uri.clone());
        let state = uuid::Uuid::new_v4().to_string();
        Ok(format!(
            "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent&code_challenge={}&code_challenge_method=S256&state={}",
            GOOGLE_AUTH_URL,
            urlencoding::encode(self.config.client_id.trim()),
            urlencoding::encode(&redirect_uri),
            urlencoding::encode(SCOPES),
            urlencoding::encode(&challenge),
            urlencoding::encode(&state),
        ))
    }

    pub async fn exchange_code(&mut self, code: &str) -> Result<(), String> {
        let verifier = self
            .pkce_verifier
            .take()
            .ok_or("PKCE verifier missing. Please retry login.")?;
        let redirect_uri = self
            .redirect_uri
            .take()
            .ok_or("Redirect URI missing. Please retry login.")?;
        let client_id = self.config.client_id.trim().to_string();
        let client_secret = self.config.client_secret.trim().to_string();

        let mut params = vec![
            ("client_id", client_id.as_str()),
            ("code", code),
            ("redirect_uri", redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier.as_str()),
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

    async fn ensure_token(&mut self) -> Result<String, String> {
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

    /// Find or create the "Selah 時間割" calendar
    async fn ensure_calendar(&mut self) -> Result<String, String> {
        if !self.sync_state.calendar_id.is_empty() {
            let token = self.ensure_token().await?;
            let resp = self
                .http
                .get(format!(
                    "{}/calendars/{}",
                    GCAL_API_BASE,
                    urlencoding::encode(&self.sync_state.calendar_id)
                ))
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|e| format!("カレンダー確認失敗: {}", e))?;
            if resp.status().is_success() {
                return Ok(self.sync_state.calendar_id.clone());
            }
            self.sync_state.calendar_id.clear();
            self.sync_state.event_map.clear();
        }

        let token = self.ensure_token().await?;
        let resp = self
            .http
            .get(format!("{}/users/me/calendarList", GCAL_API_BASE))
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| format!("カレンダー一覧取得失敗: {}", e))?;
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("カレンダー一覧レスポンス解析失敗: {}", e))?;
        if let Some(items) = body["items"].as_array() {
            for item in items {
                if item["summary"].as_str() == Some(CALENDAR_SUMMARY) {
                    if let Some(id) = item["id"].as_str() {
                        self.sync_state.calendar_id = id.to_string();
                        save_sync_state(&self.sync_state)?;
                        return Ok(id.to_string());
                    }
                }
            }
        }

        let token = self.ensure_token().await?;
        let resp = self
            .http
            .post(format!("{}/calendars", GCAL_API_BASE))
            .bearer_auth(&token)
            .json(&serde_json::json!({ "summary": CALENDAR_SUMMARY, "timeZone": "Asia/Tokyo" }))
            .send()
            .await
            .map_err(|e| format!("カレンダー作成失敗: {}", e))?;
        if !resp.status().is_success() {
            let err: serde_json::Value = resp.json().await.unwrap_or_default();
            return Err(format!("カレンダー作成失敗: {}", err));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("カレンダー作成レスポンス解析失敗: {}", e))?;
        let cal_id = body["id"]
            .as_str()
            .ok_or("カレンダーID取得失敗")?
            .to_string();
        self.sync_state.calendar_id = cal_id.clone();
        self.sync_state.event_map.clear();
        save_sync_state(&self.sync_state)?;
        log::info!("Created Google Calendar: {}", cal_id);
        Ok(cal_id)
    }

    /// Sync this week's timetable to Google Calendar.
    /// Parses week_label for the Monday date, creates one event per class per day.
    /// Cancelled classes are skipped. Stale events from this week are deleted.
    pub async fn sync_timetable(
        &mut self,
        entries: Vec<CalendarSyncEntry>,
        week_label: String,
    ) -> Result<String, String> {
        let monday = parse_week_start(&week_label)?;
        let cal_id = self.ensure_calendar().await?;

        // Build desired events: key = "YYYY-MM-DD-period"
        let mut desired: std::collections::HashMap<String, &CalendarSyncEntry> =
            std::collections::HashMap::new();
        for entry in &entries {
            if entry.is_cancelled {
                continue;
            }
            let date = monday + chrono::Duration::days(day_offset(&entry.day));
            let key = format!("{}-{}", date.format("%Y-%m-%d"), entry.period);
            desired.insert(key, entry);
        }

        // Keys belonging to this week (Mon..Sat)
        let week_prefixes: Vec<String> = (0..6)
            .map(|off| {
                (monday + chrono::Duration::days(off))
                    .format("%Y-%m-%d")
                    .to_string()
            })
            .collect();
        let is_this_week = |k: &str| week_prefixes.iter().any(|p| k.starts_with(p));

        // Delete stale events from this week
        let old_keys: Vec<String> = self
            .sync_state
            .event_map
            .keys()
            .filter(|k| is_this_week(k))
            .cloned()
            .collect();
        let mut deleted = 0usize;
        for key in &old_keys {
            if !desired.contains_key(key) {
                if let Some(event_id) = self.sync_state.event_map.remove(key) {
                    let _ = self.delete_event(&cal_id, &event_id).await;
                    deleted += 1;
                }
            }
        }

        // Create or update
        let mut created = 0usize;
        let mut updated = 0usize;
        for (key, entry) in &desired {
            let date_str = &key[..10];
            let times = crate::config::PERIOD_TIMES;
            let idx = (entry.period - 1).clamp(0, 6) as usize;
            let (sh, sm, eh, em) = times[idx];
            let start_dt = format!("{}T{:02}:{:02}:00", date_str, sh, sm);
            let end_dt = format!("{}T{:02}:{:02}:00", date_str, eh, em);

            let event_body = serde_json::json!({
                "summary": entry.course_name,
                "location": entry.room,
                "start": { "dateTime": start_dt, "timeZone": "Asia/Tokyo" },
                "end": { "dateTime": end_dt, "timeZone": "Asia/Tokyo" },
            });

            if let Some(existing_id) = self.sync_state.event_map.get(key).cloned() {
                match self.update_event(&cal_id, &existing_id, &event_body).await {
                    Ok(_) => {
                        updated += 1;
                    }
                    Err(_) => {
                        self.sync_state.event_map.remove(key);
                        if let Ok(id) = self.create_event(&cal_id, &event_body).await {
                            self.sync_state.event_map.insert(key.clone(), id);
                            created += 1;
                        }
                    }
                }
            } else if let Ok(id) = self.create_event(&cal_id, &event_body).await {
                self.sync_state.event_map.insert(key.clone(), id);
                created += 1;
            }
        }

        save_sync_state(&self.sync_state)?;
        let week_count = self
            .sync_state
            .event_map
            .keys()
            .filter(|k| is_this_week(k))
            .count();
        log::info!(
            "Google Calendar sync: created={}, updated={}, deleted={}",
            created,
            updated,
            deleted
        );
        Ok(format!(
            "Google Calendar: {}件同期 (新規{} / 更新{} / 削除{})",
            week_count, created, updated, deleted
        ))
    }

    async fn create_event(
        &mut self,
        cal_id: &str,
        body: &serde_json::Value,
    ) -> Result<String, String> {
        let token = self.ensure_token().await?;
        let resp = self
            .http
            .post(format!(
                "{}/calendars/{}/events",
                GCAL_API_BASE,
                urlencoding::encode(cal_id)
            ))
            .bearer_auth(&token)
            .json(body)
            .send()
            .await
            .map_err(|e| format!("イベント作成失敗: {}", e))?;
        if !resp.status().is_success() {
            let err: serde_json::Value = resp.json().await.unwrap_or_default();
            return Err(format!("イベント作成失敗: {}", err));
        }
        let result: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("イベント作成レスポンス解析失敗: {}", e))?;
        Ok(result["id"].as_str().unwrap_or("").to_string())
    }

    async fn update_event(
        &mut self,
        cal_id: &str,
        event_id: &str,
        body: &serde_json::Value,
    ) -> Result<(), String> {
        let token = self.ensure_token().await?;
        let resp = self
            .http
            .put(format!(
                "{}/calendars/{}/events/{}",
                GCAL_API_BASE,
                urlencoding::encode(cal_id),
                urlencoding::encode(event_id)
            ))
            .bearer_auth(&token)
            .json(body)
            .send()
            .await
            .map_err(|e| format!("イベント更新失敗: {}", e))?;
        if !resp.status().is_success() {
            let err: serde_json::Value = resp.json().await.unwrap_or_default();
            return Err(format!("イベント更新失敗: {}", err));
        }
        Ok(())
    }

    async fn delete_event(&mut self, cal_id: &str, event_id: &str) -> Result<(), String> {
        let token = self.ensure_token().await?;
        let resp = self
            .http
            .delete(format!(
                "{}/calendars/{}/events/{}",
                GCAL_API_BASE,
                urlencoding::encode(cal_id),
                urlencoding::encode(event_id)
            ))
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| format!("イベント削除失敗: {}", e))?;
        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::GONE {
            let err: serde_json::Value = resp.json().await.unwrap_or_default();
            return Err(format!("イベント削除失敗: {}", err));
        }
        Ok(())
    }

    pub fn status(&self) -> GoogleCalStatus {
        GoogleCalStatus {
            authenticated: self.is_authenticated(),
            calendar_exists: !self.sync_state.calendar_id.is_empty(),
            synced_events: self.sync_state.event_map.len(),
        }
    }

    pub async fn clear_calendar(&mut self, delete_calendar: bool) -> Result<String, String> {
        let cal_id = self.sync_state.calendar_id.clone();
        if cal_id.is_empty() {
            return Ok("Google Calendarは未作成です".into());
        }
        if delete_calendar {
            let token = self.ensure_token().await?;
            let resp = self
                .http
                .delete(format!(
                    "{}/calendars/{}",
                    GCAL_API_BASE,
                    urlencoding::encode(&cal_id)
                ))
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|e| format!("カレンダー削除失敗: {}", e))?;
            if !resp.status().is_success() && resp.status() != reqwest::StatusCode::NOT_FOUND {
                let err: serde_json::Value = resp.json().await.unwrap_or_default();
                return Err(format!("カレンダー削除失敗: {}", err));
            }
            self.sync_state = SyncState::default();
            save_sync_state(&self.sync_state)?;
            Ok("Google Calendarを削除しました".into())
        } else {
            let event_ids: Vec<(String, String)> = self.sync_state.event_map.drain().collect();
            let mut deleted = 0;
            for (_, eid) in &event_ids {
                if self.delete_event(&cal_id, eid).await.is_ok() {
                    deleted += 1;
                }
            }
            save_sync_state(&self.sync_state)?;
            Ok(format!("{}件のイベントを削除しました", deleted))
        }
    }

    pub fn disconnect(&mut self) {
        self.clear_token();
        self.sync_state = SyncState::default();
        let _ = std::fs::remove_file(sync_state_path());
    }
}
