use crate::client;
use crate::config;
use crate::luna_client;
use crate::luna_parser;
use crate::LunaState;
use chrono::TimeZone;
use serde::Deserialize;
use std::sync::atomic::AtomicU32;
use std::sync::LazyLock;
use tauri::State;

#[path = "luna_commands/downloads.rs"]
mod downloads;
#[path = "luna_commands/navigation.rs"]
mod navigation;

pub use downloads::*;
pub use navigation::*;

static UNIVERSITY_DETAIL_COUNTER: AtomicU32 = AtomicU32::new(0);
const LUNA_DETAIL_CACHE_VERSION: &str = "v2";
const LUNA_REPORT_DETAIL_CACHE_VERSION: &str = "v1";
const LUNA_ANNOUNCEMENT_CACHE_VERSION: &str = "v2";
const LUNA_DETAIL_RETRY_ATTEMPTS: usize = 3;
const LUNA_FORUM_FILE_MAX_BYTES: usize = 100 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LunaDiscussionUploadFile {
    pub file_name: String,
    pub file_base64: String,
}

// ── Cached selectors (compiled once, reused across all calls) ──
macro_rules! sel {
    ($name:ident, $s:expr) => {
        static $name: LazyLock<scraper::Selector> =
            LazyLock::new(|| scraper::Selector::parse($s).expect(concat!("bad selector: ", $s)));
    };
}
sel!(SEL_META_REFRESH, "meta[http-equiv='refresh']");
sel!(SEL_IFRAME_SRC, "iframe[src]");
sel!(SEL_SCRIPT, "script");
sel!(SEL_A_HREF, "a[href]");
sel!(SEL_BODY, "body");
sel!(SEL_FORM, "form");
sel!(SEL_REPORT_FORM, "form#reportSubmissionForm");
sel!(SEL_HIDDEN_INPUT, "input[type='hidden']");
sel!(SEL_UPDATE_INFO_LIST, ".update-info-list");
sel!(SEL_DETAIL_VERT, ".contents-detail.contents-vertical");
sel!(
    SEL_DETAIL_TITLE,
    "#osiraseTitle, .block-title-txt, .contents-title-txt"
);
sel!(
    SEL_THREAD_POST_MARKER,
    ".thread-post-area, #threadPostListArea, .postContentsText"
);

/// Briefly lock Luna client, check auth and clone http. Releases lock immediately.
async fn luna_http(state: &LunaState) -> Result<reqwest::Client, String> {
    let luna = state.client.lock().await;
    if !luna.authenticated {
        return Err(luna_client::LUNA_AUTH_REQUIRED_MSG.into());
    }
    Ok(luna.http.clone())
}

/// Luna GET: fetch a page without holding the lock.
async fn luna_get(http: &reqwest::Client, path: &str) -> Result<String, String> {
    let url = format!("{}{}", config::LUNA_BASE, path);
    client::fetch_with_redirect(
        http,
        &url,
        config::LUNA_BASE,
        luna_client::LUNA_SESSION_EXPIRED_MSG,
        luna_client::is_luna_session_expired,
    )
    .await
}

fn normalize_detail_title(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !c.is_whitespace()
                && !matches!(
                    c,
                    '|' | '｜' | '【' | '】' | '[' | ']' | '(' | ')' | '（' | '）' | ':' | '：'
                )
        })
        .collect::<String>()
        .to_lowercase()
}

fn title_matches_expected(actual: &str, expected: Option<&str>) -> bool {
    let Some(expected) = expected.map(str::trim).filter(|s| !s.is_empty()) else {
        return true;
    };
    let actual = normalize_detail_title(actual);
    let expected = normalize_detail_title(expected);
    !actual.is_empty()
        && !expected.is_empty()
        && (actual.contains(&expected) || expected.contains(&actual))
}

fn has_detail_payload(data: &luna_parser::LunaDetailPage) -> bool {
    !data.sections.is_empty() || !data.meta.is_empty() || !data.attachments.is_empty()
}

fn is_report_detail_path(path: &str) -> bool {
    path.contains("/lms/course/report/submission")
}

fn is_course_top_path(path: &str) -> bool {
    let raw = path.split('#').next().unwrap_or(path);
    raw == "/lms/course"
        || raw.starts_with("/lms/course?")
        || raw == "/lms/contents"
        || raw.starts_with("/lms/contents?")
}

fn normalize_luna_detail_path(path: &str) -> String {
    path.replace("&amp;", "&")
}

fn finalize_report_detail(
    mut data: luna_parser::LunaDetailPage,
    expected_title: Option<&str>,
) -> luna_parser::LunaDetailPage {
    if data.title.trim().is_empty() {
        if let Some(expected) = expected_title.filter(|s| !s.trim().is_empty()) {
            data.title = expected.to_string();
        }
    }
    data
}

fn has_blacklisted_cached_sections(data: &luna_parser::LunaDetailPage) -> bool {
    data.sections
        .iter()
        .any(|section| crate::luna_parser::is_blacklisted_system_notice_text(&section.body))
}

fn finalize_generic_detail(
    mut data: luna_parser::LunaDetailPage,
    expected_title: Option<&str>,
) -> luna_parser::LunaDetailPage {
    if data.title.trim().is_empty() {
        if let Some(expected) = expected_title.filter(|s| !s.trim().is_empty()) {
            data.title = expected.to_string();
        }
    }
    data
}

fn has_generic_detail_structure(html: &str) -> bool {
    let doc = scraper::Html::parse_document(html);
    let has_title =
        doc.select(&SEL_DETAIL_TITLE).next().is_some() || html.contains("course-title-txt");
    let has_detail_rows = doc.select(&SEL_DETAIL_VERT).next().is_some();
    let has_report_form = doc.select(&SEL_REPORT_FORM).next().is_some();
    let has_forum_post = doc.select(&SEL_THREAD_POST_MARKER).next().is_some();
    let has_downloads = html.contains("downloadFile");
    let has_updates = doc.select(&SEL_UPDATE_INFO_LIST).next().is_some();

    (has_detail_rows || has_report_form || has_forum_post || has_downloads)
        && (has_title || has_report_form || has_forum_post)
        && !(has_updates && !has_detail_rows && !has_report_form && !has_forum_post)
}

fn has_announcement_detail_structure(html: &str) -> bool {
    let doc = scraper::Html::parse_document(html);
    let has_title = doc.select(&SEL_DETAIL_TITLE).next().is_some();
    let has_detail_rows = doc.select(&SEL_DETAIL_VERT).next().is_some();
    has_title && has_detail_rows
}

fn is_valid_generic_detail_response(
    html: &str,
    data: &luna_parser::LunaDetailPage,
    expected_title: Option<&str>,
) -> bool {
    title_matches_expected(&data.title, expected_title)
        && has_detail_payload(data)
        && has_generic_detail_structure(html)
}

fn is_valid_announcement_detail_response(
    html: &str,
    data: &luna_parser::LunaDetailPage,
    expected_title: Option<&str>,
) -> bool {
    title_matches_expected(&data.title, expected_title)
        && has_detail_payload(data)
        && has_announcement_detail_structure(html)
}

fn is_usable_cached_detail_response(
    path: &str,
    data: &luna_parser::LunaDetailPage,
    expected_title: Option<&str>,
) -> bool {
    if has_blacklisted_cached_sections(data) {
        return false;
    }
    if is_report_detail_path(path) {
        return true;
    }
    let _ = expected_title;
    has_detail_payload(data)
}

fn is_soft_usable_generic_detail_response(
    html: &str,
    data: &luna_parser::LunaDetailPage,
    expected_title: Option<&str>,
) -> bool {
    has_detail_payload(data)
        && has_generic_detail_structure(html)
        && (title_matches_expected(&data.title, expected_title) || data.title.trim().is_empty())
}

fn is_soft_usable_announcement_detail_response(
    html: &str,
    data: &luna_parser::LunaDetailPage,
    expected_title: Option<&str>,
) -> bool {
    has_detail_payload(data)
        && has_announcement_detail_structure(html)
        && (title_matches_expected(&data.title, expected_title) || data.title.trim().is_empty())
}

async fn refresh_luna_detail_context(http: &reqwest::Client) {
    let _ = luna_get(http, "/lms/home").await;
}

/// Luna redirects unauthorised / context-less requests to the home page
/// (`<title>時間割</title>` with the timetable grid). Detect that response so
/// the caller can surface a meaningful error instead of parsing zero posts and
/// leaving the renderer stuck on a loading spinner forever.
fn looks_like_luna_home_redirect(html: &str) -> bool {
    let head = html.get(..2048).unwrap_or(html);
    head.contains("<title>時間割</title>")
        || (head.contains("<title>Luna") && html.contains("div-table-data-row"))
}

fn unstable_detail_error_message(kind: &str) -> String {
    match kind {
        "announcement" => {
            "Luna お知らせ詳細の読込が一時的に不安定でした。自動で再取得できなかったため、少し待ってから再度お試しください。".into()
        }
        _ => {
            "Luna 詳細ページの読込が一時的に不安定でした。自動で再取得できなかったため、少し待ってから再度お試しください。".into()
        }
    }
}

/// Luna GET with Referer header — required for form pages that serve CSRF tokens.
async fn luna_get_with_referer(
    http: &reqwest::Client,
    path: &str,
    referer_path: &str,
) -> Result<String, String> {
    let url = format!("{}{}", config::LUNA_BASE, path);
    let referer = format!("{}{}", config::LUNA_BASE, referer_path);
    let mut current_url = url;
    for i in 0..10 {
        let resp = http
            .get(&current_url)
            .header("Referer", &referer)
            .send()
            .await
            .map_err(|e| format!("リクエスト失敗: {}", e))?;
        let status = resp.status();
        if status.is_redirection() {
            if let Some(loc) = resp.headers().get("location") {
                let loc_str = loc.to_str().unwrap_or_default();
                current_url = if loc_str.starts_with('/') {
                    format!("{}{}", config::LUNA_BASE, loc_str)
                } else {
                    loc_str.to_string()
                };
                log::debug!(
                    "luna_get_with_referer redirect #{} -> {}",
                    i + 1,
                    client::safe_truncate(&current_url, 120)
                );
                if current_url.contains("sso.kwansei.ac.jp") {
                    return Err(luna_client::LUNA_SESSION_EXPIRED_MSG.into());
                }
                continue;
            }
        }
        if !status.is_success() {
            return Err(format!("HTTP {}", status));
        }
        let body = resp
            .text()
            .await
            .map_err(|e| format!("レスポンス読取失敗: {}", e))?;
        if luna_client::is_luna_session_expired(&body) {
            return Err(luna_client::LUNA_SESSION_EXPIRED_MSG.into());
        }
        return Ok(body);
    }
    Err("リダイレクトが多すぎます".into())
}

