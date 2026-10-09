//! Acknowledged UI work: async callers never block an executor thread or
//! produce their next animation frame before the main thread finishes this one.

use std::sync::atomic::{AtomicU64, Ordering};

use tauri::AppHandle;
use tokio::sync::oneshot;

pub(crate) type MainThreadJob = Box<dyn FnOnce() + Send + 'static>;

#[derive(Clone, Copy)]
pub(crate) struct MainThreadAnimation {
    generation: &'static AtomicU64,
    token: u64,
}

impl MainThreadAnimation {
    pub(crate) fn start(generation: &'static AtomicU64) -> Self {
        Self::with_token(
            generation,
            generation.fetch_add(1, Ordering::Relaxed).wrapping_add(1),
        )
    }

    pub(crate) fn with_token(generation: &'static AtomicU64, token: u64) -> Self {
        Self { generation, token }
    }

    pub(crate) fn is_current(self) -> bool {
        self.generation.load(Ordering::Relaxed) == self.token
    }

    pub(crate) async fn read<T: Send + 'static>(
        self,
        app: &AppHandle,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Option<T> {
        self.read_with(
            |job| app.run_on_main_thread(job).map_err(|err| err.to_string()),
            work,
        )
        .await
    }

    pub(crate) async fn frame(self, app: &AppHandle, work: impl FnOnce() + Send + 'static) -> bool {
        self.read(app, work).await.is_some()
    }

    pub(crate) async fn read_with<T: Send + 'static>(
        self,
        enqueue: impl FnOnce(MainThreadJob) -> Result<(), String>,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Option<T> {
        match dispatch(enqueue, move || self.is_current(), work).await {
            Ok(value) => value,
            Err(error) => {
                log::warn!("native animation main-thread dispatch failed: {error}");
                None
            }
        }
    }
}

async fn dispatch<T: Send + 'static>(
    enqueue: impl FnOnce(MainThreadJob) -> Result<(), String>,
    is_current: impl Fn() -> bool + Send + 'static,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<Option<T>, String> {
    if !is_current() {
        return Ok(None);
    }
    let (tx, rx) = oneshot::channel();
    enqueue(Box::new(move || {
        // A dropped waiter must not leave a queued UI side effect behind.
        if tx.is_closed() {
            return;
        }
        let value = is_current().then(work);
        let _ = tx.send(value);
    }))?;
    rx.await
        .map_err(|_| "main-thread job was dropped before acknowledgement".to_owned())
}

#[cfg(test)]
mod tests;
