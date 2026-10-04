//! Download artifacts and analysis-document assembly for SenseA.
//!
//! Tracks which files can be reused, turns download results into analysis
//! documents, and migrates older artifact records. The course run loop stays
//! in the parent.

use super::*;

pub(super) fn collect_download_value(
    downloaded: &mut Vec<(String, String, String, PathBuf, String)>,
    external_links: &mut Vec<String>,
    seen_paths: &mut HashSet<String>,
    kind: &str,
    title: &str,
    source_fingerprint: &str,
    value: &Value,
) {
    if let Some(url) = value.get("url").and_then(Value::as_str) {
        if !url.trim().is_empty() && !external_links.iter().any(|item| item == url) {
            external_links.push(url.to_string());
        }
    }
    let Some(path) = value.get("saved_path").and_then(Value::as_str) else {
        return;
    };
    if path.trim().is_empty() || !seen_paths.insert(path.to_string()) {
        return;
    }
    let filename = value
        .get("filename")
        .or_else(|| value.get("attachment_name"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("")
        });
    downloaded.push((
        kind.to_string(),
        title.to_string(),
        filename.to_string(),
        PathBuf::from(path),
        source_fingerprint.to_string(),
    ));
}

pub(super) fn build_analysis_documents(
    downloaded: &[(String, String, String, PathBuf, String)],
    previous: &CourseAutomationStatus,
) -> Vec<AnalysisDocument> {
    let mut documents = Vec::new();
    for (kind, title, filename, path, source_fingerprint) in downloaded {
        let mut document = AnalysisDocument {
            kind: kind.clone(),
            title: title.clone(),
            filename: filename.clone(),
            path: path.to_string_lossy().to_string(),
            content: String::new(),
            source_fingerprint: source_fingerprint.clone(),
            load_error: String::new(),
            images: Vec::new(),
        };
        match previous_document_analysis(previous, &document).map(|item| item.status.as_str()) {
            // Reused on a later pass: a done analysis is migrated, a skipped one
            // stays skipped. Neither re-reads the file.
            Some("done") => {}
            Some("skipped") => document.load_error = DOC_SKIP_MARKER.into(),
            _ => match crate::agent_tools::read_downloaded_text(path) {
                Ok(text) => document.content = truncate_chars(&text, MAX_FILE_TEXT_CHARS),
                Err(error) => load_document_images_or_error(path, &mut document, &error),
            },
        }
        documents.push(document);
    }
    documents
}

pub(super) fn analysis_document_from_download_entry(
    kind: &str,
    title: &str,
    filename: &str,
    path: &Path,
    source_fingerprint: &str,
) -> AnalysisDocument {
    AnalysisDocument {
        kind: kind.to_string(),
        title: title.to_string(),
        filename: filename.to_string(),
        path: path.to_string_lossy().to_string(),
        content: String::new(),
        source_fingerprint: source_fingerprint.to_string(),
        load_error: String::new(),
        images: Vec::new(),
    }
}

pub(super) fn should_process_downloaded_delta(
    previous: &CourseAutomationStatus,
    entry: &(String, String, String, PathBuf, String),
) -> bool {
    let (kind, title, filename, path, source_fingerprint) = entry;
    let document =
        analysis_document_from_download_entry(kind, title, filename, path, source_fingerprint);
    should_process_document_delta(previous, &document)
}

/// When text extraction fails for a PDF, fall back to its embedded page images
/// so a vision model can still read it. If a PDF yields neither text nor images
/// it is skipped (terminal). Non-PDF failures keep the original error.
pub(super) fn load_document_images_or_error(
    path: &Path,
    document: &mut AnalysisDocument,
    text_error: &str,
) {
    let is_pdf = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"));
    if is_pdf {
        // 1) Cheap path: pass through embedded JPEG images (scanned PDFs).
        // 2) Fallback: rasterize the pages (vector PDFs with no images).
        let images = crate::agent_tools::read_downloaded_images(path)
            .ok()
            .filter(|images| !images.is_empty())
            .or_else(|| {
                crate::agent_tools::render_pdf_images(path)
                    .map_err(|error| log::warn!("[course_automation] PDF render failed: {}", error))
                    .ok()
                    .filter(|images| !images.is_empty())
            });
        if let Some(images) = images {
            document.images = images;
            document.content =
                "（本文をテキスト抽出できないため、添付画像から読み取ってください）".into();
            return;
        }
        // No text, no images, no render: skip rather than retry forever.
        document.load_error = DOC_SKIP_MARKER.into();
        return;
    }
    document.load_error = format!("本文抽出失敗: {}", text_error);
}

pub(super) fn failed_analysis_document(
    kind: &str,
    title: &str,
    filename: &str,
    source_fingerprint: &str,
    error: &str,
) -> AnalysisDocument {
    AnalysisDocument {
        kind: kind.to_string(),
        title: title.to_string(),
        filename: filename.to_string(),
        path: String::new(),
        content: String::new(),
        source_fingerprint: source_fingerprint.to_string(),
        load_error: error.to_string(),
        images: Vec::new(),
    }
}

