use super::json::{load_json_config, save_json_config};
use crate::client;
use serde::{Deserialize, Serialize};

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
