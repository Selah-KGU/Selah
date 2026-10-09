//! Per-integration login attempts. Call while holding the integration's client lock.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Default)]
pub(crate) struct Lifecycle {
    generation: u64,
    revision: u64,
    cancel: Option<Arc<AtomicBool>>,
}
#[derive(Clone)]
pub(crate) struct Attempt {
    pub generation: u64,
    pub cancel: Arc<AtomicBool>,
}
impl Lifecycle {
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Accepted credentials changed without cancelling an independent pending attempt.
    pub fn published(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn invalidate(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        self.generation = self.generation.wrapping_add(1);
        self.published();
    }
    pub fn begin(&mut self) -> Attempt {
        self.invalidate();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        Attempt {
            generation: self.generation,
            cancel,
        }
    }
    pub fn ensure(&self, attempt: &Attempt) -> Result<(), String> {
        if self.generation != attempt.generation || attempt.cancel.load(Ordering::Acquire) {
            Err("OAuth login was superseded or disconnected".into())
        } else {
            Ok(())
        }
    }
}

/// Only a definitive rejection of this refresh grant retires credentials.
/// Rate limits, server failures, configuration errors and malformed bodies retain them.
pub(crate) fn refresh_grant_rejected(
    status: reqwest::StatusCode,
    body: &serde_json::Value,
) -> bool {
    status.is_client_error()
        && status != reqwest::StatusCode::TOO_MANY_REQUESTS
        && body.get("error").and_then(|v| v.as_str()) == Some("invalid_grant")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_login_and_logout_retire_old_callbacks() {
        let mut state = Lifecycle::default();
        let old = state.begin();
        let new = state.begin();
        assert!(old.cancel.load(Ordering::Acquire));
        assert!(state.ensure(&old).is_err());
        assert!(state.ensure(&new).is_ok());
        state.invalidate();
        assert!(state.ensure(&new).is_err());
    }
    #[test]
    fn publication_advances_status_without_cancelling_pending_login() {
        let mut state = Lifecycle::default();
        let attempt = state.begin();
        let before = state.revision();
        state.published();
        assert!(state.revision() > before);
        assert!(state.ensure(&attempt).is_ok());
    }
    #[test]
    fn transient_and_configuration_errors_do_not_revoke_refresh_tokens() {
        for (code, error, rejected) in [
            (400, "invalid_grant", true),
            (429, "invalid_grant", false),
            (503, "invalid_grant", false),
            (500, "server_error", false),
            (400, "temporarily_unavailable", false),
            (401, "invalid_client", false),
        ] {
            assert_eq!(
                refresh_grant_rejected(
                    reqwest::StatusCode::from_u16(code).unwrap(),
                    &serde_json::json!({"error":error})
                ),
                rejected
            );
        }
        assert!(!refresh_grant_rejected(
            reqwest::StatusCode::BAD_REQUEST,
            &serde_json::Value::Null
        ));
    }
}

pub(crate) fn generate_pkce() -> (String, String) {
    use rand::Rng;
    use sha2::{Digest, Sha256};
    let mut rng = rand::thread_rng();
    let verifier: String = (0..64)
        .map(|_| {
            let idx = rng.gen_range(0..66);
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~"[idx] as char
        })
        .collect();
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    let challenge = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, hash);
    (verifier, challenge)
}