/// Luna POST: submit a form without holding the lock.
async fn luna_post(
    http: &reqwest::Client,
    path: &str,
    params: &[(String, String)],
) -> Result<String, String> {
    let url = format!("{}{}", config::LUNA_BASE, path);
    client::post_form_with_redirect(
        http,
        &url,
        config::LUNA_BASE,
        luna_client::LUNA_SESSION_EXPIRED_MSG,
        luna_client::is_luna_session_expired,
        params.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        &[],
    )
    .await
}

/// Luna POST with a page Referer. Some Luna form endpoints reject otherwise-valid
/// CSRF submissions when the request does not come from the form page.
async fn luna_post_with_referer(
    http: &reqwest::Client,
    path: &str,
    referer_path: &str,
    params: &[(String, String)],
) -> Result<String, String> {
    let url = format!("{}{}", config::LUNA_BASE, path);
    let referer = format!("{}{}", config::LUNA_BASE, referer_path);
    let headers = [("Referer", referer.as_str())];
    client::post_form_with_redirect(
        http,
        &url,
        config::LUNA_BASE,
        luna_client::LUNA_SESSION_EXPIRED_MSG,
        luna_client::is_luna_session_expired,
        params.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        &headers,
    )
    .await
}

/// Luna multipart POST: submit a multipart form without holding the lock.
async fn luna_post_multipart(
    http: &reqwest::Client,
    path: &str,
    form: reqwest::multipart::Form,
) -> Result<String, String> {
    let url = format!("{}{}", config::LUNA_BASE, path);
    let builder = http.post(&url).multipart(form);
    client::send_and_follow_redirect(
        http,
        builder,
        config::LUNA_BASE,
        luna_client::LUNA_SESSION_EXPIRED_MSG,
        luna_client::is_luna_session_expired,
    )
    .await
}

/// Luna multipart POST with _cid appended to URL (mimics Luna's AJAX interceptor).
async fn luna_post_multipart_with_cid(
    http: &reqwest::Client,
    path: &str,
    cid: &str,
    form: reqwest::multipart::Form,
) -> Result<String, String> {
    let url = format!("{}{}?_cid={}", config::LUNA_BASE, path, cid);
    let builder = http.post(&url).multipart(form);
    client::send_and_follow_redirect(
        http,
        builder,
        config::LUNA_BASE,
        luna_client::LUNA_SESSION_EXPIRED_MSG,
        luna_client::is_luna_session_expired,
    )
    .await
}

async fn luna_post_multipart_with_optional_cid(
    http: &reqwest::Client,
    path: &str,
    cid: Option<&str>,
    form: reqwest::multipart::Form,
) -> Result<String, String> {
    if let Some(cid) = cid.filter(|s| !s.is_empty()) {
        luna_post_multipart_with_cid(http, path, cid, form).await
    } else {
        luna_post_multipart(http, path, form).await
    }
}

fn add_text_fields(
    mut form: reqwest::multipart::Form,
    fields: &[(String, String)],
) -> reqwest::multipart::Form {
    for (key, value) in fields {
        form = form.text(key.clone(), value.clone());
    }
    form
}

fn has_forum_file_upload_support(html: &str) -> bool {
    html.contains("/lms/course/forum/thread_file")
        || html.contains("name=\"uploadFiles\"")
        || html.contains("class=\"fileSelectInput")
        || html.contains("files[__index__].fileId")
}

fn validate_forum_file_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("ファイル名が空です".into());
    }
    if trimmed.chars().count() > 60 {
        return Err("ファイル名は60文字以下にしてください".into());
    }
    if trimmed.chars().any(|c| {
        matches!(
            c,
            '\\' | '/' | ':' | '*' | '?' | '<' | '>' | '|' | '"' | '%' | '~' | ';'
        )
    }) {
        return Err("ファイル名に使用できない文字が含まれています".into());
    }
    Ok(())
}

fn decode_forum_upload_files(
    attachments: Option<Vec<LunaDiscussionUploadFile>>,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    use base64::Engine;

    let Some(attachments) = attachments else {
        return Ok(Vec::new());
    };
    if attachments.len() > 10 {
        return Err("添付ファイルは10個以下にしてください".into());
    }

    let mut files = Vec::new();
    for attachment in attachments {
        let file_name = attachment.file_name.trim().to_string();
        validate_forum_file_name(&file_name)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&attachment.file_base64)
            .map_err(|e| format!("Base64デコード失敗: {}", e))?;
        if bytes.is_empty() {
            return Err("ファイルサイズが0バイトです".into());
        }
        if bytes.len() > LUNA_FORUM_FILE_MAX_BYTES {
            return Err(format!(
                "「{}」は最大サイズ（100MB）を超えています。",
                file_name
            ));
        }
        files.push((file_name, bytes));
    }

    Ok(files)
}

async fn upload_forum_files(
    http: &reqwest::Client,
    cid: Option<&str>,
    base_fields: &[(String, String)],
    files: &[(String, Vec<u8>)],
) -> Result<Vec<String>, String> {
    if files.is_empty() {
        return Ok(Vec::new());
    }

    let mut form = add_text_fields(reqwest::multipart::Form::new(), base_fields);
    for (idx, (file_name, bytes)) in files.iter().enumerate() {
        form = form
            .text(format!("files[{}].fileId", idx), "0".to_string())
            .text(format!("files[{}].deleteFlag", idx), "0".to_string())
            .text(format!("files[{}].objectName", idx), String::new())
            .text(format!("files[{}].fileName", idx), file_name.clone())
            .part(
                "uploadFiles",
                reqwest::multipart::Part::bytes(bytes.clone())
                    .file_name(file_name.clone())
                    .mime_str("application/octet-stream")
                    .map_err(|e| format!("MIME error: {}", e))?,
            );
    }

    let upload_resp =
        luna_post_multipart_with_optional_cid(http, "/lms/course/forum/thread_file", cid, form)
            .await?;
    let upload_json: serde_json::Value = serde_json::from_str(&upload_resp).map_err(|e| {
        format!(
            "添付ファイルアップロード応答の解析失敗: {} — body: {}",
            e,
            crate::client::safe_truncate(&upload_resp, 200)
        )
    })?;

    let ids = upload_json
        .as_array()
        .ok_or_else(|| {
            format!(
                "添付ファイルアップロード応答が不正です: {}",
                crate::client::safe_truncate(&upload_resp, 200)
            )
        })?
        .iter()
        .filter_map(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        })
        .collect::<Vec<_>>();

    if ids.len() != files.len() {
        return Err(format!(
            "添付ファイルアップロード数が一致しません (送信={}, 応答={})",
            files.len(),
            ids.len()
        ));
    }

    Ok(ids)
}

fn append_forum_file_fields(
    fields: &mut Vec<(String, String)>,
    files: &[(String, Vec<u8>)],
    file_ids: &[String],
) {
    for (idx, (file_name, _)) in files.iter().enumerate() {
        fields.push((format!("files[{}].fileId", idx), file_ids[idx].clone()));
        fields.push((format!("files[{}].deleteFlag", idx), "0".to_string()));
        fields.push((format!("files[{}].objectName", idx), String::new()));
        fields.push((format!("files[{}].fileName", idx), file_name.clone()));
    }
}

/// Fetch a Luna page, parse it, and cache with fallback.
async fn luna_fetch_cached<T: serde::Serialize + serde::de::DeserializeOwned>(
    state: &State<'_, LunaState>,
    db: &State<'_, crate::db::Database>,
    path: &str,
    cache_key: &str,
    parse: fn(&str) -> T,
) -> Result<T, String> {
    let try_cache = |e: String| -> Result<T, String> {
        if let Ok(Some((json, _))) = db.get_data_cache(cache_key) {
            if let Ok(cached) = serde_json::from_str(&json) {
                log::info!("{}: cache fallback ({})", cache_key, e);
                return Ok(cached);
            }
        }
        Err(e)
    };
    let http = match luna_http(state).await {
        Ok(h) => h,
        Err(e) => return try_cache(e),
    };
    match luna_get(&http, path).await {
        Ok(html) => {
            let data = parse(&html);
            if let Ok(json) = serde_json::to_string(&data) {
                let _ = db.save_data_cache(cache_key, &json);
            }
            Ok(data)
        }
        Err(e) => try_cache(e),
    }
}

