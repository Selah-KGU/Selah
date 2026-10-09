use chrono::{DateTime, Local};

#[path = "cache/atomic.rs"]
mod atomic;
use atomic::{append_ndjson, atomic_write, atomic_write_with, WriteError};

mod types;
#[cfg(test)]
pub(super) use types::LiveLineDeltaOwned;
pub(super) use types::{LiveDayCache, LiveDayCacheRef, LiveLineDeltaBorrowed, LiveLineDeltaRef};
mod encoding;
mod recovery;
mod removal;
#[cfg(test)]
pub(in crate::live) use removal::{denied_fixture, remove_files as remove_day_cache_files};

use super::markdown::build_markdown;
use super::{
    format_datetime, sanitize_filename_component, LiveCourseInfo, LiveState, LiveTranscriptLine,
    SharedSummaryChunk, SharedTranscriptLine, FREE_NOTE_FOLDER_NAME,
};

#[cfg(test)]
#[path = "cache/stream_tests.rs"]
mod stream_tests;

pub(super) fn live_storage_dir(course: &LiveCourseInfo) -> std::path::PathBuf {
    if course.is_free_note {
        let dir = crate::commands::resolve_download_dir(None).join(FREE_NOTE_FOLDER_NAME);
        let _ = std::fs::create_dir_all(&dir);
        dir
    } else {
        crate::commands::resolve_download_dir(Some(&course.course_name))
    }
}

/// A release build and a `tauri dev` build can run at the same time during
/// development. They resolve the same download dir, so without a per-build tag
/// they would read/write the same day-cache + deltas files — each process
/// clobbering the other's transcript/summary state and making the periodic
/// summary-flush timer oscillate. Give the debug build its own files so the two
/// never share live state.
fn build_cache_tag() -> &'static str {
    if cfg!(debug_assertions) {
        ".dev"
    } else {
        ""
    }
}

fn day_cache_paths(
    course: &LiveCourseInfo,
) -> Option<(std::path::PathBuf, std::path::PathBuf, String)> {
    if course.is_free_note {
        return None;
    }
    let day = Local::now();
    let dir = live_storage_dir(course);
    let date = day.format("%Y%m%d").to_string();
    let name = sanitize_filename_component(&course.course_name);
    let tag = build_cache_tag();
    Some((
        dir.join(format!(".{date}_{name}_live{tag}.cache.json")),
        dir.join(format!(".{date}_{name}_live{tag}.lines.ndjson")),
        day.format("%Y-%m-%d").to_string(),
    ))
}

pub(super) fn load_day_cache(course: &LiveCourseInfo) -> Option<LiveDayCache> {
    let (path, deltas_path, today) = day_cache_paths(course)?;
    recovery::load(&path, &deltas_path, &today, &course.course_name)
}

/// Append entries from a deltas NDJSON blob into `cache.transcript_lines`.
/// - Entries with `i < cache.transcript_lines.len()` are skipped as stale
///   (already in the snapshot, e.g. after a crash between snapshot rewrite
///   and deltas truncation).
/// - A gap (`i > expected`) stops the replay so out-of-order entries can't
///   silently reorder transcripts.
#[cfg(test)]
pub(super) fn replay_deltas_into(cache: &mut LiveDayCache, deltas_text: &str) {
    recovery::replay(cache, std::io::Cursor::new(deltas_text)).unwrap();
}

/// Full snapshot rewrite. Also truncates the deltas log since the snapshot now
/// includes everything. Called on flush/finish, not per line.
pub(super) fn save_day_cache_full(
    course: &LiveCourseInfo,
    started_at: DateTime<Local>,
    transcript_lines: &[SharedTranscriptLine],
    summaries: &[SharedSummaryChunk],
) -> Result<(), String> {
    let Some((path, deltas, date)) = day_cache_paths(course) else {
        return Ok(());
    };
    let cache = LiveDayCacheRef {
        date,
        course_name: &course.course_name,
        started_at: format_datetime(started_at),
        transcript_lines,
        summaries,
    };
    persist_day_cache_to(&path, &deltas, &cache, 0, true)
}

fn persist_day_cache_to(
    path: &std::path::Path,
    deltas: &std::path::Path,
    cache: &LiveDayCacheRef<'_>,
    start: usize,
    full: bool,
) -> Result<(), String> {
    if full || !path.is_file() {
        atomic_write_with(path, |file| encoding::cache(file, cache)).map_err(|error| {
            write_error(
                error,
                "LIVEキャッシュの変換失敗",
                "LIVEキャッシュの保存失敗",
            )
        })?;
        // Stale indices are ignored during replay if removing the old journal
        // fails. The committed snapshot remains the authoritative base.
        if let Err(error) = std::fs::remove_file(deltas) {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!("[Live] old journal cleanup failed: {error}");
            }
        }
        return Ok(());
    }
    if start >= cache.transcript_lines.len() {
        return Ok(());
    }
    append_ndjson(
        deltas,
        |file| encoding::deltas(file, cache.transcript_lines, start),
        |tail| serde_json::from_slice::<LiveLineDeltaBorrowed<'_>>(tail).is_ok(),
    )
    .map_err(|error| write_error(error, "LIVE字幕の変換失敗", "LIVE字幕の保存失敗"))
}

