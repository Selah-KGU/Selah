//! One owner for university authentication. Every asynchronous result belongs
//! to a generation; signing out retires that generation before clearing data.
mod contract;
mod persistence;
mod policy;
mod records;
pub(crate) use contract::{RecoveryOutcome, RecoveryReport, ServiceRecovery, SessionError};
pub(crate) use policy::RecoveryTrigger;
mod login_commit;
mod verification;
pub(crate) use records::SessionLease;
use records::{Record, SessionRepository};
use serde::Serialize;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;
use tokio::sync::{watch, OwnedMutexGuard};
pub(crate) use verification::verify;

pub(crate) const CANCELLED: &str = "University authentication operation was cancelled";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Service {
    Kgc,
    Luna,
    Kwic,
}

impl Service {
    pub fn index(self) -> usize {
        match self {
            Self::Kgc => 0,
            Self::Luna => 1,
            Self::Kwic => 2,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Kgc => "kgc",
            Self::Luna => "luna",
            Self::Kwic => "kwic",
        }
    }
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "kgc" => Some(Self::Kgc),
            "luna" => Some(Self::Luna),
            "kwic" => Some(Self::Kwic),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Health {
    Unverified,
    Valid,
    Refreshing,
    NeedsLogin,
    Unavailable,
    SignedOut,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ServiceStatus {
    pub service: &'static str,
    pub state: Health,
    pub credentials_present: bool,
    pub last_verified_at: Option<i64>,
    pub last_checked_at: Option<i64>,
    pub last_attempt_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct SessionDiagnostics {
    pub generation: u64,
    pub revision: u64,
    pub signed_out: bool,
    pub login_persistence_pending: bool,
    pub services: [ServiceStatus; 3],
}

struct State {
    records: [Record; 3],
    snapshot: SessionDiagnostics,
    completed: [Option<RecoveryResult>; 3],
    policy: policy::RecoveryPolicy,
    last_flow: Option<Instant>,
    pending_login: Option<crate::db::AccountContext>,
}

struct RecoveryResult {
    finished: Instant,
    result: ServiceRecovery,
}

pub(crate) struct Coordinator {
    persistence: persistence::Writer,
    clock_origin: Instant,
    gate: Arc<tokio::sync::Mutex<()>>,
    changed: watch::Sender<u64>,
    state: Mutex<State>,
    commit_gate: Mutex<()>,
}

impl Coordinator {
    fn new(signed_out: bool) -> Self {
        Self::with_repository(signed_out, Box::new(records::DiskRepository))
    }

    fn with_repository(signed_out: bool, repository: Box<dyn SessionRepository>) -> Self {
        let (changed, _) = watch::channel(0);
        Self {
            persistence: persistence::Writer::new(repository),
            clock_origin: Instant::now(),
            gate: Arc::new(tokio::sync::Mutex::new(())),
            changed,
            commit_gate: Mutex::new(()),
            state: Mutex::new(State {
                records: std::array::from_fn(|_| Record::empty()),
                completed: std::array::from_fn(|_| None),
                policy: policy::RecoveryPolicy::new(0),
                last_flow: None,
                pending_login: None,
                snapshot: SessionDiagnostics {
                    generation: 0,
                    revision: 0,
                    signed_out,
                    login_persistence_pending: false,
                    services: [Service::Kgc, Service::Luna, Service::Kwic].map(|service| {
                        ServiceStatus {
                            service: service.name(),
                            credentials_present: false,
                            state: if signed_out {
                                Health::SignedOut
                            } else {
                                Health::Unverified
                            },
                            last_verified_at: None,
                            last_checked_at: None,
                            last_attempt_at: None,
                        }
                    }),
                },
            }),
        }
    }

    pub async fn lock(&self) -> OwnedMutexGuard<()> {
        self.gate.clone().lock_owned().await
    }
    pub fn generation(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot
            .generation
    }
    pub fn signed_out(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot
            .signed_out
    }
    pub fn ensure_current(&self, generation: u64) -> Result<(), String> {
        if self.generation() == generation {
            Ok(())
        } else {
            Err(CANCELLED.into())
        }
    }

    /// Subscribe before checking, so invalidation between the check and the
    /// first await cannot be missed.
    pub async fn cancelled(&self, generation: u64) {
        let mut receiver = self.changed.subscribe();
        loop {
            if *receiver.borrow_and_update() != generation {
                return;
            }
            if receiver.changed().await.is_err() {
                return;
            }
        }
    }

    fn invalidate(&self, signed_out: bool) -> u64 {
        let _commit = self.commit_gate.lock().unwrap_or_else(|e| e.into_inner());
        self.invalidate_locked(signed_out)
    }
    fn invalidate_locked(&self, signed_out: bool) -> u64 {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.pending_login = None;
        state.snapshot.login_persistence_pending = false;
        state.completed = std::array::from_fn(|_| None);
        state.policy = policy::RecoveryPolicy::new(self.clock_origin.elapsed().as_secs() as i64);
        let snapshot = &mut state.snapshot;
        snapshot.generation = snapshot.generation.wrapping_add(1);
        snapshot.revision = snapshot.revision.wrapping_add(1);
        snapshot.signed_out = signed_out;
        for service in &mut snapshot.services {
            if signed_out {
                service.credentials_present = false;
            }
            service.state = if signed_out {
                Health::SignedOut
            } else {
                Health::Unverified
            };
            service.last_verified_at = None;
            service.last_checked_at = None;
            service.last_attempt_at = None;
        }
        self.changed.send_replace(snapshot.generation);
        snapshot.generation
    }

    pub fn record(&self, generation: u64, service: Service, health: Health) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.snapshot.generation != generation || state.snapshot.signed_out {
            return;
        }
        // Only positive server evidence through accept_verified/validated can set Valid.
        if health == Health::Valid {
            return;
        }
        state.snapshot.revision = state.snapshot.revision.wrapping_add(1);
        let status = &mut state.snapshot.services[service.index()];
        status.state = health;
        let now = crate::db::epoch_secs();
        if health == Health::Refreshing {
            status.last_attempt_at = Some(now);
        }
    }

    pub fn probe_due(&self, service: Service, trigger: RecoveryTrigger) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.policy.probe_due(
            service,
            state.snapshot.services[service.index()].state,
            trigger,
            self.clock_origin.elapsed().as_secs() as i64,
        )
    }

    pub fn completed_since(
        &self,
        generation: u64,
        service: Service,
        requested: Instant,
    ) -> Option<ServiceRecovery> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.snapshot.generation != generation {
            return None;
        }
        state.completed[service.index()]
            .as_ref()
            .filter(|done| done.finished >= requested)
            .map(|done| done.result.clone())
    }

