//! A dropped async waiter must cancel its blocking native inference too.
use std::sync::{Arc, Mutex};

struct Request {
    id: String,
    finished: Mutex<bool>,
    cancel: fn(&str),
    clear: fn(&str),
}

struct CancelOnDrop(Arc<Request>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        let finished = self.0.finished.lock().unwrap_or_else(|e| e.into_inner());
        if !*finished {
            (self.0.cancel)(&self.0.id);
        }
    }
}

struct FinishOnDrop(Arc<Request>);
impl Drop for FinishOnDrop {
    fn drop(&mut self) {
        // Serialize completion/cleanup with cancellation. An abort immediately
        // after completion must not leave a fresh cancellation latch behind.
        let mut finished = self.0.finished.lock().unwrap_or_else(|e| e.into_inner());
        *finished = true;
        (self.0.clear)(&self.0.id);
    }
}

pub(super) async fn run<T: Send + 'static>(
    work: impl FnOnce(String) -> T + Send + 'static,
    cancel: fn(&str),
    clear: fn(&str),
) -> Result<T, tokio::task::JoinError> {
    let state = Arc::new(Request {
        id: format!("live-caption-{}", uuid::Uuid::new_v4()),
        finished: Mutex::new(false),
        cancel,
        clear,
    });
    let _cancel = CancelOnDrop(state.clone());
    tokio::task::spawn_blocking(move || {
        let _finish = FinishOnDrop(state.clone());
        work(state.id.clone())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::LazyLock;

    static CANCELLED: LazyLock<Mutex<HashSet<String>>> =
        LazyLock::new(|| Mutex::new(HashSet::new()));
    fn cancel(id: &str) {
        CANCELLED.lock().unwrap().insert(id.into());
    }
    fn clear(id: &str) {
        CANCELLED.lock().unwrap().remove(id);
    }

    #[tokio::test]
    async fn dropping_the_waiter_cancels_native_work_and_clears_after_it_exits() {
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let task = tokio::spawn(run(
            move |id| {
                started.send(id.clone()).unwrap();
                blocked.recv().unwrap();
                assert!(CANCELLED.lock().unwrap().contains(&id));
            },
            cancel,
            clear,
        ));
        let id = ready.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(CANCELLED.lock().unwrap().contains(&id));
        release.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while CANCELLED.lock().unwrap().contains(&id) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn completion_never_cancels_a_finished_request() {
        fn unexpected_cancel(_: &str) {
            panic!("completed request was cancelled");
        }
        assert_eq!(run(|_| 42, unexpected_cancel, clear).await.unwrap(), 42);
    }
}
