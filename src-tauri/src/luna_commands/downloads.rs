use super::{
    luna_get, luna_http, SEL_A_HREF, SEL_BODY, SEL_IFRAME_SRC, SEL_META_REFRESH, SEL_SCRIPT,
};
use crate::{config, luna_client, LunaState};
use tauri::State;

/// Luna download: download a file without holding the lock. Returns bytes.
pub(crate) async fn luna_download(http: &reqwest::Client, path: &str) -> Result<Vec<u8>, String> {
    let url = if path.starts_with("http") {
        path.to_string()
    } else {
        format!("{}{}", config::LUNA_BASE, path)
    };

    let mut current_url = url;
    for i in 0..10 {
        let resp = http
            .get(&current_url)
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header("Sec-Fetch-Dest", "document")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-Site", "same-origin")
            .send()
            .await
            .map_err(|e| format!("ダウンロード失敗: {}", e))?;

        let status = resp.status();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string();
        let content_len = resp
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string();
        let content_disp = resp
            .headers()
            .get("content-disposition")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        log::info!(
            "luna_download #{}: status={}, type={}, len={}, disp='{}'",
            i,
            status,
            content_type,
            content_len,
            content_disp
        );

        if status.is_redirection() {
            if let Some(loc) = resp.headers().get("location") {
                let loc_str = loc.to_str().unwrap_or_default();
                current_url = if loc_str.starts_with('/') {
                    format!("{}{}", config::LUNA_BASE, loc_str)
                } else {
                    loc_str.to_string()
                };
                if current_url.contains("sso.kwansei.ac.jp") {
                    return Err(luna_client::LUNA_SESSION_EXPIRED_MSG.into());
                }
                log::info!("luna_download: redirect -> {}", current_url);
                continue;
            }
        }

        if !status.is_success() {
            return Err(format!("HTTP {}", status));
        }

        if content_type.contains("text/html") {
            let text = resp
                .text()
                .await
                .map_err(|e| format!("読み取り失敗: {}", e))?;
            if luna_client::is_luna_session_expired(&text) {
                return Err(luna_client::LUNA_SESSION_EXPIRED_MSG.into());
            }
            return Ok(text.into_bytes());
        }

        return resp
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| format!("ダウンロード読み取り失敗: {}", e));
    }
    Err("リダイレクトが多すぎます".into())
}

/// Save bytes to the download folder with conflict avoidance (appends " (N)" if the file exists).
/// If course_name is provided and classify_by_course is enabled, saves into a course subfolder.
pub(crate) fn save_to_downloads(
    filename: &str,
    bytes: &[u8],
    course_name: Option<&str>,
) -> Result<String, String> {
    let downloads = crate::commands::resolve_download_dir(course_name);
    let _ = std::fs::create_dir_all(&downloads);
    let save_path = downloads.join(filename);

    let final_path = if save_path.exists() {
        let stem = std::path::Path::new(filename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("file");
        let ext = std::path::Path::new(filename)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let mut i = 1;
        loop {
            let name = if ext.is_empty() {
                format!("{} ({})", stem, i)
            } else {
                format!("{} ({}).{}", stem, i, ext)
            };
            let candidate = downloads.join(&name);
            if !candidate.exists() {
                break candidate;
            }
            if i >= 999 {
                return Err("ファイル名の競合を解決できません".into());
            }
            i += 1;
        }
    } else {
        save_path
    };

    std::fs::write(&final_path, bytes).map_err(|e| format!("ファイル保存失敗: {}", e))?;

    let path_str = final_path.to_string_lossy().to_string();
    crate::commands::record_download(filename, &path_str, course_name, "luna", bytes.len() as u64);

    Ok(path_str)
}

/// application/x-www-form-urlencoded: space -> +, encode other special chars.
pub(crate) fn form_encode(s: &str) -> String {
    let mut result = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || "-._~".contains(ch) {
            result.push(ch);
        } else if ch == ' ' {
            result.push('+');
        } else {
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            for b in s.bytes() {
                result.push_str(&format!("%{:02X}", b));
            }
        }
    }
    result
}

