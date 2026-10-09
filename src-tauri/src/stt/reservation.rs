//! Reserve under the owner lock without waiting on the registry under it.
use std::sync::{Mutex, TryLockError};

pub(crate) fn with_available_registry<S, T>(
    registry: &Mutex<S>,
    mut with_owner: impl FnMut(
        &mut dyn FnMut() -> Result<Option<T>, String>,
    ) -> Result<Option<T>, String>,
    reserve: impl FnOnce(&mut S) -> Result<T, String>,
) -> Result<T, String> {
    let mut reserve = Some(reserve);
    loop {
        let mut attempt = || match registry.try_lock() {
            Ok(mut state) => {
                reserve.take().expect("reservation is consumed once")(&mut state).map(Some)
            }
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Poisoned(_)) => Err("STT state lock failed".into()),
        };
        if let Some(result) = with_owner(&mut attempt)? {
            return Ok(result);
        }
        // The owner guard has gone out of scope. Waiting here cannot hold the
        // native/LIVE UI lock. Retry ownership after every wait; never reuse it.
        drop(
            registry
                .lock()
                .map_err(|_| "STT state lock failed".to_string())?,
        );
    }
}

#[cfg(test)]
#[path = "reservation/tests.rs"]
mod tests;
