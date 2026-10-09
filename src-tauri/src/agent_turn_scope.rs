//! One immutable inference identity per request; conversation IDs route UI only.
use std::collections::HashMap;
use std::future::Future;
use std::sync::{
    atomic::{AtomicBool, AtomicI64, Ordering},
    Arc, LazyLock, Mutex, Weak,
};
use tokio::sync::Notify;

#[derive(Default)]
struct Registry {
    conversations: HashMap<String, Weak<Turn>>,
    generations: HashMap<String, Weak<Turn>>,
}
static REGISTRY: LazyLock<Mutex<Registry>> = LazyLock::new(|| Mutex::new(Registry::default()));
tokio::task_local! { pub(crate) static CURRENT: Arc<Turn>; }

pub(crate) struct Turn {
    conversation: String,
    account: crate::db::AccountContext,
    generation: String,
    request: String,
    current: AtomicBool,
    cancelled: AtomicBool,
    cancellation: Notify,
    input_message: AtomicI64,
    messages: Mutex<Vec<i64>>,
}
/// A causal boundary plus records committed by this request. Message IDs are
/// taken only from successful SQLite commits, never guessed from content/time.
pub(crate) struct History {
    pub(crate) input_message: i64,
    pub(crate) messages: Vec<i64>,
}

impl Turn {
    pub(crate) fn account_context(&self) -> crate::db::AccountContext {
        self.account.clone()
    }
    pub(crate) fn set_input_message(&self, id: i64) -> Result<(), crate::agent_error::AgentError> {
        if id <= 0
            || self
                .input_message
                .compare_exchange(0, id, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return Err(crate::agent_error::AgentError::db(
                "入力の保存記録が一致しません",
            ));
        }
        Ok(())
    }
    pub(crate) fn record_message(&self, id: i64) {
        self.messages
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(id);
    }
    pub(crate) fn history(&self) -> Result<History, crate::agent_error::AgentError> {
        let input_message = self.input_message.load(Ordering::Acquire);
        if input_message <= 0 {
            return Err(crate::agent_error::AgentError::db(
                "入力がまだ保存されていません",
            ));
        }
        Ok(History {
            input_message,
            messages: self
                .messages
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone(),
        })
    }
    pub(crate) fn generation(&self) -> &str {
        &self.generation
    }
    pub(crate) fn request(&self) -> &str {
        &self.request
    }
    pub(crate) fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    pub(crate) fn accepts_event(&self, terminal: bool) -> bool {
        self.current.load(Ordering::Acquire) && (terminal || !self.cancelled())
    }
    async fn wait_cancelled(&self) {
        let notified = self.cancellation.notified();
        tokio::pin!(notified);
        // Register before checking the latch so cancel between the check and
        // await cannot be lost. Every waiter observes the same immutable latch.
        notified.as_mut().enable();
        if !self.cancelled() {
            notified.await;
        }
    }
    fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::AcqRel) {
            self.cancellation.notify_waiters();
            crate::agent_provider::AgentProvider::cancel(&self.generation);
        }
    }
}
impl Drop for Turn {
    fn drop(&mut self) {
        // Blocking model workers retain an Arc. Cleanup runs only after their
        // callbacks and cancellation checks have finished, including RPC abort.
        crate::agent_provider::AgentProvider::clear_cancel(&self.generation);
        let mut registry = REGISTRY.lock().unwrap_or_else(|error| error.into_inner());
        registry.generations.remove(&self.generation);
        if registry
            .conversations
            .get(&self.conversation)
            .is_some_and(|entry| entry.strong_count() == 0)
        {
            registry.conversations.remove(&self.conversation);
        }
    }
}

pub(crate) struct RunningTurn {
    pub(crate) turn: Arc<Turn>,
    finished: bool,
}
impl RunningTurn {
    pub(crate) fn begin(conversation: &str, request: Option<String>) -> Self {
        let generation = uuid::Uuid::new_v4().to_string();
        let turn = Arc::new(Turn {
            conversation: conversation.into(),
            account: crate::db::capture_account(),
            request: request
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| generation.clone()),
            generation,
            current: AtomicBool::new(true),
            cancelled: AtomicBool::new(false),
            cancellation: Notify::new(),
            input_message: AtomicI64::new(0),
            messages: Mutex::new(Vec::new()),
        });
        let previous = {
            let mut registry = REGISTRY.lock().unwrap_or_else(|error| error.into_inner());
            let previous = registry
                .conversations
                .insert(conversation.into(), Arc::downgrade(&turn))
                .and_then(|entry| entry.upgrade());
            if let Some(previous) = &previous {
                previous.current.store(false, Ordering::Release);
            }
            registry
                .generations
                .insert(turn.generation.clone(), Arc::downgrade(&turn));
            previous
        };
        if let Some(previous) = previous {
            previous.cancel();
        }
        Self {
            turn,
            finished: false,
        }
    }
    pub(crate) fn finish(&mut self) {
        // A timeout drops an async waiter, not a blocking model worker. Close
        // the request after its terminal event and keep cancellation latched
        // until every worker/callback holding the generation has exited.
        self.turn.cancel();
        self.finished = true;
    }
}
impl Drop for RunningTurn {
    fn drop(&mut self) {
        if !self.finished {
            self.turn.cancel();
        }
    }
}