pub(super) fn reusable_artifact_path(
    artifacts: &[CourseArtifactRecord],
    kind: &str,
    _title: &str,
    filename: &str,
    current_source_fingerprint: &str,
) -> Option<(PathBuf, String)> {
    artifacts
        .iter()
        .find(|artifact| {
            artifact.status == "downloaded"
                && (artifact.kind == kind || artifact.kind == "legacy")
                && artifact.filename == filename
                && (artifact.source_fingerprint.is_empty()
                    || artifact.source_fingerprint == current_source_fingerprint)
                && !artifact.path.is_empty()
                && Path::new(&artifact.path).is_file()
        })
        .map(|artifact| {
            (
                PathBuf::from(&artifact.path),
                artifact.source_fingerprint.clone(),
            )
        })
}

pub(super) fn reusable_activity_downloads(
    artifacts: &[CourseArtifactRecord],
) -> HashMap<String, crate::agent_tools::ReusableCourseDownload> {
    let mut paths = HashMap::new();
    for artifact in artifacts {
        if artifact.status != "downloaded"
            || !matches!(artifact.kind.as_str(), "announcement" | "report" | "legacy")
            || artifact.path.is_empty()
            || !Path::new(&artifact.path).is_file()
        {
            continue;
        }
        let reusable = crate::agent_tools::ReusableCourseDownload {
            path: artifact.path.clone(),
            source_fingerprint: artifact.source_fingerprint.clone(),
        };
        if !artifact.source_fingerprint.is_empty() {
            paths.insert(
                format!("fingerprint:{}", artifact.source_fingerprint),
                reusable.clone(),
            );
            continue;
        }
        if artifact.kind == "legacy" {
            paths.insert(
                activity_download_identity("announcement", &artifact.filename),
                reusable.clone(),
            );
            paths.insert(
                activity_download_identity("report", &artifact.filename),
                reusable,
            );
        } else {
            paths.insert(
                activity_download_identity(&artifact.kind, &artifact.filename),
                reusable,
            );
        }
    }
    paths
}

pub(super) fn reusable_activity_details(
    records: &[ActivityDetailCacheRecord],
) -> HashMap<String, crate::agent_tools::ReusableActivityDetail> {
    records
        .iter()
        .filter(|record| {
            !record.detail_path.trim().is_empty()
                && !record.list_fingerprint.trim().is_empty()
                && !record.source_fingerprint.trim().is_empty()
        })
        .map(|record| {
            (
                record.detail_path.clone(),
                crate::agent_tools::ReusableActivityDetail {
                    list_fingerprint: record.list_fingerprint.clone(),
                    source_fingerprint: record.source_fingerprint.clone(),
                    checked_at: record.checked_at,
                },
            )
        })
        .collect()
}

pub(super) fn has_retryable_activity_artifact_failure(artifacts: &[CourseArtifactRecord]) -> bool {
    artifacts.iter().any(|artifact| {
        artifact.status == "error"
            && matches!(artifact.kind.as_str(), "announcement" | "report" | "legacy")
    })
}

fn activity_detail_cache_id(kind: &str, detail_path: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("activity-detail|{}|{}", kind, detail_path))
    )
}

pub(super) fn activity_detail_cache_id_from_value(value: &Value) -> Option<String> {
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("activity");
    let detail_path = value
        .get("detail_path")
        .and_then(Value::as_str)
        .unwrap_or("");
    if detail_path.trim().is_empty() {
        None
    } else {
        Some(activity_detail_cache_id(kind, detail_path))
    }
}

pub(super) fn activity_detail_cache_record_from_value(
    value: &Value,
    checked_at: i64,
) -> Option<ActivityDetailCacheRecord> {
    if value.get("status").and_then(Value::as_str) != Some("detail") {
        return None;
    }
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("activity");
    let title = value.get("title").and_then(Value::as_str).unwrap_or("");
    let detail_path = value
        .get("detail_path")
        .and_then(Value::as_str)
        .unwrap_or("");
    let list_fingerprint = value
        .get("list_fingerprint")
        .and_then(Value::as_str)
        .unwrap_or("");
    let source_fingerprint = value
        .get("source_fingerprint")
        .and_then(Value::as_str)
        .unwrap_or("");
    if detail_path.trim().is_empty()
        || list_fingerprint.trim().is_empty()
        || source_fingerprint.trim().is_empty()
    {
        return None;
    }
    Some(ActivityDetailCacheRecord {
        id: activity_detail_cache_id(kind, detail_path),
        kind: kind.to_string(),
        title: title.to_string(),
        detail_path: detail_path.to_string(),
        list_fingerprint: list_fingerprint.to_string(),
        source_fingerprint: source_fingerprint.to_string(),
        checked_at,
    })
}