async fn prepare_material_tempfile(
    http: &reqwest::Client,
    idnumber: &str,
    file_name: &str,
    object_name: &str,
    resource_id: &str,
    error_prefix: &str,
) -> Result<String, String> {
    let course_url = format!("/lms/course?idnumber={}", idnumber);
    if let Err(e) = luna_get(http, &course_url).await {
        if e == luna_client::LUNA_SESSION_EXPIRED_MSG {
            return Err(e);
        }
        log::warn!("Material tempfile warmup failed (continuing): {}", e);
    }

    let tempfile_query = format!(
        "fileName={}&objectName={}&id={}&idnumber={}",
        urlencoding::encode(file_name),
        urlencoding::encode(object_name),
        urlencoding::encode(resource_id),
        urlencoding::encode(idnumber),
    );
    let tempfile_path = format!("/lms/course/make/tempfile?{}", tempfile_query);
    let tempfile_full_url = format!("{}{}", config::LUNA_BASE, tempfile_path);
    let referer_full_url = format!("{}{}", config::LUNA_BASE, course_url);
    log::info!("Material tempfile URL: {}", tempfile_path);

    let resp = http
        .get(&tempfile_full_url)
        .header("Referer", &referer_full_url)
        .header("X-Requested-With", "XMLHttpRequest")
        .header("Accept", "text/plain, */*; q=0.01")
        .send()
        .await
        .map_err(|e| format!("{}: リクエスト失敗: {}", error_prefix, e))?;

    let status = resp.status();
    let final_url = resp.url().to_string();
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let content_length = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    log::info!(
        "Material tempfile response: status={}, content-type='{}', content-length='{}', final_url='{}'",
        status,
        content_type,
        content_length,
        crate::client::safe_truncate(&final_url, 200)
    );

    if final_url.contains("sso.kwansei.ac.jp") {
        return Err(luna_client::LUNA_SESSION_EXPIRED_MSG.into());
    }
    if !status.is_success() {
        return Err(format!("{}: HTTP {}", error_prefix, status));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| format!("{}: レスポンス読取失敗: {}", error_prefix, e))?;

    if luna_client::is_luna_session_expired(&body) {
        return Err(luna_client::LUNA_SESSION_EXPIRED_MSG.into());
    }

    let file_id = body.trim().to_string();
    log::info!(
        "Material tempfile returned fileId (len={}): '{}'",
        file_id.len(),
        crate::client::safe_truncate(&file_id, 500)
    );

    if file_id.is_empty() {
        return Err(
            "Luna から応答がありませんでした。少し待ってから再度お試しください。セッションが切れている場合は再ログインしてください。".into()
        );
    }
    if file_id.contains('<') {
        return Err(
            "Luna がエラーページを返しました。再ログインのうえ、もう一度お試しください。".into(),
        );
    }

    Ok(file_id)
}

fn build_material_download_query(
    file_name: &str,
    file_id: &str,
    idnumber: &str,
    resource_id: &str,
    content_id: &str,
    end_date: &str,
    title: &str,
) -> String {
    format!(
        "fileName={}&fileId={}&idnumber={}&resourceId={}&screen=1&contentId={}&endDate={}&title={}",
        form_encode(file_name),
        form_encode(file_id),
        form_encode(idnumber),
        form_encode(resource_id),
        form_encode(content_id),
        form_encode(end_date),
        form_encode(title),
    )
}

