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

const CONFIG_KEY: &str = "gcal_config";
const LEGACY_SECRET_KEY: &str = "gcal_client_secret";

// A single strict record is authoritative for both fields. Never deserialize
// persisted data using GoogleCalConfig's bundled-credential defaults.
#[derive(serde::Serialize, serde::Deserialize)]
struct ConfigRecord {
    version: u32,
    client_id: String,
    client_secret: String,
}

#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct LegacyConfig {
    client_id: String,
    client_secret: String,
}

pub fn load_config() -> Result<GoogleCalConfig, crate::keychain::StoreError> {
    load_config_with(&config_path(), crate::keychain::get_secret, commit_record)
}

fn commit_record(json: &str) -> Result<(), String> {
    crate::keychain::update_secrets([
        (CONFIG_KEY.into(), Some(json.into())),
        (LEGACY_SECRET_KEY.into(), None),
    ])
}

fn decode_record(json: &str) -> Result<GoogleCalConfig, crate::keychain::StoreError> {
    use crate::keychain::StoreError;
    let record: ConfigRecord = serde_json::from_str(json).map_err(|_| {
        StoreError::new("corrupt_record", "Google Calendar configuration is invalid")
    })?;
    if record.version != 1 {
        return Err(StoreError::new(
            "corrupt_record",
            "Unsupported Google Calendar configuration version",
        ));
    }
    Ok(GoogleCalConfig {
        client_id: record.client_id,
        client_secret: record.client_secret,
    })
}

fn cleanup_legacy(path: &std::path::Path) {
    if let Err(error) = std::fs::remove_file(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            // The new record is already committed. Cleanup can be retried on
            // the next load without interpreting the obsolete file as current.
            log::warn!("Google Calendar legacy config cleanup pending: {error}");
        }
    }
}

fn load_config_with(
    path: &std::path::Path,
    mut read: impl FnMut(&str) -> Result<Option<String>, crate::keychain::StoreError>,
    write: impl FnOnce(&str) -> Result<(), String>,
) -> Result<GoogleCalConfig, crate::keychain::StoreError> {
    use crate::keychain::StoreError;
    if let Some(json) = read(CONFIG_KEY)? {
        let config = decode_record(&json)?;
        cleanup_legacy(path);
        return Ok(resolve_with_defaults(config));
    }
    let legacy: LegacyConfig = match std::fs::read_to_string(path) {
        Ok(json) => serde_json::from_str(&json).map_err(|_| {
            StoreError::new(
                "corrupt_record",
                "Google Calendar legacy configuration is invalid",
            )
        })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => LegacyConfig::default(),
        Err(error) => return Err(StoreError::new("read_failed", error.to_string())),
    };
    let secret = read(LEGACY_SECRET_KEY)?.unwrap_or(legacy.client_secret);
    let config = GoogleCalConfig {
        client_id: legacy.client_id,
        client_secret: secret,
    };
    save_config_with(path, &config, write).map_err(|e| StoreError::new("write_failed", e))?;
    Ok(resolve_with_defaults(config))
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
    if config.client_secret.trim().is_empty() && config.client_id == default_client_id() {
        config.client_secret = default_client_secret();
    }
    config
}

pub fn save_config(config: &GoogleCalConfig) -> Result<(), String> {
    save_config_with(&config_path(), config, commit_record)
}