/// Escape HTML special characters to prevent XSS in server-side rendered content
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Validate that a string looks like a simple numeric/alphanumeric ID
fn is_safe_param(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 20
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Fetch a Luna page (generic)
#[tauri::command]
pub async fn luna_fetch_page(state: State<'_, LunaState>, path: String) -> Result<String, String> {
    // Only allow known Luna paths
    if path.contains("://") || !path.starts_with('/') {
        return Err("許可されていないパスです".into());
    }
    let allowed_prefixes = [
        "/top",
        "/lms/",
        "/course/",
        "/notification",
        "/updateinfo",
        "/message",
        "/attend",
        "/report",
        "/survey",
        "/material",
    ];
    if !allowed_prefixes.iter().any(|p| path.starts_with(p)) {
        return Err("許可されていないパスです".into());
    }
    let http = luna_http(&state).await?;
    luna_get(&http, &path).await
}

/// Check if Luna session is valid
#[tauri::command]
pub async fn luna_check_session(state: State<'_, LunaState>) -> Result<bool, String> {
    let (http, authenticated) = {
        let luna = state.client.lock().await;
        (luna.http.clone(), luna.authenticated)
    };
    if !authenticated {
        return Ok(false);
    }
    // Validate against server without holding the lock
    let url = format!("{}/lms/timetable", crate::config::LUNA_BASE);
    match crate::client::fetch_with_redirect(
        &http,
        &url,
        crate::config::LUNA_BASE,
        crate::luna_client::LUNA_SESSION_EXPIRED_MSG,
        crate::luna_client::is_luna_session_expired,
    )
    .await
    {
        Ok(_) => {
            let luna = state.client.lock().await;
            luna.save_session();
            Ok(true)
        }
        Err(e) if e == crate::luna_client::LUNA_SESSION_EXPIRED_MSG => {
            let mut luna = state.client.lock().await;
            luna.authenticated = false;
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

/// Fetch parsed TODO list
#[tauri::command]
pub async fn luna_fetch_todo(
    state: State<'_, LunaState>,
    db: State<'_, crate::db::Database>,
) -> Result<Vec<luna_parser::LunaTodoItem>, String> {
    luna_fetch_cached(
        &state,
        &db,
        "/lms/todo",
        "luna_todo",
        luna_parser::parse_luna_todo,
    )
    .await
}

/// Fetch parsed notifications
#[tauri::command]
pub async fn luna_fetch_updates(
    state: State<'_, LunaState>,
    db: State<'_, crate::db::Database>,
) -> Result<Vec<luna_parser::LunaNotification>, String> {
    luna_fetch_cached(
        &state,
        &db,
        "/updateinfo",
        "luna_updates",
        luna_parser::parse_luna_notifications,
    )
    .await
}

/// Fetch course content page
#[tauri::command]
pub async fn luna_fetch_course_content(
    state: State<'_, LunaState>,
    idnumber: String,
) -> Result<String, String> {
    if !is_safe_param(&idnumber) {
        return Err("無効なパラメータです".into());
    }
    let http = luna_http(&state).await?;
    let path = format!("/lms/contents?idnumber={}", idnumber);
    luna_get(&http, &path).await
}

/// Fetch and parse a Luna detail page (any path)
#[tauri::command]
pub async fn luna_fetch_detail(
    state: State<'_, LunaState>,
    db: State<'_, crate::db::Database>,
    path: String,
    expected_title: Option<String>,
) -> Result<luna_parser::LunaDetailPage, String> {
    let path = normalize_luna_detail_path(&path);
    // Reject absolute URLs and enforce known Luna path prefixes
    if path.starts_with("http") || !path.starts_with('/') {
        return Err("許可されていないパスです".into());
    }
    if is_course_top_path(&path) {
        return Err(
            "Lunaの授業トップURLです。詳細ページではなくコース画面として開いてください。".into(),
        );
    }
    let is_report = is_report_detail_path(&path);
    let cache_key = if is_report {
        format!(
            "luna_report_detail:{}:{}",
            LUNA_REPORT_DETAIL_CACHE_VERSION, path
        )
    } else {
        format!("luna_detail:{}:{}", LUNA_DETAIL_CACHE_VERSION, path)
    };
    let expected_title = expected_title.as_deref();
    match luna_http(&state).await {
        Ok(http) => {
            let fetch = async {
                let mut html = luna_get(&http, &path).await?;
                let mut data = luna_parser::parse_luna_detail_page(&html);
                if is_report {
                    data = finalize_report_detail(data, expected_title);
                    #[cfg(debug_assertions)]
                    {
                        if crate::should_dump_debug_html() {
                            let filename = path.replace(['/', '?', '&'], "_");
                            let dump_path = std::env::temp_dir()
                                .join(format!("luna_report_detail{}.html", filename));
                            let _ = std::fs::write(&dump_path, &html);
                            log::info!(
                                "Luna report detail HTML dumped to {} ({} bytes)",
                                dump_path.display(),
                                html.len()
                            );
                        }
                    }
                    return Ok(data);
                }
                let mut accepted_soft = false;
                if !is_valid_generic_detail_response(&html, &data, expected_title) {
                    for attempt in 1..LUNA_DETAIL_RETRY_ATTEMPTS {
                        log::warn!(
                            "Luna detail page for '{}' looked unstable on attempt {}/{} (title='{}', sections={}, meta={}, attachments={}), refreshing and retrying",
                            path,
                            attempt,
                            LUNA_DETAIL_RETRY_ATTEMPTS,
                            data.title,
                            data.sections.len(),
                            data.meta.len(),
                            data.attachments.len()
                        );
                        refresh_luna_detail_context(&http).await;
                        tokio::time::sleep(std::time::Duration::from_millis(250 * attempt as u64))
                            .await;
                        html = luna_get(&http, &path).await?;
                        data = luna_parser::parse_luna_detail_page(&html);
                        if is_valid_generic_detail_response(&html, &data, expected_title) {
                            break;
                        }
                    }
                    if !is_valid_generic_detail_response(&html, &data, expected_title) {
                        if is_soft_usable_generic_detail_response(&html, &data, expected_title) {
                            log::warn!(
                                "Luna detail page for '{}' still looked atypical after retries; accepting soft-valid payload",
                                path
                            );
                            accepted_soft = true;
                            data = finalize_generic_detail(data, expected_title);
                        } else {
                            return Err(unstable_detail_error_message("detail"));
                        }
                    }
                }
                if !accepted_soft {
                    data = finalize_generic_detail(data, expected_title);
                }

                #[cfg(debug_assertions)]
                {
                    if crate::should_dump_debug_html() {
                        let filename = path.replace(['/', '?', '&'], "_");
                        let dump_path =
                            std::env::temp_dir().join(format!("luna_detail{}.html", filename));
                        let _ = std::fs::write(&dump_path, &html);
                        log::info!(
                            "Luna detail HTML dumped to {} ({} bytes)",
                            dump_path.display(),
                            html.len()
                        );
                    }
                }
                Ok(data)
            };

            match fetch.await {
                Ok(data) => {
                    if let Ok(json) = serde_json::to_string(&data) {
                        let _ = db.save_data_cache(&cache_key, &json);
                    }
                    Ok(data)
                }
                Err(e) => {
                    if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                        if let Ok(cached) = serde_json::from_str(&json) {
                            if is_report {
                                log::info!("luna_report_detail: cache fallback ({})", e);
                                return Ok(finalize_report_detail(cached, expected_title));
                            }
                            if is_usable_cached_detail_response(&path, &cached, expected_title) {
                                log::info!("luna_detail: cache fallback ({})", e);
                                return Ok(cached);
                            }
                            log::warn!("luna_detail: ignored stale/invalid cache fallback");
                        }
                    }
                    Err(e)
                }
            }
        }
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    if is_report {
                        log::info!("luna_report_detail: cache fallback ({})", e);
                        return Ok(finalize_report_detail(cached, expected_title));
                    }
                    if is_usable_cached_detail_response(&path, &cached, expected_title) {
                        log::info!("luna_detail: cache fallback ({})", e);
                        return Ok(cached);
                    }
                    log::warn!("luna_detail: ignored stale/invalid cache fallback");
                }
            }
            Err(e)
        }
    }
}

/// Fetch announcement detail from Luna course page
#[tauri::command]
pub async fn luna_fetch_announcement_detail(
    state: State<'_, LunaState>,
    db: State<'_, crate::db::Database>,
    idnumber: String,
    info_id: String,
    expected_title: Option<String>,
) -> Result<luna_parser::LunaDetailPage, String> {
    if !is_safe_param(&idnumber) || !is_safe_param(&info_id) {
        return Err("無効なパラメータです".into());
    }
    let cache_key = format!(
        "luna_announce:{}:{}:{}",
        LUNA_ANNOUNCEMENT_CACHE_VERSION, idnumber, info_id
    );
    let expected_title = expected_title.as_deref();
    let path = format!(
        "/lms/coursetop/information/listdetail?idnumber={}&informationId={}",
        idnumber, info_id
    );
    match luna_http(&state).await {
        Ok(http) => {
            let fetch = async {
                let mut html = luna_get(&http, &path).await?;
                let mut data = luna_parser::parse_luna_announcement_detail(&html);

                if !is_valid_announcement_detail_response(&html, &data, expected_title) {
                    for attempt in 1..LUNA_DETAIL_RETRY_ATTEMPTS {
                        log::warn!(
                            "Luna announcement detail for '{}:{}' looked unstable on attempt {}/{} (title='{}', sections={}, meta={}, attachments={}), refreshing and retrying",
                            idnumber,
                            info_id,
                            attempt,
                            LUNA_DETAIL_RETRY_ATTEMPTS,
                            data.title,
                            data.sections.len(),
                            data.meta.len(),
                            data.attachments.len()
                        );
                        refresh_luna_detail_context(&http).await;
                        tokio::time::sleep(std::time::Duration::from_millis(250 * attempt as u64))
                            .await;
                        html = luna_get(&http, &path).await?;
                        data = luna_parser::parse_luna_announcement_detail(&html);
                        if is_valid_announcement_detail_response(&html, &data, expected_title) {
                            break;
                        }
                    }
                    if !is_valid_announcement_detail_response(&html, &data, expected_title) {
                        if is_soft_usable_announcement_detail_response(&html, &data, expected_title)
                        {
                            log::warn!(
                                "Luna announcement detail for '{}:{}' still looked atypical after retries; accepting soft-valid payload",
                                idnumber,
                                info_id
                            );
                            data = finalize_generic_detail(data, expected_title);
                        } else {
                            return Err(unstable_detail_error_message("announcement"));
                        }
                    }
                }
                data = finalize_generic_detail(data, expected_title);

                #[cfg(debug_assertions)]
                {
                    if crate::should_dump_debug_html() {
                        let dump_path = std::env::temp_dir()
                            .join(format!("luna_announcement_{}_{}.html", idnumber, info_id));
                        let _ = std::fs::write(&dump_path, &html);
                        log::info!("Luna announcement detail dumped ({} bytes)", html.len());
                    }
                }
                Ok(data)
            };

            match fetch.await {
                Ok(data) => {
                    if let Ok(json) = serde_json::to_string(&data) {
                        let _ = db.save_data_cache(&cache_key, &json);
                    }
                    Ok(data)
                }
                Err(e) => {
                    if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                        if let Ok(cached) = serde_json::from_str(&json) {
                            let cache_path = format!(
                                "/lms/coursetop/information/listdetail?idnumber={}&informationId={}",
                                idnumber, info_id
                            );
                            if is_usable_cached_detail_response(
                                &cache_path,
                                &cached,
                                expected_title,
                            ) {
                                log::info!("luna_announce: cache fallback ({})", e);
                                return Ok(cached);
                            }
                            log::warn!("luna_announce: ignored stale/invalid cache fallback");
                        }
                    }
                    Err(e)
                }
            }
        }
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    let cache_path = format!(
                        "/lms/coursetop/information/listdetail?idnumber={}&informationId={}",
                        idnumber, info_id
                    );
                    if is_usable_cached_detail_response(&cache_path, &cached, expected_title) {
                        log::info!("luna_announce: cache fallback ({})", e);
                        return Ok(cached);
                    }
                    log::warn!("luna_announce: ignored stale/invalid cache fallback");
                }
            }
            Err(e)
        }
    }
}

/// Fetch and parse a Luna survey detail page
#[tauri::command]
pub async fn luna_fetch_survey_detail(
    state: State<'_, LunaState>,
    db: State<'_, crate::db::Database>,
    path: String,
) -> Result<luna_parser::LunaSurveyDetail, String> {
    if path.starts_with("http") || !path.starts_with('/') {
        return Err("許可されていないパスです".into());
    }
    let cache_key = format!("luna_survey:{}", path);
    match luna_http(&state).await {
        Ok(http) => match luna_get(&http, &path).await {
            Ok(html) => {
                #[cfg(debug_assertions)]
                {
                    if crate::should_dump_debug_html() {
                        let filename = path.replace(['/', '?', '&'], "_");
                        let dump_path =
                            std::env::temp_dir().join(format!("luna_survey{}.html", filename));
                        let _ = std::fs::write(&dump_path, &html);
                        log::info!(
                            "Luna survey detail dumped to {} ({} bytes)",
                            dump_path.display(),
                            html.len()
                        );
                    }
                }
                let data = luna_parser::parse_luna_survey_detail(&html);
                if let Ok(json) = serde_json::to_string(&data) {
                    let _ = db.save_data_cache(&cache_key, &json);
                }
                Ok(data)
            }
            Err(e) => {
                if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                    if let Ok(cached) = serde_json::from_str(&json) {
                        log::info!("luna_survey: cache fallback ({})", e);
                        return Ok(cached);
                    }
                }
                Err(e)
            }
        },
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("luna_survey: cache fallback ({})", e);
                    return Ok(cached);
                }
            }
            Err(e)
        }
    }
}