/// Download a Luna file attachment to the Downloads folder and return the saved path.
///
/// Two modes:
///   1. `url` is non-empty (legacy or direct link): download from URL directly
///   2. `url` is empty but `download_action`/`object_name` provided:
///      re-fetch the detail page via `page_path` to get fresh `_cid` token,
///      then construct the proper form-based download URL.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn luna_download_file(
    state: State<'_, LunaState>,
    url: String,
    filename: String,
    _page_path: Option<String>,
    _object_name: Option<String>,
    download_action: Option<String>,
    download_params: Option<Vec<(String, String)>>,
    course_name: Option<String>,
    _detail_title: Option<String>,
) -> Result<String, String> {
    if url.starts_with("http") {
        return Ok(url);
    }

    let http = luna_http(&state).await?;

    let bytes = if url.is_empty() {
        let action = download_action.as_deref().unwrap_or("");
        if action.is_empty() {
            return Err("ダウンロードURLが見つかりません".into());
        }

        let mut params: Vec<String> = Vec::new();
        if let Some(ref fields) = download_params {
            for (k, v) in fields {
                params.push(format!("{}={}", form_encode(k), form_encode(v)));
            }
        }

        let path_name = make_down_file_name(&filename);
        let download_url = format!("{}/{}?{}", action, path_name, params.join("&"));

        log::info!("Attachment GET: url='{}'", download_url);
        luna_download(&http, &download_url).await?
    } else {
        log::info!(
            "Attachment GET download: url='{}', filename='{}'",
            url,
            filename
        );
        luna_download(&http, &url).await?
    };

    log::info!(
        "Attachment downloaded {} bytes for '{}'",
        bytes.len(),
        filename
    );

    if bytes.is_empty() {
        return Err("ダウンロードされたファイルが空です".into());
    }

    if bytes.len() < 2000 {
        if let Ok(text) = std::str::from_utf8(&bytes) {
            if text.contains("<!DOCTYPE") || text.contains("<html") || text.contains("<HTML") {
                log::error!(
                    "Attachment download returned HTML instead of file: {}",
                    crate::client::safe_truncate(text, 500)
                );
                return Err("サーバーがファイルではなくエラーページを返しました".into());
            }
        }
    }

    save_to_downloads(&filename, &bytes, course_name.as_deref())
}

/// Replicate Luna's CommonUtil.makeDownFileName JS function:
/// replace fullwidth/halfwidth spaces with _, collapse multiple _, then encodeURI
pub(crate) fn make_down_file_name(file_name: &str) -> String {
    let mut result = file_name.replace(['\u{3000}', ' '], "_");
    while result.contains("__") {
        result = result.replace("__", "_");
    }

    let mut encoded = String::new();
    for ch in result.chars() {
        if ch.is_ascii_alphanumeric() || "-_.!~*'()".contains(ch) || ";,/?:@&=+$#".contains(ch) {
            encoded.push(ch);
        } else {
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            for b in s.bytes() {
                encoded.push_str(&format!("%{:02X}", b));
            }
        }
    }
    encoded
}

/// Download a Luna material file (requires tempfile preparation + form-based download)
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn luna_download_material(
    state: State<'_, LunaState>,
    idnumber: String,
    file_name: String,
    object_name: String,
    resource_id: String,
    file_type: String,
    material_id: Option<String>,
    display_name: Option<String>,
    end_date: Option<String>,
    course_name: Option<String>,
    _material_title: Option<String>,
) -> Result<String, String> {
    let http = luna_http(&state).await?;

    log::info!(
        "Material download: file='{}', object='{}', resource='{}', type='{}', matId={:?}",
        file_name,
        object_name,
        resource_id,
        file_type,
        material_id
    );

    let file_id = prepare_material_tempfile(
        &http,
        &idnumber,
        &file_name,
        &object_name,
        &resource_id,
        "ファイル準備失敗",
    )
    .await?;

    let path_encoded_name = make_down_file_name(&file_name);
    let base_path = if file_type == "0" {
        format!("/lms/course/materialref/setfiledown/{}", path_encoded_name)
    } else {
        format!(
            "/lms/course/materialref/sethtmlfiledown/{}",
            path_encoded_name
        )
    };
    let dl_title = display_name.unwrap_or_default();
    let content_id = material_id.unwrap_or_default();
    let title_val = if file_type != "0" { &dl_title } else { "" };
    let end_date_val = end_date.unwrap_or_default();
    let query_string = build_material_download_query(
        &file_name,
        &file_id,
        &idnumber,
        &resource_id,
        &content_id,
        &end_date_val,
        title_val,
    );
    let full_download_url = format!("{}?{}", base_path, query_string);

    log::info!("Material download full URL: {}", full_download_url);

    let bytes = luna_download(&http, &full_download_url).await?;

    log::info!("Material downloaded {} bytes", bytes.len());

    if bytes.len() < 1000 {
        if let Ok(text) = std::str::from_utf8(&bytes) {
            if text.contains("<!DOCTYPE") || text.contains("<html") {
                log::error!(
                    "Download returned HTML instead of file: {}",
                    crate::client::safe_truncate(text, 500)
                );
                return Err("サーバーがファイルではなくエラーページを返しました".into());
            }
        }
    }

    if bytes.is_empty() {
        return Err("ダウンロードされたファイルが空です".into());
    }

    save_to_downloads(&file_name, &bytes, course_name.as_deref())
}

