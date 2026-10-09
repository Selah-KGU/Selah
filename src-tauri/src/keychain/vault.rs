//! Transaction boundary for the secret store. Never publish an uncommitted
//! mutation, and never interpret an unreadable store as an empty one.
use serde::Serialize;
use std::collections::HashMap;

pub(super) type Secrets = HashMap<String, String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Keychain,
    File,
}

impl Mode {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "keychain" => Ok(Self::Keychain),
            "file" => Ok(Self::File),
            _ => Err("Unknown secret storage mode".into()),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Keychain => "keychain",
            Self::File => "file",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StoreError {
    pub kind: &'static str,
    pub message: String,
}

impl StoreError {
    pub fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for StoreError {}
impl From<StoreError> for String {
    fn from(error: StoreError) -> Self {
        error.message
    }
}

pub(super) trait Storage {
    fn load(&mut self, mode: Mode) -> Result<Secrets, StoreError>;
    fn save(&mut self, mode: Mode, secrets: &Secrets) -> Result<(), StoreError>;
    fn commit_mode(&mut self, mode: Mode) -> Result<(), StoreError>;
    fn retire(&mut self, mode: Mode) -> Result<(), StoreError>;
    fn erase(&mut self) -> Result<(), StoreError>;
}

#[derive(Clone, Serialize)]
pub struct SecretStoreStatus {
    pub backend: String,
    pub state: String,
    pub last_error: Option<String>,
}

pub(super) struct Vault<S> {
    pub storage: S,
    mode: Mode,
    secrets: Option<Secrets>,
    pending: Option<Secrets>,
    deferred: HashMap<String, Option<String>>,
    pending_retire: Option<Mode>,
    error: Option<StoreError>,
    load_attempted: bool,
    erased: bool,
}

impl<S: Storage> Vault<S> {
    pub fn new(storage: S, mode: Mode) -> Self {
        Self {
            storage,
            mode,
            secrets: None,
            pending: None,
            deferred: HashMap::new(),
            pending_retire: None,
            error: None,
            load_attempted: false,
            erased: false,
        }
    }

    fn ready(&mut self) -> Result<(), StoreError> {
        if self.erased {
            return Err(StoreError::new(
                "erased",
                "Secret store was erased; restart is required",
            ));
        }
        if !self.load_attempted {
            self.load_attempted = true;
            match self.storage.load(self.mode) {
                Ok(secrets) => self.secrets = Some(secrets),
                Err(error) => self.error = Some(error),
            }
        }
        if self.secrets.is_none() {
            return Err(self
                .error
                .as_ref()
                .cloned()
                .unwrap_or_else(|| StoreError::new("unavailable", "Secret store unavailable")));
        }
        Ok(())
    }

    pub fn get(&mut self, key: &str) -> Result<Option<String>, StoreError> {
        self.ready()?;
        Ok(self.secrets.as_ref().expect("loaded").get(key).cloned())
    }

    /// Keep explicit write/delete intent even if the initial unlock failed.
    /// Never synthesize an empty vault or expose these uncommitted values.
    pub fn update(
        &mut self,
        changes: impl IntoIterator<Item = (String, Option<String>)>,
    ) -> Result<(), String> {
        if self.erased {
            return self.ready().map_err(Into::into);
        }
        self.deferred.extend(changes);
        self.ready()?;
        let edits = self.deferred.clone();
        self.mutate(|secrets| {
            for (key, value) in edits {
                match value {
                    Some(value) => {
                        secrets.insert(key, value);
                    }
                    None => {
                        secrets.remove(&key);
                    }
                }
            }
        })?;
        self.deferred.clear();
        Ok(())
    }

    pub fn mutate(&mut self, change: impl FnOnce(&mut Secrets)) -> Result<(), String> {
        self.ready()?;
        let mut next = self
            .pending
            .as_ref()
            .or(self.secrets.as_ref())
            .expect("loaded")
            .clone();
        change(&mut next);
        // Retry a failed commit even when the next snapshot is unchanged.
        if self.error.is_none() && self.secrets.as_ref() == Some(&next) {
            return Ok(());
        }
        match self.storage.save(self.mode, &next) {
            Ok(()) => {
                self.secrets = Some(next);
                self.pending = None;
                self.error = None;
                self.finish_cleanup()
            }
            Err(error) => {
                let message = error.message.clone();
                self.pending = Some(next);
                self.error = Some(error);
                Err(message)
            }
        }
    }

    pub fn retry_load(&mut self) -> Result<(), String> {
        if self.erased {
            return self.ready().map_err(Into::into);
        }
        if self.secrets.is_none() {
            self.load_attempted = false;
            self.error = None;
        }
        self.ready()?;
        if self.pending.is_some() || !self.deferred.is_empty() {
            self.update([])?;
        }
        self.finish_cleanup()
    }