/// Fetch and parse a Luna inquiry (お問い合わせ / メッセージ) detail page.
///
/// Inquiry URLs come in two flavours:
///   /lms/course/inquiry/post?idnumber=X&inquiryId=Y         — direct
///   /lms/course/inquiry/firstSet?idnumber=X&inquiryId=Y     — landing page that
///     auto-redirects via JS. We can request `post` directly with the same params.
#[tauri::command]
pub async fn luna_fetch_inquiry_detail(
    state: State<'_, LunaState>,
    db: State<'_, crate::db::Database>,
    path: String,
) -> Result<luna_parser::LunaInquiryDetail, String> {
    if path.starts_with("http") || !path.starts_with('/') {
        return Err("許可されていないパスです".into());
    }
    let cache_key = format!("luna_inquiry:{}", path);
    // Same redirect trap as the forum endpoints: the inquiry page bounces to
    // /lms/home unless Referer points at the owning course top.
    let referer_path = {
        let idn = extract_url_param(&path, "idnumber").unwrap_or_default();
        if !idn.is_empty() {
            format!("/lms/course?idnumber={}", idn)
        } else {
            "/lms/home".to_string()
        }
    };
    match luna_http(&state).await {
        Ok(http) => match luna_get_with_referer(&http, &path, &referer_path).await {
            Ok(html) => {
                if looks_like_luna_home_redirect(&html) {
                    return Err(
                        "Lunaがホーム画面にリダイレクトしました。メッセージページが見つかりません。"
                            .into(),
                    );
                }
                #[cfg(debug_assertions)]
                {
                    if crate::should_dump_debug_html() {
                        let filename = path.replace(['/', '?', '&'], "_");
                        let dump_path =
                            std::env::temp_dir().join(format!("luna_inquiry{}.html", filename));
                        let _ = std::fs::write(&dump_path, &html);
                        log::info!(
                            "Luna inquiry detail dumped to {} ({} bytes)",
                            dump_path.display(),
                            html.len()
                        );
                    }
                }
                let data = luna_parser::parse_luna_inquiry_detail(&html);
                if let Ok(json) = serde_json::to_string(&data) {
                    let _ = db.save_data_cache(&cache_key, &json);
                }
                Ok(data)
            }
            Err(e) => {
                if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                    if let Ok(cached) = serde_json::from_str(&json) {
                        log::info!("luna_inquiry: cache fallback ({})", e);
                        return Ok(cached);
                    }
                }
                Err(e)
            }
        },
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("luna_inquiry: cache fallback ({})", e);
                    return Ok(cached);
                }
            }
            Err(e)
        }
    }
}

/// Upload a single attachment for an inquiry reply.
///
/// Inquiry pages bundle ONE file slot per post (unlike forum threads which take
/// `files[N]`), so attachments are uploaded individually and the resulting
/// fileId / fileName are echoed back to be inserted into the reply form.
async fn upload_inquiry_attachment(
    http: &reqwest::Client,
    upload_action: &str,
    csrf: &str,
    idnumber: &str,
    file: &LunaDiscussionUploadFile,
) -> Result<(String, String, String), String> {
    use base64::Engine;

    let file_name = file.file_name.trim().to_string();
    validate_forum_file_name(&file_name)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&file.file_base64)
        .map_err(|e| format!("Base64デコード失敗: {}", e))?;
    if bytes.is_empty() {
        return Err("ファイルサイズが0バイトです".into());
    }
    if bytes.len() > LUNA_FORUM_FILE_MAX_BYTES {
        return Err(format!(
            "「{}」は最大サイズ（100MB）を超えています。",
            file_name
        ));
    }

    let form = reqwest::multipart::Form::new()
        .text("_csrf", csrf.to_string())
        .text("idnumber", idnumber.to_string())
        .part(
            "uploadFile",
            reqwest::multipart::Part::bytes(bytes)
                .file_name(file_name.clone())
                .mime_str("application/octet-stream")
                .map_err(|e| format!("MIME error: {}", e))?,
        );

    let resp = luna_post_multipart(http, upload_action, form).await?;

    // Luna echoes JSON like { "success": true, "fileId": "...", "fileName": "...", "scanStatus": "..." }.
    // Shape inferred from the forum/report upload responses — adjust if Luna's real reply differs.
    let json: serde_json::Value = serde_json::from_str(&resp).map_err(|e| {
        format!(
            "添付ファイルアップロード応答の解析失敗: {} — body: {}",
            e,
            crate::client::safe_truncate(&resp, 200)
        )
    })?;

    if json.get("success").and_then(|v| v.as_bool()) == Some(false) {
        let msg = json
            .get("message")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| crate::client::safe_truncate(&resp, 200).to_string());
        return Err(format!("アップロード失敗: {}", msg));
    }

    let file_id = json
        .get("fileId")
        .and_then(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        })
        .ok_or("fileId が見つかりません")?;
    let returned_name = json
        .get("fileName")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| file_name.clone());
    let scan_status = json
        .get("scanStatus")
        .and_then(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        })
        .unwrap_or_default();

    Ok((file_id, returned_name, scan_status))
}

/// Post a reply to a Luna inquiry (お問い合わせ / メッセージ) thread.
///
/// Flow:
///   1. GET the inquiry page → parse hidden form fields (_csrf, idnumber,
///      inquiryId, inquiryPosts.inquiry.title, inquiryPosts.inquiry.authorId).
///   2. If an attachment is present, upload it via `inquiry_upfile` and capture
///      the returned fileId / fileName.
///   3. POST the reply form to `inquirySetForm`'s action (typically
///      `/lms/course/inquiry/postSet`) with the comment text/html and any
///      uploaded file references.
#[tauri::command]
pub async fn luna_reply_inquiry(
    state: State<'_, LunaState>,
    url: String,
    content: String,
    attachment: Option<LunaDiscussionUploadFile>,
) -> Result<String, String> {
    if url.starts_with("http") || !url.starts_with('/') {
        return Err("許可されていないパスです".into());
    }
    let content = content.trim().to_string();
    if content.is_empty() && attachment.is_none() {
        return Err("内容または添付ファイルが必要です".into());
    }

    let http = luna_http(&state).await?;
    let html = luna_get(&http, &url).await?;
    let data = luna_parser::parse_luna_inquiry_detail(&html);

    if data.post_action.is_empty() {
        return Err("メッセージ送信フォームが見つかりません".into());
    }

    let csrf = data
        .post_form_fields
        .iter()
        .find(|(k, _)| k == "_csrf")
        .map(|(_, v)| v.clone())
        .ok_or("_csrf トークンが見つかりません")?;
    let idnumber = if !data.idnumber.is_empty() {
        data.idnumber.clone()
    } else {
        extract_url_param(&url, "idnumber").unwrap_or_default()
    };
    if idnumber.is_empty() {
        return Err("idnumber が見つかりません".into());
    }

    // Optional attachment upload (single file slot).
    let uploaded = if let Some(file) = attachment.as_ref() {
        if data.upload_action.is_empty() {
            return Err("このメッセージは添付ファイル送信に対応していません".into());
        }
        Some(upload_inquiry_attachment(&http, &data.upload_action, &csrf, &idnumber, file).await?)
    } else {
        None
    };

    let content_html = if content.is_empty() {
        String::new()
    } else {
        format!("<p>{}</p>", html_escape(&content))
    };
    let content_delta = if content.is_empty() {
        String::new()
    } else {
        serde_json::json!({ "ops": [{"insert": format!("{}\n", content)}] }).to_string()
    };

    // Start from the parsed hidden fields so anything Luna adds in the future
    // (e.g. a new authorId variant) flows through automatically.
    let mut post_params: Vec<(String, String)> = data.post_form_fields.clone();

    let mut set_field = |key: &str, value: String| {
        if let Some(slot) = post_params.iter_mut().find(|(k, _)| k == key) {
            slot.1 = value;
        } else {
            post_params.push((key.to_string(), value));
        }
    };

    set_field("inquiryCommentText", content_delta);
    set_field("inquiryCommentHtml", content_html);
    set_field("inquiryComment", content.clone());
    set_field("clickedButton", "send".to_string());

    if let Some((file_id, file_name, scan_status)) = uploaded {
        set_field("inputFileId", file_id);
        set_field("inputFileName", file_name.clone());
        set_field("originalFileName", file_name);
        set_field("scanStatus", scan_status);
    }

    let form = add_text_fields(reqwest::multipart::Form::new(), &post_params);
    let resp = luna_post_multipart(&http, &data.post_action, form).await?;

    if resp.contains("\"success\":false") {
        return Err(format!(
            "送信失敗: {}",
            crate::client::safe_truncate(&resp, 200)
        ));
    }

    log::info!("Inquiry reply submitted ({} bytes response)", resp.len());
    Ok("メッセージを送信しました".to_string())
}

/// Submit survey answers to Luna
#[tauri::command]
pub async fn luna_submit_survey(
    state: State<'_, LunaState>,
    form_fields: Vec<(String, String)>,
    answers: std::collections::HashMap<String, serde_json::Value>,
    submit_path: Option<String>,
    referer_path: Option<String>,
) -> Result<(), String> {
    // Build the full POST params: hidden fields + user answers
    let mut params: Vec<(String, String)> = Vec::new();

    // Add all hidden form fields (includes _cid, _csrf, idnumber, surveyId, takeFlag,
    // answer[N].surveyNo, answer[N].surveyNoSub, answerDetail[N].*, enableSurveyItems[N])
    for (k, v) in &form_fields {
        params.push((k.clone(), v.clone()));
    }

    // Merge user answers: answers map is {questionIndex: selectedValue | selectedValues[]}
    for (idx_str, value) in &answers {
        let idx: usize = idx_str.parse().map_err(|_| "無効な質問インデックスです")?;
        let (answer_name, answer_value) = survey_answer_payload(idx, value);
        let values = survey_answer_values(answer_value, !answer_name.is_empty());
        for (item_idx, answer_value) in values.iter().enumerate() {
            let field_name = survey_answer_field_name(&answer_name, idx, item_idx);
            // Replace existing empty field or add new one
            let mut found = false;
            for p in &mut params {
                if p.0 == field_name {
                    p.1 = answer_value.clone();
                    found = true;
                    break;
                }
            }
            if !found {
                params.push((field_name, answer_value.clone()));
            }
        }
    }

    let http = luna_http(&state).await?;
    let submit_path = normalize_luna_relative_path(
        submit_path
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("/lms/course/surveys/take"),
    )?;
    let referer_path = normalize_luna_relative_path(
        referer_path
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&submit_path),
    )?;
    log::debug!(
        "luna_submit_survey: posting to {} with referer {} ({} fields)",
        client::safe_truncate(&submit_path, 120),
        client::safe_truncate(&referer_path, 120),
        params.len()
    );
    let response = luna_post_with_referer(&http, &submit_path, &referer_path, &params).await?;

    if let Some(error) = detect_survey_submit_error(&response) {
        return Err(error);
    }

    Ok(())
}