pub(super) fn upsert_activity_detail_cache(
    records: &mut Vec<ActivityDetailCacheRecord>,
    record: ActivityDetailCacheRecord,
) {
    if let Some(existing) = records.iter_mut().find(|item| item.id == record.id) {
        *existing = record;
    } else {
        records.push(record);
    }
}

pub(super) fn activity_source_fingerprint_for_source_info(value: &Value) -> String {
    if value.get("status").and_then(Value::as_str) == Some("detail_error") {
        return String::new();
    }
    value
        .get("source_fingerprint")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

pub(super) fn activity_detail_snapshot_value(value: &Value) -> Value {
    let status = match value.get("status").and_then(Value::as_str).unwrap_or("") {
        "detail_cached" => "detail",
        other => other,
    };
    json!({
        "status": status,
        "kind": value.get("kind").and_then(Value::as_str).unwrap_or(""),
        "title": value.get("title").and_then(Value::as_str).unwrap_or(""),
        "detailPath": value.get("detail_path").and_then(Value::as_str).unwrap_or(""),
        "listFingerprint": value.get("list_fingerprint").and_then(Value::as_str).unwrap_or(""),
        "sourceFingerprint": value.get("source_fingerprint").and_then(Value::as_str).unwrap_or(""),
    })
}

pub(super) fn artifact_id(kind: &str, filename: &str) -> String {
    format!("{:x}", Sha256::digest(format!("{}|{}", kind, filename)))
}

pub(super) fn artifact_from_download_value(
    kind: &str,
    title: &str,
    filename: &str,
    source_fingerprint: &str,
    value: &Value,
) -> CourseArtifactRecord {
    let path = value
        .get("saved_path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    CourseArtifactRecord {
        id: artifact_id(kind, filename),
        kind: kind.to_string(),
        title: title.to_string(),
        filename: filename.to_string(),
        source_fingerprint: source_fingerprint.to_string(),
        status: if path.is_empty() {
            "external".into()
        } else {
            "downloaded".into()
        },
        path,
        error: String::new(),
    }
}

pub(super) fn failed_artifact(
    kind: &str,
    title: &str,
    filename: &str,
    source_fingerprint: &str,
    error: &str,
) -> CourseArtifactRecord {
    CourseArtifactRecord {
        id: format!("{}:error", artifact_id(kind, filename)),
        kind: kind.to_string(),
        title: title.to_string(),
        filename: filename.to_string(),
        source_fingerprint: source_fingerprint.to_string(),
        status: "error".into(),
        error: error.to_string(),
        ..Default::default()
    }
}

pub(super) fn upsert_artifact(
    artifacts: &mut Vec<CourseArtifactRecord>,
    artifact: CourseArtifactRecord,
) {
    if artifact.status == "downloaded" {
        artifacts.retain(|item| {
            item.status != "error"
                || item.kind != artifact.kind
                || item.filename != artifact.filename
        });
    }
    if let Some(existing) = artifacts.iter_mut().find(|item| item.id == artifact.id) {
        if existing.status != "downloaded" || artifact.status == "downloaded" {
            *existing = artifact;
        }
    } else {
        artifacts.push(artifact);
    }
}

pub(super) fn migrate_legacy_artifacts(status: &mut CourseAutomationStatus) {
    for analysis in &status.document_analyses {
        if analysis.path.is_empty() || !Path::new(&analysis.path).is_file() {
            continue;
        }
        let artifact = CourseArtifactRecord {
            id: artifact_id(&analysis.kind, &analysis.filename),
            kind: analysis.kind.clone(),
            title: analysis.title.clone(),
            filename: analysis.filename.clone(),
            path: analysis.path.clone(),
            source_fingerprint: analysis.source_fingerprint.clone(),
            status: "downloaded".into(),
            error: String::new(),
        };
        if !status
            .artifacts
            .iter()
            .any(|existing| existing.id == artifact.id)
        {
            status.artifacts.push(artifact);
        }
    }
    for path in &status.downloaded_files {
        if !Path::new(path).is_file() {
            continue;
        }
        let filename = Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if filename.is_empty()
            || status.artifacts.iter().any(|artifact| {
                artifact.status == "downloaded"
                    && artifact.filename == filename
                    && artifact.path == *path
            })
        {
            continue;
        }
        status.artifacts.push(CourseArtifactRecord {
            id: artifact_id("legacy", filename),
            kind: "legacy".into(),
            filename: filename.to_string(),
            path: path.clone(),
            status: "downloaded".into(),
            ..Default::default()
        });
    }
}

fn activity_download_identity(kind: &str, filename: &str) -> String {
    format!("identity:{}|{}", kind, filename)
}

pub(super) fn migrate_successful_analysis(
    previous: &DocumentAnalysis,
    document: &AnalysisDocument,
    fingerprint: String,
) -> DocumentAnalysis {
    DocumentAnalysis {
        id: document_id(document),
        fingerprint,
        source_fingerprint: document.source_fingerprint.clone(),
        kind: document.kind.clone(),
        title: document.title.clone(),
        filename: document.filename.clone(),
        path: document.path.clone(),
        ..previous.clone()
    }
}
