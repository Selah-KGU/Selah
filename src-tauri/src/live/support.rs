use std::sync::Arc;

use chrono::{DateTime, Local};
use tauri::Emitter;

use super::{LiveSessionSnapshot, LiveState, MIN_AI_SUMMARIZATION_DURATION_SECS};

pub(in crate::live) fn empty_snapshot() -> LiveSessionSnapshot {
    LiveSessionSnapshot {
        active: false,
        course: None,
        started_at: None,
        transcript_lines: Arc::new(Vec::new()),
        pending_lines: Arc::new(Vec::new()),
        summaries: Arc::new(Vec::new()),
        next_summary_at_ms: None,
        summarizing: false,
    }
}

pub(in crate::live) fn emit_live_finish_progress(app: &tauri::AppHandle, step: &str) {
    let _ = app.emit("live-finish-progress", serde_json::json!({ "step": step }));
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
        .0
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|session| session.snapshot()))
        .unwrap_or_else(empty_snapshot)
}

pub(in crate::live) fn emit_live_update(app: &tauri::AppHandle, state: &LiveState) {
    let _ = app.emit("live-session-updated", current_snapshot(state));
}

pub(in crate::live) fn live_ai_config() -> Result<crate::ai::AiConfig, String> {
    let cfg = crate::ai::load_ai_config();
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
    crate::ai::load_ai_config()
        .live_summary_interval_minutes
        .max(5) as i64
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