fn normalize_luna_relative_path(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let path = if let Some(rest) = trimmed.strip_prefix(config::LUNA_BASE) {
        rest
    } else {
        trimmed
    };
    if path.contains("://") || path.contains('\n') || path.contains('\r') || !path.starts_with('/')
    {
        return Err("許可されていないパスです".into());
    }
    Ok(path.to_string())
}

fn detect_survey_submit_error(response: &str) -> Option<String> {
    if response.contains("回答期間を過ぎている") {
        return Some("回答期間を過ぎています".to_string());
    }
    if response.contains("answer-type-") && response.contains("-error") {
        let text = html_to_compact_text(response);
        for marker in ["入力してください", "選択してください", "文字以内", "エラー"]
        {
            if text.contains(marker) {
                return Some(format!("回答を送信できませんでした: {}", marker));
            }
        }
        if response.contains("回答する") && response.contains("survey_question_subblock") {
            return Some("回答を送信できませんでした。入力内容を確認してください".to_string());
        }
    }
    if response.contains("survey_question_subblock") && response.contains("answer-btn") {
        return Some("回答が受け付けられませんでした。入力内容を確認してください".to_string());
    }
    None
}

fn html_to_compact_text(html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn survey_answer_payload(idx: usize, value: &serde_json::Value) -> (String, &serde_json::Value) {
    if let serde_json::Value::Object(obj) = value {
        let name = obj
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("answer[{}].answerItem[0].answer", idx));
        let answer_value = obj.get("value").unwrap_or(value);
        return (name, answer_value);
    }
    (format!("answer[{}].answerItem[0].answer", idx), value)
}

fn survey_answer_values(value: &serde_json::Value, keep_empty: bool) -> Vec<String> {
    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(|s| s.to_string()))
            .filter(|s| keep_empty || !s.is_empty())
            .collect(),
        serde_json::Value::String(s) if keep_empty || !s.is_empty() => vec![s.clone()],
        serde_json::Value::Number(n) => vec![n.to_string()],
        serde_json::Value::Bool(b) => vec![b.to_string()],
        _ => Vec::new(),
    }
}

fn survey_answer_field_name(base_name: &str, idx: usize, item_idx: usize) -> String {
    if item_idx == 0 {
        return base_name.to_string();
    }
    let marker = ".answerItem[";
    if let Some(start) = base_name.find(marker) {
        let after_start = start + marker.len();
        if let Some(end_rel) = base_name[after_start..].find(']') {
            let end = after_start + end_rel;
            return format!(
                "{}{}{}",
                &base_name[..after_start],
                item_idx,
                &base_name[end..]
            );
        }
    }
    format!("answer[{}].answerItem[{}].answer", idx, item_idx)
}

/// Prefetch the attendance send form to extract time-window metadata
/// (送信可能日時, 遅刻時間, 内容) without submitting.
#[tauri::command]
pub async fn luna_prefetch_attendance_form(
    state: State<'_, LunaState>,
    idnumber: String,
    attendance_id: String,
) -> Result<serde_json::Value, String> {
    if !is_safe_param(&idnumber) || !is_safe_param(&attendance_id) {
        return Err("無効なパラメータです".into());
    }

    let http = luna_http(&state).await?;
    let send_path = format!(
        "/lms/course/attendances/send?idnumber={}&attendanceId={}",
        idnumber, attendance_id
    );
    let referer_path = format!("/lms/course?idnumber={}#attendance", idnumber);

    let html = match luna_get_with_referer(&http, &send_path, &referer_path).await {
        Ok(body) => body,
        Err(_) => luna_get(&http, &send_path).await?,
    };

    if html.contains("登録期間外") {
        return Err("登録期間外です".into());
    }
    if html.contains("登録済") || html.contains("出席済") {
        return Ok(serde_json::json!({ "already_registered": true }));
    }

    // Parse the contents-detail blocks to extract time info
    let doc = scraper::Html::parse_document(&html);
    sel!(SEL_DETAIL_BLOCK, ".contents-detail.contents-vertical");
    sel!(SEL_HEADER_TXT, ".contents-header.contents-header-txt");
    sel!(SEL_INPUT_AREA, ".contents-input-area");

    let mut open_start = String::new();
    let mut open_end = String::new();
    let mut late_start = String::new();
    let mut late_end = String::new();
    let mut content_text = String::new();

    for block in doc.select(&SEL_DETAIL_BLOCK) {
        let header_text = block
            .select(&SEL_HEADER_TXT)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let spans: Vec<String> = block
            .select(&SEL_INPUT_AREA)
            .next()
            .map(|area| {
                area.children()
                    .filter_map(|n| {
                        n.value().as_element().and_then(|e| {
                            if e.name() == "span" {
                                Some(
                                    scraper::ElementRef::wrap(n)
                                        .map(|er| er.text().collect::<String>().trim().to_string())
                                        .unwrap_or_default(),
                                )
                            } else {
                                None
                            }
                        })
                    })
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        if header_text.contains("送信可能日時") || header_text.contains("ログイン期間")
        {
            if !spans.is_empty() {
                open_start = spans[0].clone();
            }
            if spans.len() >= 3 {
                open_end = spans[2].clone();
            }
        } else if header_text.contains("遅刻時間") {
            if !spans.is_empty() {
                late_start = spans[0].clone();
            }
            if spans.len() >= 3 {
                late_end = spans[2].clone();
            }
        } else if header_text.contains("内容") && !header_text.contains("パスワード") {
            let area_text = block
                .select(&SEL_INPUT_AREA)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            if !area_text.is_empty() {
                content_text = area_text;
            }
        }
    }

    Ok(serde_json::json!({
        "already_registered": false,
        "open_start": open_start,
        "open_end": open_end,
        "late_start": late_start,
        "late_end": late_end,
        "content": content_text,
    }))
}

/// Submit attendance registration (出席登録)
/// Flow: GET send page -> parse hidden form -> POST submit (up to 2 rounds)
#[tauri::command]
pub async fn luna_submit_attendance(
    state: State<'_, LunaState>,
    idnumber: String,
    attendance_id: String,
    one_time_pass: Option<String>,
    comment: Option<String>,
) -> Result<String, String> {
    if !is_safe_param(&idnumber) || !is_safe_param(&attendance_id) {
        return Err("無効なパラメータです".into());
    }

    let http = luna_http(&state).await?;
    let send_path = format!(
        "/lms/course/attendances/send?idnumber={}&attendanceId={}",
        idnumber, attendance_id
    );
    let referer_path = format!("/lms/course?idnumber={}#attendance", idnumber);

    let mut html = match luna_get_with_referer(&http, &send_path, &referer_path).await {
        Ok(body) => body,
        Err(_) => luna_get(&http, &send_path).await?,
    };

    if html.contains("登録期間外") {
        return Err("登録期間外です".into());
    }
    if html.contains("登録済") || html.contains("出席済") {
        return Ok("すでに登録済みです".into());
    }

    for _ in 0..2 {
        if html.contains("完了") || html.contains("登録しました") || html.contains("登録済")
        {
            return Ok("出席を登録しました".into());
        }

        let (action, mut fields) = match extract_form_fields(&html, "/attendances") {
            Some(v) => v,
            None => break,
        };
        if fields.is_empty() {
            break;
        }

        if let Some(pass) = one_time_pass.as_ref() {
            upsert_field(&mut fields, "oneTimePass", pass.clone());
        }
        if let Some(cmt) = comment.as_ref() {
            upsert_field(&mut fields, "comment", cmt.clone());
        }

        if let Some(current_pass) = field_value(&fields, "oneTimePass") {
            if current_pass.trim().is_empty() {
                return Err("出席パスワードを入力してください".into());
            }
        }

        let submit_url = if action.starts_with("http") {
            action.clone()
        } else {
            format!("{}{}", config::LUNA_BASE, action)
        };

        let referer = format!("{}{}", config::LUNA_BASE, send_path);
        let builder = http
            .post(&submit_url)
            .header("Referer", &referer)
            .form(&fields);
        html = client::send_and_follow_redirect(
            &http,
            builder,
            config::LUNA_BASE,
            luna_client::LUNA_SESSION_EXPIRED_MSG,
            luna_client::is_luna_session_expired,
        )
        .await?;
    }

    if html.contains("完了") || html.contains("登録しました") || html.contains("登録済")
    {
        Ok("出席を登録しました".into())
    } else if html.contains("登録期間外") {
        Err("登録期間外です".into())
    } else {
        Err("出席登録フォームを完了できませんでした".into())
    }
}

/// Fetch and parse course top page (/lms/course?idnumber=XXX)
#[tauri::command]
pub async fn luna_fetch_course_detail(
    state: State<'_, LunaState>,
    db: State<'_, crate::db::Database>,
    idnumber: String,
) -> Result<luna_parser::LunaCourseContents, String> {
    if !is_safe_param(&idnumber) {
        return Err("無効なパラメータです".into());
    }
    let cache_key = format!("luna_course:{}", idnumber);
    let http = match luna_http(&state).await {
        Ok(h) => h,
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("luna_course: cache fallback ({})", e);
                    return Ok(cached);
                }
            }
            return Err(e);
        }
    };

    let course_path = format!("/lms/course?idnumber={}", idnumber);
    let contents_path = format!("/lms/contents?idnumber={}", idnumber);

    // Fetch course top page — Luna sometimes returns an incomplete/redirect page
    // on the very first access after session restore, so we retry once if menus are empty.
    let course_html = match luna_get(&http, &course_path).await {
        Ok(html) => html,
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("luna_course: cache fallback ({})", e);
                    return Ok(cached);
                }
            }
            return Err(e);
        }
    };
    let mut result = luna_parser::parse_luna_course_contents(&course_html, &idnumber);

    if result.menus.is_empty() {
        log::warn!(
            "Course page for {} returned no menus ({}B), retrying...",
            idnumber,
            course_html.len()
        );
        #[cfg(debug_assertions)]
        {
            if crate::should_dump_debug_html() {
                let dump =
                    std::env::temp_dir().join(format!("luna_course_{}_initial.html", idnumber));
                let _ = std::fs::write(&dump, &course_html);
            }
        }
        // Retry: the first request may have warmed up the Luna session/course state
        if let Ok(retry_html) = luna_get(&http, &course_path).await {
            let retry_result = luna_parser::parse_luna_course_contents(&retry_html, &idnumber);
            if !retry_result.menus.is_empty() {
                log::info!("Retry succeeded for course {}", idnumber);
                result = retry_result;
            }
            #[cfg(debug_assertions)]
            {
                if crate::should_dump_debug_html() {
                    let dump = std::env::temp_dir().join(format!("luna_course_{}.html", idnumber));
                    let _ = std::fs::write(&dump, &retry_html);
                }
            }
        }
    } else {
        #[cfg(debug_assertions)]
        {
            if crate::should_dump_debug_html() {
                let dump = std::env::temp_dir().join(format!("luna_course_{}.html", idnumber));
                let _ = std::fs::write(&dump, &course_html);
            }
        }
    }

    // Fetch contents top page (actual content items)
    let contents_html = match luna_get(&http, &contents_path).await {
        Ok(html) => html,
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("luna_course: cache fallback (contents fetch: {})", e);
                    return Ok(cached);
                }
            }
            return Err(e);
        }
    };

    #[cfg(debug_assertions)]
    {
        if crate::should_dump_debug_html() {
            let dump_path = std::env::temp_dir().join(format!("luna_contents_{}.html", idnumber));
            let _ = std::fs::write(&dump_path, &contents_html);
        }
    }

    // Merge actual content items from contents page
    let (materials, reports, examinations, discussions, surveys) =
        luna_parser::parse_luna_contents_page(&contents_html);
    result.materials = materials;
    result.reports = reports;
    result.examinations = examinations;
    result.discussions = discussions;
    result.surveys = surveys;

    // Cache the complete result
    if let Ok(json) = serde_json::to_string(&result) {
        let _ = db.save_data_cache(&cache_key, &json);
    }

    Ok(result)
}

