// timetable.rs — AI-driven schedule: fetch KGC + Luna raw data, enrich, then AI analysis.

#[path = "timetable/ai_analysis.rs"]
mod ai_analysis;
#[path = "timetable/enrich.rs"]
mod enrich;
#[path = "timetable/kgc_week.rs"]
mod kgc_week;
#[path = "timetable/luna_sync.rs"]
mod luna_sync;
#[path = "timetable/snapshot.rs"]
mod snapshot;
#[path = "timetable/syllabus.rs"]
mod syllabus;
#[path = "timetable/sync.rs"]
mod sync;
#[path = "timetable/util.rs"]
mod util;

use self::ai_analysis::load_ai_cache;
pub use ai_analysis::*;

use util::{day_int_to_str, day_str_to_int, safe_preview};

pub(in crate::timetable) use enrich::enrich_schedule_inner;
pub(in crate::timetable) use kgc_week::fetch_next_week_kgc;
pub(in crate::timetable) use luna_sync::{
    clear_kgc_if_expired, is_luna_auth_error, retain_current_communities, sync_luna_timetable,
};
pub(in crate::timetable) use syllabus::batch_fetch_syllabi;
pub(in crate::timetable) use sync::{load_kgc_warning, STALE_SEMESTER_KGC_WARNING};

pub use enrich::*;
pub use snapshot::*;
pub use sync::*;
