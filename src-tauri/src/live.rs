use chrono::{DateTime, Duration as ChronoDuration, Local};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};
use tokio::sync::Notify;

mod ai_output;
mod cache;
mod commands;
mod generation;
mod markdown;
mod prompts;
mod types;

#[path = "live/flush.rs"]
mod flush;
#[path = "live/support.rs"]
mod support;
#[path = "live/time.rs"]
mod time;
#[path = "live/whiteboard.rs"]
mod whiteboard;

pub(in crate::live) use flush::{
    flush_final_summary_with_retry, flush_session_summary, live_flush_summary_with_side_effects,
    start_live_flush_driver,
};
pub(in crate::live) use support::{
    current_snapshot, emit_live_finish_progress, emit_live_update, empty_snapshot, live_ai_config,
    live_summary_interval_minutes, sanitize_filename_component, sanitize_model_output,
    should_require_finish_chunk_ai, should_run_finish_ai, should_skip_ai_summarization,
};
#[cfg(test)]
pub(in crate::live) use time::transcript_line_datetime;
pub(in crate::live) use time::{
    effective_batch_started_at, format_datetime, format_time, last_transcript_line_datetime,
    latest_summary_end_datetime,
};
pub(in crate::live) use whiteboard::{
    enrich_whiteboard_source_excerpts, format_current_chunk_for_whiteboard,
    format_full_history_for_whiteboard, format_recent_summary_context,
};

use self::cache::{
    auto_save_day_cache, formal_markdown_filename, live_storage_dir, load_day_cache,
    remove_day_cache, save_day_cache_full, write_formal_markdown_file, write_partial_markdown_file,
};
use self::markdown::build_markdown;
use ai_output::{
    clamp_chars, extract_json_object, format_latest_whiteboard_context, latest_whiteboard,
    parse_chunk_ai_result, reconcile_whiteboard, value_to_trimmed_string,
};
#[cfg(test)]
use cache::{
    replay_deltas_into, LiveDayCache, LiveDayCacheRef, LiveLineDeltaOwned, LiveLineDeltaRef,
};
pub use commands::*;
pub(in crate::live) use generation::*;
pub(in crate::live) use prompts::*;
use types::LiveChunkAiResult;
pub use types::{
    LiveCourseInfo, LiveSaveResult, LiveSessionSnapshot, LiveSummaryChunk, LiveTermExplanation,
    LiveTodoSuggestion, LiveTodoSuggestionsEvent, LiveTranscriptLine, LiveWhiteboard,
    LiveWhiteboardEdge, LiveWhiteboardNode,
};

const MIN_AI_SUMMARIZATION_DURATION_SECS: i64 = 120;
const MAX_LIVE_TERM_EXPLANATION_CHARS: usize = 220;
const LIVE_FLUSH_FORCE_WAIT_ATTEMPTS: usize = 1200;
const LIVE_FLUSH_FORCE_WAIT_MS: u64 = 250;
// Backend driver wake-up cadence only. The actual generation interval is the
// user setting measured from `batch_started_at`, not these polling caps.
const LIVE_FLUSH_DRIVER_MAX_SLEEP_SECS: u64 = 30;
const LIVE_FLUSH_DRIVER_IDLE_SLEEP_SECS: u64 = 30;
const LIVE_FLUSH_DRIVER_MIN_SLEEP_SECS: u64 = 1;
// Whiteboard nodes/edges are intentionally uncapped: the board must accumulate
// the full course/recording as it grows, so a hard ceiling silently forces the
// model to compress earlier branches. Per-field length and the relationship
// guards in `parse_live_whiteboard` are the remaining safety nets.
const FREE_NOTE_FOLDER_NAME: &str = "自由ノート";
const PRE_AI_OVERALL_SUMMARY: &str =
    "### 全体要約\n_(AI生成前の一時保存です。生成完了後に更新します)_";

pub struct LiveState(Mutex<Option<LiveSession>>, Arc<Notify>);

#[derive(Debug, Clone)]
struct LiveSession {
    session_id: String,
    course: LiveCourseInfo,
    started_at: DateTime<Local>,
    transcript_lines: Arc<Vec<LiveTranscriptLine>>,
    pending_lines: Arc<Vec<LiveTranscriptLine>>,
    summaries: Arc<Vec<LiveSummaryChunk>>,
    /// Timestamp of the last finalized subtitle line covered by a successful
    /// summary chunk. Initialized to session start for the first chunk.
    batch_started_at: DateTime<Local>,
    flush_in_flight: bool,
    /// True when this session began with no prior cache for today —
    /// i.e. it owns the on-disk .md/day_cache and cancel may scrub them.
    /// False when resumed from an earlier session today; cancel must leave
    /// the prior content intact.
    is_fresh_start: bool,
    /// How many entries of `transcript_lines` have already been persisted
    /// (either in the main cache snapshot or appended to the deltas log).
    /// Drives the incremental day-cache write.
    persisted_line_count: usize,
}

impl LiveSession {
    fn snapshot(&self) -> LiveSessionSnapshot {
        // Next scheduled periodic summary: the interval measured from the
        // effective batch start (last covered line / batch start). It's an
        // estimate — the actual flush also waits for ≥3 pending lines — but
        // good enough to drive a countdown.
        let next_summary_at_ms = Some(
            (effective_batch_started_at(self)
                + ChronoDuration::minutes(live_summary_interval_minutes()))
            .timestamp_millis(),
        );
        // All three Vec<...> are Arc-wrapped, so cloning is a refcount bump.
        LiveSessionSnapshot {
            active: true,
            course: Some(self.course.clone()),
            started_at: Some(format_datetime(self.started_at)),
            transcript_lines: Arc::clone(&self.transcript_lines),
            pending_lines: Arc::clone(&self.pending_lines),
            summaries: Arc::clone(&self.summaries),
            next_summary_at_ms,
            summarizing: self.flush_in_flight,
        }
    }
}

impl LiveState {
    pub fn new() -> Self {
        Self(Mutex::new(None), Arc::new(Notify::new()))
    }

    pub fn notify_flush_driver(&self) {
        self.1.notify_waiters();
    }

    fn flush_notify(&self) -> Arc<Notify> {
        Arc::clone(&self.1)
    }
}

#[cfg(test)]
mod tests;
