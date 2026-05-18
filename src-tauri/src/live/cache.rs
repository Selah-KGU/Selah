use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::markdown::build_markdown;
use super::{
    format_datetime, sanitize_filename_component, LiveCourseInfo, LiveState, LiveSummaryChunk,
    LiveTranscriptLine, FREE_NOTE_FOLDER_NAME,
};

const CACHE_DEBOUNCE: Duration = Duration::from_secs(30);
static LAST_CACHE_WRITE: AtomicU64 = AtomicU64::new(0);

fn instant_now_ms() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let origin = *START.get_or_init(Instant::now);
    Instant::now().saturating_duration_since(origin).as_millis() as u64
}

/// Sidecar JSON that persists accumulated session data across stop/start within the same course day.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct LiveDayCache {
    pub(super) date: String, // YYYY-MM-DD
    pub(super) course_name: String,
    pub(super) started_at: String,
    pub(super) transcript_lines: Vec<LiveTranscriptLine>,
    pub(super) summaries: Vec<LiveSummaryChunk>,
}

pub(super) fn live_storage_dir(course: &LiveCourseInfo) -> std::path::PathBuf {
    if course.is_free_note {
        let dir = crate::commands::resolve_download_dir(None).join(FREE_NOTE_FOLDER_NAME);
        let _ = std::fs::create_dir_all(&dir);
        dir
    } else {
        crate::commands::resolve_download_dir(Some(&course.course_name))
    }
}

/// Single transcript line appended to the deltas log. Field names are short
/// (`i`/`t`/`a`) because we write one of these per spoken line — saves bytes
/// over a session.
#[derive(Debug, Serialize)]
pub(super) struct LiveLineDeltaRef<'a> {
    pub(super) i: usize,
    pub(super) t: &'a str,
    pub(super) a: &'a str,
}

#[derive(Debug, Deserialize)]
pub(super) struct LiveLineDeltaOwned {
    pub(super) i: usize,
    pub(super) t: String,
    pub(super) a: String,
}

/// Borrowing view of `LiveDayCache` used only for serialization, so we don't
/// have to deep-clone the transcript Vec every rewrite.
#[derive(Debug, Serialize)]
pub(super) struct LiveDayCacheRef<'a> {
    pub(super) date: String,
    pub(super) course_name: &'a str,
    pub(super) started_at: String,
    pub(super) transcript_lines: &'a [LiveTranscriptLine],
    pub(super) summaries: &'a [LiveSummaryChunk],
}

fn day_cache_path(course: &LiveCourseInfo) -> Option<std::path::PathBuf> {
    if course.is_free_note {
        return None;
    }
    let dir = live_storage_dir(course);
    let date_str = Local::now().format("%Y%m%d").to_string();
    let safe_name = sanitize_filename_component(&course.course_name);
    Some(dir.join(format!(".{}_{}_live.cache.json", date_str, safe_name)))
}

/// Append-only NDJSON log of transcript lines not yet folded into the main
/// snapshot. Lets us avoid rewriting the full cache every 30s.
fn day_cache_deltas_path(course: &LiveCourseInfo) -> Option<std::path::PathBuf> {
    if course.is_free_note {
        return None;
    }
    let dir = live_storage_dir(course);
    let date_str = Local::now().format("%Y%m%d").to_string();
    let safe_name = sanitize_filename_component(&course.course_name);
    Some(dir.join(format!(".{}_{}_live.lines.ndjson", date_str, safe_name)))
}

pub(super) fn load_day_cache(course: &LiveCourseInfo) -> Option<LiveDayCache> {
    let path = day_cache_path(course)?;
    let data = std::fs::read_to_string(&path).ok()?;
    let mut cache: LiveDayCache = serde_json::from_str(&data).ok()?;
    let today = Local::now().format("%Y-%m-%d").to_string();
    if cache.date != today || cache.course_name != course.course_name {
        // stale cache from a different day; nuke both sides.
        let _ = std::fs::remove_file(&path);
        if let Some(d) = day_cache_deltas_path(course) {
            let _ = std::fs::remove_file(d);
        }
        return None;
    }
    // Replay any deltas not yet folded into the snapshot. A crash between cache
    // rewrite and deltas truncation can leave stale entries with `i` less than
    // the snapshot's line count — we filter those out.
    if let Some(deltas_path) = day_cache_deltas_path(course) {
        if let Ok(deltas_data) = std::fs::read_to_string(&deltas_path) {
            replay_deltas_into(&mut cache, &deltas_data);
        }
    }
    Some(cache)
}

/// Append entries from a deltas NDJSON blob into `cache.transcript_lines`.
/// - Entries with `i < cache.transcript_lines.len()` are skipped as stale
///   (already in the snapshot, e.g. after a crash between snapshot rewrite
///   and deltas truncation).
/// - A gap (`i > expected`) stops the replay so out-of-order entries can't
///   silently reorder transcripts.
pub(super) fn replay_deltas_into(cache: &mut LiveDayCache, deltas_text: &str) {
    for raw in deltas_text.lines() {
        if raw.trim().is_empty() {
            continue;
        }
        let Ok(delta) = serde_json::from_str::<LiveLineDeltaOwned>(raw) else {
            continue;
        };
        let expected = cache.transcript_lines.len();
        if delta.i < expected {
            continue;
        }
        if delta.i != expected {
            break;
        }
        cache.transcript_lines.push(LiveTranscriptLine {
            text: delta.t,
            at: delta.a,
        });
    }
}

