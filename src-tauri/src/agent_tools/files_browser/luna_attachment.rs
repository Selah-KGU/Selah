//! Resolve and download Luna activity attachments.

use super::*;
use std::collections::HashSet;

pub struct LunaAttachmentResolved {
    title: String,
    pub(super) course_name: String,
    detail_path: String,
    detail_url: String,
    pub(super) attachment: crate::luna_parser::LunaAttachment,
}

pub struct SavedLunaAttachment {
    pub(super) saved_path: String,
}

pub async fn download_resolved_luna_attachment(
    app: &tauri::AppHandle,
    resolved: &LunaAttachmentResolved,
) -> Result<SavedLunaAttachment, String> {
    let bytes = fetch_luna_attachment_bytes(app, &resolved.attachment).await?;
    if bytes.is_empty() {
        return Err("添付データが空です".into());
    }
    let saved_path = crate::luna_commands::save_to_downloads(
        &resolved.attachment.name,
        &bytes,
        Some(&resolved.course_name),
    )?;
    Ok(SavedLunaAttachment { saved_path })
}

async fn fetch_luna_attachment_bytes(
    app: &tauri::AppHandle,
    attachment: &crate::luna_parser::LunaAttachment,
) -> Result<Vec<u8>, String> {
    let luna_state = app.state::<crate::LunaState>();
    let http = {
        let luna = luna_state.session();
        if !luna.has_credentials() {
            return Err(crate::luna_client::LUNA_AUTH_REQUIRED_MSG.into());
        }
        luna.http().clone()
    };

    let download_url = if attachment.url.is_empty() {
        let action = attachment.download_action.as_str();
        if action.is_empty() {
            return Err("添付のダウンロード情報が不足しています".into());
        }
        let params = attachment
            .download_params
            .iter()
            .map(|(k, v)| {
                format!(
                    "{}={}",
                    crate::luna_commands::form_encode(k),
                    crate::luna_commands::form_encode(v)
                )
            })
            .collect::<Vec<_>>()
            .join("&");
        let path_name = crate::luna_commands::make_down_file_name(&attachment.name);
        format!("{}/{}?{}", action, path_name, params)
    } else {
        attachment.url.clone()
    };

    crate::luna_commands::luna_download(&http, &download_url).await
}

async fn resolve_luna_attachment(
    app: &tauri::AppHandle,
    title: &str,
    attachment_name: &str,
) -> Result<LunaAttachmentResolved, String> {
    resolve_luna_attachment_with_lid(app, title, attachment_name, "").await
}

