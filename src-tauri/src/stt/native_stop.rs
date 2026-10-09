//! Native callbacks stop immediately when available, otherwise queue by input ID.
use super::runtime::{SttInputState, STT_SESSION};
use std::ops::Deref;
use std::sync::{Mutex, TryLockError};

fn stop_owned(state: &SttInputState, input_id: &str) {
    if let Some(session) = state.active.as_ref() {
        session.request_stop_for_owner(Some("native_agent"), None, Some(input_id));
    }
}

pub(in crate::stt) fn request(input_id: &str) -> Result<(), String> {
    request_on_registry(&STT_SESSION, input_id).map(|_| ())
}

// Return a handle for isolated tests. Dropping it leaves an accepted stop alive.
fn request_on_registry<R>(
    registry: R,
    input_id: &str,
) -> Result<Option<tauri::async_runtime::JoinHandle<Result<(), String>>>, String>
where
    R: Deref<Target = Mutex<SttInputState>> + Send + 'static,
{
    match registry.try_lock() {
        Ok(state) => {
            stop_owned(&state, input_id);
            return Ok(None);
        }
        Err(TryLockError::Poisoned(_)) => return Err("STT state lock failed".into()),
        Err(TryLockError::WouldBlock) => {}
    }
    let input_id = input_id.to_owned();
    Ok(Some(tauri::async_runtime::spawn_blocking(move || {
        let result = registry
            .lock()
            .map_err(|_| "STT state lock failed".to_string())
            .map(|state| stop_owned(&state, &input_id));
        if let Err(error) = &result {
            log::warn!("[stt] native input stop failed: {error}");
        }
        result
    })))
}

#[cfg(test)]
#[path = "native_stop/tests.rs"]
mod tests;
