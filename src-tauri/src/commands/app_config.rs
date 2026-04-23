use crate::client;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

fn load_json_config<T: Default + DeserializeOwned>(path: &std::path::Path) -> T {
    if path.exists() {
        if let Ok(data) = std::fs::read_to_string(path) {
            if let Ok(cfg) = serde_json::from_str(&data) {
                return cfg;
            }
        }
    }
    T::default()
}

fn save_json_config<T: Serialize>(
    path: &std::path::Path,
    config: &T,
    label: &str,
) -> Result<(), String> {
    let data = serde_json::to_string_pretty(config)
        .map_err(|e| format!("JSON serialization error: {}", e))?;
    std::fs::write(path, &data).map_err(|e| format!("Failed to write {}: {}", label, e))?;
    Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadConfig {
    pub download_dir: String,
    pub classify_by_course: bool,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            download_dir: String::new(),
            classify_by_course: true,
        }
    }
}

fn download_config_path() -> std::path::PathBuf {
    client::data_dir().join("download_config.json")
}

pub fn load_download_config() -> DownloadConfig {
    load_json_config(&download_config_path())
}

#[tauri::command]
pub fn get_download_config() -> DownloadConfig {
    load_download_config()
}

#[tauri::command]
pub fn save_download_config(config: DownloadConfig) -> Result<(), String> {
    if !config.download_dir.is_empty() {
        let p = std::path::Path::new(&config.download_dir);
        if !p.is_absolute() {
            return Err("ダウンロードディレクトリは絶対パスで指定してください".into());
        }
        std::fs::create_dir_all(p)
            .map_err(|e| format!("ディレクトリの作成に失敗しました: {}", e))?;
    }
    save_json_config(&download_config_path(), &config, "download config")
}

#[tauri::command]
pub async fn select_download_dir() -> Result<String, String> {
    let result = rfd::AsyncFileDialog::new()
        .set_title("ダウンロードフォルダを選択")
        .pick_folder()
        .await;

    match result {
        Some(handle) => Ok(handle.path().to_string_lossy().to_string()),
        None => Err("cancelled".into()),
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationConfig {
    pub notify_important: bool,
    pub notify_faculty: bool,
    pub notify_class: bool,
    pub notify_class_general: bool,
    pub notify_class_announcement: bool,
    pub notify_class_assignment: bool,
    pub notify_class_exam: bool,
    pub notify_class_discussion: bool,
    pub notify_class_survey: bool,
    pub notify_class_attendance: bool,
    pub notify_other: bool,
    pub notify_mail: bool,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            notify_important: true,
            notify_faculty: true,
            notify_class: true,
            notify_class_general: true,
            notify_class_announcement: true,
            notify_class_assignment: true,
            notify_class_exam: true,
            notify_class_discussion: true,
            notify_class_survey: true,
            notify_class_attendance: true,
            notify_other: true,
            notify_mail: true,
        }
    }
}

fn notification_config_path() -> std::path::PathBuf {
    client::data_dir().join("notification_config.json")
}

pub fn load_notification_config() -> NotificationConfig {
    load_json_config(&notification_config_path())
}

#[tauri::command]
pub fn get_notification_config() -> NotificationConfig {
    load_notification_config()
}

#[tauri::command]
pub fn save_notification_config(config: NotificationConfig) -> Result<(), String> {
    save_json_config(&notification_config_path(), &config, "notification config")
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NativeAgentConfig {
    pub floating_orb_enabled: bool,
    pub subtitle_overlay_enabled: bool,
}

impl Default for NativeAgentConfig {
    fn default() -> Self {
        Self {
            floating_orb_enabled: false,
            subtitle_overlay_enabled: false,
        }
    }
}

fn native_agent_config_path() -> std::path::PathBuf {
    client::data_dir().join("native_agent_config.json")
}

pub fn load_native_agent_config() -> NativeAgentConfig {
    load_json_config(&native_agent_config_path())
}

#[tauri::command]
pub fn get_native_agent_config() -> NativeAgentConfig {
    load_native_agent_config()
}

#[tauri::command]
pub fn save_native_agent_config(
    _app: tauri::AppHandle,
    config: NativeAgentConfig,
) -> Result<(), String> {
    save_json_config(&native_agent_config_path(), &config, "native agent config")?;

    #[cfg(target_os = "macos")]
    {
        if config.floating_orb_enabled {
            let _ = crate::macos_native_agent::open_orb(&_app);
        } else {
            let _ = crate::macos_native_agent::close_orb(&_app);
        }
        if config.subtitle_overlay_enabled {
            let _ = crate::macos_subtitle_overlay::open_overlay(&_app);
        } else {
            let _ = crate::macos_subtitle_overlay::close_overlay(&_app);
        }
    }
    #[cfg(target_os = "windows")]
    {
        if config.subtitle_overlay_enabled {
            let _ = crate::windows_subtitle_overlay::open_overlay(&_app);
        } else {
            let _ = crate::windows_subtitle_overlay::close_overlay(&_app);
        }
    }

    Ok(())
}

/// Open the real-time subtitle floating overlay and start STT.
#[tauri::command]
pub fn open_subtitle_overlay(_app: tauri::AppHandle) -> Result<(), String> {
    if !load_native_agent_config().subtitle_overlay_enabled {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        return crate::macos_subtitle_overlay::open_overlay(&_app);
    }
    #[cfg(target_os = "windows")]
    {
        return crate::windows_subtitle_overlay::open_overlay(&_app);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Err("Real-time subtitle overlay is not supported on this platform".into())
}

/// Stop STT and close the subtitle overlay.
#[tauri::command]
pub fn close_subtitle_overlay(_app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        return crate::macos_subtitle_overlay::close_overlay(&_app);
    }
    #[cfg(target_os = "windows")]
    {
        return crate::windows_subtitle_overlay::close_overlay(&_app);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Ok(())
}

/// Returns whether the subtitle overlay is currently open.
#[tauri::command]
pub fn subtitle_overlay_is_open() -> bool {
    #[cfg(target_os = "macos")]
    {
        return crate::macos_subtitle_overlay::is_open();
    }
    #[cfg(target_os = "windows")]
    {
        return crate::windows_subtitle_overlay::is_open();
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    false
}

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