    /// Called under the shared flow gate. Overlapping work reuses the exact
    /// outcome; later callers get an explicit deferral from the same policy.
    pub fn recovery_result(
        &self,
        generation: u64,
        service: Service,
        requested: Instant,
        trigger: RecoveryTrigger,
    ) -> Option<ServiceRecovery> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.snapshot.generation != generation {
            return None;
        }
        if let Some(done) = &state.completed[service.index()] {
            if done.finished >= requested {
                return Some(done.result.clone());
            }
        }
        state
            .policy
            .retry_at(
                service,
                trigger,
                self.clock_origin.elapsed().as_secs() as i64,
            )
            .map(|retry_at| {
                let mut result = ServiceRecovery::new(service, RecoveryOutcome::Deferred);
                result.retry_at = (retry_at != i64::MAX).then(|| {
                    crate::db::epoch_secs().saturating_add(
                        retry_at.saturating_sub(self.clock_origin.elapsed().as_secs() as i64),
                    )
                });
                result
            })
    }
    pub fn finish_recovery(&self, generation: u64, service: Service, result: &ServiceRecovery) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.snapshot.generation == generation && !state.snapshot.signed_out {
            state.policy.completed(
                service,
                self.clock_origin.elapsed().as_secs() as i64,
                result.outcome,
            );
            state.completed[service.index()] = Some(RecoveryResult {
                finished: Instant::now(),
                result: result.clone(),
            });
        }
    }
    pub async fn pace_flow(&self) {
        let wait = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last_flow
            .and_then(|last| std::time::Duration::from_secs(2).checked_sub(last.elapsed()));
        if let Some(wait) = wait {
            tokio::time::sleep(wait).await;
        }
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last_flow = Some(Instant::now());
    }

    pub fn snapshot(&self) -> SessionDiagnostics {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot
            .clone()
    }
}

