use super::json::{load_json_config, save_json_config};
use crate::client;
use serde::{Deserialize, Serialize};

// ---- Secret storage preferences -------------------------------------------

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SecurityConfig {
    /// Where app secrets (API keys, tokens, cookies) are stored:
    ///   "keychain" (default) — OS keychain as primary, with the machine-bound
    ///                          encrypted file kept as a fallback.
    ///   "file"               — the machine-bound encrypted file ONLY; the OS
    ///                          keychain is never read or written.
    pub secret_store: String,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            secret_store: "keychain".to_string(),
        }
    }
}

fn security_config_path() -> std::path::PathBuf {
    client::data_dir().join("security_config.json")
}

pub fn load_security_config() -> SecurityConfig {
    load_json_config(&security_config_path())
}

#[tauri::command]
pub fn get_security_config() -> SecurityConfig {
    load_security_config()
}

#[tauri::command]
pub fn save_security_config(config: SecurityConfig) -> Result<(), String> {
    save_json_config(&security_config_path(), &config, "security config")?;
    // Apply the new keep/drop-fallback choice to the on-disk file immediately.
    crate::keychain::reapply_storage_policy();
    Ok(())
}
