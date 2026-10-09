use std::sync::Arc;

use chrono::{DateTime, Local};
use tauri::Emitter;

use super::notification::LiveSessionNotification;
use super::types::LiveSessionUpdate;
use super::{LiveFinishPhase, LiveSessionSnapshot, LiveState, MIN_AI_SUMMARIZATION_DURATION_SECS};

pub(in crate::live) fn empty_snapshot() -> LiveSessionSnapshot {
    LiveSessionSnapshot {
        update_revision: 0,
        session_id: None,
        active: false,
        course: None,
        started_at: None,
        transcript_lines: Arc::new(Vec::new()),
        pending_lines: Arc::new(Vec::new()),
        summaries: Arc::new(Vec::new()),
        next_summary_at_ms: None,
        summarizing: false,
        finish_phase: None,
        finish_revision: 0,
    }
}

pub(in crate::live) fn emit_live_finish_progress(
    app: &tauri::AppHandle,
    state: &LiveState,
    expected: &str,
    phase: LiveFinishPhase,
) -> Result<(), String> {
    let progress = state.set_finish_phase(expected, phase)?;
    let _ = app.emit("live-finish-progress", progress);
    Ok(())
}

pub(in crate::live) fn sanitize_model_output(text: &str) -> String {
    let mut s = text.replace("<think>", "").replace("</think>", "");
    while let Some(start) = s.find("<think") {
        if let Some(end) = s[start..].find("</think>") {
            let end_idx = start + end + "</think>".len();
            s.replace_range(start..end_idx, "");
        } else {
            s.truncate(start);
            break;
        }
    }
    s.trim().to_string()
}

pub(in crate::live) fn sanitize_filename_component(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            _ => c,
        })
        .collect();
    let trimmed = s.trim().trim_matches('.');
    if trimmed.is_empty() {
        "live".into()
    } else {
        trimmed.to_string()
    }
}

pub(in crate::live) fn current_snapshot(state: &LiveState) -> LiveSessionSnapshot {
    state
        .session
        .lock()
        .ok()
        .map(|guard| state.capture_snapshot(guard.as_ref()))
        .unwrap_or_else(empty_snapshot)
}

pub(in crate::live) fn emit_live_update(app: &tauri::AppHandle, state: &LiveState) {
    emit_session_update(app, state, false);
}

pub(in crate::live) fn emit_session_update(
    app: &tauri::AppHandle,
    state: &LiveState,
    include_summary: bool,
) {
    match current_notification(state, include_summary) {
        Ok(update) => {
            let _ = app.emit("live-session-updated", update);
        }
        Err(error) => log::warn!("[Live] session update capture failed: {error}"),
    }
}

pub(in crate::live) fn current_notification(
    state: &LiveState,
    include_summary: bool,
) -> Result<LiveSessionNotification, String> {
    let (update, reference) = capture_update(state, include_summary)?;
    Ok(LiveSessionNotification::new(update, reference))
}

#[cfg(test)]
pub(in crate::live) fn current_update(
    state: &LiveState,
    include_summary: bool,
) -> Result<LiveSessionUpdate, String> {
    capture_update(state, include_summary).map(|(update, _)| update)
}

fn capture_update(
    state: &LiveState,
    include_summary: bool,
) -> Result<(LiveSessionUpdate, Option<usize>), String> {
    let guard = state
        .session
        .lock()
        .map_err(|_| "Live state lock failed".to_owned())?;
    let session = guard.as_ref();
    // Only pointer identity is checked under the capture lock. An independently
    // equal new board stays complete, so this path never hashes/scans its nodes.
    let reference = session.filter(|_| include_summary).and_then(|session| {
        let latest = session.summaries.last()?.whiteboard.as_ref()?;
        let previous = session.summaries[..session.summaries.len() - 1]
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, chunk)| chunk.whiteboard.as_ref().map(|board| (index, board)))?;
        Arc::ptr_eq(latest, previous.1).then_some(previous.0)
    });
    Ok((
        LiveSessionUpdate {
            update_revision: state.next_snapshot_revision(),
            session_id: session.map(|s| s.session_id.clone()),
            active: session.is_some(),
            course: session.map(|s| s.course.clone()),
            started_at: session.map(|s| super::format_datetime(s.started_at)),
            next_summary_at_ms: session.and_then(|s| s.next_summary_at_ms()),
            summarizing: session.is_some_and(|s| s.flush_in_flight),
            finish_phase: session.and_then(|s| s.finish_phase),
            finish_revision: session.map_or(0, |s| s.finish_revision),
            transcript_line_count: session.map_or(0, |s| s.transcript_lines.len()),
            pending_line_count: session.map_or(0, |s| s.pending_lines.len()),
            summary_count: session.map_or(0, |s| s.summaries.len()),
            latest_summary: session
                .filter(|_| include_summary)
                .and_then(|s| s.summaries.last().cloned()),
        },
        reference,
    ))
}

pub(in crate::live) fn live_ai_config() -> Result<crate::ai::AiConfig, String> {
    validate_live_ai_config(crate::ai::load_ai_config())
}

/// Validate this request's resolved settings without rereading credentials.
pub(in crate::live) fn validate_live_ai_config(
    cfg: crate::ai::AiConfig,
) -> Result<crate::ai::AiConfig, String> {
    cfg.ensure_credentials_readable()?;
    if !cfg.ai_enabled {
        return Err("Live要約にはAIを有効にしてください".into());
    }
    if cfg.provider == "local" {
        crate::local_ai_support::ensure_supported()?;
    } else if cfg.api_key.is_empty() {
        // Surfaces the real cause instead of letting the request 401 silently.
        // Most common in packaged builds when the keychain item is unreadable.
        log::warn!(
            "[Live] AI summarization aborted: api_key empty for provider '{}' (secret store unreadable?)",
            cfg.provider
        );
        return Err("AI APIキーを読み込めませんでした。設定でAPIキーを保存し直してください".into());
    }
    Ok(cfg)
}

pub(in crate::live) fn live_summary_interval_minutes() -> i64 {
    crate::ai::live_summary_interval_minutes()
}

pub(in crate::live) fn should_skip_ai_summarization(
    started_at: DateTime<Local>,
    now: DateTime<Local>,
) -> bool {
    now.signed_duration_since(started_at).num_seconds() < MIN_AI_SUMMARIZATION_DURATION_SECS
}

pub(in crate::live) fn should_run_finish_ai(
    provider: &str,
    started_at: DateTime<Local>,
    ended_at: DateTime<Local>,
) -> bool {
    provider != "local" && !should_skip_ai_summarization(started_at, ended_at)
}

pub(in crate::live) fn should_require_finish_chunk_ai(
    started_at: DateTime<Local>,
    ended_at: DateTime<Local>,
    pending_line_count: usize,
) -> bool {
    pending_line_count > 0 && !should_skip_ai_summarization(started_at, ended_at)
}

#[cfg(test)]
#[path = "tests/updates.rs"]
mod tests;
