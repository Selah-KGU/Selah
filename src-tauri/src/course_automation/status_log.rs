//! Run-log and analysis-status merging for a SenseA cycle.
//!
//! Keeps the bounded log, merges document analyses, and turns failure
//! counts into the stage shown to the user.

use super::*;

/// Appends a run-log entry (one per operation) and keeps the log bounded.
pub(super) fn push_run_log(status: &mut CourseAutomationStatus, level: &str, message: String) {
    status.run_log.push(RunLogEntry {
        at: epoch_secs(),
        level: level.to_string(),
        message,
    });
    let len = status.run_log.len();
    if len > 40 {
        status.run_log.drain(0..len - 40);
    }
}

pub(super) fn push_ai_usage(status: &mut CourseAutomationStatus, usage: AiUsageEstimate) {
    status.ai_usage.push(usage);
    let len = status.ai_usage.len();
    if len > 40 {
        status.ai_usage.drain(0..len - 40);
    }
}

pub(super) fn upsert_document_analysis(
    analyses: &mut Vec<DocumentAnalysis>,
    analysis: DocumentAnalysis,
) {
    if analysis.status == "done" {
        analyses.retain(|item| {
            item.status != "error"
                || item.kind != analysis.kind
                || item.title != analysis.title
                || item.filename != analysis.filename
        });
    }
    if let Some(existing) = analyses.iter_mut().find(|item| item.id == analysis.id) {
        if existing.status != "done" || analysis.status == "done" {
            *existing = analysis;
        }
    } else {
        analyses.push(analysis);
    }
}

pub(super) fn merge_unique_ids(
    existing: &[String],
    additional: impl IntoIterator<Item = String>,
) -> Vec<String> {
    let mut merged = existing.to_vec();
    for id in additional {
        if !merged.iter().any(|existing_id| existing_id == &id) {
            merged.push(id);
        }
    }
    merged
}

pub(super) fn remove_consumed_ids(pending: &mut Vec<String>, consumed: &[String]) {
    if consumed.is_empty() {
        return;
    }
    pending.retain(|id| !consumed.iter().any(|consumed_id| consumed_id == id));
}

pub(super) fn retryable_failure_count(status: &CourseAutomationStatus) -> usize {
    status
        .artifacts
        .iter()
        .filter(|item| item.status == "error")
        .count()
        + status
            .document_analyses
            .iter()
            .filter(|item| item.status == "error")
            .count()
        + status
            .print_results
            .iter()
            .filter(|item| matches!(item.status.as_str(), "error" | "not_found"))
            .count()
        + usize::from(!status.pending_notification_ids.is_empty())
        + usize::from(status.pending_seat_notification)
}

pub(super) fn analysis_failure_count(status: &CourseAutomationStatus) -> usize {
    status
        .artifacts
        .iter()
        .filter(|item| item.status == "error")
        .count()
        + status
            .document_analyses
            .iter()
            .filter(|item| item.status == "error")
            .count()
}

pub(super) fn analysis_failure_summary(count: usize) -> String {
    if count <= 1 {
        "一部の資料の分析に失敗しました".into()
    } else {
        format!("{count}件の資料の分析に失敗しました")
    }
}

pub(super) fn final_run_stage(
    current_stage: &str,
    outcome_ok: bool,
    analysis_failures: usize,
) -> String {
    if !outcome_ok {
        return "error".into();
    }
    if analysis_failures > 0 {
        return "partial_error".into();
    }
    match current_stage {
        "unchanged" | "pending_summary" => current_stage.into(),
        _ => "done".into(),
    }
}

pub(super) fn migrate_legacy_status_errors(status: &mut CourseAutomationStatus) {
    status
        .run_log
        .retain(|entry| entry.message != LEGACY_ALL_DOCUMENTS_FAILED_ERROR);
    for entry in &mut status.run_log {
        if entry.message == LEGACY_AI_TIMEOUT_180_ERROR {
            entry.message = LEGACY_AI_TIMEOUT_ERROR.into();
        }
    }

    let analysis_failures = analysis_failure_count(status);
    if analysis_failures > 0
        && (status.last_error == LEGACY_ALL_DOCUMENTS_FAILED_ERROR
            || status.last_error == LEGACY_AI_TIMEOUT_180_ERROR
            || status.last_error.trim().is_empty()
            || matches!(
                status.stage.as_str(),
                "done" | "unchanged" | "pending_summary"
            ))
    {
        status.stage = final_run_stage(status.stage.as_str(), true, analysis_failures);
        status.last_ok = Some(false);
        status.last_error = analysis_failure_summary(analysis_failures);
        return;
    }

    if status.last_error == LEGACY_ALL_DOCUMENTS_FAILED_ERROR {
        status.last_error = "資料の分析に失敗しました".into();
    } else if status.last_error == LEGACY_AI_TIMEOUT_180_ERROR {
        status.last_error = LEGACY_AI_TIMEOUT_ERROR.into();
    }
}
