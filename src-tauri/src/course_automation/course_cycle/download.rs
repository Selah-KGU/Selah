use super::*;

pub(super) async fn download_course_sources(
    app: &AppHandle,
    luna_id: &str,
    db: &Database,
    config: &CourseAutomationConfig,
    previous: &CourseAutomationStatus,
    force_all: bool,
    run_started_at: i64,
    status: &mut CourseAutomationStatus,
) -> Result<
    (
        Vec<(String, String, String, PathBuf, String)>,
        Vec<AnalysisDocument>,
        Vec<SourceDocumentInfo>,
        String,
    ),
    String,
> {
    let contents = crate::agent_tools::fetch_luna_course_contents(app, luna_id).await?;
    if !contents.course_name.trim().is_empty() {
        status.course_name = contents.course_name.clone();
    }
    let mut source_snapshot = json!({
        "course": &contents.course_name,
        "materials": &contents.materials,
        "announcements": &contents.announcements,
        "reports": &contents.reports,
    });

    status.stage = "downloading".into();
    save_status_and_emit(app, &db, &status)?;
    let mut downloaded = Vec::<(String, String, String, PathBuf, String)>::new();
    let mut artifacts = previous.artifacts.clone();
    let mut activity_detail_cache = previous.activity_detail_cache.clone();
    let mut external_links = Vec::new();
    let mut activity_documents = Vec::new();
    let mut current_source_infos = Vec::<SourceDocumentInfo>::new();
    let mut current_activity_detail_cache_ids = HashSet::new();
    let mut seen_paths = HashSet::new();
    for material in contents
        .materials
        .iter()
        .filter(|_| config.monitor_materials)
    {
        for file in &material.files {
            let filename = if file.file_name.trim().is_empty() {
                file.display_name.trim()
            } else {
                file.file_name.trim()
            };
            if filename.is_empty() {
                continue;
            }
            let source_fingerprint = material_source_fingerprint(file)?;
            current_source_infos.push(SourceDocumentInfo {
                kind: "material".into(),
                title: material.title.clone(),
                filename: filename.to_string(),
                source_fingerprint: source_fingerprint.clone(),
            });
            if let Some((path, persisted_source_fingerprint)) = reusable_artifact_path(
                &artifacts,
                "material",
                &material.title,
                filename,
                &source_fingerprint,
            ) {
                let persisted_source_fingerprint = if persisted_source_fingerprint.is_empty() {
                    source_fingerprint.clone()
                } else {
                    persisted_source_fingerprint
                };
                upsert_artifact(
                    &mut artifacts,
                    CourseArtifactRecord {
                        id: artifact_id("material", filename),
                        kind: "material".into(),
                        title: material.title.clone(),
                        filename: filename.to_string(),
                        path: path.to_string_lossy().to_string(),
                        source_fingerprint: persisted_source_fingerprint.clone(),
                        status: "downloaded".into(),
                        error: String::new(),
                    },
                );
                status.artifacts = artifacts.clone();
                save_status_and_emit(app, &db, &status)?;
                downloaded.push((
                    "material".into(),
                    material.title.clone(),
                    filename.to_string(),
                    path,
                    persisted_source_fingerprint,
                ));
                continue;
            }
            match crate::agent_tools::download_luna_course_material(app, luna_id, filename).await {
                Ok(Some(value)) => {
                    collect_download_value(
                        &mut downloaded,
                        &mut external_links,
                        &mut seen_paths,
                        "material",
                        &material.title,
                        &source_fingerprint,
                        &value,
                    );
                    upsert_artifact(
                        &mut artifacts,
                        artifact_from_download_value(
                            "material",
                            &material.title,
                            filename,
                            &source_fingerprint,
                            &value,
                        ),
                    );
                    status.artifacts = artifacts.clone();
                    save_status_and_emit(app, &db, &status)?;
                }
                Ok(None) => {
                    upsert_artifact(
                        &mut artifacts,
                        failed_artifact(
                            "material",
                            &material.title,
                            filename,
                            &source_fingerprint,
                            "資料ファイルが見つかりません",
                        ),
                    );
                    status.artifacts = artifacts.clone();
                    save_status_and_emit(app, &db, &status)?;
                }
                Err(error) => {
                    log::warn!(
                        "[course_automation] material download failed '{}': {}",
                        filename,
                        error
                    );
                    upsert_artifact(
                        &mut artifacts,
                        failed_artifact(
                            "material",
                            &material.title,
                            filename,
                            &source_fingerprint,
                            &format!("ダウンロード失敗: {}", error),
                        ),
                    );
                    status.artifacts = artifacts.clone();
                    save_status_and_emit(app, &db, &status)?;
                }
            }
        }
    }
    let reusable_activity_paths = reusable_activity_downloads(&artifacts);
    let reusable_activity_details = reusable_activity_details(&activity_detail_cache);
    let force_activity_detail_fetch =
        force_all || has_retryable_activity_artifact_failure(&artifacts);
    let mut activity_kinds = Vec::new();
    if config.monitor_announcements {
        activity_kinds.push("announcement");
    }
    if config.monitor_assignments {
        activity_kinds.push("report");
    }
    let activity_values = if activity_kinds.is_empty() {
        Vec::new()
    } else {
        crate::agent_tools::download_luna_activity_attachments(
            app,
            luna_id,
            &contents,
            &activity_kinds,
            &reusable_activity_paths,
            &reusable_activity_details,
            ACTIVITY_DETAIL_REVALIDATE_AFTER_SECS,
            run_started_at,
            force_activity_detail_fetch,
        )
        .await?
    };
    source_snapshot["activityDetails"] = Value::Array(
        activity_values
            .iter()
            .filter(|value| {
                matches!(
                    value.get("status").and_then(Value::as_str),
                    Some("detail" | "detail_error" | "detail_cached")
                )
            })
            .map(activity_detail_snapshot_value)
            .collect(),
    );
    let fingerprint = sha256_json(&source_snapshot)?;
    for value in activity_values {
        let value_status = value.get("status").and_then(Value::as_str).unwrap_or("");
        let kind = value
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("activity");
        let title = value.get("title").and_then(Value::as_str).unwrap_or("");
        let filename = value
            .get("filename")
            .or_else(|| value.get("attachment_name"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if matches!(value_status, "detail" | "detail_error" | "detail_cached") {
            if let Some(cache_id) = activity_detail_cache_id_from_value(&value) {
                current_activity_detail_cache_ids.insert(cache_id);
            }
        }
        current_source_infos.push(SourceDocumentInfo {
            kind: kind.to_string(),
            title: title.to_string(),
            filename: filename.to_string(),
            source_fingerprint: activity_source_fingerprint_for_source_info(&value),
        });
        if matches!(Some(value_status), Some("downloaded" | "reused" | "error")) {
            let source_fingerprint = value
                .get("source_fingerprint")
                .and_then(Value::as_str)
                .unwrap_or("");
            let artifact = if value.get("status").and_then(Value::as_str) == Some("error") {
                failed_artifact(
                    kind,
                    title,
                    filename,
                    source_fingerprint,
                    value
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("ダウンロード失敗"),
                )
            } else {
                artifact_from_download_value(kind, title, filename, source_fingerprint, &value)
            };
            upsert_artifact(&mut artifacts, artifact);
            status.artifacts = artifacts.clone();
            save_status_and_emit(app, &db, &status)?;
        }
        if value_status == "detail" {
            if let Some(cache_record) =
                activity_detail_cache_record_from_value(&value, run_started_at)
            {
                upsert_activity_detail_cache(&mut activity_detail_cache, cache_record);
            }
            let content = value.get("content").and_then(Value::as_str).unwrap_or("");
            let meta = value.get("meta").cloned().unwrap_or_else(|| json!([]));
            activity_documents.push(AnalysisDocument {
                kind: kind.to_string(),
                title: title.to_string(),
                filename: String::new(),
                path: String::new(),
                content: truncate_chars(&format!("{}\n{}", content, meta), MAX_FILE_TEXT_CHARS),
                source_fingerprint: value
                    .get("source_fingerprint")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                load_error: String::new(),
                images: Vec::new(),
            });
            continue;
        }
        if matches!(Some(value_status), Some("detail_error" | "error")) {
            if value_status == "detail_error" {
                activity_documents.push(failed_analysis_document(
                    kind,
                    title,
                    "",
                    value
                        .get("source_fingerprint")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                    value
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("取得失敗"),
                ));
            }
            continue;
        }
        collect_download_value(
            &mut downloaded,
            &mut external_links,
            &mut seen_paths,
            kind,
            title,
            value
                .get("source_fingerprint")
                .and_then(Value::as_str)
                .unwrap_or(""),
            &value,
        );
    }
    activity_detail_cache.retain(|record| current_activity_detail_cache_ids.contains(&record.id));
    status.activity_detail_cache = activity_detail_cache.clone();
    status.downloaded_files = merge_unique_ids(
        &previous.downloaded_files,
        downloaded
            .iter()
            .map(|(_, _, _, path, _)| path.to_string_lossy().to_string()),
    );
    status.external_links = merge_unique_ids(&previous.external_links, external_links);
    Ok((
        downloaded,
        activity_documents,
        current_source_infos,
        fingerprint,
    ))
}