    /// Retire all writers before deleting anything. Even a failed erase may
    /// only be retried as an erase; a delayed OAuth response cannot recreate it.
    pub fn erase(&mut self) -> Result<(), String> {
        self.erased = true;
        self.secrets = None;
        self.pending = None;
        self.deferred.clear();
        self.pending_retire = None;
        match self.storage.erase() {
            Ok(()) => {
                self.error = Some(StoreError::new(
                    "erased",
                    "Restart required after data deletion",
                ));
                Ok(())
            }
            Err(error) => {
                let message = error.message.clone();
                self.error = Some(error);
                Err(message)
            }
        }
    }

    fn finish_cleanup(&mut self) -> Result<(), String> {
        if let Some(previous) = self.pending_retire {
            if let Err(error) = self.storage.retire(previous) {
                let message = error.message.clone();
                self.error = Some(error);
                return Err(message);
            }
            self.pending_retire = None;
            self.error = None;
        }
        Ok(())
    }

    pub fn migrate(&mut self, target: Mode) -> Result<(), String> {
        self.ready()?;
        self.update([])?;
        self.finish_cleanup()?;
        if target == self.mode {
            return self.mutate(|_| {});
        }
        let secrets = self
            .pending
            .as_ref()
            .or(self.secrets.as_ref())
            .expect("loaded")
            .clone();
        let prepared = self
            .storage
            .save(target, &secrets)
            .and_then(|_| self.storage.commit_mode(target));
        if let Err(error) = prepared {
            let message = error.message.clone();
            self.error = Some(error);
            return Err(message);
        }
        let previous = self.mode;
        self.mode = target;
        self.secrets = Some(secrets);
        self.pending = None;
        self.error = None;
        // Only after both data and preference are durable may the source go.
        self.pending_retire = Some(previous);
        self.finish_cleanup()
    }

