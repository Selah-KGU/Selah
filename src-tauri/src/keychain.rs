//! Durable secrets: a random key in the OS credential store protects an
//! authenticated, atomically replaced vault. The selected file-only mode keeps
//! its explicit weaker, machine-derived protection without keychain access.
pub(crate) mod config;
mod disk;
pub(crate) mod file;
mod machine;
mod platform;
pub(crate) mod tokens;
mod vault;

use disk::DiskStore;
use platform::NativeCredentials;
use std::sync::{LazyLock, Mutex};
use vault::{Mode, Vault};
pub use vault::{SecretStoreStatus, StoreError};

type NativeVault = Vault<DiskStore<NativeCredentials>>;
static VAULT: LazyLock<Mutex<Option<NativeVault>>> = LazyLock::new(|| Mutex::new(None));

fn with_vault<R>(f: impl FnOnce(&mut NativeVault) -> R) -> R {
    let mut guard = VAULT.lock().unwrap_or_else(|e| e.into_inner());
    let vault = guard.get_or_insert_with(|| {
        let directory = crate::paths::data_dir();
        let mode =
            Mode::parse(&config::load_security_config().secret_store).unwrap_or(Mode::Keychain);
        Vault::new(
            DiskStore::new(
                directory.join("secrets"),
                directory.join("security_config.json"),
                NativeCredentials,
            ),
            mode,
        )
    });
    f(vault)
}

pub fn get_secret(key: &str) -> Result<Option<String>, StoreError> {
    with_vault(|vault| vault.get(key))
}

pub fn set_secret(key: &str, value: &str) -> Result<(), String> {
    with_vault(|vault| vault.update([(key.into(), Some(value.into()))]))
}

/// Commit related credential changes in one vault snapshot, including retries.
pub(crate) fn update_secrets(
    changes: impl IntoIterator<Item = (String, Option<String>)>,
) -> Result<(), String> {
    with_vault(|vault| vault.update(changes))
}

pub fn delete_secrets(keys: &[&str]) -> Result<(), String> {
    with_vault(|vault| vault.update(keys.iter().map(|key| ((*key).into(), None))))
}

pub fn erase_all() -> Result<(), String> {
    with_vault(|vault| vault.erase())
}

/// Resume an explicitly requested purge before initializing the shared vault.
pub(crate) fn erase_persisted_for_reset() -> Result<(), String> {
    use vault::Storage;
    let directory = crate::paths::data_dir();
    DiskStore::new(
        directory.join("secrets"),
        directory.join("security_config.json"),
        NativeCredentials,
    )
    .erase()
    .map_err(|error| error.message)
}

pub fn get_cookie_secret(key: &str) -> Result<Option<String>, StoreError> {
    get_secret(&format!("cookie.{key}"))
}

pub fn set_cookie_secret(key: &str, value: &str) -> Result<(), String> {
    set_secret(&format!("cookie.{key}"), value)
}

pub fn migrate_storage(mode: &str) -> Result<(), String> {
    let target = Mode::parse(mode)?;
    with_vault(|vault| vault.migrate(target))
}

pub fn get_secret_store_status() -> SecretStoreStatus {
    with_vault(|vault| vault.status())
}

pub fn retry_store() -> Result<(), String> {
    with_vault(|vault| vault.retry_load())
}

pub fn prewarm() {
    std::thread::spawn(|| {
        let _ = get_secret_store_status();
    });
}
