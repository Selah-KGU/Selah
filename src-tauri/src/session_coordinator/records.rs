//! Authoritative service records. Callers receive immutable leases, never a
//! mutable client. Generation + jar identity guard every asynchronous commit.
use super::{Coordinator, Health, Service};
use crate::{auth::AuthSession, client::CookieClientParts, keychain::StoreError};
use std::sync::Arc;

pub(super) struct Record {
    transport: CookieClientParts,
    present: bool,
    pub(super) identity: Option<AuthSession>,
}
impl Record {
    pub(super) fn empty() -> Self {
        let (cookie_store, http) = crate::client::new_cookie_client();
        Self {
            transport: CookieClientParts { http, cookie_store },
            present: false,
            identity: None,
        }
    }
}

/// An immutable view of one accepted jar. HTTP may rotate that jar's cookies;
/// only the manager can accept a replacement or change authentication health.
#[derive(Clone)]
pub(crate) struct SessionLease {
    generation: u64,
    service: Service,
    http: reqwest::Client,
    cookies: Arc<reqwest_cookie_store::CookieStoreMutex>,
    identity: Option<AuthSession>,
    present: bool,
    health: Health,
}
impl SessionLease {
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }
    pub fn identity(&self) -> Option<&AuthSession> {
        self.identity.as_ref()
    }
    pub fn has_credentials(&self) -> bool {
        self.present
    }
    pub fn is_verified(&self) -> bool {
        self.health == Health::Valid
    }
}

pub(crate) trait SessionRepository: Send + Sync {
    fn load(
        &self,
        service: Service,
    ) -> Result<Option<(CookieClientParts, Option<AuthSession>)>, StoreError>;
    fn save(
        &self,
        service: Service,
        cookies: &reqwest_cookie_store::CookieStoreMutex,
        identity: Option<&AuthSession>,
    ) -> Result<(), StoreError>;
    fn delete(&self, service: Service) -> Result<(), StoreError>;
}
pub(super) struct DiskRepository;
impl SessionRepository for DiskRepository {
    fn load(
        &self,
        service: Service,
    ) -> Result<Option<(CookieClientParts, Option<AuthSession>)>, StoreError> {
        crate::client::load_session_record(service.cookie_key(), service == Service::Kgc)
    }
    fn save(
        &self,
        service: Service,
        cookies: &reqwest_cookie_store::CookieStoreMutex,
        identity: Option<&AuthSession>,
    ) -> Result<(), StoreError> {
        crate::client::save_session_record(cookies, service.cookie_key(), identity)
            .map_err(|e| StoreError::new("write_failed", e))?;
        if service == Service::Kgc {
            let _ = std::fs::remove_file(crate::paths::data_dir().join("session.json"));
        }
        Ok(())
    }
    fn delete(&self, service: Service) -> Result<(), StoreError> {
        if service == Service::Kgc {
            let _ = std::fs::remove_file(crate::paths::data_dir().join("session.json"));
        }
        crate::client::delete_cookie_jar(service.cookie_key())
            .map_err(|e| StoreError::new("write_failed", e))
    }
}

impl Service {
    pub fn cookie_key(self) -> &'static str {
        match self {
            Self::Kgc => crate::client::KGC_COOKIES_KEY,
            Self::Luna => crate::luna_client::LUNA_COOKIES_KEY,
            Self::Kwic => crate::kwic_client::KWIC_COOKIES_KEY,
        }
    }
}

