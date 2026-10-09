use super::{Coordinator, Service, CANCELLED};
use std::path::Path;

impl Coordinator {
    pub(crate) fn mark_login_pending(&self, generation: u64) -> Result<(), String> {
        let _commit = self.commit_gate.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.snapshot.generation != generation
            || state.snapshot.signed_out
            || state.snapshot.services[Service::Kgc.index()]
                .last_verified_at
                .is_none()
        {
            return Err(CANCELLED.into());
        }
        state.pending_login = Some(crate::db::AccountContext {
            generation,
            username: state.records[0]
                .identity
                .as_ref()
                .map(|s| s.username.clone()),
        });
        state.snapshot.login_persistence_pending = true;
        state.snapshot.revision = state.snapshot.revision.wrapping_add(1);
        Ok(())
    }

    pub(crate) fn checkpoint_and_commit(&self, marker: &Path) -> Result<(), String> {
        // Capture the identity before waiting: a newer login cannot be committed
        // using an older checkpoint's completion.
        let pending = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending_login
            .clone();
        self.checkpoint()?;
        let Some(pending) = pending else {
            return Ok(());
        };
        let _commit = self.commit_gate.lock().unwrap_or_else(|e| e.into_inner());
        if pending != self.account_context() {
            return Err(CANCELLED.into());
        }
        if self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending_login
            .as_ref()
            != Some(&pending)
        {
            return Ok(());
        }
        match std::fs::remove_file(marker) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Commit university login: {e}")),
        }
        #[cfg(unix)]
        if let Some(parent) = marker.parent() {
            std::fs::File::open(parent)
                .and_then(|dir| dir.sync_all())
                .map_err(|e| format!("Commit university login directory: {e}"))?;
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.pending_login = None;
        state.snapshot.login_persistence_pending = false;
        state.snapshot.revision = state.snapshot.revision.wrapping_add(1);
        Ok(())
    }

    pub(super) fn sign_out_at(&self, marker: &Path) -> Result<(), String> {
        let _commit = self.commit_gate.lock().unwrap_or_else(|e| e.into_inner());
        crate::keychain::file::atomic_write(marker, b"signed-out\n")?;
        self.invalidate_locked(true);
        Ok(())
    }
}
