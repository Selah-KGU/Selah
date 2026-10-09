use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

use crate::luna_parser;

/// SQLite database — raw data warehouse for KGC / Luna + AI schedule cache.
///
/// Tables:
///   kgc_courses     – raw KGC timetable entries (per week_label)
///   luna_courses     – raw Luna timetable entries
///   session_plans    – parsed 授業計画 keyed by kgc_code
///   luna_counts      – Luna LMS activity counts keyed by luna_id
///   ai_schedule_cache – cached AI-generated schedule JSON
#[derive(Clone)]
pub struct Database {
    conn: account::AccountConnection,
}
#[path = "db/account.rs"]
mod account;
pub(crate) use account::AccountDb;
pub(crate) use account::{account_work, capture_account, AccountContext};

#[path = "db/agent_store.rs"]
mod agent_store;
#[path = "db/cache.rs"]
mod cache;
#[path = "db/cache_timestamps.rs"]
mod cache_timestamps;
#[path = "db/kgc.rs"]
mod kgc;
#[path = "db/kgc_detail.rs"]
mod kgc_detail;
#[path = "db/luna.rs"]
mod luna;
#[path = "db/luna_activities.rs"]
mod luna_activities;
#[path = "db/luna_counts.rs"]
mod luna_counts;
#[path = "db/luna_scope.rs"]
mod luna_scope;
#[path = "db/revisions.rs"]
mod revisions;
#[path = "db/schedule.rs"]
mod schedule;
#[path = "db/schema.rs"]
mod schema;
#[path = "db/scoped_rows.rs"]
mod scoped_rows;
#[path = "db/session_plans.rs"]
mod session_plans;
#[path = "db/types.rs"]
mod types;

pub(crate) use cache::source_hash;
pub use cache::CacheDeltaRow;
pub use cache_timestamps::CacheTimestampBatch;
pub use luna_scope::epoch_secs;
pub(crate) use luna_scope::luna_course_matches_snapshot;
pub use types::*;