pub async fn resolve_luna_attachment_with_lid(
    app: &tauri::AppHandle,
    title: &str,
    attachment_name: &str,
    luna_id_filter: &str,
) -> Result<LunaAttachmentResolved, String> {
    let db = app.state::<Database>().scope();
    let acts = db.get_all_luna_activities().unwrap_or_default();

    // Filter by luna_id if provided
    let filtered_acts: Vec<_> = if !luna_id_filter.is_empty() {
        acts.into_iter()
            .filter(|a| a.luna_id == luna_id_filter)
            .collect()
    } else {
        acts
    };

    let needle = title.to_lowercase();
    let row = filtered_acts
        .iter()
        .find(|a| a.title == title)
        .or_else(|| {
            filtered_acts
                .iter()
                .find(|a| a.title.to_lowercase().contains(&needle))
        })
        .or_else(|| {
            filtered_acts
                .iter()
                .find(|a| needle.contains(&a.title.to_lowercase()) && !a.title.is_empty())
        })
        .ok_or_else(|| format!("「{}」に一致する活動が見つかりません", title))?;
    if row.detail_path.is_empty() {
        return Err(format!("「{}」には詳細ページのパスがありません", row.title));
    }

    let luna_courses = db.get_luna_courses().unwrap_or_default();
    let course_name = luna_courses
        .iter()
        .find(|c| c.luna_id == row.luna_id)
        .map(|c| c.name.clone())
        .unwrap_or_default();

    let (html, cache_age) = fetch_luna_detail_html_with_age(app, &row.detail_path).await?;
    let detail_url = format!("{}{}", crate::config::LUNA_BASE, row.detail_path);

    let parse_detail = |h: &str| -> crate::luna_parser::LunaDetailPage {
        if row.activity_type == "announcement" {
            crate::luna_parser::parse_luna_announcement_detail(h)
        } else {
            crate::luna_parser::parse_luna_detail_page(h)
        }
    };

    let mut detail = parse_detail(&html);

    // If the cached page yielded no attachments AND the cache is not brand-new,
    // force a fresh fetch — the cache may have been stored before the attachment was uploaded.
    // Skip the re-fetch when age ≤ 60 s: the page was just refreshed and has no attachments.
    if detail.attachments.is_empty() && cache_age > 60 {
        log::debug!(
            "No attachments in cached HTML (age={}s) for '{}', forcing fresh fetch",
            cache_age,
            row.detail_path
        );
        let db = app.state::<Database>().scope();
        let cache_key = format!("luna_detail_html:{}", row.detail_path);
        let _ = db.delete_data_cache(&cache_key);

        match fetch_luna_detail_html_cached(app, &row.detail_path).await {
            Ok(fresh_html) => {
                detail = parse_detail(&fresh_html);
            }
            Err(e) => {
                log::warn!("Fresh fetch failed for '{}': {}", row.detail_path, e);
                // Keep detail as-is (empty attachments) — error will surface below
            }
        }
    }

    let attachment = if attachment_name.is_empty() {
        detail.attachments.first()
    } else {
        let needle = attachment_name.to_lowercase();
        detail
            .attachments
            .iter()
            .find(|a| a.name == attachment_name)
            .or_else(|| {
                detail
                    .attachments
                    .iter()
                    .find(|a| a.name.to_lowercase().contains(&needle))
            })
            .or_else(|| {
                detail
                    .attachments
                    .iter()
                    .find(|a| needle.contains(&a.name.to_lowercase()))
            })
    }
    .cloned()
    .ok_or_else(|| {
        if attachment_name.is_empty() {
            format!("「{}」には開ける添付がありません", row.title)
        } else {
            format!(
                "「{}」の添付「{}」が見つかりません",
                row.title, attachment_name
            )
        }
    })?;

    Ok(LunaAttachmentResolved {
        title: row.title.clone(),
        course_name,
        detail_path: row.detail_path.clone(),
        detail_url,
        attachment,
    })
}