    pub fn status(&mut self) -> SecretStoreStatus {
        let _ = self.ready();
        SecretStoreStatus {
            backend: self.mode.name().into(),
            state: self
                .error
                .as_ref()
                .map(|e| e.kind)
                .unwrap_or("ready")
                .into(),
            last_error: self.error.as_ref().map(|e| e.message.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_store_replays_latest_write_and_delete_without_discarding_unrelated_data() {
        let mut vault = Vault::new(
            Fake {
                fail_load: true,
                data: Secrets::from([
                    ("revoked".into(), "old".into()),
                    ("other".into(), "keep".into()),
                ]),
                ..Default::default()
            },
            Mode::Keychain,
        );
        assert!(vault
            .update([("rotated".into(), Some("first".into()))])
            .is_err());
        assert!(vault
            .update([
                ("rotated".into(), Some("latest".into())),
                ("revoked".into(), None)
            ])
            .is_err());
        assert!(vault.get("rotated").is_err());
        assert!(!vault.storage.events.contains(&"save"));
        vault.storage.fail_load = false;
        vault.retry_load().unwrap();
        assert_eq!(vault.get("rotated").unwrap().as_deref(), Some("latest"));
        assert_eq!(vault.get("other").unwrap().as_deref(), Some("keep"));
        assert!(vault.get("revoked").unwrap().is_none());
        assert_eq!(vault.status().state, "ready");
    }

    #[test]
    fn failed_rotation_followed_by_logout_cannot_be_replayed_on_unlock() {
        let mut vault = Vault::new(Fake::default(), Mode::Keychain);
        vault
            .update([("cookie".into(), Some("old".into()))])
            .unwrap();
        vault.storage.fail_save = true;
        assert!(vault
            .update([("cookie".into(), Some("new".into()))])
            .is_err());
        assert!(vault.update([("cookie".into(), None)]).is_err());
        vault.storage.fail_save = false;
        vault.retry_load().unwrap();
        assert!(vault.get("cookie").unwrap().is_none());
        assert!(vault.storage.data.is_empty());
    }

    #[derive(Default)]
    struct Fake {
        data: Secrets,
        fail_load: bool,
        fail_save: bool,
        fail_config: bool,
        events: Vec<&'static str>,
    }
    impl Storage for Fake {
        fn load(&mut self, _: Mode) -> Result<Secrets, StoreError> {
            self.events.push("load");
            if self.fail_load {
                Err(StoreError::new("locked", "locked"))
            } else {
                Ok(self.data.clone())
            }
        }
        fn save(&mut self, _: Mode, data: &Secrets) -> Result<(), StoreError> {
            self.events.push("save");
            if self.fail_save {
                Err(StoreError::new("write_failed", "disk full"))
            } else {
                self.data = data.clone();
                Ok(())
            }
        }
        fn commit_mode(&mut self, _: Mode) -> Result<(), StoreError> {
            self.events.push("config");
            if self.fail_config {
                Err(StoreError::new("write_failed", "config failed"))
            } else {
                Ok(())
            }
        }
        fn retire(&mut self, _: Mode) -> Result<(), StoreError> {
            self.events.push("retire");
            Ok(())
        }
        fn erase(&mut self) -> Result<(), StoreError> {
            self.events.push("erase");
            self.data.clear();
            Ok(())
        }
    }

    #[test]
    fn failed_write_preserves_committed_memory_and_retry_commits() {
        let mut vault = Vault::new(Fake::default(), Mode::Keychain);
        vault
            .mutate(|m| {
                m.insert("token".into(), "old".into());
            })
            .unwrap();
        vault.storage.fail_save = true;
        assert!(vault
            .mutate(|m| {
                m.insert("token".into(), "new".into());
            })
            .is_err());
        assert_eq!(vault.get("token").unwrap().as_deref(), Some("old"));
        assert_eq!(vault.status().state, "write_failed");
        vault.storage.fail_save = false;
        vault
            .mutate(|m| {
                m.insert("token".into(), "new".into());
            })
            .unwrap();
        assert_eq!(vault.get("token").unwrap().as_deref(), Some("new"));
        assert_eq!(vault.status().state, "ready");
    }

    #[test]
    fn locked_store_is_not_overwritten_and_can_be_unlocked_without_restart() {
        let mut vault = Vault::new(
            Fake {
                fail_load: true,
                ..Fake::default()
            },
            Mode::Keychain,
        );
        assert!(vault
            .mutate(|m| {
                m.insert("token".into(), "new".into());
            })
            .is_err());
        assert_eq!(vault.storage.events, ["load"]);
        vault
            .storage
            .data
            .insert("existing".into(), "preserved".into());
        vault.storage.fail_load = false;
        vault.retry_load().unwrap();
        assert_eq!(vault.get("existing").unwrap().as_deref(), Some("preserved"));
    }

    #[test]
    fn retry_commits_all_pending_changes_together_without_false_saved_reads() {
        let mut vault = Vault::new(Fake::default(), Mode::Keychain);
        vault.storage.fail_save = true;
        assert!(vault
            .mutate(|m| {
                m.insert("cookie".into(), "fresh".into());
            })
            .is_err());
        assert!(vault
            .mutate(|m| {
                m.insert("token".into(), "rotated".into());
            })
            .is_err());
        assert_eq!(vault.get("cookie").unwrap(), None);
        vault.storage.fail_save = false;
        vault.retry_load().unwrap();
        assert_eq!(vault.get("cookie").unwrap().as_deref(), Some("fresh"));
        assert_eq!(vault.get("token").unwrap().as_deref(), Some("rotated"));
    }

    #[test]
    fn erase_discards_pending_rotation_and_rejects_late_writes_and_unlock() {
        let mut vault = Vault::new(Fake::default(), Mode::Keychain);
        vault.storage.fail_save = true;
        assert!(vault
            .mutate(|m| {
                m.insert("token".into(), "pending".into());
            })
            .is_err());
        vault.erase().unwrap();
        let events = vault.storage.events.clone();
        assert!(vault.retry_load().is_err());
        assert!(vault
            .mutate(|m| {
                m.insert("token".into(), "late".into());
            })
            .is_err());
        assert_eq!(vault.get("token").unwrap_err().kind, "erased");
        assert_eq!(vault.status().state, "erased");
        assert!(vault
            .update([("token".into(), Some("late".into()))])
            .is_err());
        assert_eq!(vault.storage.events, events);
        assert!(vault.storage.data.is_empty());
    }

    #[test]
    fn migration_commits_data_then_preference_before_retiring_source() {
        let mut vault = Vault::new(Fake::default(), Mode::Keychain);
        vault.migrate(Mode::File).unwrap();
        assert_eq!(vault.storage.events, ["load", "save", "config", "retire"]);
        assert_eq!(vault.status().backend, "file");
    }

    #[test]
    fn failed_migration_never_retires_source_or_changes_active_mode() {
        for fail_config in [false, true] {
            let mut vault = Vault::new(
                Fake {
                    fail_save: !fail_config,
                    fail_config,
                    ..Fake::default()
                },
                Mode::Keychain,
            );
            assert!(vault.migrate(Mode::File).is_err());
            assert!(!vault.storage.events.contains(&"retire"));
            assert_eq!(vault.status().backend, "keychain");
        }
    }
}
