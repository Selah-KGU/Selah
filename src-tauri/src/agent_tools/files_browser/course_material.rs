//! Match and download Luna course material files.

use super::*;

#[derive(Clone)]
pub struct MatchedCourseMaterial {
    course_name: String,
    material_title: String,
    pub(super) file: crate::luna_parser::LunaMaterialFile,
}

pub fn effective_material_filename(file: &crate::luna_parser::LunaMaterialFile) -> String {
    if file.file_name.trim().is_empty() {
        file.display_name.trim().to_string()
    } else {
        file.file_name.trim().to_string()
    }
}

fn loose_filename_match(candidate: &str, requested: &str) -> bool {
    let candidate = candidate.trim();
    let requested = requested.trim();
    if candidate.is_empty() || requested.is_empty() {
        return false;
    }
    if candidate == requested {
        return true;
    }
    let candidate_norm = normalize_text(candidate);
    let requested_norm = normalize_text(requested);
    !candidate_norm.is_empty()
        && !requested_norm.is_empty()
        && (candidate_norm == requested_norm
            || candidate_norm.contains(&requested_norm)
            || requested_norm.contains(&candidate_norm))
}

pub fn match_material_file(
    contents: &crate::luna_parser::LunaCourseContents,
    filename: &str,
) -> Option<MatchedCourseMaterial> {
    for material in &contents.materials {
        for file in &material.files {
            let effective_name = effective_material_filename(file);
            let candidates = [
                effective_name.as_str(),
                file.file_name.as_str(),
                file.display_name.as_str(),
                material.title.as_str(),
            ];
            if candidates
                .iter()
                .any(|candidate| loose_filename_match(candidate, filename))
            {
                return Some(MatchedCourseMaterial {
                    course_name: contents.course_name.clone(),
                    material_title: material.title.clone(),
                    file: file.clone(),
                });
            }
        }
    }
    None
}

fn cached_luna_course_contents(
    db: &Database,
    luna_id: &str,
) -> Option<crate::luna_parser::LunaCourseContents> {
    let cache_key = format!("luna_course:{}", luna_id);
    db.get_data_cache(&cache_key)
        .ok()
        .flatten()
        .and_then(|(json_str, _)| serde_json::from_str(&json_str).ok())
}

pub async fn fetch_luna_course_contents_for_download(
    app: &tauri::AppHandle,
    luna_id: &str,
) -> Result<crate::luna_parser::LunaCourseContents, String> {
    let luna_state = app.state::<crate::LunaState>();
    let http = {
        let luna = luna_state.session();
        if !luna.has_credentials() {
            return Err(crate::luna_client::LUNA_AUTH_REQUIRED_MSG.into());
        }
        luna.http().clone()
    };

    let course_url = format!(
        "{}/lms/course?idnumber={}",
        crate::config::LUNA_BASE,
        luna_id
    );
    let contents_url = format!(
        "{}/lms/contents?idnumber={}",
        crate::config::LUNA_BASE,
        luna_id
    );
    let course_html = crate::client::fetch_with_redirect(
        &http,
        &course_url,
        crate::config::LUNA_BASE,
        crate::luna_client::LUNA_SESSION_EXPIRED_MSG,
        crate::luna_client::is_luna_session_expired,
    )
    .await
    .map_err(|e| format!("Luna course取得失敗: {}", e))?;
    let mut contents = crate::luna_parser::parse_luna_course_contents(&course_html, luna_id);

    let contents_html = crate::client::fetch_with_redirect(
        &http,
        &contents_url,
        crate::config::LUNA_BASE,
        crate::luna_client::LUNA_SESSION_EXPIRED_MSG,
        crate::luna_client::is_luna_session_expired,
    )
    .await
    .map_err(|e| format!("Luna contents取得失敗: {}", e))?;
    let (materials, reports, examinations, discussions, surveys) =
        crate::luna_parser::parse_luna_contents_page(&contents_html);
    contents.materials = materials;
    contents.reports = reports;
    contents.examinations = examinations;
    contents.discussions = discussions;
    contents.surveys = surveys;

    if let Ok(json) = serde_json::to_string(&contents) {
        let db = app.state::<Database>().scope();
        let _ = db.save_data_cache(&format!("luna_course:{}", luna_id), &json);
    }
    Ok(contents)
}