pub async fn download_all_luna_activity_attachments(
    app: &tauri::AppHandle,
    luna_id: &str,
    contents: &crate::luna_parser::LunaCourseContents,
    activity_types: &[&str],
    reusable_paths: &std::collections::HashMap<String, super::ReusableCourseDownload>,
    reusable_details: &std::collections::HashMap<String, super::ReusableActivityDetail>,
    detail_cache_ttl_secs: i64,
    now: i64,
    force_detail_fetch: bool,
) -> Result<Vec<Value>, String> {
    let activities = current_course_activity_sources(luna_id, contents, activity_types);
    let mut results = Vec::new();
    for (activity_type, title, detail_path) in activities {
        let list_fingerprint =
            activity_detail_list_fingerprint(&activity_type, &title, &detail_path)?;
        if !force_detail_fetch {
            if let Some(cached) = reusable_details.get(&detail_path).filter(|cached| {
                cached_activity_detail_is_fresh(
                    cached,
                    &list_fingerprint,
                    now,
                    detail_cache_ttl_secs,
                )
            }) {
                results.push(json!({
                    "status": "detail_cached",
                    "kind": &activity_type,
                    "title": &title,
                    "detail_path": &detail_path,
                    "list_fingerprint": list_fingerprint,
                    "source_fingerprint": &cached.source_fingerprint,
                }));
                continue;
            }
        }
        let html = match fetch_luna_detail_html_fresh(app, &detail_path).await {
            Ok(html) => html,
            Err(error) => {
                if !force_detail_fetch {
                    if let Some(cached) = reusable_details.get(&detail_path).filter(|cached| {
                        cached_activity_detail_matches_source(cached, &list_fingerprint)
                    }) {
                        results.push(json!({
                            "status": "detail_cached",
                            "stale": true,
                            "kind": &activity_type,
                            "title": &title,
                            "detail_path": &detail_path,
                            "list_fingerprint": list_fingerprint,
                            "source_fingerprint": &cached.source_fingerprint,
                            "error": error,
                        }));
                        continue;
                    }
                }
                results.push(json!({
                    "status": "detail_error",
                    "kind": &activity_type,
                    "title": &title,
                    "detail_path": &detail_path,
                    "list_fingerprint": list_fingerprint,
                    "source_fingerprint": list_fingerprint,
                    "error": error,
                }));
                continue;
            }
        };
        let detail = if activity_type == "announcement" {
            crate::luna_parser::parse_luna_announcement_detail(&html)
        } else {
            crate::luna_parser::parse_luna_detail_page(&html)
        };
        let detail_source_fingerprint = course_automation_source_fingerprint(&json!({
            "kind": &activity_type,
            "title": &title,
            "detailPath": &detail_path,
            "sections": &detail.sections,
            "meta": &detail.meta,
            "attachments": &detail.attachments,
        }))?;
        results.push(json!({
            "status": "detail",
            "kind": &activity_type,
            "title": &title,
            "detail_path": &detail_path,
            "list_fingerprint": list_fingerprint,
            "source_fingerprint": detail_source_fingerprint,
            "content": detail.sections.iter()
                .map(|section| format!("{}\n{}", section.heading, section.body))
                .collect::<Vec<_>>()
                .join("\n\n"),
            "meta": &detail.meta,
            "attachments": detail.attachments.iter().map(|attachment| &attachment.name).collect::<Vec<_>>(),
        }));
        for attachment in detail.attachments {
            let course_name = if detail.course_name.trim().is_empty() {
                contents.course_name.clone()
            } else {
                detail.course_name.clone()
            };
            let resolved = LunaAttachmentResolved {
                title: title.clone(),
                course_name,
                detail_path: detail_path.clone(),
                detail_url: format!("{}{}", crate::config::LUNA_BASE, detail_path),
                attachment,
            };
            let source_fingerprint = course_automation_source_fingerprint(&json!({
                "kind": &activity_type,
                "title": &resolved.title,
                "detailPath": &resolved.detail_path,
                "attachment": &resolved.attachment,
            }))?;
            let fingerprint_key = format!("fingerprint:{}", source_fingerprint);
            let identity_key = format!("identity:{}|{}", activity_type, resolved.attachment.name);
            if let Some(reusable) = reusable_paths
                .get(&fingerprint_key)
                .or_else(|| reusable_paths.get(&identity_key))
                .filter(|item| std::path::Path::new(&item.path).is_file())
            {
                let persisted_source_fingerprint = if reusable.source_fingerprint.is_empty() {
                    source_fingerprint.clone()
                } else {
                    reusable.source_fingerprint.clone()
                };
                results.push(json!({
                    "status": "reused",
                    "kind": &activity_type,
                    "title": resolved.title,
                    "filename": resolved.attachment.name,
                    "saved_path": reusable.path,
                    "source_fingerprint": persisted_source_fingerprint,
                }));
                continue;
            }
            match download_resolved_luna_attachment(app, &resolved).await {
                Ok(saved) => results.push(json!({
                    "status": "downloaded",
                    "kind": &activity_type,
                    "title": resolved.title,
                    "filename": resolved.attachment.name,
                    "saved_path": saved.saved_path,
                    "source_fingerprint": source_fingerprint,
                })),
                Err(error) => results.push(json!({
                    "status": "error",
                    "kind": &activity_type,
                    "title": resolved.title,
                    "filename": resolved.attachment.name,
                    "source_fingerprint": source_fingerprint,
                    "error": error,
                })),
            }
        }
    }
    Ok(results)
}

