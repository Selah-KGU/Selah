//! macOS storage of the shared native Agent state machine.
pub(super) use crate::native_agent_state::*;
use std::sync::{LazyLock, Mutex};
pub(super) static SHARED: LazyLock<Mutex<SharedState>> =
    LazyLock::new(|| Mutex::new(SharedState::default()));
