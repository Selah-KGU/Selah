//! Luna discussion fetch, new thread, reply, and thread posts.

use super::*;

/// Fetch discussion thread detail (posts list) from Luna
#[tauri::command]
pub async fn luna_fetch_discussion_detail(
    state: State<'_, LunaState>,
    db: State<'_, crate::db::Database>,
    url: String,
) -> Result<luna_parser::LunaDiscussionThread, String> {
    if url.starts_with("http") || !url.starts_with('/') {
        return Err("許可されていないパスです".into());
    }
    let cache_key = format!("luna_disc:{}", url);
    // Same redirect trap as /forums/thread — themetop requires a referer from
    // the owning course top, otherwise Luna 302s to /lms/home.
    let referer_path = {
        let idn = extract_url_param(&url, "idnumber").unwrap_or_default();
        if !idn.is_empty() {
            format!("/lms/course?idnumber={}", idn)
        } else {
            "/lms/home".to_string()
        }
    };
    match luna_http(&state).await {
        Ok(http) => match luna_get_with_referer(&http, &url, &referer_path).await {
            Ok(html) => {
                #[cfg(debug_assertions)]
                {
                    if crate::should_dump_debug_html() {
                        let dump_path = std::env::temp_dir().join(format!(
                            "luna_discussion_{}.html",
                            url.replace(['/', '?', '&'], "_")
                        ));
                        let _ = std::fs::write(&dump_path, &html);
                        log::info!("Discussion HTML dumped ({} bytes)", html.len());
                    }
                }
                if looks_like_luna_home_redirect(&html) {
                    return Err(
                        "Lunaがホーム画面にリダイレクトしました。掲示板ページが見つかりません。"
                            .into(),
                    );
                }
                let data = luna_parser::parse_luna_discussion_thread(&html);
                if let Ok(json) = serde_json::to_string(&data) {
                    let _ = db.save_data_cache(&cache_key, &json);
                }
                Ok(data)
            }
            Err(e) => {
                if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                    if let Ok(cached) = serde_json::from_str(&json) {
                        log::info!("{}: cache fallback ({})", cache_key, e);
                        return Ok(cached);
                    }
                }
                Err(e)
            }
        },
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("{}: cache fallback ({})", cache_key, e);
                    return Ok(cached);
                }
            }
            Err(e)
        }
    }
}

/// Post a new thread to a Luna discussion forum
/// Flow: 1) GET setthread page → extract _csrf (no _cid on this page)
///       2) POST /lms/course/forums/setthread with _method=put and Quill fields
#[tauri::command]
pub async fn luna_post_discussion(
    state: State<'_, LunaState>,
    url: String,
    title: String,
    content: String,
    attachments: Option<Vec<LunaDiscussionUploadFile>>,
) -> Result<String, String> {
    let http = luna_http(&state).await?;

    // Extract idnumber and forumId from the themetop URL
    let idnumber = extract_url_param(&url, "idnumber").ok_or("idnumber が見つかりません")?;
    let forum_id = extract_url_param(&url, "forumId").ok_or("forumId が見つかりません")?;

    // Step 1: Fetch the setthread page to get tokens
    let themetop_path = format!(
        "/lms/course/forums/themetop?idnumber={}&forumId={}",
        idnumber, forum_id
    );
    let setthread_url = format!(
        "/lms/course/forums/setthread?idnumber={}&forumId={}&threadId=&groupId=",
        idnumber, forum_id
    );
    let html = luna_get_with_referer(&http, &setthread_url, &themetop_path).await?;

    log::info!("Setthread HTML fetched ({} bytes)", html.len());
    let upload_files = decode_forum_upload_files(attachments)?;
    if !upload_files.is_empty() && !has_forum_file_upload_support(&html) {
        return Err("この掲示板は添付ファイル投稿に対応していません".into());
    }

    // setthread page only has _csrf (no _cid), plus _method=put
    let cid = extract_input_value(&html, "_cid");
    let csrf = extract_input_value(&html, "_csrf").ok_or_else(|| {
        let has_form = html.contains("<form");
        let has_login = html.contains("linkCommonLogin") && html.contains("login-body");
        format!(
            "_csrf トークンが見つかりません (len={}, has_form={}, login_page={})",
            html.len(),
            has_form,
            has_login
        )
    })?;

    log::info!(
        "New thread: idnumber={}, forumId={}, title={}",
        idnumber,
        forum_id,
        title
    );

    // Build Quill Delta JSON for the content
    let content_json = serde_json::json!({
        "ops": [{"insert": format!("{}\n", content)}]
    })
    .to_string();

    // Step 2: POST with _method=put (Luna emulates PUT via POST)
    // Field names match the actual form: threadContentsText, threadContentsHtml, threadContents
    let mut post_params = Vec::new();
    if let Some(cid) = cid.clone() {
        post_params.push(("_cid".to_string(), cid));
    }
    post_params.extend([
        ("_csrf".to_string(), csrf.clone()),
        ("_method".to_string(), "put".to_string()),
        ("idnumber".to_string(), idnumber.clone()),
        ("forumId".to_string(), forum_id.clone()),
        ("threadId".to_string(), String::new()),
        ("groupId".to_string(), String::new()),
        ("threadTitle".to_string(), title.clone()),
        ("threadContentsText".to_string(), content_json),
        (
            "threadContentsHtml".to_string(),
            format!("<p>{}</p>", html_escape(&content)),
        ),
        ("threadContents".to_string(), content.clone()),
    ]);

    let uploaded_file_ids =
        upload_forum_files(&http, cid.as_deref(), &post_params, &upload_files).await?;
    append_forum_file_fields(&mut post_params, &upload_files, &uploaded_file_ids);

    let resp = if upload_files.is_empty() {
        luna_post(&http, "/lms/course/forums/setthread", &post_params).await?
    } else {
        let form = add_text_fields(reqwest::multipart::Form::new(), &post_params);
        luna_post_multipart(&http, "/lms/course/forums/setthread", form).await?
    };

    if resp.contains("\"success\":false") {
        return Err(format!(
            "投稿失敗: {}",
            crate::client::safe_truncate(&resp, 200)
        ));
    }

    log::info!("New thread submitted successfully");
    Ok("スレッドを登録しました".to_string())
}