/// Detect report submission type by fetching the submission page
/// Returns "text", "file", or "both"
#[tauri::command]
pub async fn luna_check_report_type(
    state: State<'_, LunaState>,
    idnumber: String,
    report_id: String,
    period: Option<String>,
) -> Result<String, String> {
    let http = luna_http(&state).await?;
    let url = format!(
        "/lms/course/report/submission?idnumber={}&reportId={}",
        idnumber, report_id
    );
    let html = luna_get(&http, &url).await?;

    let has_textarea =
        html.contains("id=\"submissionText\"") || html.contains("name=\"submissionText\"");
    // File upload: look for file input or drag-and-drop area
    let has_file = html.contains("id=\"uploadFile\"")
        || html.contains("name=\"uploadFile\"")
        || html.contains("type=\"file\"")
        || html.contains("dragAndDrop");

    if !has_textarea && !has_file {
        if let Some(message) = report_submission_unavailable_message(&html, period.as_deref()) {
            return Err(message);
        }
    }

    let result = match (has_textarea, has_file) {
        (true, true) => "both",
        (true, false) => "text",
        (false, true) => "file",
        (false, false) => "file", // default fallback
    };
    log::info!(
        "Report type detection: idnumber={}, reportId={}, textarea={}, file={} → {}",
        idnumber,
        report_id,
        has_textarea,
        has_file,
        result
    );
    Ok(result.into())
}

/// Submit a report (課題提出) to Luna
/// Flow: 1) GET submission page → extract _cid, _csrf
///       2) POST /lms/course/report/upload (multipart) → get fileId
///       3) POST /lms/course/report/submission → confirm
#[tauri::command]
pub async fn luna_submit_report(
    state: State<'_, LunaState>,
    idnumber: String,
    report_id: String,
    period: Option<String>,
    file_name: String,
    file_base64: String,
) -> Result<String, String> {
    use base64::Engine;
    let http = luna_http(&state).await?;

    // Decode base64 file data
    let file_bytes = base64::engine::general_purpose::STANDARD
        .decode(&file_base64)
        .map_err(|e| format!("Base64デコード失敗: {}", e))?;

    log::info!(
        "Report submission: idnumber={}, reportId={}, file={} ({}B)",
        idnumber,
        report_id,
        file_name,
        file_bytes.len()
    );

    // Step 1: Fetch the submission page to get _cid and _csrf tokens
    let submission_url = format!(
        "/lms/course/report/submission?idnumber={}&reportId={}",
        idnumber, report_id
    );
    let page_html = luna_get(&http, &submission_url).await?;

    let cid = extract_report_token(&page_html, "_cid", period.as_deref())?;
    let csrf = extract_report_token(&page_html, "_csrf", period.as_deref())?;

    log::info!(
        "Report tokens: _cid={}..., _csrf={}...",
        crate::client::safe_truncate(&cid, 8),
        crate::client::safe_truncate(&csrf, 8)
    );

    // Step 2: Upload file via multipart POST (AJAX endpoint — _cid goes in URL)
    let upload_form = reqwest::multipart::Form::new()
        .text("_cid", cid.clone())
        .text("_csrf", csrf.clone())
        .text("method", "0".to_string())
        .text("idnumber", idnumber.clone())
        .text("reportId", report_id.clone())
        .part(
            "uploadFile",
            reqwest::multipart::Part::bytes(file_bytes)
                .file_name(file_name.clone())
                .mime_str("application/octet-stream")
                .map_err(|e| format!("MIME error: {}", e))?,
        );

    let upload_resp =
        luna_post_multipart_with_cid(&http, "/lms/course/report/upload", &cid, upload_form).await?;

    let upload_json: serde_json::Value = serde_json::from_str(&upload_resp).map_err(|e| {
        format!(
            "アップロード応答の解析失敗: {} — body: {}",
            e,
            crate::client::safe_truncate(&upload_resp, 200)
        )
    })?;

    if upload_json.get("success").and_then(|v| v.as_bool()) != Some(true) {
        let msg = upload_json
            .get("message")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_else(|| crate::client::safe_truncate(&upload_resp, 200).to_string());
        return Err(format!("アップロード失敗: {}", msg));
    }

    let file_id = upload_json
        .get("fileId")
        .and_then(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        })
        .ok_or("fileId が見つかりません")?;

    log::info!("Report file uploaded: fileId={}", file_id);

    // Step 3: Submit to confirmation page (url-encoded form POST)
    // The browser JS clears file inputs before submit, so only text fields are sent.
    // Include fileName (comment field) as empty since the original form has it.
    let submit_params = [
        ("_cid", cid.clone()),
        ("_csrf", csrf.clone()),
        ("method", "0".to_string()),
        ("idnumber", idnumber.clone()),
        ("reportId", report_id.clone()),
        ("fileId[0]", file_id.clone()),
        ("originalFileName[0]", file_name.clone()),
        ("deleteFlag[0]", "0".to_string()),
        ("rowCounter", "1".to_string()),
        ("fileName", "".to_string()),
    ];

    let submit_url = format!("{}/lms/course/report/submission", config::LUNA_BASE);
    let raw_resp = http
        .post(&submit_url)
        .form(&submit_params)
        .send()
        .await
        .map_err(|e| format!("確認画面リクエスト失敗: {}", e))?;

    let step3_status = raw_resp.status();
    let step3_url = raw_resp.url().to_string();
    let step3_location = raw_resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let step3_content_type = raw_resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    log::info!(
        "Step 3 raw: status={}, url={}, location={:?}, content-type={:?}",
        step3_status,
        client::safe_truncate(&step3_url, 120),
        step3_location,
        step3_content_type
    );

    let confirm_html = if step3_status.is_redirection() {
        if let Some(loc) = &step3_location {
            let next_url = if loc.starts_with('/') {
                format!("{}{}", config::LUNA_BASE, loc)
            } else {
                loc.clone()
            };
            log::info!(
                "Step 3 redirect -> {}",
                client::safe_truncate(&next_url, 120)
            );
            client::fetch_with_redirect(
                &http,
                &next_url,
                config::LUNA_BASE,
                luna_client::LUNA_SESSION_EXPIRED_MSG,
                luna_client::is_luna_session_expired,
            )
            .await?
        } else {
            raw_resp
                .text()
                .await
                .map_err(|e| format!("レスポンス読取失敗: {}", e))?
        }
    } else {
        raw_resp
            .text()
            .await
            .map_err(|e| format!("レスポンス読取失敗: {}", e))?
    };

    #[cfg(debug_assertions)]
    {
        if crate::should_dump_debug_html() {
            let dump_path = std::env::temp_dir().join("luna_report_confirm.html");
            let _ = std::fs::write(&dump_path, &confirm_html);
            log::info!(
                "Report confirm page dumped to {} ({} bytes)",
                dump_path.display(),
                confirm_html.len()
            );
        }
    }

    if confirm_html.is_empty() {
        return Err("確認画面が空です。セッションが切れている可能性があります。".into());
    }

    // Step 4: Parse confirmation page and submit the registration form
    let (register_action, register_fields) = {
        let confirm_doc = scraper::Html::parse_document(&confirm_html);
        let confirm_cid = extract_input_value(&confirm_html, "_cid").unwrap_or_else(|| cid.clone());
        let confirm_csrf = extract_input_value(&confirm_html, "_csrf").unwrap_or(csrf);

        // Find the reportSubmissionForm specifically (not other forms on the page)
        let mut action = String::new();
        let mut fields: Vec<(String, String)> = Vec::new();

        if let Some(form_el) = confirm_doc.select(&SEL_REPORT_FORM).next() {
            if let Some(a) = form_el.value().attr("action") {
                action = a.to_string();
            }
            for input_el in form_el.select(&SEL_HIDDEN_INPUT) {
                let name = input_el.value().attr("name").unwrap_or_default();
                let value = input_el.value().attr("value").unwrap_or_default();
                if !name.is_empty() {
                    // JS changes _method from "post" to "put" when clicking "登録する"
                    if name == "_method" {
                        fields.push(("_method".to_string(), "put".to_string()));
                    } else {
                        fields.push((name.to_string(), value.to_string()));
                    }
                }
            }
        }

        // Fallback: find form by action
        if action.is_empty() {
            for form_el in confirm_doc.select(&SEL_FORM) {
                if let Some(a) = form_el.value().attr("action") {
                    if a.contains("/report/submission") && !a.contains("download") {
                        action = a.to_string();
                        break;
                    }
                }
            }
        }

        if fields.is_empty() {
            fields = vec![
                ("_cid".into(), confirm_cid),
                ("_csrf".into(), confirm_csrf),
                ("_method".into(), "put".into()),
                ("method".into(), "0".into()),
                ("idnumber".into(), idnumber.clone()),
                ("reportId".into(), report_id.clone()),
                ("submissionText".into(), "".into()),
                ("dragAndDrop".into(), "false".into()),
            ];
        }

        log::info!(
            "Step 4 fields: {:?}",
            fields
                .iter()
                .map(|(k, v)| format!("{}={}", k, client::safe_truncate(v, 20)))
                .collect::<Vec<_>>()
        );

        (action, fields)
    }; // confirm_doc dropped here

    if register_action.is_empty() {
        // Maybe the confirmation page submitted directly — check for success indicators
        if confirm_html.contains("提出が完了") || confirm_html.contains("提出済") {
            log::info!("Report submitted directly (no confirmation step)");
            return Ok(format!("「{}」を提出しました", file_name));
        }
        log::warn!("No registration form found on confirmation page");
        return Err("確認画面に登録フォームが見つかりません。dump を確認してください。".into());
    }

    log::info!(
        "Report confirm form action: {}, fields: {}",
        register_action,
        register_fields.len()
    );

    let register_url = format!("{}{}", config::LUNA_BASE, register_action);
    let raw_resp4 = http
        .post(&register_url)
        .form(&register_fields)
        .send()
        .await
        .map_err(|e| format!("登録リクエスト失敗: {}", e))?;

    let step4_status = raw_resp4.status();
    let step4_url = raw_resp4.url().to_string();
    let step4_location = raw_resp4
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let step4_content_type = raw_resp4
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    log::info!(
        "Step 4 raw: status={}, url={}, location={:?}, content-type={:?}",
        step4_status,
        client::safe_truncate(&step4_url, 120),
        step4_location,
        step4_content_type
    );

    let register_resp = if step4_status.is_redirection() {
        if let Some(loc) = &step4_location {
            let next_url = if loc.starts_with('/') {
                format!("{}{}", config::LUNA_BASE, loc)
            } else {
                loc.clone()
            };
            log::info!(
                "Step 4 redirect -> {}",
                client::safe_truncate(&next_url, 120)
            );
            client::fetch_with_redirect(
                &http,
                &next_url,
                config::LUNA_BASE,
                luna_client::LUNA_SESSION_EXPIRED_MSG,
                luna_client::is_luna_session_expired,
            )
            .await?
        } else {
            raw_resp4
                .text()
                .await
                .map_err(|e| format!("レスポンス読取失敗: {}", e))?
        }
    } else {
        raw_resp4
            .text()
            .await
            .map_err(|e| format!("レスポンス読取失敗: {}", e))?
    };

    #[cfg(debug_assertions)]
    {
        if crate::should_dump_debug_html() {
            let dump_path2 = std::env::temp_dir().join("luna_report_register_result.html");
            let _ = std::fs::write(&dump_path2, &register_resp);
            log::info!(
                "Report register response dumped to {} ({} bytes)",
                dump_path2.display(),
                register_resp.len()
            );
        }
    }

    // Verify: the result page should show completion
    if register_resp.contains("提出が完了") || register_resp.contains("完了しました") {
        log::info!("Report registration confirmed by response content");
        Ok(format!("「{}」を提出しました", file_name))
    } else if register_resp.is_empty() {
        // Some Luna actions return empty on success redirect
        log::info!("Report registration response empty, verifying...");

        // Re-fetch the original page to verify
        let verify_html = luna_get(&http, &submission_url).await?;
        #[cfg(debug_assertions)]
        {
            if crate::should_dump_debug_html() {
                let dump_path3 = std::env::temp_dir().join("luna_report_verify.html");
                let _ = std::fs::write(&dump_path3, &verify_html);
            }
        }

        // Check for "既に提出済みの成果物" section containing actual files
        // NOT just "提出済" in comments or the file_name which might match the user's name
        let has_submitted_section = verify_html.contains("submittedFile")
            || verify_html.contains("既に提出済みの成果物</")  // closed tag means content follows
            || {
                // Check if the submitted artifacts section has content (not just empty comments)
                if let Some(pos) = verify_html.find("既に提出済みの成果物") {
                    let after = &verify_html[pos..std::cmp::min(pos + 500, verify_html.len())];
                    after.contains("downloadFile") || after.contains("file-name")
                } else {
                    false
                }
            };

        if has_submitted_section {
            log::info!("Report submitted and verified via re-fetch");
            Ok(format!("「{}」を提出しました", file_name))
        } else {
            log::warn!("Report submission verification failed — no submitted files found in re-fetched page");
            Ok(format!("「{}」を提出しました（未確認）", file_name))
        }
    } else {
        log::info!(
            "Report registration completed (response {} bytes)",
            register_resp.len()
        );
        Ok(format!("「{}」を提出しました", file_name))
    }
}