/// Resolve an HTML-type material to its actual external URL.
/// Same tempfile+sethtmlfiledown flow as download, but parses the HTML for the link.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn luna_resolve_material_link(
    state: State<'_, LunaState>,
    idnumber: String,
    file_name: String,
    object_name: String,
    resource_id: String,
    file_type: String,
    material_id: Option<String>,
    display_name: Option<String>,
    end_date: Option<String>,
) -> Result<String, String> {
    let http = luna_http(&state).await?;

    log::info!(
        "Material link resolve: file='{}', resource='{}', type='{}'",
        file_name,
        resource_id,
        file_type
    );

    let file_id = prepare_material_tempfile(
        &http,
        &idnumber,
        &file_name,
        &object_name,
        &resource_id,
        "Failed to prepare tempfile",
    )
    .await?;

    let path_encoded_name = make_down_file_name(&file_name);
    let base_path = format!(
        "/lms/course/materialref/sethtmlfiledown/{}",
        path_encoded_name
    );
    let dl_title = display_name.unwrap_or_default();
    let content_id = material_id.unwrap_or_default();
    let end_date_val = end_date.unwrap_or_default();
    let query_string = build_material_download_query(
        &file_name,
        &file_id,
        &idnumber,
        &resource_id,
        &content_id,
        &end_date_val,
        &dl_title,
    );
    let full_url = format!("{}{}?{}", config::LUNA_BASE, base_path, query_string);

    let resp = http
        .get(&full_url)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    let final_url = resp.url().to_string();
    let html = resp.text().await.unwrap_or_default();

    log::info!(
        "Material link HTML (len={}, final_url={}): {}",
        html.len(),
        final_url,
        crate::client::safe_truncate(&html, 1000)
    );

    if !final_url.contains("luna.kwansei.ac.jp") {
        return Ok(final_url);
    }

    let doc = scraper::Html::parse_document(&html);

    if let Some(meta) = doc.select(&SEL_META_REFRESH).next() {
        if let Some(content) = meta.value().attr("content") {
            if let Some(idx) = content.to_lowercase().find("url=") {
                let url = content[idx + 4..]
                    .trim()
                    .trim_matches(|c| c == '\'' || c == '"');
                if !url.is_empty() {
                    return Ok(url.to_string());
                }
            }
        }
    }

    if let Some(iframe) = doc.select(&SEL_IFRAME_SRC).next() {
        if let Some(src) = iframe.value().attr("src") {
            if src.starts_with("http") {
                return Ok(src.to_string());
            }
        }
    }

    for script in doc.select(&SEL_SCRIPT) {
        let text = script.text().collect::<String>();
        for pattern in &[
            "window.location.href",
            "window.location",
            "location.href",
            "window.open(",
        ] {
            if let Some(idx) = text.find(pattern) {
                let after = &text[idx + pattern.len()..];
                let start = after.find(['\'', '"']);
                if let Some(s) = start {
                    let quote = after.as_bytes()[s] as char;
                    if let Some(e) = after[s + 1..].find(quote) {
                        let url = &after[s + 1..s + 1 + e];
                        if url.starts_with("http") {
                            return Ok(url.to_string());
                        }
                    }
                }
            }
        }
    }

    for a in doc.select(&SEL_A_HREF) {
        if let Some(href) = a.value().attr("href") {
            if href.starts_with("http") && !href.contains("luna.kwansei.ac.jp") {
                return Ok(href.to_string());
            }
        }
    }

    let body_text = doc
        .select(&SEL_BODY)
        .next()
        .map(|b| b.text().collect::<String>().trim().to_string())
        .unwrap_or_default();
    if body_text.starts_with("http") && !body_text.contains(' ') {
        return Ok(body_text);
    }

    Err("リンク先のURLを抽出できませんでした".into())
}