/// Reply to an existing thread
/// Flow: 1) GET thread page → extract _cid, _csrf, hidden fields
///       2) POST /lms/course/forums/thread (multipart) with Quill fields
#[tauri::command]
pub async fn luna_reply_discussion(
    state: State<'_, LunaState>,
    url: String,
    content: String,
    parent_post_id: Option<String>,
    attachments: Option<Vec<LunaDiscussionUploadFile>>,
) -> Result<String, String> {
    let http = luna_http(&state).await?;

    // Fetch thread page to get tokens (with Referer from themetop)
    let referer_path = {
        let idn = extract_url_param(&url, "idnumber").unwrap_or_default();
        let fid = extract_url_param(&url, "forumId").unwrap_or_default();
        format!(
            "/lms/course/forums/themetop?idnumber={}&forumId={}",
            idn, fid
        )
    };
    let html = luna_get_with_referer(&http, &url, &referer_path).await?;

    log::info!("Reply HTML fetched ({} bytes)", html.len());
    let upload_files = decode_forum_upload_files(attachments)?;
    if !upload_files.is_empty() && !has_forum_file_upload_support(&html) {
        return Err("この掲示板は添付ファイル投稿に対応していません".into());
    }

    let cid = extract_input_value(&html, "_cid").ok_or_else(|| {
        let has_form = html.contains("<form");
        let has_login = html.contains("linkCommonLogin") && html.contains("login-body");
        format!(
            "_cid トークンが見つかりません (len={}, has_form={}, login_page={})",
            html.len(),
            has_form,
            has_login
        )
    })?;
    let csrf = extract_input_value(&html, "_csrf").ok_or("_csrf トークンが見つかりません")?;
    let idnumber = extract_input_value(&html, "idnumber")
        .or_else(|| extract_url_param(&url, "idnumber"))
        .ok_or("idnumber が見つかりません")?;
    let forum_id = extract_input_value(&html, "forumId")
        .or_else(|| extract_url_param(&url, "forumId"))
        .ok_or("forumId が見つかりません")?;
    let thread_id = extract_input_value(&html, "threadId")
        .or_else(|| extract_url_param(&url, "threadId"))
        .ok_or("threadId が見つかりません")?;

    log::info!(
        "Reply: idnumber={}, forumId={}, threadId={}",
        idnumber,
        forum_id,
        thread_id
    );

    // Extract additional hidden fields from the actual form
    let current_thread =
        extract_input_value(&html, "currentThread").unwrap_or_else(|| "0".to_string());
    let address_type =
        extract_input_value(&html, "forum.addressType").unwrap_or_else(|| "0".to_string());
    let group_id = extract_input_value(&html, "forum.groupId").unwrap_or_default();
    let time_start = extract_input_value(&html, "forum.timeStart").unwrap_or_default();

    let content_json = serde_json::json!({
        "ops": [{"insert": format!("{}\n", content)}]
    })
    .to_string();

    // Build multipart form matching the actual thread page form (enctype="multipart/form-data")
    let mut post_params = vec![
        ("_cid".to_string(), cid.clone()),
        ("_csrf".to_string(), csrf.clone()),
        ("idnumber".to_string(), idnumber),
        ("forumId".to_string(), forum_id),
        ("threadId".to_string(), thread_id),
        ("forum.addressType".to_string(), address_type),
        ("forum.groupId".to_string(), group_id),
        ("forum.timeStart".to_string(), time_start),
        ("currentThread".to_string(), current_thread),
        ("postContentsText".to_string(), content_json),
        (
            "postContentsHtml".to_string(),
            format!("<p>{}</p>", html_escape(&content)),
        ),
        ("postContents".to_string(), content.clone()),
        ("postSendFlag".to_string(), "false".to_string()),
        ("postId".to_string(), String::new()),
        (
            "parentPostId".to_string(),
            parent_post_id.unwrap_or_default(),
        ),
        ("editFlag".to_string(), "1".to_string()),
        ("editAuthority".to_string(), String::new()),
    ];

    let uploaded_file_ids =
        upload_forum_files(&http, Some(&cid), &post_params, &upload_files).await?;
    append_forum_file_fields(&mut post_params, &upload_files, &uploaded_file_ids);

    let form = add_text_fields(reqwest::multipart::Form::new(), &post_params);

    let resp = luna_post_multipart(&http, "/lms/course/forums/thread", form).await?;

    if resp.contains("\"success\":false") {
        return Err(format!(
            "投稿失敗: {}",
            crate::client::safe_truncate(&resp, 200)
        ));
    }

    log::info!("Reply submitted successfully");
    Ok("返信しました".to_string())
}

