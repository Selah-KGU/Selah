use super::json::{load_json_config, save_json_config};
use crate::client;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CalendarConfig {
    pub spring_start: String,
    pub fall_start: String,
    pub syscal_enabled: bool,
    pub syscal_auto_sync: bool,
    pub gcal_auto_sync: bool,
    pub cal_sync_interval: u32,
}

impl Default for CalendarConfig {
    fn default() -> Self {
        Self {
            spring_start: String::new(),
            fall_start: String::new(),
            syscal_enabled: false,
            syscal_auto_sync: false,
            gcal_auto_sync: false,
            cal_sync_interval: 12,
        }
    }
}

fn calendar_config_path() -> std::path::PathBuf {
    client::data_dir().join("calendar_config.json")
}

pub fn load_calendar_config() -> CalendarConfig {
    load_json_config(&calendar_config_path())
}

#[derive(Serialize)]
pub struct CalendarSettings {
    #[serde(flatten)]
    config: CalendarConfig,
    account_username: Option<String>,
    account_generation: u64,
    connection_id: Option<String>,
    calendar_id: String,
}

#[tauri::command]
pub async fn get_calendar_config(
    state: tauri::State<'_, crate::GCalState>,
) -> Result<CalendarSettings, String> {
    let mut config = load_calendar_config();
    let gcal = state.client.lock().await;
    let account = crate::session_coordinator::SESSIONS.account_context();
    config.gcal_auto_sync &= gcal.auto_sync_allowed(&account);
    Ok(CalendarSettings {
        config,
        account_username: account.username,
        account_generation: account.generation,
        connection_id: gcal.token.as_ref().map(|token| token.connection_id.clone()),
        calendar_id: gcal.sync_state.calendar_id.clone(),
    })
}

#[tauri::command]
pub async fn save_calendar_config(
    state: tauri::State<'_, crate::GCalState>,
    db: crate::db::AccountDb,
    config: CalendarConfig,
    account_username: Option<String>,
    account_generation: Option<u64>,
    connection_id: Option<String>,
    calendar_id: Option<String>,
) -> Result<(), String> {
    db.ensure_current_account()?;
    let account = crate::db::capture_account();
    if config.gcal_auto_sync
        && (account.username != account_username || Some(account.generation) != account_generation)
    {
        return Err(
            "大学アカウントが変更されました。設定を開き直して同期先を確認してください".into(),
        );
    }
    for (label, val) in [
        ("春学期開始日", &config.spring_start),
        ("秋学期開始日", &config.fall_start),
    ] {
        if !val.is_empty() && chrono::NaiveDate::parse_from_str(val, "%Y-%m-%d").is_err() {
            return Err(format!("{}の日付形式が不正です (YYYY-MM-DD)", label));
        }
    }
    let mut gcal = state.client.lock().await;
    db.ensure_current_account()?;
    gcal.configure_auto_sync(
        config.gcal_auto_sync,
        &account,
        connection_id.as_deref(),
        calendar_id.as_deref(),
    )
    .await?;
    db.ensure_current_account()?;
    save_json_config(&calendar_config_path(), &config, "calendar config")
}
