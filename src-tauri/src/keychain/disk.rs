use super::{
    file, machine,
    platform::CredentialStore,
    vault::{Mode, Secrets, Storage, StoreError},
};
use base64::Engine;
use rand::RngCore;
use std::path::{Path, PathBuf};

#[cfg(debug_assertions)]
const LEGACY_ACCOUNT: &str = "secret_bundle_v1_dev";
#[cfg(not(debug_assertions))]
const LEGACY_ACCOUNT: &str = "secret_bundle_v1";
#[cfg(debug_assertions)]
const KEY_ACCOUNT: &str = "vault_key_v2_dev";
#[cfg(not(debug_assertions))]
const KEY_ACCOUNT: &str = "vault_key_v2";

pub(super) struct DiskStore<K> {
    directory: PathBuf,
    config: PathBuf,
    credentials: K,
    master: Option<[u8; 32]>,
}

fn io_error(message: impl Into<String>) -> StoreError {
    StoreError::new("write_failed", message)
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, StoreError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(StoreError::new(
            "unavailable",
            format!("Read secret store: {e}"),
        )),
    }
}

fn remove(path: &Path) -> Result<(), StoreError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(StoreError::new(
            "cleanup_failed",
            format!("Remove old secret store: {e}"),
        )),
    }
}

impl<K: CredentialStore> DiskStore<K> {
    pub fn new(directory: PathBuf, config: PathBuf, credentials: K) -> Self {
        Self {
            directory,
            config,
            credentials,
            master: None,
        }
    }

    fn path(&self, mode: Mode) -> PathBuf {
        let build = if cfg!(debug_assertions) { ".dev" } else { "" };
        self.directory
            .join(format!("vault.v2.{}{build}.enc", mode.name()))
    }

    fn legacy_path(&self) -> PathBuf {
        self.directory.join(if cfg!(debug_assertions) {
            "bundle.v1.dev.enc"
        } else {
            "bundle.v1.enc"
        })
    }