async fn find_course_material_file(
    app: &tauri::AppHandle,
    luna_id: &str,
    filename: &str,
) -> Result<Option<MatchedCourseMaterial>, String> {
    let db = app.state::<Database>().scope();
    if let Some(contents) = cached_luna_course_contents(&db, luna_id) {
        if let Some(matched) = match_material_file(&contents, filename) {
            return Ok(Some(matched));
        }
    }

    let fresh = fetch_luna_course_contents_for_download(app, luna_id).await?;
    Ok(match_material_file(&fresh, filename))
}

pub async fn download_course_material_from_contents(
    app: &tauri::AppHandle,
    luna_id: &str,
    filename: &str,
) -> Result<Option<Value>, String> {
    let Some(mut matched) = find_course_material_file(app, luna_id, filename).await? else {
        return Ok(None);
    };
    if matched.course_name.trim().is_empty() {
        let db = app.state::<Database>().scope();
        matched.course_name = db
            .get_luna_courses()
            .unwrap_or_default()
            .into_iter()
            .find(|c| c.luna_id == luna_id)
            .map(|c| c.name)
            .unwrap_or_default();
    }

    let attachment_name = effective_material_filename(&matched.file);
    if !matched.file.external_url.trim().is_empty() {
        return Ok(Some(json!({
            "status": "external_url",
            "filename": attachment_name,
            "display_name": matched.file.display_name,
            "material_title": matched.material_title,
            "url": matched.file.external_url,
            "course": matched.course_name,
            "source": { "service": "luna", "luna_id": luna_id, "kind": "course_material" },
        })));
    }

    let luna_state = app.state::<crate::LunaState>();
    let saved_path = crate::luna_commands::download_luna_material_file(
        luna_state.inner(),
        luna_id,
        &matched.file,
        if matched.course_name.trim().is_empty() {
            None
        } else {
            Some(matched.course_name.as_str())
        },
    )
    .await?;
    Ok(Some(json!({
        "status": "downloaded",
        "filename": attachment_name,
        "display_name": matched.file.display_name,
        "material_title": matched.material_title,
        "saved_path": saved_path,
        "course": matched.course_name,
        "source": { "service": "luna", "luna_id": luna_id, "kind": "course_material" },
    })))
}

pub async fn auto_download_missing_file(
    app: &tauri::AppHandle,
    path: &Path,
) -> Result<PathBuf, String> {
    if path.exists() {
        return Ok(path.to_path_buf());
    }
    log::info!(
        "File not found locally: {:?}. Attempting auto-download...",
        path
    );
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if filename.is_empty() {
        return Err("Filename is empty".into());
    }
    let parent = path.parent();
    let course_dir_name = parent
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or_default();

    let db = app.state::<Database>().scope();
    let acts = db.get_all_luna_activities().unwrap_or_default();

    let mut candidate_acts: Vec<_> = if !course_dir_name.is_empty() {
        let luna_courses = db.get_luna_courses().unwrap_or_default();
        let target_luna_ids: Vec<String> = luna_courses
            .iter()
            .filter(|c| {
                let simplified_db = crate::commands::simplify_course_name(&c.name);
                let simplified_dir = crate::commands::simplify_course_name(course_dir_name);
                simplified_db
                    .to_lowercase()
                    .contains(&simplified_dir.to_lowercase())
                    || simplified_dir
                        .to_lowercase()
                        .contains(&simplified_db.to_lowercase())
            })
            .map(|c| c.luna_id.clone())
            .collect();
        acts.into_iter()
            .filter(|a| target_luna_ids.contains(&a.luna_id))
            .collect()
    } else {
        acts
    };

    if candidate_acts.is_empty() {
        candidate_acts = db.get_all_luna_activities().unwrap_or_default();
    }

    log::info!(
        "Searching across {} candidate Luna activities for attachment '{}'",
        candidate_acts.len(),
        filename
    );

    let mut candidate_luna_ids = Vec::new();
    for act in &candidate_acts {
        if !candidate_luna_ids
            .iter()
            .any(|id: &String| id == &act.luna_id)
        {
            candidate_luna_ids.push(act.luna_id.clone());
        }
    }
    for luna_id in candidate_luna_ids {
        match download_course_material_from_contents(app, &luna_id, filename).await {
            Ok(Some(value)) => {
                if let Some(saved_path) = value.get("saved_path").and_then(|v| v.as_str()) {
                    let saved_p = PathBuf::from(saved_path);
                    if saved_p.exists() {
                        log::info!(
                            "Successfully auto-downloaded missing material file to {:?}",
                            saved_p
                        );
                        return Ok(saved_p);
                    }
                }
            }
            Ok(None) => {}
            Err(e) => log::warn!(
                "Course material auto-download failed for luna_id='{}': {}",
                luna_id,
                e
            ),
        }
    }

    for act in candidate_acts {
        if act.detail_path.is_empty() {
            continue;
        }
        if let Ok(resolved) =
            resolve_luna_attachment_with_lid(app, &act.title, filename, &act.luna_id).await
        {
            log::info!(
                "Found attachment in activity '{}', downloading...",
                act.title
            );
            if let Ok(saved) = download_resolved_luna_attachment(app, &resolved).await {
                let saved_p = PathBuf::from(&saved.saved_path);
                if saved_p.exists() {
                    log::info!("Successfully auto-downloaded missing file to {:?}", saved_p);
                    return Ok(saved_p);
                }
            }
        }
    }

    Err(format!(
        "ファイルが見つかりません。自動ダウンロードも失敗しました: {}",
        filename
    ))
}