pub fn current_course_activity_sources(
    luna_id: &str,
    contents: &crate::luna_parser::LunaCourseContents,
    activity_types: &[&str],
) -> Vec<(String, String, String)> {
    let mut activities: Vec<(String, String, String)> = Vec::new();
    if activity_types.contains(&"announcement") {
        activities.extend(contents.announcements.iter().map(|announcement| {
            (
                "announcement".to_string(),
                announcement.title.clone(),
                format!(
                    "/lms/coursetop/information/listdetail?idnumber={}&informationId={}",
                    luna_id, announcement.info_id
                ),
            )
        }));
    }
    if activity_types.contains(&"report") {
        activities.extend(
            contents
                .reports
                .iter()
                .filter(|report| !report.url.trim().is_empty())
                .map(|report| {
                    let detail_path = report
                        .url
                        .strip_prefix(crate::config::LUNA_BASE)
                        .unwrap_or(&report.url)
                        .to_string();
                    ("report".to_string(), report.title.clone(), detail_path)
                }),
        );
    }
    let mut seen = HashSet::new();
    activities.retain(|(_, _, path)| seen.insert(path.clone()));
    activities
}

fn activity_detail_list_fingerprint(
    activity_type: &str,
    title: &str,
    detail_path: &str,
) -> Result<String, String> {
    course_automation_source_fingerprint(&json!({
        "kind": activity_type,
        "title": title,
        "detailPath": detail_path,
    }))
}

pub fn cached_activity_detail_is_fresh(
    cached: &super::ReusableActivityDetail,
    list_fingerprint: &str,
    now: i64,
    ttl_secs: i64,
) -> bool {
    ttl_secs > 0
        && cached_activity_detail_matches_source(cached, list_fingerprint)
        && now.saturating_sub(cached.checked_at) < ttl_secs
}

pub fn cached_activity_detail_matches_source(
    cached: &super::ReusableActivityDetail,
    list_fingerprint: &str,
) -> bool {
    !cached.source_fingerprint.trim().is_empty() && cached.list_fingerprint == list_fingerprint
}

fn course_automation_source_fingerprint(value: &Value) -> Result<String, String> {
    use sha2::{Digest, Sha256};

    let raw = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(raw)))
}

pub async fn open_luna_attachment(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let attachment_name = args
        .get("attachment_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if title.is_empty() {
        return Err("titleを指定してください".into());
    }

    let resolved = resolve_luna_attachment(app, title, &attachment_name).await?;
    let attachment = &resolved.attachment;

    if attachment.url.starts_with("http") {
        crate::commands::open_external_url(
            app.clone(),
            attachment.url.clone(),
            Some(attachment.name.clone()),
        )
        .await?;
        return Ok(json!({
            "status": "opened_external",
            "title": resolved.title,
            "attachment_name": attachment.name,
            "url": attachment.url,
            "course": resolved.course_name,
            "source": { "service": "luna", "detail_path": resolved.detail_path, "detail_url": resolved.detail_url },
        }));
    }

    let saved = download_resolved_luna_attachment(app, &resolved).await?;
    let saved_path = saved.saved_path;
    crate::commands::open_downloaded_file(app.clone(), saved_path.clone()).await?;

    Ok(json!({
        "status": "downloaded_and_opened",
        "title": resolved.title,
        "attachment_name": attachment.name,
        "saved_path": saved_path,
        "course": resolved.course_name,
        "source": { "service": "luna", "detail_path": resolved.detail_path, "detail_url": resolved.detail_url },
    }))
}

pub async fn download_luna_attachment(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let attachment_name = args
        .get("attachment_name")
        .or_else(|| args.get("filename"))
        .or_else(|| args.get("file_name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let luna_id = args
        .get("luna_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    if title.is_empty() {
        return Err("titleを指定してください".into());
    }

    let resolved = resolve_luna_attachment_with_lid(app, title, &attachment_name, luna_id).await?;
    let attachment = &resolved.attachment;

    if attachment.url.starts_with("http") {
        return Ok(json!({
            "status": "external_url",
            "title": resolved.title,
            "attachment_name": attachment.name,
            "url": attachment.url,
            "course": resolved.course_name,
            "source": { "service": "luna", "detail_path": resolved.detail_path, "detail_url": resolved.detail_url },
        }));
    }

    let saved = download_resolved_luna_attachment(app, &resolved).await?;
    Ok(json!({
        "status": "downloaded",
        "title": resolved.title,
        "attachment_name": attachment.name,
        "saved_path": saved.saved_path,
        "course": resolved.course_name,
        "source": { "service": "luna", "detail_path": resolved.detail_path, "detail_url": resolved.detail_url },
    }))
}