pub(super) fn signout_marker() -> std::path::PathBuf {
    crate::client::data_dir().join(if cfg!(debug_assertions) {
        "university-signed-out.dev"
    } else {
        "university-signed-out"
    })
}

pub(crate) static SESSIONS: LazyLock<Coordinator> =
    LazyLock::new(|| Coordinator::new(signout_marker().exists()));

/// Caller holds the authentication gate. Existing async reads become obsolete
/// when a new interactive login starts, including reads from other WebViews.
pub(crate) fn begin_interactive(app: &tauri::AppHandle) -> u64 {
    let generation = SESSIONS.invalidate(false);
    emit_generation(app);
    generation
}

pub(crate) async fn confirm_interactive(generation: u64) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        SESSIONS.ensure_current(generation)?;
        SESSIONS.checkpoint_and_commit(&signout_marker())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Persist exit intent and retire pending commits under the same dedicated gate.
/// File I/O never holds the session state mutex.
pub(crate) fn sign_out(app: &tauri::AppHandle) -> Result<(), String> {
    SESSIONS.sign_out_at(&signout_marker())?;
    emit_generation(app);
    Ok(())
}

fn emit_generation(app: &tauri::AppHandle) {
    use tauri::Emitter;
    let _ = app.emit("university-auth-generation", SESSIONS.snapshot());
}

#[tauri::command]
pub(crate) fn get_session_diagnostics() -> SessionDiagnostics {
    SESSIONS.snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_recovery_shares_failure_and_later_calls_are_deferred() {
        let coordinator = Coordinator::new(false);
        let generation = coordinator.generation();
        let requested = Instant::now();
        let failure = ServiceRecovery::unavailable(Service::Luna, "offline");
        assert!(coordinator
            .recovery_result(
                generation,
                Service::Luna,
                requested,
                RecoveryTrigger::Background
            )
            .is_none());
        coordinator.finish_recovery(generation, Service::Luna, &failure);
        assert_eq!(
            coordinator.recovery_result(
                generation,
                Service::Luna,
                requested,
                RecoveryTrigger::Manual
            ),
            Some(failure)
        );
        assert_eq!(
            coordinator
                .recovery_result(
                    generation,
                    Service::Luna,
                    Instant::now(),
                    RecoveryTrigger::Background
                )
                .unwrap()
                .outcome,
            RecoveryOutcome::Deferred
        );
        assert!(coordinator
            .recovery_result(
                generation,
                Service::Kwic,
                requested,
                RecoveryTrigger::Background
            )
            .is_none());
        let current = coordinator.invalidate(false);
        coordinator.finish_recovery(
            generation,
            Service::Luna,
            &ServiceRecovery::new(Service::Luna, RecoveryOutcome::Verified),
        );
        assert!(coordinator
            .recovery_result(
                current,
                Service::Luna,
                requested,
                RecoveryTrigger::Background
            )
            .is_none());
    }

    #[test]
    fn cancellation_unblocks_auth_gate_and_obsolete_queued_work_is_rejected() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            let coordinator = Arc::new(Coordinator::new(false));
            let generation = coordinator.generation();
            let owner = coordinator.clone();
            let (started, receiver) = tokio::sync::oneshot::channel();
            let running = tokio::spawn(async move {
                let _guard = owner.lock().await;
                started.send(()).unwrap();
                owner.cancelled(generation).await;
            });
            receiver.await.unwrap();
            let queued = coordinator.clone();
            let second = tokio::spawn(async move {
                let _guard = queued.lock().await;
                queued.ensure_current(generation)
            });
            coordinator.invalidate(true);
            running.await.unwrap();
            assert_eq!(second.await.unwrap(), Err(CANCELLED.into()));
            // A cancellation before subscription must also resolve immediately.
            tokio::time::timeout(
                std::time::Duration::from_millis(100),
                coordinator.cancelled(generation),
            )
            .await
            .unwrap();
        });
    }
}

#[tauri::command]
pub(crate) fn get_university_session_snapshot() -> RecoveryReport {
    let (snapshot, identity) = SESSIONS.overview();
    RecoveryReport {
        results: Vec::new(),
        snapshot,
        identity,
    }
}
