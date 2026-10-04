//! Document identity and source-change detection for SenseA.
//!
//! Fingerprints a downloaded document, decides whether a delta cycle should
//! reprocess it, and records added, removed, or changed sources.

use super::*;

pub(super) fn document_label(document: &AnalysisDocument) -> String {
    if document.filename.trim().is_empty() {
        document.title.clone()
    } else {
        document.filename.clone()
    }
}

pub(super) fn document_id(document: &AnalysisDocument) -> String {
    if !document.source_fingerprint.is_empty() {
        return format!(
            "{:x}",
            Sha256::digest(format!(
                "{}|{}|{}",
                document.kind, document.filename, document.source_fingerprint
            ))
        );
    }
    // Identity is deliberately path-free: the organizer moves a file between theme
    // folders, and that must NOT change who the document is — otherwise the next
    // cycle can't find its prior analysis and re-runs the AI on an unchanged file.
    format!(
        "{:x}",
        Sha256::digest(format!(
            "{}|{}|{}",
            document.kind, document.title, document.filename
        ))
    )
}

pub(super) fn document_fingerprint(document: &AnalysisDocument) -> Result<String, String> {
    // Version fingerprint excludes `path` for the same reason as `document_id`:
    // relocating a file (organize) is not a content change, so it must not look
    // like one and trigger re-analysis. `content` + `sourceFingerprint` carry the
    // real "has this document changed" signal.
    sha256_json(&json!({
        "kind": document.kind,
        "title": document.title,
        "filename": document.filename,
        "content": document.content,
        "sourceFingerprint": document.source_fingerprint,
    }))
}

pub(super) fn previous_document_analysis<'a>(
    previous: &'a CourseAutomationStatus,
    document: &AnalysisDocument,
) -> Option<&'a DocumentAnalysis> {
    let id = document_id(document);
    previous
        .document_analyses
        .iter()
        .find(|analysis| analysis.id == id)
        .or_else(|| {
            previous.document_analyses.iter().find(|analysis| {
                analysis.status == "done"
                    && analysis.source_fingerprint.is_empty()
                    && analysis.kind == document.kind
                    && analysis.title == document.title
                    && analysis.filename == document.filename
            })
        })
}

pub(super) fn should_process_document_delta(
    previous: &CourseAutomationStatus,
    document: &AnalysisDocument,
) -> bool {
    match previous_document_analysis(previous, document) {
        Some(previous_analysis)
            if matches!(previous_analysis.status.as_str(), "done" | "skipped") =>
        {
            if !document.source_fingerprint.is_empty() {
                let same_version = previous_analysis.id == document_id(document)
                    || previous_analysis.source_fingerprint == document.source_fingerprint;
                if same_version {
                    return false;
                }
                // Legacy success records without a source fingerprint still
                // get one migration pass so future cycles can skip by source
                // identity without re-reading the file.
                if previous_analysis.source_fingerprint.is_empty() {
                    return true;
                }
            }
            document_fingerprint(document)
                .map(|fingerprint| fingerprint != previous_analysis.fingerprint)
                .unwrap_or(true)
        }
        _ => true,
    }
}

pub(super) fn should_pause_delta_cycle_after_analysis(
    force_all: bool,
    previous: Option<&DocumentAnalysis>,
    analysis: &DocumentAnalysis,
) -> bool {
    !force_all
        && analysis.status == "done"
        && analysis.trigger_decision == "immediate"
        && previous.is_none_or(|item| item.status != "done")
}

fn normalized_source_part(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn source_document_key(kind: &str, title: &str, filename: &str) -> String {
    format!(
        "{}|{}|{}",
        normalized_source_part(kind),
        normalized_source_part(title),
        normalized_source_part(filename)
    )
}

fn analysis_source_key(analysis: &DocumentAnalysis) -> String {
    source_document_key(&analysis.kind, &analysis.title, &analysis.filename)
}

fn document_source_key(document: &AnalysisDocument) -> String {
    source_document_key(&document.kind, &document.title, &document.filename)
}

fn source_info_key(info: &SourceDocumentInfo) -> String {
    source_document_key(&info.kind, &info.title, &info.filename)
}

fn source_kind_monitored(kind: &str, config: &CourseAutomationConfig) -> bool {
    match kind {
        "material" | "legacy" => config.monitor_materials,
        "announcement" => config.monitor_announcements,
        "report" => config.monitor_assignments,
        _ => true,
    }
}

fn source_event_attention(previous: &DocumentAnalysis) -> bool {
    previous.trigger_decision == "immediate"
        || previous.trigger_decision == "observe"
        || !previous.findings.is_empty()
        || !previous.print_instruction.trim().is_empty()
        || !previous.seat_evidence.is_empty()
}

fn source_event_id(event: &str, previous_id: &str, current_id: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!(
            "source-event|{}|{}|{}",
            event, previous_id, current_id
        ))
    )
}

