//! Remote and turn-level cancellation registry.

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

pub const CANCELLED_MSG: &str = "推論はキャンセルされました";
// ─────────────────────── Cancel registry (remote) ───────────────────────

static REMOTE_CANCEL: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));
pub static TURN_CANCEL: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

pub fn cancel_remote(gen_id: &str) {
    if let Ok(mut set) = REMOTE_CANCEL.lock() {
        set.insert(gen_id.to_string());
    }
}

pub fn is_remote_cancelled(gen_id: &str) -> bool {
    REMOTE_CANCEL
        .lock()
        .map(|s| s.contains(gen_id))
        .unwrap_or(false)
}

pub fn clear_remote_cancel(gen_id: &str) {
    if let Ok(mut set) = REMOTE_CANCEL.lock() {
        set.remove(gen_id);
    }
}

/// Derive the gen id used during the planning phase. Keeping this prefix
/// distinct from the answer-phase id lets `cancel` clear both reliably even
/// when the same conv id is reused.
pub fn plan_gen_id(conv_id: &str) -> String {
    if conv_id.is_empty() {
        String::new()
    } else {
        format!("plan:{}", conv_id)
    }
}