fn write_error(error: WriteError<serde_json::Error>, conversion: &str, storage: &str) -> String {
    match error {
        WriteError::Content(error) if !error.is_io() => format!("{conversion}: {error}"),
        WriteError::Content(error) => format!("{storage}: {error}"),
        WriteError::Storage(error) => format!("{storage}: {error}"),
    }
}

pub(super) fn remove_day_cache(course: &LiveCourseInfo) -> Result<(), String> {
    if let Some((path, deltas, _)) = day_cache_paths(course) {
        removal::remove_files(&path, &deltas)?;
    }
    Ok(())
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
    transcript_lines: &[SharedTranscriptLine],
    summaries: &[SharedSummaryChunk],
) -> Result<(), String> {
    if transcript_lines.is_empty() {
        return Ok(());
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
    write_formal_markdown_file(course, started_at, &markdown).map(|_| ())
}

pub(super) fn write_formal_markdown_file(
    course: &LiveCourseInfo,
    started_at: DateTime<Local>,
    markdown: &str,
) -> Result<std::path::PathBuf, String> {
    let dir = live_storage_dir(course);
    std::fs::create_dir_all(&dir).map_err(|e| format!("保存先フォルダ作成失敗: {}", e))?;
    let path = dir.join(formal_markdown_filename(course, started_at));
    atomic_write(&path, markdown.as_bytes()).map_err(|e| format!("Markdown保存失敗: {}", e))?;
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
    Ok(path)
}

/// Queue a debounced save. Storage work never runs on the decoder thread.
pub(super) fn auto_save_day_cache(state: &LiveState, force: bool) {
    super::persistence::LivePersistence::schedule(state, force);
}

pub(super) fn persist_plan(plan: &super::persistence::SavePlan) -> Result<(), String> {
    if let Some((path, deltas, date)) = day_cache_paths(&plan.course) {
        let cache = LiveDayCacheRef {
            date,
            course_name: &plan.course.course_name,
            started_at: format_datetime(plan.started_at),
            transcript_lines: &plan.lines,
            summaries: &plan.summaries,
        };
        persist_day_cache_to(&path, &deltas, &cache, plan.start, plan.full)?;
    }
    if plan.markdown {
        write_partial_markdown_file(&plan.course, plan.started_at, &plan.lines, &plan.summaries)?;
    }
    Ok(())
}

#[cfg(test)]
mod storage_tests {
    use super::*;

    #[test]
    fn first_incremental_save_creates_a_recoverable_base_and_torn_tail_can_retry() {
        let dir = std::env::temp_dir().join(format!("selah-live-storage-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("day.cache.json");
        let deltas = dir.join("day.lines.ndjson");
        let mut lines = vec![LiveTranscriptLine {
            at: "10:00:00".into(),
            text: "first".into(),
        }
        .into()];
        let save = |lines: &[SharedTranscriptLine], start, full| {
            let cache = LiveDayCacheRef {
                date: "2026-10-07".into(),
                course_name: "test",
                started_at: "2026-10-07 10:00:00".into(),
                transcript_lines: lines,
                summaries: &[],
            };
            persist_day_cache_to(&path, &deltas, &cache, start, full).unwrap();
        };
        save(&lines, 0, false);
        let original = std::fs::read(&path).unwrap();
        assert!(!deltas.exists());
        let initial: LiveDayCache = serde_json::from_slice(&original).unwrap();
        assert_eq!(initial.transcript_lines[0].text, "first");
        lines.push(
            LiveTranscriptLine {
                at: "10:00:01".into(),
                text: "second".into(),
            }
            .into(),
        );
        save(&lines, 1, false);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        let mut damaged = std::fs::read(&deltas).unwrap();
        damaged.extend_from_slice(br#"{"i":2,"t":"torn"#);
        std::fs::write(&deltas, damaged).unwrap();
        lines.push(
            LiveTranscriptLine {
                at: "10:00:02".into(),
                text: "third".into(),
            }
            .into(),
        );
        save(&lines, 2, false);
        let mut recovered: LiveDayCache = serde_json::from_slice(&original).unwrap();
        replay_deltas_into(&mut recovered, &std::fs::read_to_string(&deltas).unwrap());
        assert_eq!(
            recovered
                .transcript_lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["first", "second", "third"]
        );
        save(&lines, 0, true);
        assert!(!deltas.exists());
        let folded: LiveDayCache = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(folded.transcript_lines.len(), 3);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
