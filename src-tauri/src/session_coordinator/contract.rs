use super::{Service, SessionDiagnostics};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecoveryOutcome {
    Verified,
    NeedsLogin,
    Unavailable,
    Deferred,
    SignedOut,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ServiceRecovery {
    pub service: Service,
    pub outcome: RecoveryOutcome,
    pub recovered: bool,
    pub retry_at: Option<i64>,
    pub message: Option<String>,
}
impl ServiceRecovery {
    pub fn new(service: Service, outcome: RecoveryOutcome) -> Self {
        Self {
            service,
            outcome,
            recovered: false,
            retry_at: None,
            message: None,
        }
    }
    pub fn unavailable(service: Service, error: impl ToString) -> Self {
        Self {
            message: Some(error.to_string()),
            ..Self::new(service, RecoveryOutcome::Unavailable)
        }
    }
}
#[derive(Serialize)]
pub(crate) struct RecoveryReport {
    pub results: Vec<ServiceRecovery>,
    pub snapshot: SessionDiagnostics,
    pub identity: Option<crate::auth::AuthSession>,
}
impl RecoveryReport {
    pub fn verified(&self, service: Service) -> bool {
        self.results
            .iter()
            .any(|r| r.service == service && r.outcome == RecoveryOutcome::Verified)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub(crate) enum SessionError {
    Cancelled,
    InvalidService(String),
    Storage(crate::keychain::StoreError),
    Unavailable(String),
    NeedsLogin,
}
impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str(super::CANCELLED),
            Self::InvalidService(name) => write!(f, "Unknown university service: {name}"),
            Self::Storage(error) => error.fmt(f),
            Self::Unavailable(message) => f.write_str(message),
            Self::NeedsLogin => f.write_str("University sign-in required"),
        }
    }
}
impl From<SessionError> for String {
    fn from(error: SessionError) -> Self {
        error.to_string()
    }
}
impl From<crate::keychain::StoreError> for SessionError {
    fn from(error: crate::keychain::StoreError) -> Self {
        Self::Storage(error)
    }
}
