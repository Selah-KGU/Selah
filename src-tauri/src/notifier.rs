//! Notification polling and native dispatch.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

use crate::background_refresh;
use crate::commands::{self, NotificationConfig};
use crate::db::Database;
use crate::kwic_commands::KwicPortalHome;
use crate::mail::MailMessage;
use crate::parser::NotificationsData;
use crate::read_state::LunaNotifSeenEntry;
use crate::{KgcState, KwicState, LunaState, MailState};

#[path = "notifier/seen.rs"]
mod seen;
#[path = "notifier/sources.rs"]
mod sources;
#[path = "notifier/sync.rs"]
mod sync;
#[path = "notifier/types.rs"]
mod types;

pub(in crate::notifier) use seen::*;
pub(in crate::notifier) use sources::*;
pub use sync::*;
#[cfg(test)]
pub(in crate::notifier) use sync::{
    cache_refresh_due, kgc_notification_max_age, notifications_json_is_empty,
};
pub(in crate::notifier) use types::{
    BootstrapMode, BootstrapState, CourseNotificationKind, SyncRunDebug, BOOTSTRAP_GRACE_PERIOD,
    HIDDEN_SKIP_MAX, INITIAL_SYNC_DELAY, KGC_NOTIFICATION_MAX_AGE_SECS, POLL_INTERVAL,
};
pub use types::{
    NotificationClickTarget, NotificationDebugInfo, NotificationEventDebugInfo,
    NotificationLastSyncDebugInfo, NotificationPollState, NotificationSourceDebugInfo,
};

#[cfg(test)]
#[path = "notifier/tests.rs"]
mod tests;
