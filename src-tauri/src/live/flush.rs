use std::sync::Arc;

use chrono::{Duration as ChronoDuration, Local};
use tauri::{Emitter, Manager};

use super::support::{
    emit_live_update, empty_snapshot, live_summary_interval_minutes, should_skip_ai_summarization,
};
use super::time::{effective_batch_started_at, format_time, last_transcript_line_datetime};
use super::{
    auto_save_day_cache, build_chunk_title, latest_whiteboard, reconcile_whiteboard,
    summarize_chunk, write_partial_markdown_file, LiveSessionSnapshot, LiveState, LiveSummaryChunk,
    LIVE_FLUSH_DRIVER_IDLE_SLEEP_SECS, LIVE_FLUSH_DRIVER_MAX_SLEEP_SECS,
    LIVE_FLUSH_DRIVER_MIN_SLEEP_SECS, LIVE_FLUSH_FORCE_WAIT_ATTEMPTS, LIVE_FLUSH_FORCE_WAIT_MS,
    MIN_AI_SUMMARIZATION_DURATION_SECS,
};

pub(in crate::live) async fn flush_session_summary(
    state: &LiveState,
    force: bool,
) -> Result<LiveSessionSnapshot, String> {
    let mut wait_attempts = 0usize;
    let summary_interval_minutes = live_summary_interval_minutes();
    let (session_id, course, lines, recent_summaries, range_start, range_end, chunk_index) = loop {
        let captured = {
            let now = Local::now();
            let mut guard = state
                .0
                .lock()
                .map_err(|_| "Live state lock failed".to_string())?;
            let session = guard
                .as_mut()
                .ok_or_else(|| "Liveセッションが開始されていません".to_string())?;
            if session.flush_in_flight {
                let snapshot = session.snapshot();
                if !force || wait_attempts >= LIVE_FLUSH_FORCE_WAIT_ATTEMPTS {
                    return Ok(snapshot);
                }
                None
            } else {
                if session.pending_lines.is_empty() {
                    return Ok(session.snapshot());
                }
                if should_skip_ai_summarization(session.started_at, now) {
                    return Ok(session.snapshot());
                }
                let batch_started_at = effective_batch_started_at(session);
                if !force
                    && now.signed_duration_since(batch_started_at).num_minutes()
                        < summary_interval_minutes
                {
                    return Ok(session.snapshot());
                }
                // Scheduled summaries follow the original noise guard: wait
                // until at least a few finalized STT segments accumulated.
                // Forced flushes on stop still include any remaining content.
                if !force && session.pending_lines.len() < 3 {
                    return Ok(session.snapshot());
                }
                let lines = session.pending_lines.clone();
                let range_end =
                    last_transcript_line_datetime(session.started_at, lines.as_ref(), now);
                session.flush_in_flight = true;
                Some((
                    session.session_id.clone(),
                    session.course.clone(),
                    lines,
                    session.summaries.clone(),
                    batch_started_at,
                    range_end,
                    session.summaries.len() + 1,
                ))
            }
        };
        if let Some(captured) = captured {
            break captured;
        }
        wait_attempts += 1;
        tokio::time::sleep(std::time::Duration::from_millis(LIVE_FLUSH_FORCE_WAIT_MS)).await;
    };

    let range_label = format!("{}-{}", format_time(range_start), format_time(range_end));
    let chunk_ai_result = summarize_chunk(&course, &lines, &recent_summaries, &range_label).await;
    let summarized_line_count = lines.len();
    let chunk_ai = match chunk_ai_result {
        Ok(chunk_ai) => chunk_ai,
        Err(err) => {
            let mut guard = state
                .0
                .lock()
                .map_err(|_| "Live state lock failed".to_string())?;
            if let Some(session) = guard.as_mut() {
                if session.session_id == session_id {
                    session.flush_in_flight = false;
                }
            }
            return Err(err);
        }
    };
    {
        let mut guard = state
            .0
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        let Some(session) = guard.as_mut() else {
            return Ok(empty_snapshot());
        };
        if session.session_id != session_id {
            return Ok(session.snapshot());
        }
    }
    let reconciled_board =
        reconcile_whiteboard(latest_whiteboard(&recent_summaries), chunk_ai.whiteboard);

    let mut guard = state
        .0
        .lock()
        .map_err(|_| "Live state lock failed".to_string())?;
    let Some(session) = guard.as_mut() else {
        return Ok(empty_snapshot());
    };
    if session.session_id != session_id {
        return Ok(session.snapshot());
    }
    session.flush_in_flight = false;
    if session.pending_lines.is_empty() {
        return Ok(session.snapshot());
    }
    let summary = LiveSummaryChunk {
        title: build_chunk_title(chunk_index, range_start, range_end),
        range_label: range_label.clone(),
        body: chunk_ai.body,
        line_count: lines.len(),
        terms: chunk_ai.terms,
        whiteboard: reconciled_board,
    };
    Arc::make_mut(&mut session.summaries).push(summary);
    let pending = Arc::make_mut(&mut session.pending_lines);
    let drain_count = summarized_line_count.min(pending.len());
    pending.drain(0..drain_count);
    session.batch_started_at = range_end;
    Ok(session.snapshot())
}

/// Final-flush retry on stop. Unlike scheduled chunks, the closing segment has
/// no driver loop to retry it, so a single transient AI failure would lose the
/// last segment permanently. Retry a few times with backoff before giving up.

