use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn default_ms_client_id() -> String {
    crate::embedded_keys::decode(&[
        0x4A, 0x00, 0x59, 0x07, 0x51, 0x19, 0x09, 0x14, 0x44, 0x06, 0x15, 0x53, 0x04, 0x1F, 0x02,
        0x16, 0x52, 0x5F, 0x4C, 0x0A, 0x15, 0x09, 0x12, 0x44, 0x55, 0x1E, 0x01, 0x06, 0x06, 0x55,
        0x41, 0x5C, 0x08, 0x56, 0x5D, 0x1E,
    ])
}

const MAIL_CONFIG_FILE: &str = "ms_mail_config.json";

/// User-configurable mail settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct MailConfig {
    /// Azure AD Application (client) ID. Empty = use default.
    pub client_id: String,
}

impl MailConfig {
    /// Returns the effective client_id (user-configured or default)
    pub fn effective_client_id(&self) -> String {
        if self.client_id.trim().is_empty() {
            default_ms_client_id()
        } else {
            self.client_id.trim().to_string()
        }
    }
}

fn config_path() -> PathBuf {
    crate::client::data_dir().join(MAIL_CONFIG_FILE)
}

pub fn load_config() -> MailConfig {
    let path = config_path();
    if path.exists() {
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str(&data) {
                return cfg;
            }
        }
    }
    MailConfig::default()
}

pub fn save_config(config: &MailConfig) -> Result<(), String> {
    let path = config_path();
    let data = serde_json::to_string_pretty(config)
        .map_err(|e| format!("JSON serialization error: {}", e))?;
    std::fs::write(&path, &data).map_err(|e| format!("Failed to write mail config: {}", e))?;
    Ok(())
}
