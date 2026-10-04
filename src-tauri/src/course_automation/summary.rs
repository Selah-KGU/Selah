//! Course analysis normalization, archive, and summary refresh.

use super::*;

pub(super) fn normalize_course_analysis(
    mut analysis: AgentCourseAnalysis,
    previous_archive: &[ArchivedGroup],
    today: &str,
) -> AgentCourseAnalysis {
    analysis.summary = truncate_chars(analysis.summary.trim(), 240);
    analysis.findings = items::normalize_items(
        std::mem::take(&mut analysis.findings),
        "finding",
        today,
        280,
    );
    // Memory keeps its expired items: active stay, expired fold into the archive.
    let (active, expired) = items::partition_by_expiry(
        std::mem::take(&mut analysis.standing_context),
        "memory",
        today,
        280,
    );
    analysis.standing_context = active;
    let stale = expired.into_iter().map(|item| item.text).collect();
    // Floor the archive with the previous one so history can't be lost if the
    // model omits it; the model's groups merge on top (dedup keeps it tidy).
    let merged_groups = previous_archive
        .iter()
        .cloned()
        .chain(std::mem::take(&mut analysis.archived_context))
        .collect();
    analysis.archived_context = consolidate_archive(merged_groups, stale);
    analysis.seat.assignment = truncate_chars(analysis.seat.assignment.trim(), 80);
    analysis.seat.evidence = normalize_short_list(analysis.seat.evidence, 6, 280);
    let mut seen_prints = HashSet::new();
    analysis.print_candidates.retain(|candidate| {
        !candidate.filename.trim().is_empty() && seen_prints.insert(candidate.filename.clone())
    });
    analysis.print_candidates.truncate(12);
    for candidate in &mut analysis.print_candidates {
        candidate.reason = truncate_chars(candidate.reason.trim(), 240);
        candidate.category = truncate_chars(candidate.category.trim(), 80);
    }
    analysis
}

/// Heading used for expired memories that reached the archive without the model
/// having grouped them yet. The model is asked to re-file them into proper
/// groups on the next round; until then they live here so nothing is lost.
pub(super) const ARCHIVE_FALLBACK_LABEL: &str = "過去の項目";

/// Normalizes the model's consolidated past-memory groups and folds in any
/// freshly-expired `fallback_items` the model didn't group itself. Groups with
/// the same label are merged, items are trimmed and deduped within a group, and
/// empty groups are dropped. There is no cap — size is bounded by the model
/// consolidating related items into compact sub-entries rather than by a limit.
pub(super) fn consolidate_archive(
    groups: Vec<ArchivedGroup>,
    fallback_items: Vec<String>,
) -> Vec<ArchivedGroup> {
    fn insert(result: &mut Vec<ArchivedGroup>, label: &str, item: String) {
        let item = truncate_chars(item.trim(), 280);
        if item.is_empty() {
            return;
        }
        if let Some(group) = result.iter_mut().find(|group| group.label == label) {
            if !group.items.iter().any(|existing| existing == &item) {
                group.items.push(item);
            }
        } else {
            result.push(ArchivedGroup {
                label: label.to_string(),
                items: vec![item],
            });
        }
    }

    let mut result: Vec<ArchivedGroup> = Vec::new();
    for group in groups {
        let label = truncate_chars(group.label.trim(), 80);
        let label = if label.is_empty() {
            ARCHIVE_FALLBACK_LABEL.to_string()
        } else {
            label
        };
        for item in group.items {
            insert(&mut result, &label, item);
        }
    }
    for item in fallback_items {
        insert(&mut result, ARCHIVE_FALLBACK_LABEL, item);
    }
    result.retain(|group| !group.items.is_empty());
    result
}

/// Drops user-state entries whose item is no longer produced, so the overlay
/// stays bounded. A state persists exactly while its item keeps appearing. As
/// more facets adopt `Item`, add their ids to `present` here.
pub(super) fn prune_item_states(status: &mut CourseAutomationStatus) {
    let present: HashSet<String> = status
        .analysis
        .findings
        .iter()
        .chain(status.analysis.standing_context.iter())
        .map(|item| item.id.clone())
        .collect();
    items::prune_states(&mut status.item_states, &present);
}

pub(super) fn normalize_short_list(
    values: Vec<String>,
    max_items: usize,
    max_chars: usize,
) -> Vec<String> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for value in values {
        let value = truncate_chars(value.trim(), max_chars);
        if value.is_empty() || !seen.insert(value.clone()) {
            continue;
        }
        normalized.push(value);
        if normalized.len() >= max_items {
            break;
        }
    }
    normalized
}

pub(super) fn proactive_notification_body(analysis: &AgentCourseAnalysis) -> String {
    let mut lines = Vec::new();
    if !analysis.summary.trim().is_empty() {
        lines.push(analysis.summary.trim().to_string());
    }
    lines.extend(
        analysis
            .findings
            .iter()
            .take(3)
            .map(|item| format!("・{}", item.text)),
    );
    truncate_chars(&lines.join("\n"), 600)
}

pub(super) fn document_notification_key(analysis: &DocumentAnalysis) -> String {
    format!("{}:{}", analysis.id, analysis.fingerprint)
}

pub(super) fn material_source_fingerprint(
    file: &crate::luna_parser::LunaMaterialFile,
) -> Result<String, String> {
    sha256_json(&json!({
        "fileName": file.file_name,
        "displayName": file.display_name,
        "objectName": file.object_name,
        "resourceId": file.resource_id,
        "materialId": file.material_id,
        "fileType": file.file_type,
        "externalUrl": file.external_url,
    }))
}

pub(super) fn should_refresh_summary(
    previous_summary: &str,
    pending_count: usize,
    agent_requests_immediate_summary: bool,
    agent_requests_observe_followup: bool,
    pending_source_event_count: usize,
) -> bool {
    previous_summary.trim().is_empty()
        || agent_requests_immediate_summary
        || agent_requests_observe_followup
        || pending_source_event_count > 0
        || pending_count >= FULL_SUMMARY_NEW_ITEM_THRESHOLD
}

fn summary_priority(analysis: &DocumentAnalysis) -> usize {
    match analysis.trigger_decision.as_str() {
        "immediate" => 0,
        "observe" => 1,
        _ => 2,
    }
}

pub(super) fn prioritize_summary_items(items: &mut [DocumentAnalysis]) {
    items.sort_by_key(summary_priority);
}

pub(super) fn has_observe_followup(
    analyses: &[DocumentAnalysis],
    pending_summary_ids: &[String],
    newly_analyzed_ids: &[String],
) -> bool {
    let has_new_pending_evidence = newly_analyzed_ids.iter().any(|id| {
        pending_summary_ids
            .iter()
            .any(|pending_id| pending_id == id)
    });
    if !has_new_pending_evidence || pending_summary_ids.len() < 2 {
        return false;
    }
    analyses.iter().any(|analysis| {
        analysis.status == "done"
            && analysis.trigger_decision == "observe"
            && pending_summary_ids.iter().any(|id| id == &analysis.id)
    })
}

/// Whitespace-collapsed, lowercased form for matching print categories, so minor
/// naming drift ("ワークシート" vs "ワークシート ") doesn't re-ask for approval.
/// Approvals still store the original string for display.
pub(super) fn normalize_category(category: &str) -> String {
    category
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