/// Admission is synchronous: registration and the blocking preparation job
/// exist before the IPC completion future is scheduled or polled.
pub(crate) struct Admission<T> {
    pub(crate) running: RunningTurn,
    preparation: tauri::async_runtime::JoinHandle<Result<T, crate::agent_error::AgentError>>,
}
impl<T: Send + 'static> Admission<T> {
    pub(crate) fn start(
        conversation: &str,
        request: Option<String>,
        prepare: impl FnOnce(Arc<Turn>) -> Result<T, crate::agent_error::AgentError> + Send + 'static,
    ) -> Self {
        let running = RunningTurn::begin(conversation, request);
        let owner = running.turn.clone();
        // Use Tauri's handle: IPC admission is also called outside a Tokio task.
        let preparation = tauri::async_runtime::spawn_blocking(move || prepare(owner));
        Self {
            running,
            preparation,
        }
    }
    pub(crate) async fn prepared(&mut self) -> Result<T, crate::agent_error::AgentError> {
        (&mut self.preparation)
            .await
            .map_err(crate::agent_error::AgentError::task)?
    }
}

pub(crate) fn current(id: &str) -> Option<Arc<Turn>> {
    CURRENT
        .try_with(|turn| {
            if id == turn.conversation || id == turn.generation {
                Some(turn.clone())
            } else {
                None
            }
        })
        .ok()
        .flatten()
}

pub(crate) fn generation_cancelled(id: &str) -> bool {
    let id = id.strip_prefix("plan:").unwrap_or(id);
    let entry = {
        REGISTRY
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .generations
            .get(id)
            .cloned()
    };
    entry
        .and_then(|entry| entry.upgrade())
        .is_some_and(|turn| turn.cancelled())
}

pub(crate) fn cancel(conversation: &str, request: Option<&str>) -> bool {
    let entry = {
        REGISTRY
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .conversations
            .get(conversation)
            .cloned()
    };
    let Some(turn) = entry.and_then(|entry| entry.upgrade()) else {
        return false;
    };
    if request.is_some_and(|id| id != turn.request) {
        return false;
    }
    turn.cancel();
    true
}

/// A successfully deleted conversation cannot deliver even a terminal event.
/// Invalidate under the admission lock, then wake/cancel outside it so a new
/// request cannot be accidentally retired by a delayed old cancellation.
pub(crate) fn retire_conversation(conversation: &str) {
    let owner = {
        let registry = REGISTRY.lock().unwrap_or_else(|error| error.into_inner());
        let owner = registry
            .conversations
            .get(conversation)
            .and_then(Weak::upgrade);
        if let Some(owner) = &owner {
            owner.current.store(false, Ordering::Release);
        }
        owner
    };
    if let Some(owner) = owner {
        owner.cancel();
    }
}

/// Drop a provider's async HTTP/model waiter promptly on cancellation. Local
/// blocking workers keep their own Arc and observe the same cancellation latch.
pub(crate) async fn until_cancelled<T>(
    turn: Option<&Turn>,
    future: impl Future<Output = Result<T, crate::agent_error::AgentError>>,
) -> Result<T, crate::agent_error::AgentError> {
    match turn {
        Some(turn) => tokio::select! {
            biased;
            _ = turn.wait_cancelled() => Err(crate::agent_error::AgentError::Cancelled),
            result = future => result,
        },
        None => future.await,
    }
}

#[cfg(test)]
#[path = "agent_turn_scope/tests.rs"]
mod tests;

/// Preserve the account that admitted an inference across its later tools and
/// persistence operations, even if a different login is now active.
pub(crate) fn account_context() -> Option<crate::db::AccountContext> {
    CURRENT.try_with(|turn| turn.account.clone()).ok()
}
