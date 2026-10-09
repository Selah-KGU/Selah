//! OAuth token migration distinguishes missing, unreadable and corrupt data.
use super::StoreError;
use serde::{de::DeserializeOwned, Serialize};
use std::path::Path;

pub(crate) fn restore<T: DeserializeOwned>(
    key: &str,
    legacy: &Path,
) -> Result<Option<T>, StoreError> {
    if revoked(legacy)? {
        cleanup(key, legacy)?;
        return Ok(None);
    }
    restore_with(
        legacy,
        || super::get_secret(key),
        |json| super::set_secret(key, json),
    )
}
fn restore_with<T: DeserializeOwned>(
    legacy: &Path,
    read: impl FnOnce() -> Result<Option<String>, StoreError>,
    write: impl FnOnce(&str) -> Result<(), String>,
) -> Result<Option<T>, StoreError> {
    if let Some(json) = read()? {
        let token = serde_json::from_str(&json)
            .map_err(|_| StoreError::new("corrupt_record", "Saved OAuth token is invalid"))?;
        let _ = std::fs::remove_file(legacy);
        return Ok(Some(token));
    }
    let json = match std::fs::read_to_string(legacy) {
        Ok(json) => json,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(StoreError::new("read_failed", e.to_string())),
    };
    let token = serde_json::from_str(&json)
        .map_err(|_| StoreError::new("corrupt_record", "Legacy OAuth token is invalid"))?;
    write(&json).map_err(|e| StoreError::new("write_failed", e))?;
    let _ = std::fs::remove_file(legacy);
    Ok(Some(token))
}
fn marker(legacy: &Path) -> std::path::PathBuf {
    legacy.with_extension("signed-out")
}
fn revoked(legacy: &Path) -> Result<bool, StoreError> {
    match std::fs::metadata(marker(legacy)) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(StoreError::new("read_failed", e.to_string())),
    }
}
fn remove(path: &Path) -> Result<(), StoreError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(StoreError::new("write_failed", e.to_string())),
    }
}
fn cleanup(key: &str, legacy: &Path) -> Result<(), StoreError> {
    super::delete_secrets(&[key]).map_err(|e| StoreError::new("write_failed", e))?;
    remove(legacy)
}
pub(crate) fn revoke(key: &str, legacy: &Path) -> Result<(), StoreError> {
    revoke_with(legacy, || cleanup(key, legacy))
}
fn revoke_with(
    legacy: &Path,
    delete: impl FnOnce() -> Result<(), StoreError>,
) -> Result<(), StoreError> {
    super::file::atomic_write(&marker(legacy), b"signed-out\n")
        .map_err(|e| StoreError::new("write_failed", e))?;
    // Keep this non-secret intent even after cleanup; only a committed new login removes it.
    delete()
}
pub(crate) fn save<T: Serialize>(
    key: &str,
    legacy: &Path,
    token: Option<&T>,
    new_login: bool,
) -> Result<(), StoreError> {
    let Some(token) = token else {
        return if revoked(legacy)? {
            cleanup(key, legacy)
        } else {
            Ok(())
        };
    };
    save_with(legacy, token, new_login, |json| {
        super::set_secret(key, json).map_err(|e| StoreError::new("write_failed", e))
    })
}
fn save_with<T: Serialize>(
    legacy: &Path,
    token: &T,
    new_login: bool,
    write: impl FnOnce(&str) -> Result<(), StoreError>,
) -> Result<(), StoreError> {
    if revoked(legacy)? && !new_login {
        return Err(StoreError::new(
            "signed_out",
            "OAuth connection was disconnected",
        ));
    }
    let json = serde_json::to_string(token)
        .map_err(|e| StoreError::new("encode_failed", e.to_string()))?;
    write(&json)?;
    remove(legacy)?;
    if new_login {
        remove(&marker(legacy))?;
        #[cfg(unix)]
        if let Some(parent) = legacy.parent() {
            std::fs::File::open(parent)
                .and_then(|dir| dir.sync_all())
                .map_err(|e| StoreError::new("write_failed", e.to_string()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_logout_survives_restart_and_only_committed_login_clears_it() {
        let dir = std::env::temp_dir().join(format!("selah-revoke-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("token.json");
        std::fs::write(&path, "123").unwrap();
        assert!(revoke_with(&path, || Err(StoreError::new("locked", "locked"))).is_err());
        assert!(revoked(&path).unwrap()); // independent disk read after failed cleanup
        assert!(save_with(&path, &456, false, |_| panic!("stale writer")).is_err());
        assert!(save_with(&path, &456, true, |_| Err(StoreError::new(
            "locked", "locked"
        )))
        .is_err());
        assert!(revoked(&path).unwrap());
        save_with(&path, &456, true, |_| Ok(())).unwrap();
        assert!(!revoked(&path).unwrap());
        assert!(!path.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn locked_or_corrupt_vault_never_falls_back_to_legacy_token() {
        let path = std::env::temp_dir().join(format!("selah-token-{}", uuid::Uuid::new_v4()));
        std::fs::write(&path, "123").unwrap();
        for saved in [
            Err(StoreError::new("locked", "locked")),
            Ok(Some("invalid".into())),
        ] {
            assert!(restore_with::<u32>(&path, || saved, |_| panic!("must not migrate")).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "123");
        }
        assert!(restore_with::<u32>(&path, || Ok(None), |_| Err("disk full".into())).is_err());
        assert!(path.exists());
        assert_eq!(
            restore_with::<u32>(&path, || Ok(None), |_| Ok(())).unwrap(),
            Some(123)
        );
        assert!(!path.exists());
    }
}