fn save_config_with(
    path: &std::path::Path,
    config: &GoogleCalConfig,
    write: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(), String> {
    let json = serde_json::to_string(&ConfigRecord {
        version: 1,
        client_id: config.client_id.clone(),
        client_secret: config.client_secret.clone(),
    })
    .map_err(|e| e.to_string())?;
    // Vault replacement commits ID + secret + legacy-key removal together.
    write(&json)?;
    cleanup_legacy(path);
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
    crate::keychain::file::atomic_write(&sync_state_path(), data.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod credential_tests {
    use super::*;
    use crate::keychain::StoreError;
    use std::cell::RefCell;

    struct TempConfig(PathBuf);
    impl TempConfig {
        fn new() -> Self {
            let dir =
                std::env::temp_dir().join(format!("selah-gcal-config-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&dir).unwrap();
            Self(dir.join("config.json"))
        }
    }
    impl Drop for TempConfig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
        }
    }
    fn record(id: &str, secret: &str) -> String {
        serde_json::to_string(&ConfigRecord {
            version: 1,
            client_id: id.into(),
            client_secret: secret.into(),
        })
        .unwrap()
    }

    #[test]
    fn failed_save_preserves_complete_old_pair_and_retry_replaces_both() {
        let path = TempConfig::new();
        let committed = RefCell::new(record("old-id", "old-secret"));
        let next = GoogleCalConfig {
            client_id: "new-id".into(),
            client_secret: "new-secret".into(),
        };
        assert!(save_config_with(&path.0, &next, |_| Err("disk full".into())).is_err());
        let restored = load_config_with(
            &path.0,
            |_| Ok(Some(committed.borrow().clone())),
            |_| panic!("no migration"),
        )
        .unwrap();
        assert_eq!(restored.client_id, "old-id");
        assert_eq!(restored.client_secret, "old-secret");
        save_config_with(&path.0, &next, |json| {
            *committed.borrow_mut() = json.into();
            Ok(())
        })
        .unwrap();
        let restored = decode_record(&committed.borrow()).unwrap();
        assert_eq!(restored.client_id, "new-id");
        assert_eq!(restored.client_secret, "new-secret");
        assert!(!path.0.exists()); // No second, separately committed ID file.
    }

    #[test]
    fn migration_failure_keeps_legacy_and_retry_preserves_vault_secret() {
        let path = TempConfig::new();
        let legacy = r#"{"client_id":"custom","client_secret":"legacy"}"#;
        std::fs::write(&path.0, legacy).unwrap();
        let read = |key: &str| Ok((key == LEGACY_SECRET_KEY).then(|| "newer-secret".into()));
        assert_eq!(
            load_config_with(&path.0, read, |_| Err("disk full".into()))
                .unwrap_err()
                .kind,
            "write_failed"
        );
        assert_eq!(std::fs::read_to_string(&path.0).unwrap(), legacy);
        let restored = load_config_with(&path.0, read, |json| {
            let saved = decode_record(json).unwrap();
            assert_eq!(saved.client_id, "custom");
            assert_eq!(saved.client_secret, "newer-secret");
            Ok(())
        })
        .unwrap();
        assert_eq!(restored.client_secret, "newer-secret");
        assert!(!path.0.exists());
    }

    #[test]
    fn interrupted_cleanup_uses_committed_pair_even_if_legacy_is_broken() {
        let path = TempConfig::new();
        // A directory makes legacy removal fail on all supported platforms.
        std::fs::create_dir(&path.0).unwrap();
        let next = GoogleCalConfig {
            client_id: "new".into(),
            client_secret: "secret".into(),
        };
        let committed = RefCell::new(None);
        save_config_with(&path.0, &next, |json| {
            *committed.borrow_mut() = Some(json.to_owned());
            Ok(())
        })
        .unwrap();
        let restored = load_config_with(
            &path.0,
            |key| {
                assert_eq!(key, CONFIG_KEY);
                Ok(committed.borrow().clone())
            },
            |_| panic!("must not remigrate"),
        )
        .unwrap();
        assert_eq!(restored.client_id, "new");
        assert_eq!(restored.client_secret, "secret");
    }

    #[test]
    fn corrupt_record_or_locked_vault_never_falls_back_or_overwrites() {
        let path = TempConfig::new();
        std::fs::write(&path.0, r#"{"client_id":"old","client_secret":"old"}"#).unwrap();
        for json in [
            "{broken",
            r#"{"version":1,"client_id":"id"}"#,
            r#"{"version":2,"client_id":"id","client_secret":"secret"}"#,
        ] {
            assert_eq!(
                load_config_with(
                    &path.0,
                    |_| Ok(Some(json.into())),
                    |_| panic!("must not overwrite")
                )
                .unwrap_err()
                .kind,
                "corrupt_record"
            );
        }
        assert_eq!(
            load_config_with(
                &path.0,
                |_| Err(StoreError::new("locked", "locked")),
                |_| panic!("must not overwrite")
            )
            .unwrap_err()
            .kind,
            "locked"
        );
        assert!(path.0.exists());
    }

    #[test]
    fn corrupt_or_unreadable_legacy_never_migrates_credentials() {
        let path = TempConfig::new();
        std::fs::write(&path.0, "{broken").unwrap();
        let read = |key: &str| {
            assert_eq!(key, CONFIG_KEY);
            Ok(None)
        };
        assert_eq!(
            load_config_with(&path.0, read, |_| panic!("no write"))
                .unwrap_err()
                .kind,
            "corrupt_record"
        );
        std::fs::remove_file(&path.0).unwrap();
        std::fs::create_dir(&path.0).unwrap();
        assert_eq!(
            load_config_with(&path.0, read, |_| panic!("no write"))
                .unwrap_err()
                .kind,
            "read_failed"
        );
    }

    #[test]
    fn absent_fields_never_manufacture_a_bundled_secret_for_migration() {
        let path = TempConfig::new();
        let default = load_config_with(
            &path.0,
            |_| Ok(None),
            |json| {
                let stored = decode_record(json).unwrap();
                assert!(stored.client_id.is_empty());
                assert!(stored.client_secret.is_empty());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(default.client_id, default_client_id());
        std::fs::write(&path.0, r#"{"client_id":"custom"}"#).unwrap();
        let custom = load_config_with(&path.0, |_| Ok(None), |_| Ok(())).unwrap();
        assert_eq!(custom.client_id, "custom");
        assert!(custom.client_secret.is_empty());
    }

    #[test]
    fn legacy_plaintext_is_committed_before_removal() {
        let path = TempConfig::new();
        std::fs::write(
            &path.0,
            r#"{"client_id":"custom","client_secret":"legacy"}"#,
        )
        .unwrap();
        load_config_with(
            &path.0,
            |_| Ok(None),
            |json| {
                assert!(path.0.exists());
                assert_eq!(decode_record(json).unwrap().client_secret, "legacy");
                Ok(())
            },
        )
        .unwrap();
        assert!(!path.0.exists());
    }
}
