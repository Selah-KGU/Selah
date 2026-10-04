use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;

use tauri::{AppHandle, Emitter};

use crate::ai::AiConfig;
use crate::background_refresh::BackendSessionStatusPayload;
use crate::db::{epoch_secs, Database};

use super::types::{
    AiRefreshItemStatus, AiRefreshStatus, AI_STATUS_CACHE_KEY, SCHEDULE_INPUT_MAX_AGE_SECS,
};

pub(in crate::ai_refresh) fn fresh_course_names(db: &Database) -> Result<String, String> {
    let snap = db.get_snapshot_state()?.unwrap_or_default();
    if !timestamp_is_fresh(snap.updated_at, SCHEDULE_INPUT_MAX_AGE_SECS) {
        return Ok(String::new());
    }
    let scope = crate::academic_period::visible_weeks(
        &snap.current_week_label,
        &snap.next_week_label,
        &snap.luna_year,
        &snap.luna_term,
        chrono::Local::now().date_naive(),
    );
    let raw = db.build_raw_data(&scope.current, &scope.next, Vec::new())?;
    Ok(raw
        .kgc_entries_current
        .iter()
        .chain(raw.kgc_entries_next.iter())
        .map(|entry| entry.name.trim())
        .filter(|name| !name.is_empty())
        .collect::<HashSet<_>>()
        .into_iter()
        .take(24)
        .collect::<Vec<_>>()
        .join("、"))
}
pub(in crate::ai_refresh) fn ai_unavailable_reason(config: &AiConfig) -> Option<String> {
    if !config.ai_enabled {
        return Some("AI機能が無効です".to_string());
    }
    if config.provider == "local" {
        if let Err(reason) = crate::local_ai_support::ensure_supported() {
            return Some(reason);
        }
    } else if config.api_key.trim().is_empty() {
        return Some("AI APIキーが未設定です".to_string());
    }
    None
}

pub(in crate::ai_refresh) async fn ai_session_block_reason(
    app: &AppHandle,
) -> Result<Option<String>, String> {
    let status = current_ai_session(app).await?;
    Ok(ai_session_block_reason_from_status(&status))
}

pub(in crate::ai_refresh) async fn current_ai_session(
    app: &AppHandle,
) -> Result<BackendSessionStatusPayload, String> {
    // Scheduling only needs a local availability snapshot. It must not trigger
    // network validation or hidden SAML recovery on its own.
    crate::background_refresh::sync_backend_session_status(app, false).await
}

pub(in crate::ai_refresh) fn ai_session_block_reason_from_status(
    status: &BackendSessionStatusPayload,
) -> Option<String> {
    if status.session_expired {
        return Some("セッション期限切れのためAI定期更新を実行しません".to_string());
    }
    if !status.kgc_session_present {
        return Some("未ログインのためAI定期更新を実行しません".to_string());
    }
    None
}

pub(in crate::ai_refresh) fn is_no_data_error(err: &str) -> bool {
    err.contains("まだありません")
        || err.contains("TODO項目がありません")
        || err.contains("読み込んでください")
        || err.contains("未ログイン")
        || err.contains("最新ではない")
}

pub(in crate::ai_refresh) fn item_status(
    key: &str,
    label: &str,
    status: &str,
    error: &str,
) -> AiRefreshItemStatus {
    AiRefreshItemStatus {
        key: key.to_string(),
        label: label.to_string(),
        status: status.to_string(),
        error: error.to_string(),
    }
}

pub(in crate::ai_refresh) fn update_item_status(
    status: &mut AiRefreshStatus,
    key: &str,
    next_status: &str,
    error: &str,
) {
    if let Some(item) = status.items.iter_mut().rev().find(|item| item.key == key) {
        item.status = next_status.to_string();
        item.error = error.to_string();
    }
}

pub(in crate::ai_refresh) fn item_attempted(item: &AiRefreshItemStatus) -> bool {
    matches!(item.status.as_str(), "done" | "error")
}

pub(in crate::ai_refresh) fn load_cache_json<T: for<'de> Deserialize<'de>>(
    db: &Database,
    key: &str,
) -> Option<T> {
    db.get_data_cache(key)
        .ok()
        .flatten()
        .and_then(|(json, _)| serde_json::from_str(&json).ok())
}

pub(in crate::ai_refresh) fn fresh_cache_json(
    db: &Database,
    key: &str,
    max_age_secs: i64,
) -> Option<String> {
    db.get_data_cache(key)
        .ok()
        .flatten()
        .and_then(|(json, updated_at)| timestamp_is_fresh(updated_at, max_age_secs).then_some(json))
}

pub(in crate::ai_refresh) fn cache_is_fresh(db: &Database, key: &str, max_age_secs: i64) -> bool {
    db.get_data_cache(key)
        .ok()
        .flatten()
        .map(|(_, updated_at)| timestamp_is_fresh(updated_at, max_age_secs))
        .unwrap_or(false)
}

pub(in crate::ai_refresh) fn timestamp_is_fresh(updated_at: i64, max_age_secs: i64) -> bool {
    updated_at > 0 && epoch_secs() - updated_at <= max_age_secs
}

pub(in crate::ai_refresh) fn load_status(db: &Database) -> AiRefreshStatus {
    let mut status: AiRefreshStatus = db
        .get_data_cache(AI_STATUS_CACHE_KEY)
        .ok()
        .flatten()
        .and_then(|(json, _)| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    // running is process-local state. Never trust a persisted value after
    // restart or crash; active calls set it from AiRefreshState instead.
    status.running = false;
    status
}

pub(in crate::ai_refresh) fn save_status_and_emit(
    app: &AppHandle,
    db: &Database,
    status: &AiRefreshStatus,
) -> Result<(), String> {
    let json = serde_json::to_string(status).map_err(|e| e.to_string())?;
    db.save_data_cache(AI_STATUS_CACHE_KEY, &json)?;
    let _ = app.emit("backend-ai-refresh-status", status);
    Ok(())
}

pub(in crate::ai_refresh) fn record_status(
    app: &AppHandle,
    db: &Database,
    status: &AiRefreshStatus,
    should_record: bool,
) -> Result<(), String> {
    if should_record {
        save_status_and_emit(app, db, status)?;
    }
    Ok(())
}

pub(in crate::ai_refresh) fn emit_cache_updated(app: &AppHandle, keys: Vec<String>) {
    if let Err(e) = app.emit("backend-cache-updated", json!({ "keys": keys })) {
        log::warn!("[ai_refresh] backend-cache-updated emit failed: {}", e);
    }
}
