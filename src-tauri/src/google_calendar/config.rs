use std::path::PathBuf;

use super::types::{default_client_id, default_client_secret, GoogleCalConfig, SyncState};
use super::{CONFIG_FILE, SYNC_STATE_FILE, TOKEN_FILE};

pub(super) fn token_path() -> PathBuf {
    crate::client::data_dir().join(TOKEN_FILE)
}
pub(super) fn sync_state_path() -> PathBuf {
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

    // Empty fields mean "use built-in default" — serde's default only fires on
    // missing keys, not empty strings, so we have to refill explicitly.
    if cfg.client_id.trim().is_empty() {
        cfg.client_id = default_client_id();
    }
    if cfg.client_secret.trim().is_empty() {
        cfg.client_secret = default_client_secret();
    }

    cfg
}

#[cfg(test)]
pub(crate) fn default_client_id_for_test() -> String {
    default_client_id()
}
#[cfg(test)]
pub(crate) fn default_client_secret_for_test() -> String {
    default_client_secret()
}

/// Fill empty fields with the built-in defaults. Used when the user wants to
/// rely on the bundled credentials (leaves the inputs blank).
pub fn resolve_with_defaults(mut config: GoogleCalConfig) -> GoogleCalConfig {
    if config.client_id.trim().is_empty() {
        config.client_id = default_client_id();
    }
    if config.client_secret.trim().is_empty() {
        config.client_secret = default_client_secret();
    }
    config
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

pub(super) fn load_sync_state() -> SyncState {
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

pub(super) fn save_sync_state(state: &SyncState) -> Result<(), String> {
    let data =
        serde_json::to_string_pretty(state).map_err(|e| format!("同期状態の保存に失敗: {}", e))?;
    std::fs::write(sync_state_path(), &data)
        .map_err(|e| format!("同期状態ファイルの書き込みに失敗: {}", e))?;
    Ok(())
}
