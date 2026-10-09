//! Track accepted input until its blocking storage job completes, not inference.
use std::sync::{Arc, LazyLock, Mutex};
use tokio::sync::Notify;

pub(crate) static INPUT_SAVES: LazyLock<Arc<PendingPersistence>> =
    LazyLock::new(|| Arc::new(PendingPersistence::default()));

#[derive(Default)]
struct Pending {
    sealed: bool,
    count: usize,
    monitoring_exit: bool,
    failure: Option<String>,
}

#[derive(Default)]
pub(crate) struct PendingPersistence {
    pending: Mutex<Pending>,
    changed: Notify,
}

pub(crate) struct SavePermit {
    owner: Arc<PendingPersistence>,
    completed: bool,
}

impl PendingPersistence {
    /// Reserve synchronously before handing accepted speech to another task.
    pub(crate) fn reserve(self: &Arc<Self>) -> Result<SavePermit, String> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if pending.sealed {
            return Err("アプリケーションを終了中です".into());
        }
        pending.count += 1;
        Ok(SavePermit {
            owner: Arc::clone(self),
            completed: false,
        })
    }

    pub(crate) fn begin_shutdown(&self) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.monitoring_exit = true;
        pending.failure = None;
    }

    /// Seal only after STT's final callbacks have reserved their storage jobs.
    pub(crate) fn seal(&self) {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .sealed = true;
    }

    pub(crate) fn reopen(&self) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.sealed = false;
        pending.monitoring_exit = false;
        pending.failure = None;
    }

    pub(crate) async fn drained(&self) -> Result<(), String> {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
                if pending.count == 0 {
                    return pending.failure.clone().map_or(Ok(()), Err);
                }
            }
            changed.await;
        }
    }
}

impl SavePermit {
    pub(crate) fn complete(mut self, result: Result<(), &str>) {
        self.completed = true;
        if let Err(error) = result {
            let mut pending = self.owner.pending.lock().unwrap_or_else(|e| e.into_inner());
            if pending.monitoring_exit && pending.failure.is_none() {
                pending.failure = Some(error.to_string());
            }
        }
    }
}

impl Drop for SavePermit {
    fn drop(&mut self) {
        let mut pending = self.owner.pending.lock().unwrap_or_else(|e| e.into_inner());
        if !self.completed && pending.monitoring_exit && pending.failure.is_none() {
            pending.failure = Some("受け付けた入力の保存処理が中断されました".into());
        }
        pending.count -= 1;
        let drained = pending.count == 0;
        drop(pending);
        if drained {
            self.owner.changed.notify_waiters();
        }
    }
}

#[cfg(test)]
#[path = "pending_persistence/tests.rs"]
mod tests;
