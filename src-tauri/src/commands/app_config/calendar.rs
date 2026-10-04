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

#[tauri::command]
pub fn get_calendar_config() -> CalendarConfig {
    load_calendar_config()
}

#[tauri::command]
pub fn save_calendar_config(config: CalendarConfig) -> Result<(), String> {
    for (label, val) in [
        ("春学期開始日", &config.spring_start),
        ("秋学期開始日", &config.fall_start),
    ] {
        if !val.is_empty() && chrono::NaiveDate::parse_from_str(val, "%Y-%m-%d").is_err() {
            return Err(format!("{}の日付形式が不正です (YYYY-MM-DD)", label));
        }
    }
    save_json_config(&calendar_config_path(), &config, "calendar config")
}