impl Coordinator {
    pub fn account_context(&self) -> crate::db::AccountContext {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        crate::db::AccountContext {
            generation: state.snapshot.generation,
            username: if state.snapshot.signed_out {
                None
            } else {
                state.records[0]
                    .identity
                    .as_ref()
                    .map(|identity| identity.username.clone())
            },
        }
    }
    pub fn overview(&self) -> (super::SessionDiagnostics, Option<AuthSession>) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let identity = if state.snapshot.signed_out {
            None
        } else {
            state.records[0].identity.clone()
        };
        (state.snapshot.clone(), identity)
    }
    pub fn lease(&self, service: Service) -> SessionLease {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let record = &state.records[service.index()];
        SessionLease {
            generation: state.snapshot.generation,
            service,
            http: record.transport.http.clone(),
            cookies: record.transport.cookie_store.clone(),
            identity: if state.snapshot.signed_out {
                None
            } else {
                record.identity.clone()
            },
            present: record.present && !state.snapshot.signed_out,
            health: state.snapshot.services[service.index()].state,
        }
    }

    pub fn restore(&self, service: Service) -> Result<bool, StoreError> {
        // The ordered worker performs storage I/O without holding session state.
        let (generation, original) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.snapshot.signed_out
                || state.records[service.index()].present
                || (service == Service::Kgc && state.records[0].identity.is_some())
            {
                return Ok(false);
            }
            (
                state.snapshot.generation,
                state.records[service.index()]
                    .transport
                    .cookie_store
                    .clone(),
            )
        };
        let loaded = self.persistence.load(service);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.snapshot.signed_out
            || state.snapshot.generation != generation
            || !Arc::ptr_eq(
                &original,
                &state.records[service.index()].transport.cookie_store,
            )
        {
            return Err(StoreError::new("cancelled", super::CANCELLED));
        }
        let loaded = loaded.map_err(|error| {
            state.snapshot.revision = state.snapshot.revision.wrapping_add(1);
            state.snapshot.services[service.index()].state = Health::Unavailable;
            error
        })?;
        let Some((transport, identity)) = loaded else {
            return Ok(false);
        };
        if service == Service::Kgc && identity.is_none() {
            state.snapshot.revision = state.snapshot.revision.wrapping_add(1);
            state.snapshot.services[service.index()].state = Health::Unavailable;
            return Err(StoreError::new(
                "corrupt_record",
                "KGC credentials have no identity",
            ));
        }
        let present = transport
            .cookie_store
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter_unexpired()
            .next()
            .is_some();
        state.snapshot.revision = state.snapshot.revision.wrapping_add(1);
        state.records[service.index()] = Record {
            transport,
            identity,
            present,
        };
        let status = &mut state.snapshot.services[service.index()];
        status.state = if present {
            Health::Unverified
        } else {
            Health::NeedsLogin
        };
        status.credentials_present = present;
        Ok(present)
    }

    /// Only a positively verified candidate may replace a live jar. Failed
    /// disk persistence does not revoke valid in-memory credentials; vault
    /// status and checkpoint retries retain that separate failure explicitly.
    pub fn accept_verified(
        &self,
        generation: u64,
        service: Service,
        transport: CookieClientParts,
        identity: Option<AuthSession>,
    ) -> Result<(), String> {
        let _commit = self.commit_gate.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.snapshot.generation != generation || state.snapshot.signed_out {
            return Err(super::CANCELLED.into());
        }
        if service == Service::Kgc
            && identity
                .as_ref()
                .is_none_or(|identity| identity.username.trim().is_empty())
        {
            return Err("Verified KGC session requires account identity".into());
        }
        if service == Service::Kgc
            && state.records[0].identity.as_ref().map(|s| &s.username)
                != identity.as_ref().map(|s| &s.username)
        {
            for other in [Service::Luna, Service::Kwic] {
                self.clear_locked(&mut state, other);
            }
        }
        state.snapshot.revision = state.snapshot.revision.wrapping_add(1);
        state.records[service.index()] = Record {
            transport,
            identity,
            present: true,
        };
        let status = &mut state.snapshot.services[service.index()];
        status.state = Health::Valid;
        status.credentials_present = true;
        status.last_verified_at = Some(crate::db::epoch_secs());
        status.last_checked_at = status.last_verified_at;
        state
            .policy
            .probed(service, self.clock_origin.elapsed().as_secs() as i64);
        self.persist_record(service, &state.records[service.index()]);
        Ok(())
    }

    /// Commit a response only to the exact jar and account that issued it.
    pub fn validated(
        &self,
        lease: &SessionLease,
        health: Health,
        identity: Option<AuthSession>,
    ) -> Result<(), String> {
        let _commit = self.commit_gate.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !lease.present
            || state.snapshot.generation != lease.generation
            || state.snapshot.signed_out
            || !Arc::ptr_eq(
                &lease.cookies,
                &state.records[lease.service.index()].transport.cookie_store,
            )
        {
            return Err(super::CANCELLED.into());
        }
        state.snapshot.revision = state.snapshot.revision.wrapping_add(1);
        if health == Health::NeedsLogin {
            self.clear_locked(&mut state, lease.service);
        } else if let Some(identity) = identity {
            if lease.service == Service::Kgc {
                if identity.username.trim().is_empty() {
                    return Err("KGC account identity is missing".into());
                }
                if state.records[0].identity.as_ref().map(|old| &old.username)
                    != Some(&identity.username)
                {
                    for other in [Service::Luna, Service::Kwic] {
                        self.clear_locked(&mut state, other);
                    }
                }
            }
            state.records[lease.service.index()].identity = Some(identity);
        }
        state
            .policy
            .probed(lease.service, self.clock_origin.elapsed().as_secs() as i64);
        let status = &mut state.snapshot.services[lease.service.index()];
        status.state = health;
        status.last_checked_at = Some(crate::db::epoch_secs());
        if health == Health::Valid {
            status.last_verified_at = Some(crate::db::epoch_secs());
            self.persist_record(lease.service, &state.records[lease.service.index()]);
        }
        Ok(())
    }

    fn clear_locked(&self, state: &mut super::State, service: Service) {
        state.snapshot.revision = state.snapshot.revision.wrapping_add(1);
        let retained_identity = if service == Service::Kgc && !state.snapshot.signed_out {
            state.records[0].identity.clone()
        } else {
            None
        };
        state.records[service.index()] = Record::empty();
        state.records[service.index()].identity = retained_identity;
        let status = &mut state.snapshot.services[service.index()];
        status.state = if state.snapshot.signed_out {
            Health::SignedOut
        } else {
            Health::NeedsLogin
        };
        status.credentials_present = false;
        status.last_verified_at = None;
        state.completed[service.index()] = None;
        if state.records[service.index()].identity.is_some() {
            // Empty cookies revoke authentication while retaining the account
            // that owns offline data. Explicit sign-out deletes this record.
            self.persist_record(service, &state.records[service.index()]);
        } else {
            self.persistence.delete(service);
        }
    }
    pub fn clear_all(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        for service in [Service::Kgc, Service::Luna, Service::Kwic] {
            self.clear_locked(&mut state, service);
        }
    }
    fn persist_record(&self, service: Service, record: &Record) {
        self.persistence.save(
            service,
            record.transport.cookie_store.clone(),
            record.identity.clone(),
        );
    }
    pub fn checkpoint(&self) -> Result<(), String> {
        let completion = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if !state.snapshot.signed_out {
                for service in [Service::Kgc, Service::Luna, Service::Kwic] {
                    let record = &state.records[service.index()];
                    if record.present || record.identity.is_some() {
                        self.persist_record(service, record);
                    }
                }
            }
            // Enqueued under the state lock: later mutations cannot overtake us.
            self.persistence.barrier()
        };
        completion.recv().map_err(|e| e.to_string())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    #[derive(Default)]
    struct Memory {
        load_error: Mutex<Option<StoreError>>,
        write_error: Mutex<Option<StoreError>>,
        deleted: Mutex<Vec<Service>>,
        writes: Mutex<Vec<Service>>,
    }
    impl SessionRepository for Arc<Memory> {
        fn load(
            &self,
            _: Service,
        ) -> Result<Option<(CookieClientParts, Option<AuthSession>)>, StoreError> {
            if let Some(error) = self.load_error.lock().unwrap().clone() {
                return Err(error);
            }
            Ok(Some((parts(), None)))
        }
        fn save(
            &self,
            service: Service,
            _: &reqwest_cookie_store::CookieStoreMutex,
            _: Option<&AuthSession>,
        ) -> Result<(), StoreError> {
            self.writes.lock().unwrap().push(service);
            self.write_error.lock().unwrap().clone().map_or(Ok(()), Err)
        }
        fn delete(&self, service: Service) -> Result<(), StoreError> {
            self.deleted.lock().unwrap().push(service);
            self.write_error.lock().unwrap().clone().map_or(Ok(()), Err)
        }
    }
    fn parts() -> CookieClientParts {
        let (cookie_store, http) = crate::client::new_cookie_client();
        cookie_store
            .lock()
            .unwrap()
            .parse(
                "sid=test; Path=/",
                &reqwest::Url::parse("https://example.test").unwrap(),
            )
            .unwrap();
        CookieClientParts { http, cookie_store }
    }
    fn fixture() -> (Coordinator, Arc<Memory>) {
        let disk = Arc::new(Memory::default());
        (
            Coordinator::with_repository(false, Box::new(disk.clone())),
            disk,
        )
    }
    fn identity(username: &str) -> AuthSession {
        AuthSession {
            username: username.into(),
            display_name: username.into(),
            student_id: username.into(),
            faculty: String::new(),
            department: String::new(),
        }
    }
    #[test]
    fn pending_login_retries_storage_and_commits_exit_marker_without_reauthentication() {
        let (manager, disk) = fixture();
        let dir = std::env::temp_dir().join(format!("selah-login-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let marker = dir.join("signed-out");
        std::fs::write(&marker, "signed-out").unwrap();
        *disk.write_error.lock().unwrap() = Some(StoreError::new("write_failed", "disk full"));
        manager
            .accept_verified(0, Service::Kgc, parts(), Some(identity("alice")))
            .unwrap();
        manager.mark_login_pending(0).unwrap();
        assert!(manager.checkpoint_and_commit(&marker).is_err());
        assert!(manager.lease(Service::Kgc).is_verified());
        assert!(manager.snapshot().login_persistence_pending);
        assert!(marker.exists());
        *disk.write_error.lock().unwrap() = None;
        // Also retries the secondary-service deletions that failed before login.
        manager.checkpoint_and_commit(&marker).unwrap();
        assert!(!marker.exists());
        assert!(!manager.snapshot().login_persistence_pending);
        assert_eq!(manager.account_context().username.as_deref(), Some("alice"));
        manager.checkpoint_and_commit(&marker).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn marker_failure_stays_pending_and_logout_retires_the_retry() {
        let (manager, _) = fixture();
        let dir = std::env::temp_dir().join(format!("selah-login-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let marker = dir.join("signed-out");
        std::fs::create_dir(&marker).unwrap();
        manager
            .accept_verified(0, Service::Kgc, parts(), Some(identity("alice")))
            .unwrap();
        manager.mark_login_pending(0).unwrap();
        assert!(manager.checkpoint_and_commit(&marker).is_err());
        assert!(manager.snapshot().login_persistence_pending);
        std::fs::remove_dir(&marker).unwrap();
        manager.sign_out_at(&marker).unwrap();
        manager.clear_all();
        manager.checkpoint_and_commit(&marker).unwrap();
        assert!(marker.exists());
        assert!(manager.snapshot().signed_out);
        assert!(!manager.snapshot().login_persistence_pending);
        assert!(manager.mark_login_pending(0).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn locked_restore_is_unavailable_and_cannot_checkpoint_an_empty_replacement() {
        let (manager, disk) = fixture();
        *disk.load_error.lock().unwrap() = Some(StoreError::new("locked", "Locked"));
        assert_eq!(manager.restore(Service::Luna).unwrap_err().kind, "locked");
        assert_eq!(manager.snapshot().services[1].state, Health::Unavailable);
        assert!(!manager.lease(Service::Luna).has_credentials());
        manager.checkpoint().unwrap();
        assert!(disk.writes.lock().unwrap().is_empty());
        assert!(disk.deleted.lock().unwrap().is_empty());
        *disk.load_error.lock().unwrap() = None;
        assert!(manager.restore(Service::Luna).unwrap());
        assert_eq!(manager.snapshot().services[1].state, Health::Unverified);
    }

    #[test]
    fn restore_is_unverified_and_unavailable_does_not_destroy_saved_credentials() {
        let (manager, disk) = fixture();
        assert!(manager.restore(Service::Luna).unwrap());
        let lease = manager.lease(Service::Luna);
        assert!(lease.has_credentials());
        assert!(!lease.is_verified());
        manager
            .validated(&lease, Health::Unavailable, None)
            .unwrap();
        assert!(manager.lease(Service::Luna).has_credentials());
        assert!(disk.deleted.lock().unwrap().is_empty());
        assert!(manager.snapshot().services[1].last_verified_at.is_none());
        manager.validated(&lease, Health::Valid, None).unwrap();
        assert!(manager.lease(Service::Luna).is_verified());
        manager.validated(&lease, Health::NeedsLogin, None).unwrap();
        assert!(!manager.lease(Service::Luna).has_credentials());
        manager.checkpoint().unwrap();
        assert_eq!(*disk.deleted.lock().unwrap(), [Service::Luna]);
    }
    #[test]
    fn late_expiry_cannot_clear_a_replacement_jar_in_the_same_generation() {
        let (manager, disk) = fixture();
        manager
            .accept_verified(0, Service::Luna, parts(), None)
            .unwrap();
        let old = manager.lease(Service::Luna);
        manager
            .accept_verified(0, Service::Luna, parts(), None)
            .unwrap();
        assert!(manager.validated(&old, Health::NeedsLogin, None).is_err());
        assert!(manager.lease(Service::Luna).is_verified());
        assert!(disk.deleted.lock().unwrap().is_empty());
    }
    #[test]
    fn logout_rejects_late_login_validation_and_checkpoint() {
        let (manager, disk) = fixture();
        manager
            .accept_verified(0, Service::Luna, parts(), None)
            .unwrap();
        let old = manager.lease(Service::Luna);
        manager.checkpoint().unwrap();
        manager.invalidate(true);
        let writes = disk.writes.lock().unwrap().len();
        assert!(manager.validated(&old, Health::Valid, None).is_err());
        assert!(manager
            .accept_verified(0, Service::Kwic, parts(), None)
            .is_err());
        manager.checkpoint().unwrap();
        assert_eq!(disk.writes.lock().unwrap().len(), writes);
        assert!(manager.snapshot().signed_out);
        assert!(!manager.snapshot().services[1].credentials_present);
    }
    #[test]
    fn expiry_revokes_cookies_without_losing_the_offline_account_owner() {
        let (manager, disk) = fixture();
        manager
            .accept_verified(0, Service::Kgc, parts(), Some(identity("alice")))
            .unwrap();
        let lease = manager.lease(Service::Kgc);
        manager.validated(&lease, Health::NeedsLogin, None).unwrap();
        manager.checkpoint().unwrap();
        assert_eq!(manager.account_context().username.as_deref(), Some("alice"));
        assert!(!manager.lease(Service::Kgc).has_credentials());
        assert!(manager.snapshot().services[0].last_verified_at.is_none());
        assert!(!disk.deleted.lock().unwrap().contains(&Service::Kgc));
        assert_eq!(
            manager
                .lease(Service::Kgc)
                .cookies
                .lock()
                .unwrap()
                .iter_unexpired()
                .count(),
            0
        );
        manager.invalidate(true);
        manager.clear_all();
        manager.checkpoint().unwrap();
        assert!(manager.account_context().username.is_none());
        assert!(disk.deleted.lock().unwrap().contains(&Service::Kgc));
    }

    #[test]
    fn verification_identity_change_retires_secondary_credentials() {
        let (manager, _) = fixture();
        manager
            .accept_verified(0, Service::Kgc, parts(), Some(identity("alice")))
            .unwrap();
        manager
            .accept_verified(0, Service::Luna, parts(), None)
            .unwrap();
        manager
            .accept_verified(0, Service::Kwic, parts(), None)
            .unwrap();
        let lease = manager.lease(Service::Kgc);
        manager
            .validated(&lease, Health::Valid, Some(identity("bob")))
            .unwrap();
        assert_eq!(manager.account_context().username.as_deref(), Some("bob"));
        assert!(!manager.lease(Service::Luna).has_credentials());
        assert!(!manager.lease(Service::Kwic).has_credentials());
        manager.checkpoint().unwrap();
    }

    #[test]
    fn identity_only_record_restores_account_without_authentication_proof() {
        struct IdentityOnly;
        impl SessionRepository for IdentityOnly {
            fn load(
                &self,
                _: Service,
            ) -> Result<Option<(CookieClientParts, Option<AuthSession>)>, StoreError> {
                Ok(Some((Record::empty().transport, Some(identity("alice")))))
            }
            fn save(
                &self,
                _: Service,
                _: &reqwest_cookie_store::CookieStoreMutex,
                _: Option<&AuthSession>,
            ) -> Result<(), StoreError> {
                Ok(())
            }
            fn delete(&self, _: Service) -> Result<(), StoreError> {
                Ok(())
            }
        }
        let manager = Coordinator::with_repository(false, Box::new(IdentityOnly));
        assert!(!manager.restore(Service::Kgc).unwrap());
        assert_eq!(manager.account_context().username.as_deref(), Some("alice"));
        assert!(!manager.lease(Service::Kgc).has_credentials());
        assert_eq!(manager.snapshot().services[0].state, Health::NeedsLogin);
        assert!(manager.snapshot().services[0].last_verified_at.is_none());
    }

    #[test]
    fn switching_identity_retires_both_secondary_services_atomically() {
        let (manager, disk) = fixture();
        manager
            .accept_verified(0, Service::Kgc, parts(), Some(identity("one")))
            .unwrap();
        manager
            .accept_verified(0, Service::Luna, parts(), None)
            .unwrap();
        manager
            .accept_verified(0, Service::Kwic, parts(), None)
            .unwrap();
        let old_luna = manager.lease(Service::Luna);
        manager
            .accept_verified(0, Service::Kgc, parts(), Some(identity("two")))
            .unwrap();
        assert_eq!(
            manager.lease(Service::Kgc).identity().unwrap().username,
            "two"
        );
        assert!(!manager.lease(Service::Luna).has_credentials());
        assert!(!manager.lease(Service::Kwic).has_credentials());
        assert!(manager.validated(&old_luna, Health::Valid, None).is_err());
        manager.checkpoint().unwrap();
        assert_eq!(
            &disk.deleted.lock().unwrap()[2..],
            [Service::Luna, Service::Kwic]
        );
    }
}

#[cfg(test)]
mod concurrency_tests {
    use super::*;
    use std::sync::{mpsc, Mutex};
    struct BlockedStore {
        started: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
        saved: Arc<Mutex<bool>>,
    }
    impl SessionRepository for BlockedStore {
        fn load(
            &self,
            _: Service,
        ) -> Result<Option<(CookieClientParts, Option<AuthSession>)>, StoreError> {
            Ok(None)
        }
        fn save(
            &self,
            _: Service,
            _: &reqwest_cookie_store::CookieStoreMutex,
            _: Option<&AuthSession>,
        ) -> Result<(), StoreError> {
            self.started.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
            *self.saved.lock().unwrap() = true;
            Ok(())
        }
        fn delete(&self, _: Service) -> Result<(), StoreError> {
            *self.saved.lock().unwrap() = false;
            Ok(())
        }
    }
    #[test]
    fn slow_storage_does_not_hold_state_and_logout_deletion_follows_queued_save() {
        let (started, waiting) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let saved = Arc::new(Mutex::new(false));
        let manager = Arc::new(Coordinator::with_repository(
            false,
            Box::new(BlockedStore {
                started,
                release: Mutex::new(released),
                saved: saved.clone(),
            }),
        ));
        let (cookie_store, http) = crate::client::new_cookie_client();
        manager
            .accept_verified(
                0,
                Service::Luna,
                CookieClientParts { cookie_store, http },
                None,
            )
            .unwrap();
        waiting
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        let (done, completion) = mpsc::channel();
        let other = manager.clone();
        let thread = std::thread::spawn(move || {
            assert!(other.lease(Service::Luna).has_credentials());
            other.invalidate(true);
            other.clear_all();
            done.send(()).unwrap();
        });
        let responsive = completion.recv_timeout(std::time::Duration::from_secs(2));
        release.send(()).unwrap(); // Always release even if the regression assertion fails.
        thread.join().unwrap();
        responsive.expect("storage held the session state lock");
        manager.checkpoint().unwrap();
        assert!(
            !*saved.lock().unwrap(),
            "late save resurrected deleted credentials"
        );
        assert!(manager.snapshot().signed_out);
    }

    #[test]
    fn logout_during_login_checkpoint_keeps_the_durable_exit_marker() {
        let (started, waiting) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let manager = Arc::new(Coordinator::with_repository(
            false,
            Box::new(BlockedStore {
                started,
                release: Mutex::new(released),
                saved: Arc::new(Mutex::new(false)),
            }),
        ));
        let dir = std::env::temp_dir().join(format!("selah-login-race-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let marker = dir.join("signed-out");
        let (cookie_store, http) = crate::client::new_cookie_client();
        manager
            .accept_verified(
                0,
                Service::Kgc,
                CookieClientParts { cookie_store, http },
                Some(AuthSession {
                    username: "alice".into(),
                    display_name: "Alice".into(),
                    student_id: "alice".into(),
                    faculty: String::new(),
                    department: String::new(),
                }),
            )
            .unwrap();
        manager.mark_login_pending(0).unwrap();
        waiting
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        let other = manager.clone();
        let other_marker = marker.clone();
        let checkpoint = std::thread::spawn(move || other.checkpoint_and_commit(&other_marker));
        // Sign-out must remain responsive while the storage worker is blocked.
        manager.sign_out_at(&marker).unwrap();
        manager.clear_all();
        release.send(()).unwrap();
        // A checkpoint admitted before logout may have queued one more save.
        // Pre-release it as well; no timing assumption controls the worker.
        release.send(()).unwrap();
        let _ = checkpoint.join().unwrap();
        manager.checkpoint().unwrap();
        assert!(marker.exists());
        assert!(manager.snapshot().signed_out);
        assert!(!manager.snapshot().login_persistence_pending);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
