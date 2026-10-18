// Luna owns the timetable; KGC and AI provide optional enrichment.

#[path = "timetable/ai_analysis.rs"]
mod ai_analysis;
#[path = "timetable/enrich.rs"]
mod enrich;
#[path = "timetable/kgc_sync.rs"]
mod kgc_sync;
#[path = "timetable/kgc_week.rs"]
mod kgc_week;
#[path = "timetable/luna_sync.rs"]
mod luna_sync;
#[path = "timetable/primary.rs"]
mod primary;
#[path = "timetable/snapshot.rs"]
mod snapshot;
#[path = "timetable/syllabus.rs"]
mod syllabus;
#[path = "timetable/sync.rs"]
mod sync;
#[path = "timetable/util.rs"]
mod util;

pub use ai_analysis::*;

use util::{day_int_to_str, day_str_to_int, safe_preview};

pub(in crate::timetable) use enrich::enrich_schedule_inner;
pub(crate) use kgc_sync::spawn_kgc_supplement;
pub(in crate::timetable) use kgc_week::fetch_next_week_kgc;
pub(in crate::timetable) use luna_sync::{
    clear_kgc_if_expired, is_luna_auth_error, retain_current_communities, sync_luna_timetable,
};
pub(crate) use primary::primary_courses;
pub(in crate::timetable) use syllabus::batch_fetch_syllabi;

pub(crate) use ai_analysis::AI_CACHE_MAX_AGE;
pub(crate) use enrich::spawn_luna_counts;
pub use enrich::*;
pub use snapshot::*;
pub use sync::*;
