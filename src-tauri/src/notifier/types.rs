//! Notification poll state and debug records.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

pub(in crate::notifier) const INITIAL_SYNC_DELAY: Duration = Duration::from_secs(10);
pub(in crate::notifier) const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);
pub(in crate::notifier) const KGC_NOTIFICATION_MAX_AGE_SECS: i64 = 12 * 60 * 60;
pub(in crate::notifier) const HIDDEN_SKIP_MAX: u32 = 6;
pub(in crate::notifier) const BOOTSTRAP_GRACE_PERIOD: Duration = Duration::from_secs(6 * 60);

pub struct NotificationPollState {
    pub(in crate::notifier) running: AtomicBool,
    pub(in crate::notifier) debug: Mutex<NotificationRuntimeDebugState>,
}

impl NotificationPollState {
    pub fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
            debug: Mutex::new(NotificationRuntimeDebugState::default()),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct NotificationEventDebugInfo {
    pub at_epoch: i64,
    pub source: String,
    pub status: String,
    pub title: String,
    pub body: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NotificationClickTarget {
    pub source: String,
    pub id: String,
    pub title: String,
    pub date: String,
    pub category: String,
    pub tab: Option<String>,
    pub url: Option<String>,
    pub course_info: Option<String>,
    pub information_type: Option<String>,
    pub person_category_cd: Option<String>,
    pub category_cd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct NotificationLastSyncDebugInfo {
    pub started_at_epoch: Option<i64>,
    pub finished_at_epoch: Option<i64>,
    pub status: String,
    pub error: String,
    pub bootstrap_mode: String,
    pub suppress_push: bool,
    pub dispatched: usize,
    pub failed: usize,
    pub suppressed: usize,
    pub muted: usize,
    pub seeded_sources: Vec<String>,
    pub fetch_failures: Vec<String>,
}

#[derive(Debug, Default)]
pub(in crate::notifier) struct NotificationRuntimeDebugState {
    pub(in crate::notifier) last_sync: NotificationLastSyncDebugInfo,
    pub(in crate::notifier) recent_events: Vec<NotificationEventDebugInfo>,
}

#[derive(Clone, Copy)]
pub(in crate::notifier) enum CourseNotificationKind {
    General,
    Announcement,
    Assignment,
    Exam,
    Discussion,
    Survey,
    Attendance,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::notifier) enum BootstrapMode {
    Silent,
    Finalize,
    Normal,
}

#[derive(Clone, Copy)]
pub(in crate::notifier) struct BootstrapState {
    pub(in crate::notifier) mode: BootstrapMode,
    pub(in crate::notifier) should_mark_complete: bool,
    pub(in crate::notifier) should_mark_started_at: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct NotificationSourceDebugInfo {
    pub source: String,
    pub authenticated: bool,
    pub initialized: bool,
    pub has_seen_state: bool,
    pub seen_count: usize,
}

#[derive(Debug, Serialize)]
pub struct NotificationDebugInfo {
    pub poll_running: bool,
    pub delivery_note: String,
    pub bootstrap_mode: String,
    pub suppress_push: bool,
    pub bootstrap_complete: bool,
    pub bootstrap_started_at_epoch: Option<i64>,
    pub bootstrap_started_ago_secs: Option<i64>,
    pub grace_period_secs: u64,
    pub authenticated_sources: Vec<String>,
    pub sources: Vec<NotificationSourceDebugInfo>,
    pub last_sync: NotificationLastSyncDebugInfo,
    pub recent_events: Vec<NotificationEventDebugInfo>,
}

#[derive(Default)]
pub(in crate::notifier) struct SyncRunDebug {
    pub(in crate::notifier) started_at_epoch: i64,
    pub(in crate::notifier) bootstrap_mode: String,
    pub(in crate::notifier) suppress_push: bool,
    pub(in crate::notifier) dispatched: usize,
    pub(in crate::notifier) failed: usize,
    pub(in crate::notifier) suppressed: usize,
    pub(in crate::notifier) muted: usize,
    pub(in crate::notifier) seeded_sources: Vec<String>,
    pub(in crate::notifier) fetch_failures: Vec<String>,
    pub(in crate::notifier) recent_events: Vec<NotificationEventDebugInfo>,
}