/// Fetch thread posts (the posts within a specific thread)
/// The thread page has a #threadPostList area loaded via form submit
#[tauri::command]
pub async fn luna_fetch_thread_posts(
    state: State<'_, LunaState>,
    db: State<'_, crate::db::Database>,
    url: String,
) -> Result<luna_parser::LunaDiscussionThread, String> {
    if url.starts_with("http") || !url.starts_with('/') {
        return Err("許可されていないパスです".into());
    }
    let cache_key = format!("luna_thread:{}", url);
    // Luna rejects /forums/thread requests that don't carry a Referer from the
    // matching themetop — it silently 302s to /lms/home (the timetable). Pin the
    // referer to the same idnumber/forumId so the post stream actually loads.
    let referer_path = {
        let idn = extract_url_param(&url, "idnumber").unwrap_or_default();
        let fid = extract_url_param(&url, "forumId").unwrap_or_default();
        if !idn.is_empty() && !fid.is_empty() {
            format!(
                "/lms/course/forums/themetop?idnumber={}&forumId={}",
                idn, fid
            )
        } else {
            "/lms/home".to_string()
        }
    };
    match luna_http(&state).await {
        Ok(http) => match luna_get_with_referer(&http, &url, &referer_path).await {
            Ok(html) => {
                #[cfg(debug_assertions)]
                {
                    if crate::should_dump_debug_html() {
                        let dump_path = std::env::temp_dir().join(format!(
                            "luna_thread_{}.html",
                            url.replace(['/', '?', '&'], "_")
                        ));
                        let _ = std::fs::write(&dump_path, &html);
                    }
                }
                if looks_like_luna_home_redirect(&html) {
                    return Err(
                        "Lunaがホーム画面にリダイレクトしました。掲示板ページが見つかりません。"
                            .into(),
                    );
                }
                let data = luna_parser::parse_luna_thread_detail(&html);
                if let Ok(json) = serde_json::to_string(&data) {
                    let _ = db.save_data_cache(&cache_key, &json);
                }
                Ok(data)
            }
            Err(e) => {
                if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                    if let Ok(cached) = serde_json::from_str(&json) {
                        log::info!("{}: cache fallback ({})", cache_key, e);
                        return Ok(cached);
                    }
                }
                Err(e)
            }
        },
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("{}: cache fallback ({})", cache_key, e);
                    return Ok(cached);
                }
            }
            Err(e)
        }
    }
}