fn source_event_label(analysis: &DocumentAnalysis) -> String {
    if analysis.filename.trim().is_empty() {
        analysis.title.clone()
    } else {
        analysis.filename.clone()
    }
}

pub(super) fn detect_source_events(
    previous: &CourseAutomationStatus,
    current_sources: &[SourceDocumentInfo],
    current_documents: &[AnalysisDocument],
    config: &CourseAutomationConfig,
) -> Vec<SourceEvent> {
    let current_source_keys = current_sources
        .iter()
        .map(source_info_key)
        .collect::<HashSet<_>>();
    let current_source_by_key = current_sources
        .iter()
        .filter(|source| !source.source_fingerprint.is_empty())
        .map(|source| (source_info_key(source), source))
        .collect::<HashMap<_, _>>();
    let current_document_ids = current_documents
        .iter()
        .map(document_id)
        .collect::<HashSet<_>>();
    let current_matched_previous_ids = current_documents
        .iter()
        .filter_map(|document| {
            previous_document_analysis(previous, document).map(|item| item.id.clone())
        })
        .collect::<HashSet<_>>();
    let current_analyzable_by_key = current_documents
        .iter()
        .filter(|document| document.load_error.is_empty())
        .map(|document| (document_source_key(document), document))
        .collect::<HashMap<_, _>>();

    let mut events = Vec::new();
    let mut seen = HashSet::new();
    for analysis in &previous.document_analyses {
        if analysis.status != "done" || !source_kind_monitored(&analysis.kind, config) {
            continue;
        }
        if current_document_ids.contains(&analysis.id)
            || current_matched_previous_ids.contains(&analysis.id)
        {
            continue;
        }
        let source_key = analysis_source_key(analysis);
        let (event, current_id, detail) =
            if let Some(current_document) = current_analyzable_by_key.get(&source_key) {
                let current_id = document_id(current_document);
                if current_id == analysis.id {
                    continue;
                }
                (
                    "changed",
                    current_id,
                    "同じ題名/ファイル名の資料が新しい版に置き換わりました".to_string(),
                )
            } else if let Some(current_source) = current_source_by_key.get(&source_key) {
                if analysis.source_fingerprint.is_empty()
                    || analysis.source_fingerprint == current_source.source_fingerprint
                {
                    continue;
                }
                (
                    "changed",
                    current_source.source_fingerprint.clone(),
                    "同じ題名/ファイル名の資料が新しい版に置き換わりました".to_string(),
                )
            } else if !current_source_keys.contains(&source_key) {
                (
                    "removed",
                    String::new(),
                    "前回まで存在した資料が現在の Luna 一覧にありません".to_string(),
                )
            } else {
                continue;
            };
        let id = source_event_id(event, &analysis.id, &current_id);
        if !seen.insert(id.clone()) {
            continue;
        }
        events.push(SourceEvent {
            id,
            event: event.to_string(),
            document_id: analysis.id.clone(),
            kind: analysis.kind.clone(),
            title: analysis.title.clone(),
            filename: analysis.filename.clone(),
            previous_summary: truncate_chars(&analysis.summary, 240),
            detail: format!("{}: {}", source_event_label(analysis), detail),
            attention: source_event_attention(analysis),
        });
    }
    events
}

pub(super) fn merge_source_events(
    existing: &[SourceEvent],
    current: Vec<SourceEvent>,
) -> Vec<SourceEvent> {
    let mut merged = existing.to_vec();
    for event in current {
        if let Some(existing) = merged.iter_mut().find(|item| item.id == event.id) {
            *existing = event;
        } else {
            merged.push(event);
        }
    }
    merged
}