/// Submit a text-based report (テキスト入力課題) to Luna
/// Flow: 1) GET submission page → extract _cid, _csrf
///       2) POST /lms/course/report/submission with submissionText → confirm
///       3) POST confirmation form → register
#[tauri::command]
pub async fn luna_submit_report_text(
    state: State<'_, LunaState>,
    idnumber: String,
    report_id: String,
    period: Option<String>,
    submission_text: String,
) -> Result<String, String> {
    let http = luna_http(&state).await?;

    if submission_text.trim().is_empty() {
        return Err("提出テキストが空です".into());
    }

    log::info!(
        "Text report submission: idnumber={}, reportId={}, text_len={}",
        idnumber,
        report_id,
        submission_text.len()
    );

    // Step 1: Fetch the submission page for tokens
    let submission_url = format!(
        "/lms/course/report/submission?idnumber={}&reportId={}",
        idnumber, report_id
    );
    let page_html = luna_get(&http, &submission_url).await?;

    let cid = extract_report_token(&page_html, "_cid", period.as_deref())?;
    let csrf = extract_report_token(&page_html, "_csrf", period.as_deref())?;

    log::info!(
        "Text report tokens: _cid={}..., _csrf={}...",
        crate::client::safe_truncate(&cid, 8),
        crate::client::safe_truncate(&csrf, 8)
    );

    // Step 2: POST submission with text content (no file upload needed)
    let submit_params = [
        ("_cid", cid.clone()),
        ("_csrf", csrf.clone()),
        ("method", "1".to_string()),
        ("idnumber", idnumber.clone()),
        ("reportId", report_id.clone()),
        ("submissionText", submission_text.clone()),
        ("rowCounter", "0".to_string()),
        ("dragAndDrop", "false".to_string()),
    ];

    let submit_url = format!("{}/lms/course/report/submission", config::LUNA_BASE);
    let raw_resp = http
        .post(&submit_url)
        .form(&submit_params)
        .send()
        .await
        .map_err(|e| format!("提出リクエスト失敗: {}", e))?;

    let step2_status = raw_resp.status();
    log::info!("Text report step 2: status={}", step2_status);

    let confirm_html = if step2_status.is_redirection() {
        if let Some(loc) = raw_resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
        {
            let next_url = if loc.starts_with('/') {
                format!("{}{}", config::LUNA_BASE, loc)
            } else {
                loc.to_string()
            };
            client::fetch_with_redirect(
                &http,
                &next_url,
                config::LUNA_BASE,
                luna_client::LUNA_SESSION_EXPIRED_MSG,
                luna_client::is_luna_session_expired,
            )
            .await?
        } else {
            raw_resp
                .text()
                .await
                .map_err(|e| format!("レスポンス読取失敗: {}", e))?
        }
    } else {
        raw_resp
            .text()
            .await
            .map_err(|e| format!("レスポンス読取失敗: {}", e))?
    };

    #[cfg(debug_assertions)]
    {
        if crate::should_dump_debug_html() {
            let dump_path = std::env::temp_dir().join("luna_report_text_confirm.html");
            let _ = std::fs::write(&dump_path, &confirm_html);
            log::info!(
                "Text report confirm page dumped to {} ({} bytes)",
                dump_path.display(),
                confirm_html.len()
            );
        }
    }

    if confirm_html.is_empty() {
        return Err("確認画面が空です。セッションが切れている可能性があります。".into());
    }

    // Check for direct success
    if confirm_html.contains("提出が完了") || confirm_html.contains("提出済") {
        log::info!("Text report submitted directly (no confirmation step)");
        return Ok("テキストを提出しました".into());
    }

    // Step 3: Parse confirmation page and submit registration form
    let (register_action, register_fields) = {
        let confirm_doc = scraper::Html::parse_document(&confirm_html);
        let confirm_cid = extract_input_value(&confirm_html, "_cid").unwrap_or_else(|| cid.clone());
        let confirm_csrf = extract_input_value(&confirm_html, "_csrf").unwrap_or(csrf);

        let mut action = String::new();
        let mut fields: Vec<(String, String)> = Vec::new();

        if let Some(form_el) = confirm_doc.select(&SEL_REPORT_FORM).next().or_else(|| {
            confirm_doc.select(&SEL_FORM).find(|f| {
                f.value()
                    .attr("action")
                    .map(|a| a.contains("/report/submission") && !a.contains("download"))
                    .unwrap_or(false)
            })
        }) {
            if let Some(a) = form_el.value().attr("action") {
                action = a.to_string();
            }
            for input_el in form_el.select(&SEL_HIDDEN_INPUT) {
                let name = input_el.value().attr("name").unwrap_or_default();
                let value = input_el.value().attr("value").unwrap_or_default();
                if !name.is_empty() {
                    if name == "_method" {
                        fields.push(("_method".to_string(), "put".to_string()));
                    } else {
                        fields.push((name.to_string(), value.to_string()));
                    }
                }
            }
        }

        if fields.is_empty() {
            fields = vec![
                ("_cid".into(), confirm_cid),
                ("_csrf".into(), confirm_csrf),
                ("_method".into(), "put".into()),
                ("method".into(), "1".into()),
                ("idnumber".into(), idnumber.clone()),
                ("reportId".into(), report_id.clone()),
                ("submissionText".into(), submission_text.clone()),
                ("dragAndDrop".into(), "false".into()),
            ];
        }

        log::info!(
            "Text report step 3 fields: {:?}",
            fields
                .iter()
                .map(|(k, v)| format!("{}={}", k, client::safe_truncate(v, 20)))
                .collect::<Vec<_>>()
        );

        (action, fields)
    };

    if register_action.is_empty() {
        if confirm_html.contains("提出が完了")
            || confirm_html.contains("完了しました")
            || confirm_html.contains("提出済")
        {
            return Ok("テキストを提出しました".into());
        }
        return Err("確認画面に登録フォームが見つかりません".into());
    }

    let register_url = format!("{}{}", config::LUNA_BASE, register_action);
    let raw_resp3 = http
        .post(&register_url)
        .form(&register_fields)
        .send()
        .await
        .map_err(|e| format!("登録リクエスト失敗: {}", e))?;

    let step3_status = raw_resp3.status();
    log::info!("Text report step 3: status={}", step3_status);

    let _register_resp = if step3_status.is_redirection() {
        if let Some(loc) = raw_resp3
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
        {
            let next_url = if loc.starts_with('/') {
                format!("{}{}", config::LUNA_BASE, loc)
            } else {
                loc.to_string()
            };
            client::fetch_with_redirect(
                &http,
                &next_url,
                config::LUNA_BASE,
                luna_client::LUNA_SESSION_EXPIRED_MSG,
                luna_client::is_luna_session_expired,
            )
            .await?
        } else {
            raw_resp3
                .text()
                .await
                .map_err(|e| format!("レスポンス読取失敗: {}", e))?
        }
    } else {
        raw_resp3
            .text()
            .await
            .map_err(|e| format!("レスポンス読取失敗: {}", e))?
    };

    #[cfg(debug_assertions)]
    {
        if crate::should_dump_debug_html() {
            let dump_path2 = std::env::temp_dir().join("luna_report_text_result.html");
            let _ = std::fs::write(&dump_path2, &_register_resp);
        }
    }

    Ok("テキストを提出しました".into())
}

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

fn extract_url_param(url: &str, key: &str) -> Option<String> {
    let query = url.split('?').nth(1)?;
    for part in query.split('&') {
        let mut kv = part.splitn(2, '=');
        if kv.next()? == key {
            return kv.next().map(|v| v.to_string());
        }
    }
    None
}