/// Full snapshot rewrite. Also truncates the deltas log since the snapshot now
/// includes everything. Called on flush/finish, not per line.
pub(super) fn save_day_cache_full(
    course: &LiveCourseInfo,
    started_at: DateTime<Local>,
    transcript_lines: &[LiveTranscriptLine],
    summaries: &[LiveSummaryChunk],
) {
    let cache_ref = LiveDayCacheRef {
        date: Local::now().format("%Y-%m-%d").to_string(),
        course_name: &course.course_name,
        started_at: format_datetime(started_at),
        transcript_lines,
        summaries,
    };
    let Some(path) = day_cache_path(course) else {
        return;
    };
    let Ok(json) = serde_json::to_string(&cache_ref) else {
        return;
    };
    if std::fs::write(&path, json).is_ok() {
        if let Some(deltas) = day_cache_deltas_path(course) {
            let _ = std::fs::remove_file(deltas);
        }
    }
}

/// Append new transcript lines `[start..]` to the deltas log. Cheap incremental
/// write — typically a few hundred bytes vs the tens-to-hundreds of KB a full
/// snapshot rewrite would cost.
fn append_day_cache_deltas(
    course: &LiveCourseInfo,
    transcript_lines: &[LiveTranscriptLine],
    start: usize,
) {
    if start >= transcript_lines.len() {
        return;
    }
    let Some(path) = day_cache_deltas_path(course) else {
        return;
    };
    use std::io::Write;
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let mut buf = String::with_capacity((transcript_lines.len() - start) * 64);
    for (offset, line) in transcript_lines[start..].iter().enumerate() {
        let delta = LiveLineDeltaRef {
            i: start + offset,
            t: &line.text,
            a: &line.at,
        };
        if let Ok(json) = serde_json::to_string(&delta) {
            buf.push_str(&json);
            buf.push('\n');
        }
    }
    let _ = file.write_all(buf.as_bytes());
}

pub(super) fn remove_day_cache(course: &LiveCourseInfo) {
    if let Some(path) = day_cache_path(course) {
        let _ = std::fs::remove_file(path);
    }
    if let Some(deltas) = day_cache_deltas_path(course) {
        let _ = std::fs::remove_file(deltas);
    }
}

/// Stable filename for a session's formal markdown. Anchored to `started_at` so a
/// mid-session save and the final save land at the same path — finish overwrites
/// the partial file rather than leaving an orphan.
pub(super) fn formal_markdown_filename(
    course: &LiveCourseInfo,
    started_at: DateTime<Local>,
) -> String {
    if course.is_free_note {
        format!(
            "{}_{}_live.md",
            started_at.format("%Y%m%d"),
            started_at.format("%H%M%S")
        )
    } else {
        format!(
            "{}_{}_live.md",
            started_at.format("%Y%m%d"),
            sanitize_filename_component(&course.course_name)
        )
    }
}

/// Write a partial formal markdown file mid-session so a crash before stop still
/// leaves recoverable content on disk. The overall summary is a placeholder —
/// `live_finish_session` overwrites with the AI-generated overall summary at stop.
pub(super) fn write_partial_markdown_file(
    course: &LiveCourseInfo,
    started_at: DateTime<Local>,
    transcript_lines: &[LiveTranscriptLine],
    summaries: &[LiveSummaryChunk],
) {
    if transcript_lines.is_empty() {
        return;
    }
    let overall_summary = "### 全体要約\n_(セッション継続中…保存時に確定します)_".to_string();
    let markdown = build_markdown(
        course,
        started_at,
        Local::now(),
        &overall_summary,
        summaries,
        transcript_lines,
    );
    let dir = live_storage_dir(course);
    let path = dir.join(formal_markdown_filename(course, started_at));
    if std::fs::write(&path, markdown.as_bytes()).is_err() {
        return;
    }
    let path_str = path.to_string_lossy().to_string();
    let file_name = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("live.md");
    // record_download dedupes by path, so repeated mid-session calls just update
    // the size/timestamp of a single download entry rather than spawning dupes.
    crate::commands::record_download(
        file_name,
        &path_str,
        Some(&course.course_name),
        "live",
        markdown.len() as u64,
    );
}

/// Auto-save session state to day cache (non-fatal on error).
/// - `force=false` (per-line trigger, debounced): append only newly-added lines
///   to the deltas log. Tiny write, no full re-serialization.
/// - `force=true` (per-flush / finish trigger): full snapshot rewrite and
///   truncate the deltas log. Catches the latest summaries too.
pub(super) fn auto_save_day_cache(state: &LiveState, force: bool) {
    if !force {
        let now = instant_now_ms();
        let last = LAST_CACHE_WRITE.load(Ordering::Relaxed);
        if last > 0 && now.saturating_sub(last) < CACHE_DEBOUNCE.as_millis() as u64 {
            return;
        }
    }
    let Ok(mut guard) = state.0.lock() else {
        return;
    };
    let Some(session) = guard.as_mut() else {
        return;
    };
    if session.course.is_free_note {
        return;
    }

    if force {
        save_day_cache_full(
            &session.course,
            session.started_at,
            &session.transcript_lines,
            &session.summaries,
        );
        session.persisted_line_count = session.transcript_lines.len();
    } else {
        let total = session.transcript_lines.len();
        let start = session.persisted_line_count;
        if start >= total {
            return;
        }
        append_day_cache_deltas(&session.course, &session.transcript_lines, start);
        session.persisted_line_count = total;
    }
    LAST_CACHE_WRITE.store(instant_now_ms(), Ordering::Relaxed);
}
