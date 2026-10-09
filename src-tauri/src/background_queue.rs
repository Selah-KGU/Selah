//! Admit ordered work before polling its reply; keep IO off UI/async threads.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

type Job = Box<dyn FnOnce() + Send>;
#[derive(Default)]
struct State {
    pending: VecDeque<Job>,
    running: bool,
    sealed: bool,
}
pub(crate) struct Queue {
    state: Mutex<State>,
    idle: tokio::sync::Notify,
    failure: &'static str,
}

impl Queue {
    pub(crate) fn new(failure: &'static str) -> Self {
        Self {
            state: Mutex::new(State::default()),
            idle: tokio::sync::Notify::new(),
            failure,
        }
    }

    pub(crate) fn submit<R: Send + 'static>(
        self: &Arc<Self>,
        work: impl FnOnce() -> Result<R, String> + Send + 'static,
    ) -> impl std::future::Future<Output = Result<R, String>> + Send + 'static {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let failure = self.failure;
        let start = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.sealed {
                Err("アプリケーションを終了中です".to_string())
            } else {
                state.pending.push_back(Box::new(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                        .unwrap_or_else(|_| Err(format!("{failure}: worker panic")));
                    // Dropping an IPC response cannot revoke an admitted mutation.
                    let _ = sender.send(result);
                }));
                let start = !state.running;
                state.running = true;
                Ok(start)
            }
        };
        if matches!(start, Ok(true)) {
            let queue = self.clone();
            tauri::async_runtime::spawn_blocking(move || queue.drain());
        }
        async move {
            start?;
            receiver
                .await
                .map_err(|_| format!("{failure}: worker ended"))?
        }
    }

    /// Linearize the shutdown boundary with submission under the same mutex.
    pub(crate) fn seal(&self) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .sealed = true;
    }

    pub(crate) fn reopen(&self) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .sealed = false;
    }

    /// Await completed work, including commit notifications, without blocking
    /// an async thread. Mutation errors belong to their individual IPC replies;
    /// waiting here does not retry or turn them into recording-save failures.
    pub(crate) async fn drained(&self) {
        loop {
            let idle = self.idle.notified();
            tokio::pin!(idle);
            idle.as_mut().enable();
            {
                let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                if !state.running && state.pending.is_empty() {
                    return;
                }
            }
            idle.await;
        }
    }

    fn drain(&self) {
        loop {
            let job = {
                let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                match state.pending.pop_front() {
                    Some(job) => job,
                    None => {
                        state.running = false;
                        drop(state);
                        self.idle.notify_waiters();
                        return;
                    }
                }
            };
            job();
        }
    }
}

#[cfg(test)]
#[path = "background_queue/tests.rs"]
mod tests;