    fn protect_directory(&self) -> Result<(), StoreError> {
        std::fs::create_dir_all(&self.directory)
            .map_err(|e| io_error(format!("Create secret directory: {e}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.directory, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| io_error(format!("Protect secret directory: {e}")))?;
        }
        Ok(())
    }

    fn key(&mut self, mode: Mode, create: bool) -> Result<[u8; 32], StoreError> {
        if mode == Mode::File {
            return Ok(machine::machine_key());
        }
        if let Some(key) = self.master {
            return Ok(key);
        }
        let key = match self.credentials.read(KEY_ACCOUNT)? {
            Some(encoded) => base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .ok()
                .and_then(|bytes| bytes.try_into().ok())
                .ok_or_else(|| StoreError::new("corrupt", "Invalid vault master key"))?,
            None if create => {
                let mut key = [0; 32];
                rand::rngs::OsRng.fill_bytes(&mut key);
                self.credentials.write(
                    KEY_ACCOUNT,
                    &base64::engine::general_purpose::STANDARD.encode(key),
                )?;
                key
            }
            None => {
                return Err(StoreError::new(
                    "locked",
                    "Vault master key is missing; existing vault was preserved",
                ))
            }
        };
        self.master = Some(key);
        Ok(key)
    }

    fn decode(bytes: &[u8]) -> Result<Secrets, StoreError> {
        serde_json::from_slice(bytes)
            .map_err(|_| StoreError::new("corrupt", "Invalid secret vault payload"))
    }

    fn retire_legacy(&self, mode: Mode) -> Result<(), StoreError> {
        // File mode never accesses the keychain, even for migration.
        if mode == Mode::Keychain {
            self.credentials.delete(LEGACY_ACCOUNT)?;
        }
        remove(&self.legacy_path())
    }
}

impl<K: CredentialStore> Storage for DiskStore<K> {
    fn erase(&mut self) -> Result<(), StoreError> {
        // Erase is explicitly user-requested and covers both build identities.
        // Keep trying all locations, reporting failure instead of claiming success.
        let mut error = None;
        for account in [
            "secret_bundle_v1",
            "secret_bundle_v1_dev",
            "vault_key_v2",
            "vault_key_v2_dev",
        ] {
            if let Err(e) = self.credentials.delete(account) {
                error = Some(e);
            }
        }
        match std::fs::remove_dir_all(&self.directory) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                error = Some(StoreError::new(
                    "cleanup_failed",
                    format!("Delete secret vault: {e}"),
                ))
            }
        }
        self.master = None;
        error.map_or(Ok(()), Err)
    }
    fn load(&mut self, mode: Mode) -> Result<Secrets, StoreError> {
        if let Some(blob) = read_optional(&self.path(mode))? {
            let key = self.key(mode, false)?;
            let plaintext =
                file::decrypt(&key, &blob).map_err(|e| StoreError::new("corrupt", e))?;
            let secrets = Self::decode(&plaintext)?;
            // Resume cleanup after an interruption between preference commit
            // and source retirement. This receipt contains no secret material.
            if let Some(bytes) = read_optional(&self.config)? {
                if let Ok(config) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                    if config["secret_store"].as_str() == Some(mode.name()) {
                        if let Some(previous) = config["cleanup_previous_store"].as_str() {
                            let previous =
                                Mode::parse(previous).map_err(|e| StoreError::new("corrupt", e))?;
                            if previous != mode {
                                self.retire(previous)?;
                            }
                        }
                    }
                }
            }
            self.retire_legacy(mode)?;
            return Ok(secrets);
        }
        // No v2 vault: migrate the selected v1 store. Unreadable legacy data is
        // an error; it must never silently become a fresh empty installation.
        let legacy = if mode == Mode::Keychain {
            self.credentials.read(LEGACY_ACCOUNT)?
        } else {
            None
        };
        let secrets = if let Some(json) = legacy {
            Some(Self::decode(json.as_bytes())?)
        } else if let Some(blob) = read_optional(&self.legacy_path())? {
            let plaintext = file::decrypt_legacy(&machine::machine_key(), &blob)
                .map_err(|e| StoreError::new("corrupt", e))?;
            Some(Self::decode(&plaintext)?)
        } else {
            None
        };
        if let Some(secrets) = secrets {
            self.save(mode, &secrets)?;
            self.retire_legacy(mode)?;
            Ok(secrets)
        } else {
            Ok(Secrets::new())
        }
    }

    fn save(&mut self, mode: Mode, secrets: &Secrets) -> Result<(), StoreError> {
        self.protect_directory()?;
        let key = self.key(mode, !self.path(mode).exists())?;
        let plaintext =
            serde_json::to_vec(secrets).map_err(|_| io_error("Serialize secret vault"))?;
        let blob = file::encrypt(&key, &plaintext).map_err(io_error)?;
        file::atomic_write(&self.path(mode), &blob).map_err(io_error)?;
        // Verify the durable snapshot before allowing migration cleanup or
        // publishing a new in-memory committed value.
        let saved = std::fs::read(self.path(mode))
            .map_err(|e| io_error(format!("Verify secret vault: {e}")))?;
        if file::decrypt(&key, &saved).map_err(io_error)? != plaintext {
            return Err(io_error("Secret vault verification failed"));
        }
        Ok(())
    }

    fn commit_mode(&mut self, mode: Mode) -> Result<(), StoreError> {
        let previous = if mode == Mode::Keychain {
            Mode::File
        } else {
            Mode::Keychain
        };
        let config = serde_json::json!({ "secret_store": mode.name(), "cleanup_previous_store": previous.name() });
        file::atomic_write(&self.config, config.to_string().as_bytes()).map_err(io_error)
    }

    fn retire(&mut self, mode: Mode) -> Result<(), StoreError> {
        remove(&self.path(mode))?;
        self.retire_legacy(mode)?;
        if mode == Mode::Keychain {
            self.credentials.delete(KEY_ACCOUNT)?;
            self.master = None;
        }
        let target = if mode == Mode::Keychain {
            Mode::File
        } else {
            Mode::Keychain
        };
        let config = serde_json::json!({ "secret_store": target.name() });
        file::atomic_write(&self.config, config.to_string().as_bytes()).map_err(io_error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    #[derive(Default)]
    struct Credentials {
        entries: RefCell<Secrets>,
        writes: RefCell<usize>,
        denied: RefCell<bool>,
    }
    impl CredentialStore for Rc<Credentials> {
        fn read(&self, key: &str) -> Result<Option<String>, StoreError> {
            if *self.denied.borrow() {
                return Err(StoreError::new("locked", "denied"));
            }
            Ok(self.entries.borrow().get(key).cloned())
        }
        fn write(&self, key: &str, value: &str) -> Result<(), StoreError> {
            assert!(value.encode_utf16().count() * 2 <= 2560);
            *self.writes.borrow_mut() += 1;
            self.entries.borrow_mut().insert(key.into(), value.into());
            Ok(())
        }
        fn delete(&self, key: &str) -> Result<(), StoreError> {
            self.entries.borrow_mut().remove(key);
            Ok(())
        }
    }
    fn setup() -> (PathBuf, Rc<Credentials>, DiskStore<Rc<Credentials>>) {
        let dir = std::env::temp_dir().join(format!("selah-store-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let credentials = Rc::new(Credentials::default());
        let store = DiskStore::new(
            dir.join("secrets"),
            dir.join("config.json"),
            credentials.clone(),
        );
        (dir, credentials, store)
    }

    #[test]
    fn full_erase_removes_all_vault_modes_and_both_build_keys() {
        let (dir, credentials, mut store) = setup();
        let secrets = Secrets::from([("token".into(), "fake-token".into())]);
        store.save(Mode::Keychain, &secrets).unwrap();
        store.save(Mode::File, &secrets).unwrap();
        for name in [
            "secret_bundle_v1",
            "secret_bundle_v1_dev",
            "vault_key_v2",
            "vault_key_v2_dev",
        ] {
            credentials
                .entries
                .borrow_mut()
                .insert(name.into(), "fake".into());
        }
        store.erase().unwrap();
        assert!(credentials.entries.borrow().is_empty());
        assert!(!store.directory.exists());
        assert!(store.master.is_none());
        store.erase().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn large_legacy_bundle_migrates_to_one_small_stable_key() {
        let (dir, credentials, mut store) = setup();
        let secrets = Secrets::from([("cookie.sso".into(), "x".repeat(50_000))]);
        credentials.entries.borrow_mut().insert(
            LEGACY_ACCOUNT.into(),
            serde_json::to_string(&secrets).unwrap(),
        );
        assert_eq!(store.load(Mode::Keychain).unwrap(), secrets);
        assert!(!credentials.entries.borrow().contains_key(LEGACY_ACCOUNT));
        store.save(Mode::Keychain, &secrets).unwrap();
        assert_eq!(*credentials.writes.borrow(), 1);
        assert_eq!(credentials.entries.borrow().len(), 1);
        let mut reopened =
            DiskStore::new(dir.join("secrets"), dir.join("config.json"), credentials);
        assert_eq!(reopened.load(Mode::Keychain).unwrap(), secrets);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_key_or_corrupt_vault_never_falls_back_to_empty_or_legacy() {
        let (dir, credentials, mut store) = setup();
        let secrets = Secrets::from([("token".into(), "preserve".into())]);
        store.save(Mode::Keychain, &secrets).unwrap();
        let path = store.path(Mode::Keychain);
        let original = std::fs::read(&path).unwrap();
        credentials.entries.borrow_mut().remove(KEY_ACCOUNT);
        store.master = None;
        assert!(store.load(Mode::Keychain).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(*credentials.writes.borrow(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn file_mode_never_uses_credentials_and_rejects_corruption() {
        let (dir, credentials, mut store) = setup();
        *credentials.denied.borrow_mut() = true;
        let secrets = Secrets::from([("token".into(), "secret".into())]);
        store.save(Mode::File, &secrets).unwrap();
        assert_eq!(store.load(Mode::File).unwrap(), secrets);
        std::fs::write(store.path(Mode::File), b"broken").unwrap();
        assert!(store.load(Mode::File).is_err());
        assert_eq!(*credentials.writes.borrow(), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn migration_survives_restart_between_preference_commit_and_cleanup() {
        let (dir, credentials, mut store) = setup();
        let secrets = Secrets::from([("cookie.sso".into(), "remember".into())]);
        store.save(Mode::Keychain, &secrets).unwrap();
        store.save(Mode::File, &secrets).unwrap();
        store.commit_mode(Mode::File).unwrap();
        assert!(store.path(Mode::Keychain).exists());
        let mut reopened = DiskStore::new(
            dir.join("secrets"),
            dir.join("config.json"),
            credentials.clone(),
        );
        assert_eq!(reopened.load(Mode::File).unwrap(), secrets);
        assert!(!reopened.path(Mode::Keychain).exists());
        assert!(!credentials.entries.borrow().contains_key(KEY_ACCOUNT));
        let config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("config.json")).unwrap()).unwrap();
        assert_eq!(config["secret_store"], "file");
        assert!(config.get("cleanup_previous_store").is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_destination_write_keeps_legacy_credentials() {
        let (dir, credentials, mut store) = setup();
        let secrets = Secrets::from([("cookie.sso".into(), "remember".into())]);
        credentials.entries.borrow_mut().insert(
            LEGACY_ACCOUNT.into(),
            serde_json::to_string(&secrets).unwrap(),
        );
        // A file blocks creation of the vault directory, modelling disk IO failure.
        std::fs::write(&store.directory, b"blocked").unwrap();
        assert!(store.load(Mode::Keychain).is_err());
        assert!(credentials.entries.borrow().contains_key(LEGACY_ACCOUNT));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
