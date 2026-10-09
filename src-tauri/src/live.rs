use chrono::{DateTime, Duration as ChronoDuration, Local};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};
use tokio::sync::Notify;

mod admission;
mod ai_output;
mod cache;
mod commands;
#[path = "live/context_text.rs"]
mod context_text;
mod generation;
mod markdown;
mod notification;
mod persistence;
mod prompts;
mod response;
mod surface;
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
    validate_live_ai_config,
};
#[cfg(test)]
pub(in crate::live) use time::transcript_line_datetime;
pub(in crate::live) use time::{
    effective_batch_started_at, format_datetime, format_time, last_transcript_line_datetime,
    latest_summary_end_datetime,
};
pub(in crate::live) use whiteboard::{enrich_whiteboard_source_excerpts, WhiteboardContext};

pub(in crate::live) use context_text::{build_context_text, ContextPart};
#[cfg(test)]
use whiteboard::{
    format_current_chunk_for_whiteboard, format_full_history_for_whiteboard,
    format_recent_summary_context,
};

use self::cache::{
    auto_save_day_cache, formal_markdown_filename, live_storage_dir, load_day_cache,
    remove_day_cache, save_day_cache_full, write_formal_markdown_file,
};
use self::markdown::build_markdown;
pub(crate) use admission::admission_handler;
#[cfg(test)]
use ai_output::latest_whiteboard;
use ai_output::{
    clamp_chars, extract_json_object, format_latest_whiteboard_context, latest_shared_whiteboard,
    parse_chunk_ai_result, reconcile_whiteboard, value_to_trimmed_string,
};
#[cfg(test)]
use cache::{
    replay_deltas_into, LiveDayCache, LiveDayCacheRef, LiveLineDeltaOwned, LiveLineDeltaRef,
};
pub(crate) use commands::append_recognized_transcript;
pub use commands::*;
pub(in crate::live) use generation::*;
use persistence::{CacheProgress, LivePersistence};
pub(in crate::live) use prompts::*;
pub(crate) use types::LiveSessionStatus;
use types::{LiveChunkAiResult, SharedLiveChunkAiResult};
pub use types::{
    LiveCourseInfo, LiveFinishPhase, LiveFinishProgress, LiveSaveResult, LiveSessionSnapshot,
    LiveSummaryChunk, LiveSummaryChunks, LiveTermExplanation, LiveTodoSuggestion,
    LiveTodoSuggestionsEvent, LiveTranscriptLine, LiveTranscriptLines, LiveWhiteboard,
    LiveWhiteboardEdge, LiveWhiteboardNode, SharedSummaryChunk, SharedTranscriptLine,
    SharedWhiteboard,
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

#[derive(Clone)]
pub struct LiveState {
    session: Arc<Mutex<Option<LiveSession>>>,
    flush_changed: Arc<Notify>,
    persistence: Arc<LivePersistence>,
    snapshot_revision: Arc<AtomicU64>,
}

#[derive(Debug, Clone)]
struct LiveSession {
    session_id: String,
    course: LiveCourseInfo,
    started_at: DateTime<Local>,
    transcript_lines: LiveTranscriptLines,
    pending_lines: LiveTranscriptLines,
    summaries: LiveSummaryChunks,
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
    cache_progress: CacheProgress,
    finish_phase: Option<LiveFinishPhase>,
    finish_revision: u64,
}

impl LiveSession {
    fn append_summary(&mut self, summary: LiveSummaryChunk) {
        // Move its full body, terms and board once. Retained snapshots cause
        // only the reference index to be copied, never prior chunk contents.
        Arc::make_mut(&mut self.summaries).push(Arc::new(summary));
    }

    fn append_line(&mut self, line: LiveTranscriptLine) -> types::LiveTranscriptUpdate {
        // Move the accepted buffers once; retained snapshots copy only owners.
        let line = Arc::new(line);
        Arc::make_mut(&mut self.transcript_lines).push(line.clone());
        Arc::make_mut(&mut self.pending_lines).push(line.clone());
        types::LiveTranscriptUpdate {
            session_id: self.session_id.clone(),
            line_count: self.transcript_lines.len(),
            line,
            seq: None,
        }
    }

    fn next_summary_at_ms(&self) -> Option<i64> {
        // Next scheduled periodic summary: the interval measured from the
        // effective batch start (last covered line / batch start). It's an
        // estimate — the actual flush also waits for ≥3 pending lines — but
        // good enough to drive a countdown.
        self.finish_phase.is_none().then(|| {
            (effective_batch_started_at(self)
                + ChronoDuration::minutes(live_summary_interval_minutes()))
            .timestamp_millis()
        })
    }

    fn snapshot(&self) -> LiveSessionSnapshot {
        // All three Vec<...> are Arc-wrapped, so cloning is a refcount bump.
        LiveSessionSnapshot {
            update_revision: 0,
            session_id: Some(self.session_id.clone()),
            active: true,
            course: Some(self.course.clone()),
            started_at: Some(format_datetime(self.started_at)),
            transcript_lines: Arc::clone(&self.transcript_lines),
            pending_lines: Arc::clone(&self.pending_lines),
            summaries: Arc::clone(&self.summaries),
            next_summary_at_ms: self.next_summary_at_ms(),
            summarizing: self.flush_in_flight,
            finish_phase: self.finish_phase,
            finish_revision: self.finish_revision,
        }
    }

    fn completed_snapshot(&self) -> LiveSessionSnapshot {
        let mut snapshot = self.snapshot();
        snapshot.active = false;
        snapshot.finish_phase = None;
        snapshot.finish_revision += 1;
        snapshot.summarizing = false;
        snapshot.next_summary_at_ms = None;
        snapshot
    }
}

impl LiveState {
    /// Called on a blocking worker after microphone decoding has completed.
    pub(crate) fn persist_before_exit(&self) -> Result<(), String> {
        let Some(id) = self.active_session_id() else {
            return Ok(());
        };
        LivePersistence::persist_for_exit(self, &id, cache::persist_plan).map(|_| ())
    }
    /// Keep ownership stable through a short admission decision. Inspectors
    /// must not perform UI operations or IO while holding this LIVE lock.
    pub(crate) fn with_active_session_id<T>(
        &self,
        inspect: impl FnOnce(Option<&str>) -> T,
    ) -> Option<T> {
        let guard = self.session.lock().ok()?;
        Some(inspect(
            guard.as_ref().map(|session| session.session_id.as_str()),
        ))
    }

    // Call only while holding `session`: revision allocation and the captured
    // state must be ordered together, including full RPC reads and inactive UI.
    fn capture_snapshot(&self, session: Option<&LiveSession>) -> LiveSessionSnapshot {
        let mut snapshot = session
            .map(LiveSession::snapshot)
            .unwrap_or_else(empty_snapshot);
        snapshot.update_revision = self.next_snapshot_revision();
        snapshot
    }

    fn capture_completed_snapshot(&self, session: &LiveSession) -> LiveSessionSnapshot {
        let mut snapshot = session.completed_snapshot();
        snapshot.update_revision = self.next_snapshot_revision();
        snapshot
    }

    fn next_snapshot_revision(&self) -> u64 {
        self.snapshot_revision.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub(crate) fn active_session_id(&self) -> Option<String> {
        self.session
            .lock()
            .ok()?
            .as_ref()
            .map(|session| session.session_id.clone())
    }

    pub(crate) fn is_session_current(&self, id: &str) -> bool {
        self.session.lock().ok().is_some_and(|guard| {
            guard
                .as_ref()
                .is_some_and(|session| session.session_id == id)
        })
    }

    fn append_line_for_session(
        &self,
        expected: Option<&str>,
        line: LiveTranscriptLine,
    ) -> Result<Option<types::LiveTranscriptUpdate>, String> {
        let mut guard = self
            .session
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        if expected.is_some_and(|id| {
            guard
                .as_ref()
                .is_none_or(|session| session.session_id != id)
        }) {
            return Ok(None);
        }
        let session = guard
            .as_mut()
            .ok_or_else(|| "Liveセッションが開始されていません".to_string())?;
        if expected.is_none() && session.finish_phase.is_some() {
            return Err("Liveセッションを保存中です".into());
        }
        Ok(Some(session.append_line(line)))
    }

    pub(crate) fn with_microphone_owner<T>(
        &self,
        expected: &str,
        reserve: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let guard = self
            .session
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        let session = guard
            .as_ref()
            .filter(|session| session.session_id == expected)
            .ok_or_else(|| "Liveセッションが切り替わりました".to_string())?;
        if session.finish_phase.is_some() {
            return Err("Liveセッションを保存中です".into());
        }
        // Keep the LIVE lock through the short STT reservation/stop request,
        // never through model loading or teardown waiting. Cancellation and
        // replacement cannot slip between validation and the microphone action.
        reserve()
    }

    fn begin_finish(&self, expected: &str) -> Result<LiveFinishGuard, String> {
        let mut guard = self
            .session
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        let session = guard
            .as_mut()
            .filter(|session| session.session_id == expected)
            .ok_or_else(|| "Liveセッションが切り替わりました".to_string())?;
        if session.finish_phase.is_some() {
            return Err("Liveセッションを保存中です".into());
        }
        session.finish_phase = Some(LiveFinishPhase::Stopping);
        session.finish_revision += 1;
        let session_id = session.session_id.clone();
        drop(guard);
        self.notify_flush_driver();
        Ok(LiveFinishGuard {
            state: self.clone(),
            session_id,
        })
    }

    fn set_finish_phase(
        &self,
        expected: &str,
        phase: LiveFinishPhase,
    ) -> Result<LiveFinishProgress, String> {
        let mut guard = self
            .session
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        let session = guard
            .as_mut()
            .filter(|session| session.session_id == expected && session.finish_phase.is_some())
            .ok_or_else(|| "Live終了処理の所有権がありません".to_string())?;
        session.finish_phase = Some(phase);
        session.finish_revision += 1;
        Ok(LiveFinishProgress {
            session_id: session.session_id.clone(),
            finish_phase: phase,
            finish_revision: session.finish_revision,
        })
    }

    pub(crate) fn has_active_session(&self) -> bool {
        self.session
            .try_lock()
            .map(|session| session.is_some())
            .unwrap_or(false)
    }

    pub fn new() -> Self {
        Self {
            session: Arc::new(Mutex::new(None)),
            flush_changed: Arc::new(Notify::new()),
            persistence: Arc::new(LivePersistence::default()),
            snapshot_revision: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn notify_flush_driver(&self) {
        self.flush_changed.notify_waiters();
    }

    fn flush_notify(&self) -> Arc<Notify> {
        Arc::clone(&self.flush_changed)
    }
}

struct LiveFinishGuard {
    state: LiveState,
    session_id: String,
}

impl Drop for LiveFinishGuard {
    fn drop(&mut self) {
        let mut guard = self.state.session.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(session) = guard
            .as_mut()
            .filter(|session| session.session_id == self.session_id)
        {
            session.finish_phase = None;
            session.finish_revision += 1;
        }
        drop(guard);
        self.state.notify_flush_driver();
    }
}

#[cfg(test)]
mod tests;