pub async fn download_course_material(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let filename = args
        .get("filename")
        .or_else(|| args.get("file_name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if filename.is_empty() {
        return Err("filename（ファイル名）を指定してください".into());
    }
    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let luna_id = args
        .get("luna_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    log::info!(
        "download_course_material: filename='{}', title='{}', luna_id='{}'",
        filename,
        title,
        luna_id
    );

    // When title is provided, use it directly (with luna_id to disambiguate same-title activities
    // across different courses).
    if !title.is_empty() {
        let resolved = resolve_luna_attachment_with_lid(app, title, &filename, luna_id).await?;
        let attachment_name = resolved.attachment.name.clone();
        let saved = download_resolved_luna_attachment(app, &resolved).await?;
        return Ok(json!({
            "status": "downloaded",
            "filename": attachment_name,
            "saved_path": saved.saved_path,
            "course": resolved.course_name,
        }));
    }

    // No title: if luna_id is provided, scan all activities under that course for the attachment.
    if !luna_id.is_empty() {
        let mut material_error = None;
        match download_course_material_from_contents(app, luna_id, &filename).await {
            Ok(Some(value)) => return Ok(value),
            Ok(None) => {}
            Err(e) => {
                log::warn!(
                    "download_course_material: material lookup/download failed for luna_id='{}': {}",
                    luna_id,
                    e
                );
                material_error = Some(e);
            }
        }

        let db = app.state::<Database>().scope();
        let sub_acts: Vec<_> = db
            .get_all_luna_activities()
            .unwrap_or_default()
            .into_iter()
            .filter(|a| a.luna_id == luna_id && !a.detail_path.is_empty())
            .collect();
        for act in sub_acts {
            if let Ok(resolved) =
                resolve_luna_attachment_with_lid(app, &act.title, &filename, luna_id).await
            {
                let attachment_name = resolved.attachment.name.clone();
                let Ok(saved) = download_resolved_luna_attachment(app, &resolved).await else {
                    continue;
                };
                return Ok(json!({
                    "status": "downloaded",
                    "filename": attachment_name,
                    "saved_path": saved.saved_path,
                    "course": resolved.course_name,
                }));
            }
        }
        if let Some(e) = material_error {
            return Err(format!(
                "luna_id='{}'の資料「{}」の確認またはダウンロードに失敗しました: {}",
                luna_id, filename, e
            ));
        }
        return Err(format!(
            "luna_id='{}'の課程内に「{}」の添付が見つかりませんでした",
            luna_id, filename
        ));
    }

    // Last resort: scan all course-material caches, then fall back to the broad activity sweep.
    let db = app.state::<Database>().scope();
    for course in db.get_luna_courses().unwrap_or_default() {
        match download_course_material_from_contents(app, &course.luna_id, &filename).await {
            Ok(Some(value)) => return Ok(value),
            Ok(None) => {}
            Err(e) => log::warn!(
                "download_course_material: broad material lookup failed for luna_id='{}': {}",
                course.luna_id,
                e
            ),
        }
    }
    let path_to_resolve = crate::commands::default_download_dir().join(&filename);
    let resolved_path = auto_download_missing_file(app, &path_to_resolve).await?;
    Ok(json!({
        "status": "downloaded",
        "filename": filename,
        "saved_path": resolved_path.to_string_lossy(),
    }))
}