const REPORT_SUBMISSION_NOT_OPEN_MESSAGE: &str =
    "提出期間外のため、現在は提出できません。提出期間を確認してから再度お試しください。";

fn normalize_html_text(html: &str) -> String {
    let doc = scraper::Html::parse_document(html);
    doc.root_element()
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_luna_report_datetime(value: &str) -> Option<chrono::DateTime<chrono::Local>> {
    static REPORT_DATETIME_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"^\s*(\d{4})[/-](\d{1,2})[/-](\d{1,2})\s+(\d{1,2}):(\d{2})\s*$")
            .expect("valid report datetime regex")
    });
    let captures = REPORT_DATETIME_RE.captures(value)?;
    let year = captures.get(1)?.as_str().parse::<i32>().ok()?;
    let month = captures.get(2)?.as_str().parse::<u32>().ok()?;
    let day = captures.get(3)?.as_str().parse::<u32>().ok()?;
    let hour = captures.get(4)?.as_str().parse::<u32>().ok()?;
    let minute = captures.get(5)?.as_str().parse::<u32>().ok()?;
    if hour > 24 || minute > 59 || (hour == 24 && minute != 0) {
        return None;
    }

    let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
    let (date, hour) = if hour == 24 {
        (date.succ_opt()?, 0)
    } else {
        (date, hour)
    };
    let naive = date.and_hms_opt(hour, minute, 0)?;
    chrono::Local.from_local_datetime(&naive).single()
}

fn parse_luna_report_period(
    period: &str,
) -> Option<(
    chrono::DateTime<chrono::Local>,
    chrono::DateTime<chrono::Local>,
    String,
    String,
)> {
    let parts: Vec<_> = period
        .split(['~', '～'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() < 2 {
        return None;
    }
    let start = parse_luna_report_datetime(parts[0])?;
    let end = parse_luna_report_datetime(parts[1])?;
    Some((start, end, parts[0].to_string(), parts[1].to_string()))
}

fn report_period_unavailable_message(period: Option<&str>) -> Option<String> {
    let (start, end, raw_start, raw_end) = parse_luna_report_period(period?)?;
    let now = chrono::Local::now();
    if now < start {
        Some(format!(
            "提出開始前です。提出期間: {} ～ {}",
            raw_start, raw_end
        ))
    } else if now > end {
        Some(format!(
            "提出期間が終了しています。提出期間: {} ～ {}",
            raw_start, raw_end
        ))
    } else {
        None
    }
}

fn extract_report_period_from_html(html: &str) -> Option<String> {
    static REPORT_PERIOD_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(\d{4}[/-]\d{1,2}[/-]\d{1,2}\s+\d{1,2}:\d{2})\s*[~～]\s*(\d{4}[/-]\d{1,2}[/-]\d{1,2}\s+\d{1,2}:\d{2})",
        )
        .expect("valid report period regex")
    });
    let text = normalize_html_text(html);
    let captures = REPORT_PERIOD_RE.captures(&text)?;
    let start = captures.get(1)?.as_str();
    let end = captures.get(2)?.as_str();
    Some(format!("{} ～ {}", start, end))
}

fn report_submission_unavailable_message(html: &str, period: Option<&str>) -> Option<String> {
    if let Some(message) = report_period_unavailable_message(period).or_else(|| {
        extract_report_period_from_html(html)
            .and_then(|p| report_period_unavailable_message(Some(&p)))
    }) {
        return Some(message);
    }

    let text = normalize_html_text(html);
    if text.contains("提出期間外のため、提出できません")
        || text.contains("提出期間外のため、提出できません。")
        || text.contains("提出期間外")
            && (text.contains("提出できません") || text.contains("提出不可"))
    {
        return Some(REPORT_SUBMISSION_NOT_OPEN_MESSAGE.to_string());
    }
    None
}

fn extract_report_token(html: &str, name: &str, period: Option<&str>) -> Result<String, String> {
    extract_input_value(html, name).ok_or_else(|| {
        report_submission_unavailable_message(html, period)
            .unwrap_or_else(|| format!("{} トークンが見つかりません", name))
    })
}

/// Extract a hidden input value from HTML by name
fn extract_input_value(html: &str, name: &str) -> Option<String> {
    // Use scraper for reliable extraction
    let doc = scraper::Html::parse_document(html);
    let selector_str = format!("input[name=\"{}\"]", name);
    if let Ok(sel) = scraper::Selector::parse(&selector_str) {
        if let Some(el) = doc.select(&sel).next() {
            if let Some(val) = el.value().attr("value") {
                if !val.is_empty() {
                    return Some(val.to_string());
                }
            }
        }
    }
    // Fallback: regex-like search for name="xxx" ... value="yyy"
    let pattern = format!("name=\"{}\"", name);
    let pos = html.find(&pattern)?;
    let region_start = crate::client::floor_char_boundary(html, pos.saturating_sub(200));
    let region_end =
        crate::client::ceil_char_boundary(html, (pos + pattern.len() + 200).min(html.len()));
    let region = &html[region_start..region_end];
    let val_marker = "value=\"";
    let val_pos = region.find(val_marker)?;
    let rest = &region[val_pos + val_marker.len()..];
    let end = rest.find('"')?;
    let val = rest[..end].to_string();
    if !val.is_empty() {
        Some(val)
    } else {
        None
    }
}

/// Extract first matching form action + fields (hidden/text/textarea/select).
fn extract_form_fields(html: &str, action_hint: &str) -> Option<(String, Vec<(String, String)>)> {
    sel!(SEL_INPUT_NAME, "input[name]");
    sel!(SEL_TEXTAREA_NAME, "textarea[name]");
    sel!(SEL_SELECT_NAME, "select[name]");
    sel!(SEL_OPT_SELECTED, "option[selected]");
    sel!(SEL_OPTION, "option");

    let doc = scraper::Html::parse_document(html);

    let mut fallback: Option<(String, Vec<(String, String)>)> = None;
    for form in doc.select(&SEL_FORM) {
        let action = form.value().attr("action").unwrap_or_default().to_string();
        let mut fields = Vec::new();

        for input in form.select(&SEL_INPUT_NAME) {
            let name = input.value().attr("name").unwrap_or_default();
            let typ = input
                .value()
                .attr("type")
                .unwrap_or("text")
                .to_ascii_lowercase();
            if (typ == "checkbox" || typ == "radio") && input.value().attr("checked").is_none() {
                continue;
            }
            let value = input.value().attr("value").unwrap_or_default();
            if !name.is_empty() {
                fields.push((name.to_string(), value.to_string()));
            }
        }

        for ta in form.select(&SEL_TEXTAREA_NAME) {
            let name = ta.value().attr("name").unwrap_or_default();
            if !name.is_empty() {
                fields.push((name.to_string(), ta.text().collect::<String>()));
            }
        }

        for se in form.select(&SEL_SELECT_NAME) {
            let name = se.value().attr("name").unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let value = se
                .select(&SEL_OPT_SELECTED)
                .next()
                .or_else(|| se.select(&SEL_OPTION).next())
                .and_then(|o| o.value().attr("value"))
                .unwrap_or_default()
                .to_string();
            fields.push((name.to_string(), value));
        }

        if action.is_empty() || fields.is_empty() {
            continue;
        }

        if fallback.is_none() {
            fallback = Some((action.clone(), fields.clone()));
        }
        if action_hint.is_empty() || action.contains(action_hint) {
            return Some((action, fields));
        }
    }

    fallback
}

fn upsert_field(fields: &mut Vec<(String, String)>, key: &str, value: String) {
    for (k, v) in fields.iter_mut() {
        if k == key {
            *v = value;
            return;
        }
    }
    fields.push((key.to_string(), value));
}

fn field_value<'a>(fields: &'a [(String, String)], key: &str) -> Option<&'a str> {
    fields
        .iter()
        .find_map(|(k, v)| if k == key { Some(v.as_str()) } else { None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_luna_report_period_with_24_hour_deadline() {
        let (_start, end, raw_start, raw_end) =
            parse_luna_report_period("2026/04/22 17:00 ～ 2026/04/29 24:00").unwrap();

        assert_eq!(raw_start, "2026/04/22 17:00");
        assert_eq!(raw_end, "2026/04/29 24:00");
        assert_eq!(end.format("%Y/%m/%d %H:%M").to_string(), "2026/04/30 00:00");
    }

    #[test]
    fn extracts_luna_report_period_from_html_text() {
        let html = r#"
            <div class="contents-detail contents-vertical">
              <div class="contents-header contents-header-txt"><span>提出期間</span></div>
              <div class="contents-input-area">
                <span>2999/04/22 17:00</span><span>～</span><span>2999/04/29 24:00</span>
              </div>
            </div>
        "#;

        assert_eq!(
            extract_report_period_from_html(html).as_deref(),
            Some("2999/04/22 17:00 ～ 2999/04/29 24:00")
        );
    }

    #[test]
    fn reports_future_luna_period_as_before_start() {
        let message =
            report_period_unavailable_message(Some("2999/04/22 17:00 ～ 2999/04/29 24:00"))
                .unwrap();

        assert!(message.contains("提出開始前です"));
        assert!(message.contains("2999/04/22 17:00 ～ 2999/04/29 24:00"));
    }

    #[test]
    fn detects_survey_submit_returned_answer_form_as_error() {
        let html = r#"
            <div id="survey_question_subblock"></div>
            <div class="highlight-txt answer-type-textarea-error">入力してください</div>
            <a class="under-btn btn-txt btn-color answer-btn">回答する</a>
        "#;

        let error = detect_survey_submit_error(html).unwrap();
        assert!(error.contains("入力してください"));
    }

    #[test]
    fn keeps_blank_survey_comment_text_value() {
        let value = serde_json::json!({
            "name": "answer[0].commentText",
            "value": ""
        });

        let (name, answer_value) = survey_answer_payload(0, &value);
        assert_eq!(name, "answer[0].commentText");
        assert_eq!(
            survey_answer_values(answer_value, !name.is_empty()),
            vec![""]
        );
    }

    #[test]
    fn expands_survey_checkbox_answer_item_names() {
        assert_eq!(
            survey_answer_field_name("answer[3].answerItem[0].answer", 3, 0),
            "answer[3].answerItem[0].answer"
        );
        assert_eq!(
            survey_answer_field_name("answer[3].answerItem[0].answer", 3, 1),
            "answer[3].answerItem[1].answer"
        );
    }

    #[test]
    fn normalizes_luna_relative_submit_paths() {
        assert_eq!(
            normalize_luna_relative_path(
                "https://luna.kwansei.ac.jp/lms/course/surveys/take?_cid=abc"
            )
            .unwrap(),
            "/lms/course/surveys/take?_cid=abc"
        );
        assert!(
            normalize_luna_relative_path("https://example.com/lms/course/surveys/take").is_err()
        );
    }
}
