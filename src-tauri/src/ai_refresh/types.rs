use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;

pub(in crate::ai_refresh) const AI_STATUS_CACHE_KEY: &str = "ai_scheduler_status";
pub(in crate::ai_refresh) const AI_NOTIF_CACHE_KEY: &str = "ai_notif_analysis";
pub(in crate::ai_refresh) const AI_REFRESH_CHECK_SECS: u64 = 5 * 60;
pub(in crate::ai_refresh) const AI_REFRESH_STARTUP_DELAY_SECS: u64 = 35;
pub(in crate::ai_refresh) const FAST_INPUT_MAX_AGE_SECS: i64 = 15 * 60;
pub(in crate::ai_refresh) const KWIC_INPUT_MAX_AGE_SECS: i64 = 12 * 60 * 60;
pub(in crate::ai_refresh) const SCHEDULE_INPUT_MAX_AGE_SECS: i64 = 6 * 60 * 60;
pub(in crate::ai_refresh) const STABLE_INPUT_MAX_AGE_SECS: i64 = 12 * 60 * 60;

#[derive(Default)]
pub struct AiRefreshState {
    pub(in crate::ai_refresh) running: AtomicBool,
}

impl AiRefreshState {
    pub fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
        }
    }
}

pub(in crate::ai_refresh) struct AiRefreshRequest {
    keys: Option<HashSet<String>>,
}

impl AiRefreshRequest {
    pub(in crate::ai_refresh) fn new(keys: Option<Vec<String>>) -> Self {
        let keys = keys.map(|items| {
            items
                .into_iter()
                .filter(|key| !key.trim().is_empty())
                .collect::<HashSet<_>>()
        });
        Self { keys }
    }

    pub(in crate::ai_refresh) fn wants(&self, key: &str) -> bool {
        self.keys
            .as_ref()
            .map(|keys| keys.contains(key))
            .unwrap_or(true)
    }

    pub(in crate::ai_refresh) fn is_all(&self) -> bool {
        self.keys.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiRefreshStatus {
    pub running: bool,
    pub last_run: Option<i64>,
    pub last_ok: Option<bool>,
    #[serde(default)]
    pub last_error: String,
    #[serde(default)]
    pub interval_minutes: u32,
    #[serde(default)]
    pub items: Vec<AiRefreshItemStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiRefreshItemStatus {
    pub key: String,
    pub label: String,
    pub status: String,
    #[serde(default)]
    pub error: String,
}