const FINAL_FLUSH_RETRY_ATTEMPTS: usize = 3;

pub(in crate::live) async fn flush_final_summary_with_retry(
    state: &LiveState,
) -> Result<LiveSessionSnapshot, String> {
    let mut last_err = "final flush failed".to_string();
    for attempt in 1..=FINAL_FLUSH_RETRY_ATTEMPTS {
        match flush_session_summary(state, true).await {
            Ok(snapshot) => return Ok(snapshot),
            Err(err) => {
                log::warn!(
                    "[Live] final chunk flush failed on attempt {}/{}: {}",
                    attempt,
                    FINAL_FLUSH_RETRY_ATTEMPTS,
                    err
                );
                last_err = err;
                if attempt < FINAL_FLUSH_RETRY_ATTEMPTS {
                    tokio::time::sleep(std::time::Duration::from_secs(2 * attempt as u64)).await;
                }
            }
        }
    }
    Err(last_err)
}

fn live_session_matches(state: &LiveState, session_id: &str) -> bool {
    state
        .0
        .lock()
        .ok()
        .and_then(|guard| {
            guard
                .as_ref()
                .map(|session| session.session_id == session_id)
        })
        .unwrap_or(false)
}

fn live_next_scheduled_flush_delay(
    state: &LiveState,
    session_id: &str,
) -> Option<std::time::Duration> {
    let now = Local::now();
    let summary_interval_minutes = live_summary_interval_minutes();
    let guard = state.0.lock().ok()?;
    let session = guard.as_ref()?;
    if session.session_id != session_id {
        return None;
    }
    if session.pending_lines.is_empty() || session.pending_lines.len() < 3 {
        return Some(std::time::Duration::from_secs(
            LIVE_FLUSH_DRIVER_IDLE_SLEEP_SECS,
        ));
    }
    let interval_due_at =
        effective_batch_started_at(session) + ChronoDuration::minutes(summary_interval_minutes);
    let min_ai_due_at =
        session.started_at + ChronoDuration::seconds(MIN_AI_SUMMARIZATION_DURATION_SECS);
    let due_at = if interval_due_at > min_ai_due_at {
        interval_due_at
    } else {
        min_ai_due_at
    };
    let wait_ms = due_at.signed_duration_since(now).num_milliseconds();
    if wait_ms <= 0 {
        return Some(std::time::Duration::from_secs(0));
    }
    let wait = std::time::Duration::from_millis(wait_ms as u64);
    let max_wait = std::time::Duration::from_secs(LIVE_FLUSH_DRIVER_MAX_SLEEP_SECS);
    Some(if wait > max_wait { max_wait } else { wait })
}

pub(in crate::live) async fn live_flush_summary_with_side_effects(
    app: &tauri::AppHandle,
    state: &LiveState,
    force: bool,
) -> Result<LiveSessionSnapshot, String> {
    let summary_count_before = {
        let guard = state
            .0
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        guard.as_ref().map(|s| s.summaries.len()).unwrap_or(0)
    };
    let snapshot = flush_session_summary(state, force).await?;
    auto_save_day_cache(state, true);

    // Whenever the AI flush actually produced a new summary chunk, also persist
    // the formal .md file. Cheap insurance: a crash before stop now leaves a
    // real markdown on disk, not just the hidden day_cache sidecar.
    if snapshot.summaries.len() > summary_count_before {
        let info = {
            let guard = state
                .0
                .lock()
                .map_err(|_| "Live state lock failed".to_string())?;
            guard.as_ref().map(|s| {
                (
                    s.course.clone(),
                    s.started_at,
                    s.transcript_lines.clone(),
                    s.summaries.clone(),
                )
            })
        };
        if let Some((course, started_at, transcript_lines, summaries)) = info {
            write_partial_markdown_file(&course, started_at, &transcript_lines, &summaries);
        }
    }

    emit_live_update(app, state);
    Ok(snapshot)
}

pub(in crate::live) fn start_live_flush_driver(app: tauri::AppHandle, session_id: String) {
    tauri::async_runtime::spawn(async move {
        loop {
            let state = app.state::<LiveState>();
            let Some(wait) = live_next_scheduled_flush_delay(state.inner(), &session_id) else {
                break;
            };
            if !wait.is_zero() {
                let notify = state.inner().flush_notify();
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = notify.notified() => continue,
                }
            } else {
                tokio::time::sleep(std::time::Duration::from_secs(
                    LIVE_FLUSH_DRIVER_MIN_SLEEP_SECS,
                ))
                .await;
            }
            let state = app.state::<LiveState>();
            if !live_session_matches(state.inner(), &session_id) {
                break;
            }
            match live_flush_summary_with_side_effects(&app, state.inner(), false).await {
                Ok(snapshot) => {
                    if !snapshot.active {
                        break;
                    }
                }
                Err(err) => {
                    log::warn!("[Live] backend scheduled flush failed: {err}");
                    // The flush already cleared `flush_in_flight` on failure, but
                    // without pushing a fresh snapshot the UI stays stuck on
                    // "要約を生成中…". Emit the cleared state plus an error event so
                    // Live can surface the failure instead of hanging silently.
                    emit_live_update(&app, state.inner());
                    let _ = app.emit("live-summary-error", serde_json::json!({ "message": err }));
                    tokio::time::sleep(std::time::Duration::from_secs(
                        LIVE_FLUSH_DRIVER_IDLE_SLEEP_SECS,
                    ))
                    .await;
                }
            }
        }
    });
}
