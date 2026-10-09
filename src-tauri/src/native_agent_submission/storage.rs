//! Retain accepted speech after a storage error so the next quit can retry it.
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

pub(super) static STORAGE: LazyLock<Arc<VoiceStorage>> =
    LazyLock::new(|| Arc::new(VoiceStorage::default()));

pub(super) struct VoiceInput {
    pub(super) account: crate::db::AccountContext,
    pub(super) conversation_id: String,
    pub(super) text: String,
}

struct PendingVoice {
    input: Arc<VoiceInput>,
    running: bool,
    retry: bool,
}

#[derive(Default)]
pub(super) struct VoiceStorage(Mutex<HashMap<String, PendingVoice>>);

pub(super) struct VoiceAttempt {
    owner: Arc<VoiceStorage>,
    input: Arc<VoiceInput>,
    retry: bool,
    completed: bool,
}

impl VoiceStorage {
    pub(super) fn accept(self: &Arc<Self>, conversation_id: String, text: String) -> VoiceAttempt {
        let input = Arc::new(VoiceInput {
            account: crate::db::capture_account(),
            conversation_id,
            text,
        });
        let mut pending = self.0.lock().unwrap_or_else(|e| e.into_inner());
        assert!(
            !pending.contains_key(&input.conversation_id),
            "voice ID already accepted"
        );
        pending.insert(
            input.conversation_id.clone(),
            PendingVoice {
                input: input.clone(),
                running: true,
                retry: false,
            },
        );
        VoiceAttempt {
            owner: self.clone(),
            input,
            retry: false,
            completed: false,
        }
    }

    pub(super) fn retry_failed(self: &Arc<Self>) -> Vec<VoiceAttempt> {
        let mut pending = self.0.lock().unwrap_or_else(|e| e.into_inner());
        pending
            .values_mut()
            .filter_map(|voice| {
                if voice.running {
                    return None;
                }
                voice.running = true;
                Some(VoiceAttempt {
                    owner: self.clone(),
                    input: voice.input.clone(),
                    retry: voice.retry,
                    completed: false,
                })
            })
            .collect()
    }
}

impl VoiceAttempt {
    pub(super) fn persist_with<T>(
        mut self,
        write: impl FnOnce(&VoiceInput, bool) -> Result<T, String>,
    ) -> Result<T, String> {
        let result = write(&self.input, self.retry);
        if result.is_ok() {
            self.owner
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&self.input.conversation_id);
            self.completed = true;
        }
        result
    }
}

impl Drop for VoiceAttempt {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        if let Some(voice) = self
            .owner
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(&self.input.conversation_id)
        {
            voice.running = false;
            // A panic can happen after commit but before acknowledgement.
            // Retry admission checks the committed input instead of appending.
            voice.retry = true;
        }
    }
}

#[cfg(test)]
#[path = "storage/tests.rs"]
mod tests;
