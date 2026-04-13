use tauri::{State, Manager};
use crate::AppState;
use crate::config;
use crate::client;
use crate::luna_client;
use crate::luna_parser;
use std::sync::atomic::{AtomicU32, Ordering};

static LUNA_DETAIL_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Briefly lock Luna client, check auth and clone http. Releases lock immediately.
async fn luna_http(state: &AppState) -> Result<reqwest::Client, String> {
    let luna = state.luna.lock().await;
    if !luna.authenticated {
        return Err(luna_client::LUNA_AUTH_REQUIRED_MSG.into());
    }
    Ok(luna.http.clone())
}

/// Luna GET: fetch a page without holding the lock.
async fn luna_get(http: &reqwest::Client, path: &str) -> Result<String, String> {
    let url = format!("{}{}", config::LUNA_BASE, path);
    client::fetch_with_redirect(
        http, &url, config::LUNA_BASE,
        luna_client::LUNA_SESSION_EXPIRED_MSG, luna_client::is_luna_session_expired,
    ).await
}

/// Luna GET with Referer header — required for form pages that serve CSRF tokens.
async fn luna_get_with_referer(http: &reqwest::Client, path: &str, referer_path: &str) -> Result<String, String> {
    let url = format!("{}{}", config::LUNA_BASE, path);
    let referer = format!("{}{}", config::LUNA_BASE, referer_path);
    let mut current_url = url;
    for i in 0..10 {
        let resp = http.get(&current_url)
            .header("Referer", &referer)
            .send().await
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
                log::debug!("luna_get_with_referer redirect #{} -> {}", i + 1, client::safe_truncate(&current_url, 120));
                if current_url.contains("sso.kwansei.ac.jp") {
                    return Err(luna_client::LUNA_SESSION_EXPIRED_MSG.into());
                }
                continue;
            }
        }
        if !status.is_success() {
            return Err(format!("HTTP {}", status));
        }
        let body = resp.text().await.map_err(|e| format!("レスポンス読取失敗: {}", e))?;
        if luna_client::is_luna_session_expired(&body) {
            return Err(luna_client::LUNA_SESSION_EXPIRED_MSG.into());
        }
        return Ok(body);
    }
    Err("リダイレクトが多すぎます".into())
}

/// Luna POST: submit a form without holding the lock.
async fn luna_post(http: &reqwest::Client, path: &str, params: &[(String, String)]) -> Result<String, String> {
    let url = format!("{}{}", config::LUNA_BASE, path);
    client::post_form_with_redirect(
        http, &url, config::LUNA_BASE,
        luna_client::LUNA_SESSION_EXPIRED_MSG, luna_client::is_luna_session_expired,
        params.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        &[],
    ).await
}

/// Luna multipart POST: submit a multipart form without holding the lock.
async fn luna_post_multipart(http: &reqwest::Client, path: &str, form: reqwest::multipart::Form) -> Result<String, String> {
    let url = format!("{}{}", config::LUNA_BASE, path);
    let builder = http.post(&url).multipart(form);
    client::send_and_follow_redirect(
        http, builder, config::LUNA_BASE,
        luna_client::LUNA_SESSION_EXPIRED_MSG, luna_client::is_luna_session_expired,
    ).await
}

/// Luna multipart POST with _cid appended to URL (mimics Luna's AJAX interceptor).
async fn luna_post_multipart_with_cid(http: &reqwest::Client, path: &str, cid: &str, form: reqwest::multipart::Form) -> Result<String, String> {
    let url = format!("{}{}?_cid={}", config::LUNA_BASE, path, cid);
    let builder = http.post(&url).multipart(form);
    client::send_and_follow_redirect(
        http, builder, config::LUNA_BASE,
        luna_client::LUNA_SESSION_EXPIRED_MSG, luna_client::is_luna_session_expired,
    ).await
}

/// Luna download: download a file without holding the lock. Returns bytes.
async fn luna_download(http: &reqwest::Client, path: &str) -> Result<Vec<u8>, String> {
    let url = if path.starts_with("http") {
        path.to_string()
    } else {
        format!("{}{}", config::LUNA_BASE, path)
    };

    let mut current_url = url;
    for i in 0..10 {
        let resp = http.get(&current_url)
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
            .header("Sec-Fetch-Dest", "document")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-Site", "same-origin")
            .send().await
            .map_err(|e| format!("ダウンロード失敗: {}", e))?;

        let status = resp.status();
        let content_type = resp.headers().get("content-type")
            .and_then(|v| v.to_str().ok()).unwrap_or("unknown").to_string();
        let content_len = resp.headers().get("content-length")
            .and_then(|v| v.to_str().ok()).unwrap_or("unknown").to_string();
        let content_disp = resp.headers().get("content-disposition")
            .and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        log::info!("luna_download #{}: status={}, type={}, len={}, disp='{}'",
            i, status, content_type, content_len, content_disp);

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

        // Check for session expired in HTML responses
        if content_type.contains("text/html") {
            let text = resp.text().await.map_err(|e| format!("読み取り失敗: {}", e))?;
            if luna_client::is_luna_session_expired(&text) {
                return Err(luna_client::LUNA_SESSION_EXPIRED_MSG.into());
            }
            return Ok(text.into_bytes());
        }

        return resp.bytes().await
            .map(|b| b.to_vec())
            .map_err(|e| format!("ダウンロード読み取り失敗: {}", e));
    }
    Err("リダイレクトが多すぎます".into())
}

/// Save bytes to the Downloads folder with conflict avoidance (appends " (N)" if the file exists).
fn save_to_downloads(filename: &str, bytes: &[u8]) -> Result<String, String> {
    let downloads = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
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

    std::fs::write(&final_path, bytes)
        .map_err(|e| format!("ファイル保存失敗: {}", e))?;

    Ok(final_path.to_string_lossy().to_string())
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
    !s.is_empty() && s.len() <= 20 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Open a Luna detail page in a separate native window
#[tauri::command]
pub async fn luna_open_detail_window(
    app: tauri::AppHandle,
    path: String,
    title: String,
    mode: Option<String>,
    period: Option<String>,
    status: Option<String>,
    idnumber: Option<String>,
    info_id: Option<String>,
    kgc_path: Option<String>,
) -> Result<(), String> {
    let existing = app.webview_windows().keys()
        .filter(|k| k.starts_with("luna-detail-")).count();
    if existing >= 10 {
        return Err("開いているウィンドウが多すぎます。いくつか閉じてください。".into());
    }
    let id = LUNA_DETAIL_COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = format!("luna-detail-{}", id);

    let url_str = match mode.as_deref() {
        Some("material") => {
            let mut parts = format!("luna-detail.html?mode=material&title={}", urlencoding::encode(&title));
            if let Some(p) = &period {
                parts.push_str(&format!("&period={}", urlencoding::encode(p)));
            }
            if let Some(s) = &status {
                parts.push_str(&format!("&status={}", urlencoding::encode(s)));
            }
            if let Some(id) = &idnumber {
                parts.push_str(&format!("&idnumber={}", urlencoding::encode(id)));
            }
            if let Some(info) = &info_id {
                parts.push_str(&format!("&infoId={}", urlencoding::encode(info)));
            }
            parts
        }
        Some("announcement") => {
            format!(
                "luna-detail.html?mode=announcement&title={}&idnumber={}&infoId={}",
                urlencoding::encode(&title),
                urlencoding::encode(idnumber.as_deref().unwrap_or("")),
                urlencoding::encode(info_id.as_deref().unwrap_or(""))
            )
        }
        Some("discussion") => {
            format!(
                "luna-detail.html?mode=discussion&path={}&title={}",
                urlencoding::encode(&path),
                urlencoding::encode(&title)
            )
        }
        Some("report") => {
            let mut parts = format!(
                "luna-detail.html?mode=report&path={}&title={}",
                urlencoding::encode(&path),
                urlencoding::encode(&title)
            );
            if let Some(id) = &idnumber {
                parts.push_str(&format!("&idnumber={}", urlencoding::encode(id)));
            }
            if let Some(info) = &info_id {
                parts.push_str(&format!("&reportId={}", urlencoding::encode(info)));
            }
            parts
        }
        Some("thread") => {
            format!(
                "luna-detail.html?mode=thread&path={}&title={}",
                urlencoding::encode(&path),
                urlencoding::encode(&title)
            )
        }
        Some("course") => {
            let mut parts = format!(
                "luna-detail.html?mode=course&idnumber={}&title={}",
                urlencoding::encode(idnumber.as_deref().unwrap_or("")),
                urlencoding::encode(&title)
            );
            if let Some(kp) = &kgc_path {
                parts.push_str(&format!("&kgcPath={}", urlencoding::encode(kp)));
            }
            parts
        }
        _ => {
            format!("luna-detail.html?path={}&title={}", urlencoding::encode(&path), urlencoding::encode(&title))
        }
    };

    let builder = tauri::WebviewWindowBuilder::new(
        &app,
        &label,
        tauri::WebviewUrl::App(url_str.into()),
    )
    .title(&title)
    .inner_size(720.0, 780.0)
    .min_inner_size(560.0, 480.0)
    .resizable(true);

    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);

    builder
    .build()
    .map_err(|e| format!("ウィンドウ作成失敗: {}", e))?;

    Ok(())
}

/// Launch an LTI tool (Zoom, Panopto, etc.) and open the final URL in app webview
#[tauri::command]
pub async fn luna_launch_lti(app: tauri::AppHandle, state: State<'_, AppState>, path: String) -> Result<(), String> {
    let http = luna_http(&state).await?;
    let final_url = luna_client::launch_lti(&http, &path).await?;
    crate::commands::open_external_url(app, final_url, None).await
}

/// Reveal a file in Finder (restricted to app download directory)
#[tauri::command]
pub async fn luna_reveal_file(app: tauri::AppHandle, path: String) -> Result<(), String> {
    // Restrict to files under the user's Downloads or app data directory
    let p = std::path::Path::new(&path);
    let canonical = p.canonicalize().map_err(|e| format!("パスが無効です: {}", e))?;
    let allowed = dirs::download_dir().unwrap_or_else(|| {
        dirs::home_dir().map(|h| h.join("Downloads")).unwrap_or_else(std::env::temp_dir)
    });
    if !canonical.starts_with(&allowed) {
        return Err("ダウンロードフォルダ外のファイルは表示できません".into());
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener().reveal_item_in_dir(&canonical)
        .map_err(|e| format!("ファイルを表示できませんでした: {}", e))?;
    Ok(())
}

/// Fetch a Luna page (generic)
#[tauri::command]
pub async fn luna_fetch_page(
    state: State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    // Only allow known Luna paths
    if path.contains("://") || !path.starts_with('/') {
        return Err("許可されていないパスです".into());
    }
    let allowed_prefixes = ["/top", "/lms/", "/course/", "/notification", "/updateinfo", "/message", "/attend", "/report", "/survey", "/material"];
    if !allowed_prefixes.iter().any(|p| path.starts_with(p)) {
        return Err("許可されていないパスです".into());
    }
    let http = luna_http(&state).await?;
    luna_get(&http, &path).await
}

/// Check if Luna session is valid
#[tauri::command]
pub async fn luna_check_session(
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let (http, authenticated) = {
        let luna = state.luna.lock().await;
        (luna.http.clone(), luna.authenticated)
    };
    if !authenticated {
        return Ok(false);
    }
    // Validate against server without holding the lock
    let url = format!("{}/lms/timetable", crate::config::LUNA_BASE);
    match crate::client::fetch_with_redirect(
        &http, &url, crate::config::LUNA_BASE,
        crate::luna_client::LUNA_SESSION_EXPIRED_MSG, crate::luna_client::is_luna_session_expired,
    ).await {
        Ok(_) => {
            let luna = state.luna.lock().await;
            luna.save_session();
            Ok(true)
        }
        Err(e) if e == crate::luna_client::LUNA_SESSION_EXPIRED_MSG => {
            let mut luna = state.luna.lock().await;
            luna.authenticated = false;
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

/// Fetch parsed TODO list
#[tauri::command]
pub async fn luna_fetch_todo(
    state: State<'_, AppState>,
    db: State<'_, crate::db::Database>,
) -> Result<Vec<luna_parser::LunaTodoItem>, String> {
    match luna_http(&state).await {
        Ok(http) => match luna_get(&http, "/lms/todo").await {
            Ok(html) => {
                let data = luna_parser::parse_luna_todo(&html);
                if let Ok(json) = serde_json::to_string(&data) {
                    let _ = db.save_data_cache("luna_todo", &json);
                }
                Ok(data)
            }
            Err(e) => {
                if let Ok(Some((json, _))) = db.get_data_cache("luna_todo") {
                    if let Ok(cached) = serde_json::from_str(&json) {
                        log::info!("luna_todo: cache fallback ({})", e);
                        return Ok(cached);
                    }
                }
                Err(e)
            }
        },
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache("luna_todo") {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("luna_todo: cache fallback ({})", e);
                    return Ok(cached);
                }
            }
            Err(e)
        }
    }
}

/// Fetch parsed notifications
#[tauri::command]
pub async fn luna_fetch_updates(
    state: State<'_, AppState>,
    db: State<'_, crate::db::Database>,
) -> Result<Vec<luna_parser::LunaNotification>, String> {
    match luna_http(&state).await {
        Ok(http) => match luna_get(&http, "/updateinfo").await {
            Ok(html) => {
                let data = luna_parser::parse_luna_notifications(&html);
                if let Ok(json) = serde_json::to_string(&data) {
                    let _ = db.save_data_cache("luna_updates", &json);
                }
                Ok(data)
            }
            Err(e) => {
                if let Ok(Some((json, _))) = db.get_data_cache("luna_updates") {
                    if let Ok(cached) = serde_json::from_str(&json) {
                        log::info!("luna_updates: cache fallback ({})", e);
                        return Ok(cached);
                    }
                }
                Err(e)
            }
        },
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache("luna_updates") {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("luna_updates: cache fallback ({})", e);
                    return Ok(cached);
                }
            }
            Err(e)
        }
    }
}

/// Fetch course content page
#[tauri::command]
pub async fn luna_fetch_course_content(
    state: State<'_, AppState>,
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
    state: State<'_, AppState>,
    db: State<'_, crate::db::Database>,
    path: String,
) -> Result<luna_parser::LunaDetailPage, String> {
    // Reject absolute URLs and enforce known Luna path prefixes
    if path.starts_with("http") || !path.starts_with('/') {
        return Err("許可されていないパスです".into());
    }
    let cache_key = format!("luna_detail:{}", path);
    match luna_http(&state).await {
        Ok(http) => match luna_get(&http, &path).await {
            Ok(html) => {
                #[cfg(debug_assertions)]
                {
                    let filename = path.replace(['/', '?', '&'], "_");
                    let dump_path = std::env::temp_dir().join(format!("luna_detail{}.html", filename));
                    let _ = std::fs::write(&dump_path, &html);
                    log::info!("Luna detail HTML dumped to {} ({} bytes)", dump_path.display(), html.len());
                }
                let data = luna_parser::parse_luna_detail_page(&html);
                if let Ok(json) = serde_json::to_string(&data) {
                    let _ = db.save_data_cache(&cache_key, &json);
                }
                Ok(data)
            }
            Err(e) => {
                if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                    if let Ok(cached) = serde_json::from_str(&json) {
                        log::info!("luna_detail: cache fallback ({})", e);
                        return Ok(cached);
                    }
                }
                Err(e)
            }
        },
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("luna_detail: cache fallback ({})", e);
                    return Ok(cached);
                }
            }
            Err(e)
        }
    }
}

/// Fetch announcement detail from Luna course page
#[tauri::command]
pub async fn luna_fetch_announcement_detail(
    state: State<'_, AppState>,
    db: State<'_, crate::db::Database>,
    idnumber: String,
    info_id: String,
) -> Result<luna_parser::LunaDetailPage, String> {
    if !is_safe_param(&idnumber) || !is_safe_param(&info_id) {
        return Err("無効なパラメータです".into());
    }
    let cache_key = format!("luna_announce:{}:{}", idnumber, info_id);
    let path = format!(
        "/lms/coursetop/information/listdetail?idnumber={}&informationId={}",
        idnumber, info_id
    );
    match luna_http(&state).await {
        Ok(http) => match luna_get(&http, &path).await {
            Ok(html) => {
                #[cfg(debug_assertions)]
                {
                    let dump_path = std::env::temp_dir().join(format!("luna_announcement_{}_{}.html", idnumber, info_id));
                    let _ = std::fs::write(&dump_path, &html);
                    log::info!("Luna announcement detail dumped ({} bytes)", html.len());
                }
                let data = luna_parser::parse_luna_announcement_detail(&html);
                if let Ok(json) = serde_json::to_string(&data) {
                    let _ = db.save_data_cache(&cache_key, &json);
                }
                Ok(data)
            }
            Err(e) => {
                if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                    if let Ok(cached) = serde_json::from_str(&json) {
                        log::info!("luna_announce: cache fallback ({})", e);
                        return Ok(cached);
                    }
                }
                Err(e)
            }
        },
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("luna_announce: cache fallback ({})", e);
                    return Ok(cached);
                }
            }
            Err(e)
        }
    }
}

/// Fetch and parse course top page (/lms/course?idnumber=XXX)
#[tauri::command]
pub async fn luna_fetch_course_detail(
    state: State<'_, AppState>,
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
        log::warn!("Course page for {} returned no menus ({}B), retrying...", idnumber, course_html.len());
        #[cfg(debug_assertions)]
        {
            let dump = std::env::temp_dir().join(format!("luna_course_{}_initial.html", idnumber));
            let _ = std::fs::write(&dump, &course_html);
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
                let dump = std::env::temp_dir().join(format!("luna_course_{}.html", idnumber));
                let _ = std::fs::write(&dump, &retry_html);
            }
        }
    } else {
        #[cfg(debug_assertions)]
        {
            let dump = std::env::temp_dir().join(format!("luna_course_{}.html", idnumber));
            let _ = std::fs::write(&dump, &course_html);
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
        let dump_path = std::env::temp_dir().join(format!("luna_contents_{}.html", idnumber));
        let _ = std::fs::write(&dump_path, &contents_html);
    }

    // Merge actual content items from contents page
    let (materials, reports, examinations, discussions) = luna_parser::parse_luna_contents_page(&contents_html);
    result.materials = materials;
    result.reports = reports;
    result.examinations = examinations;
    result.discussions = discussions;

    // Cache the complete result
    if let Ok(json) = serde_json::to_string(&result) {
        let _ = db.save_data_cache(&cache_key, &json);
    }

    Ok(result)
}

/// Download a Luna file attachment to the Downloads folder and return the saved path
#[tauri::command]
pub async fn luna_download_file(
    state: State<'_, AppState>,
    url: String,
    filename: String,
) -> Result<String, String> {
    // For external URLs (SharePoint etc.), just return the URL for the frontend to open
    if url.starts_with("http") {
        return Ok(url);
    }

    let http = luna_http(&state).await?;
    let bytes = luna_download(&http, &url).await?;

    save_to_downloads(&filename, &bytes)
}

/// Replicate Luna's CommonUtil.makeDownFileName JS function:
/// replace fullwidth/halfwidth spaces with _, collapse multiple _, then encodeURI
fn make_down_file_name(file_name: &str) -> String {
    // Replace fullwidth space (U+3000) and regular space with _
    let mut result = file_name.replace(['\u{3000}', ' '], "_");
    // Collapse multiple underscores
    while result.contains("__") {
        result = result.replace("__", "_");
    }
    // encodeURI: encode each char, but don't encode ;,/?:@&=+$-_.!~*'()#
    // Using percent_encoding with a custom set equivalent to encodeURI
    let mut encoded = String::new();
    for ch in result.chars() {
        if ch.is_ascii_alphanumeric()
            || "-_.!~*'()".contains(ch)
            || ";,/?:@&=+$#".contains(ch)
        {
            encoded.push(ch);
        } else {
            // UTF-8 percent-encode
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
#[tauri::command]
pub async fn luna_download_material(
    state: State<'_, AppState>,
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

    log::info!("Material download: file='{}', object='{}', resource='{}', type='{}', matId={:?}",
        file_name, object_name, resource_id, file_type, material_id);

    // Step 0: Visit the course contents page first to establish server-side session context
    // (the browser is always on this page when downloading)
    let course_url = format!("/lms/course?idnumber={}", idnumber);
    let _ = luna_get(&http, &course_url).await;

    // Step 1: Prepare tempfile (GET /lms/course/make/tempfile)
    let tempfile_query = format!(
        "fileName={}&objectName={}&id={}&idnumber={}",
        urlencoding::encode(&file_name),
        urlencoding::encode(&object_name),
        urlencoding::encode(&resource_id),
        urlencoding::encode(&idnumber),
    );
    let tempfile_url = format!("/lms/course/make/tempfile?{}", tempfile_query);
    log::info!("Material tempfile URL: {}", tempfile_url);
    let file_id = luna_get(&http, &tempfile_url).await
        .map_err(|e| format!("ファイル準備失敗: {}", e))?;
    let file_id = file_id.trim().to_string();

    log::info!("Material tempfile returned fileId (len={}): '{}'", file_id.len(), crate::client::safe_truncate(&file_id, 500));

    // If tempfile returns HTML instead of a path, something went wrong
    if file_id.contains('<') || file_id.is_empty() {
        return Err(format!("tempfile returned unexpected response (len={})", file_id.len()));
    }

    // Step 2: Download via GET form submit to setfiledown/sethtmlfiledown
    // URL path uses CommonUtil.makeDownFileName (encodeURI with space→_ normalization)
    let path_encoded_name = make_down_file_name(&file_name);
    let base_path = if file_type == "0" {
        format!("/lms/course/materialref/setfiledown/{}", path_encoded_name)
    } else {
        format!("/lms/course/materialref/sethtmlfiledown/{}", path_encoded_name)
    };
    let dl_title = display_name.unwrap_or_default();
    let content_id = material_id.unwrap_or_default();
    let title_val = if file_type != "0" { &dl_title } else { "" };
    let end_date_val = end_date.unwrap_or_default();

    // Browser form GET submit uses application/x-www-form-urlencoded
    // Build the full URL manually to avoid reqwest's .query() double-encoding
    fn form_encode(s: &str) -> String {
        // application/x-www-form-urlencoded: space → +, encode other special chars
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
    let query_string = format!(
        "fileName={}&fileId={}&idnumber={}&resourceId={}&screen=1&contentId={}&endDate={}&title={}",
        form_encode(&file_name),
        form_encode(&file_id),
        form_encode(&idnumber),
        form_encode(&resource_id),
        form_encode(&content_id),
        form_encode(&end_date_val),
        form_encode(title_val),
    );
    let full_download_url = format!("{}?{}", base_path, query_string);

    log::info!("Material download full URL: {}", full_download_url);

    let bytes = luna_download(&http, &full_download_url).await?;

    log::info!("Material downloaded {} bytes", bytes.len());

    // Check if we got an HTML error page instead of the file
    if bytes.len() < 1000 {
        if let Ok(text) = std::str::from_utf8(&bytes) {
            if text.contains("<!DOCTYPE") || text.contains("<html") {
                log::error!("Download returned HTML instead of file: {}", crate::client::safe_truncate(text, 500));
                return Err("サーバーがファイルではなくエラーページを返しました".into());
            }
        }
    }

    if bytes.is_empty() {
        return Err("ダウンロードされたファイルが空です".into());
    }

    save_to_downloads(&file_name, &bytes)
}

/// Submit a report (課題提出) to Luna
/// Flow: 1) GET submission page → extract _cid, _csrf
///       2) POST /lms/course/report/upload (multipart) → get fileId
///       3) POST /lms/course/report/submission → confirm
#[tauri::command]
pub async fn luna_submit_report(
    state: State<'_, AppState>,
    idnumber: String,
    report_id: String,
    file_name: String,
    file_base64: String,
) -> Result<String, String> {
    use base64::Engine;
    let http = luna_http(&state).await?;

    // Decode base64 file data
    let file_bytes = base64::engine::general_purpose::STANDARD
        .decode(&file_base64)
        .map_err(|e| format!("Base64デコード失敗: {}", e))?;

    log::info!("Report submission: idnumber={}, reportId={}, file={} ({}B)", idnumber, report_id, file_name, file_bytes.len());

    // Step 1: Fetch the submission page to get _cid and _csrf tokens
    let submission_url = format!(
        "/lms/course/report/submission?idnumber={}&reportId={}",
        idnumber, report_id
    );
    let page_html = luna_get(&http, &submission_url).await?;

    let cid = extract_input_value(&page_html, "_cid")
        .ok_or("_cid トークンが見つかりません")?;
    let csrf = extract_input_value(&page_html, "_csrf")
        .ok_or("_csrf トークンが見つかりません")?;

    log::info!("Report tokens: _cid={}..., _csrf={}...", crate::client::safe_truncate(&cid, 8), crate::client::safe_truncate(&csrf, 8));

    // Step 2: Upload file via multipart POST (AJAX endpoint — _cid goes in URL)
    let upload_form = reqwest::multipart::Form::new()
        .text("_cid", cid.clone())
        .text("_csrf", csrf.clone())
        .text("method", "0".to_string())
        .text("idnumber", idnumber.clone())
        .text("reportId", report_id.clone())
        .part("uploadFile", reqwest::multipart::Part::bytes(file_bytes)
            .file_name(file_name.clone())
            .mime_str("application/octet-stream")
            .map_err(|e| format!("MIME error: {}", e))?
        );

    let upload_resp = luna_post_multipart_with_cid(&http, "/lms/course/report/upload", &cid, upload_form).await?;

    let upload_json: serde_json::Value = serde_json::from_str(&upload_resp)
        .map_err(|e| format!("アップロード応答の解析失敗: {} — body: {}", e, crate::client::safe_truncate(&upload_resp, 200)))?;

    if upload_json.get("success").and_then(|v| v.as_bool()) != Some(true) {
        let msg = upload_json.get("message")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join("; "))
            .unwrap_or_else(|| crate::client::safe_truncate(&upload_resp, 200).to_string());
        return Err(format!("アップロード失敗: {}", msg));
    }

    let file_id = upload_json.get("fileId")
        .and_then(|v| v.as_str().map(|s| s.to_string()).or_else(|| v.as_i64().map(|n| n.to_string())))
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
    let raw_resp = http.post(&submit_url)
        .form(&submit_params)
        .send().await
        .map_err(|e| format!("確認画面リクエスト失敗: {}", e))?;

    let step3_status = raw_resp.status();
    let step3_url = raw_resp.url().to_string();
    let step3_location = raw_resp.headers().get("location")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let step3_content_type = raw_resp.headers().get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    log::info!("Step 3 raw: status={}, url={}, location={:?}, content-type={:?}",
        step3_status, client::safe_truncate(&step3_url, 120),
        step3_location, step3_content_type);

    let confirm_html = if step3_status.is_redirection() {
        if let Some(loc) = &step3_location {
            let next_url = if loc.starts_with('/') {
                format!("{}{}", config::LUNA_BASE, loc)
            } else {
                loc.clone()
            };
            log::info!("Step 3 redirect -> {}", client::safe_truncate(&next_url, 120));
            client::fetch_with_redirect(
                &http, &next_url, config::LUNA_BASE,
                luna_client::LUNA_SESSION_EXPIRED_MSG, luna_client::is_luna_session_expired,
            ).await?
        } else {
            raw_resp.text().await.map_err(|e| format!("レスポンス読取失敗: {}", e))?
        }
    } else {
        raw_resp.text().await.map_err(|e| format!("レスポンス読取失敗: {}", e))?
    };

    // Dump confirmation page for debugging
    let dump_path = std::env::temp_dir().join("luna_report_confirm.html");
    let _ = std::fs::write(&dump_path, &confirm_html);
    log::info!("Report confirm page dumped to {} ({} bytes)", dump_path.display(), confirm_html.len());

    if confirm_html.is_empty() {
        return Err("確認画面が空です。セッションが切れている可能性があります。".into());
    }

    // Step 4: Parse confirmation page and submit the registration form
    let (register_action, register_fields) = {
        let confirm_doc = scraper::Html::parse_document(&confirm_html);
        let confirm_cid = extract_input_value(&confirm_html, "_cid")
            .unwrap_or_else(|| cid.clone());
        let confirm_csrf = extract_input_value(&confirm_html, "_csrf")
            .unwrap_or(csrf);

        // Find the reportSubmissionForm specifically (not other forms on the page)
        let form_sel = scraper::Selector::parse("form#reportSubmissionForm").unwrap();
        let input_sel = scraper::Selector::parse("input[type='hidden']").unwrap();
        let mut action = String::new();
        let mut fields: Vec<(String, String)> = Vec::new();

        if let Some(form_el) = confirm_doc.select(&form_sel).next() {
            if let Some(a) = form_el.value().attr("action") {
                action = a.to_string();
            }
            for input_el in form_el.select(&input_sel) {
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
            let any_form_sel = scraper::Selector::parse("form").unwrap();
            for form_el in confirm_doc.select(&any_form_sel) {
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

        log::info!("Step 4 fields: {:?}", fields.iter().map(|(k,v)| format!("{}={}", k, client::safe_truncate(v, 20))).collect::<Vec<_>>());

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

    log::info!("Report confirm form action: {}, fields: {}", register_action, register_fields.len());

    let register_url = format!("{}{}", config::LUNA_BASE, register_action);
    let raw_resp4 = http.post(&register_url)
        .form(&register_fields)
        .send().await
        .map_err(|e| format!("登録リクエスト失敗: {}", e))?;

    let step4_status = raw_resp4.status();
    let step4_url = raw_resp4.url().to_string();
    let step4_location = raw_resp4.headers().get("location")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let step4_content_type = raw_resp4.headers().get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    log::info!("Step 4 raw: status={}, url={}, location={:?}, content-type={:?}",
        step4_status, client::safe_truncate(&step4_url, 120),
        step4_location, step4_content_type);

    let register_resp = if step4_status.is_redirection() {
        if let Some(loc) = &step4_location {
            let next_url = if loc.starts_with('/') {
                format!("{}{}", config::LUNA_BASE, loc)
            } else {
                loc.clone()
            };
            log::info!("Step 4 redirect -> {}", client::safe_truncate(&next_url, 120));
            client::fetch_with_redirect(
                &http, &next_url, config::LUNA_BASE,
                luna_client::LUNA_SESSION_EXPIRED_MSG, luna_client::is_luna_session_expired,
            ).await?
        } else {
            raw_resp4.text().await.map_err(|e| format!("レスポンス読取失敗: {}", e))?
        }
    } else {
        raw_resp4.text().await.map_err(|e| format!("レスポンス読取失敗: {}", e))?
    };

    // Dump and check the result
    let dump_path2 = std::env::temp_dir().join("luna_report_register_result.html");
    let _ = std::fs::write(&dump_path2, &register_resp);
    log::info!("Report register response dumped to {} ({} bytes)", dump_path2.display(), register_resp.len());

    // Verify: the result page should show completion
    if register_resp.contains("提出が完了") || register_resp.contains("完了しました") {
        log::info!("Report registration confirmed by response content");
        Ok(format!("「{}」を提出しました", file_name))
    } else if register_resp.is_empty() {
        // Some Luna actions return empty on success redirect
        log::info!("Report registration response empty, verifying...");

        // Re-fetch the original page to verify
        let verify_html = luna_get(&http, &submission_url).await?;
        let dump_path3 = std::env::temp_dir().join("luna_report_verify.html");
        let _ = std::fs::write(&dump_path3, &verify_html);

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
        log::info!("Report registration completed (response {} bytes)", register_resp.len());
        Ok(format!("「{}」を提出しました", file_name))
    }
}

/// Fetch discussion thread detail (posts list) from Luna
#[tauri::command]
pub async fn luna_fetch_discussion_detail(
    state: State<'_, AppState>,
    db: State<'_, crate::db::Database>,
    url: String,
) -> Result<luna_parser::LunaDiscussionThread, String> {
    if url.starts_with("http") || !url.starts_with('/') {
        return Err("許可されていないパスです".into());
    }
    let cache_key = format!("luna_disc:{}", url);
    match luna_http(&state).await {
        Ok(http) => match luna_get(&http, &url).await {
            Ok(html) => {
                #[cfg(debug_assertions)]
                {
                    let dump_path = std::env::temp_dir().join(format!("luna_discussion_{}.html",
                        url.replace(['/', '?', '&'], "_")));
                    let _ = std::fs::write(&dump_path, &html);
                    log::info!("Discussion HTML dumped ({} bytes)", html.len());
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
    state: State<'_, AppState>,
    url: String,
    title: String,
    content: String,
) -> Result<String, String> {
    let http = luna_http(&state).await?;

    // Extract idnumber and forumId from the themetop URL
    let idnumber = extract_url_param(&url, "idnumber")
        .ok_or("idnumber が見つかりません")?;
    let forum_id = extract_url_param(&url, "forumId")
        .ok_or("forumId が見つかりません")?;

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

    // setthread page only has _csrf (no _cid), plus _method=put
    let csrf = extract_input_value(&html, "_csrf")
        .ok_or_else(|| {
            let has_form = html.contains("<form");
            let has_login = html.contains("linkCommonLogin") && html.contains("login-body");
            format!("_csrf トークンが見つかりません (len={}, has_form={}, login_page={})",
                html.len(), has_form, has_login)
        })?;

    log::info!("New thread: idnumber={}, forumId={}, title={}", idnumber, forum_id, title);

    // Build Quill Delta JSON for the content
    let content_json = serde_json::json!({
        "ops": [{"insert": format!("{}\n", content)}]
    }).to_string();

    // Step 2: POST with _method=put (Luna emulates PUT via POST)
    // Field names match the actual form: threadContentsText, threadContentsHtml, threadContents
    let post_params = vec![
        ("_csrf".to_string(), csrf),
        ("_method".to_string(), "put".to_string()),
        ("idnumber".to_string(), idnumber),
        ("forumId".to_string(), forum_id),
        ("threadId".to_string(), String::new()),
        ("groupId".to_string(), String::new()),
        ("threadTitle".to_string(), title),
        ("threadContentsText".to_string(), content_json),
        ("threadContentsHtml".to_string(), format!("<p>{}</p>", html_escape(&content))),
        ("threadContents".to_string(), content.clone()),
    ];

    let resp = luna_post(&http, "/lms/course/forums/setthread", &post_params).await?;

    if resp.contains("\"success\":false") {
        return Err(format!("投稿失敗: {}", crate::client::safe_truncate(&resp, 200)));
    }

    log::info!("New thread submitted successfully");
    Ok("スレッドを登録しました".to_string())
}

/// Reply to an existing thread
/// Flow: 1) GET thread page → extract _cid, _csrf, hidden fields
///       2) POST /lms/course/forums/thread (multipart) with Quill fields
#[tauri::command]
pub async fn luna_reply_discussion(
    state: State<'_, AppState>,
    url: String,
    content: String,
) -> Result<String, String> {
    let http = luna_http(&state).await?;

    // Fetch thread page to get tokens (with Referer from themetop)
    let referer_path = {
        let idn = extract_url_param(&url, "idnumber").unwrap_or_default();
        let fid = extract_url_param(&url, "forumId").unwrap_or_default();
        format!("/lms/course/forums/themetop?idnumber={}&forumId={}", idn, fid)
    };
    let html = luna_get_with_referer(&http, &url, &referer_path).await?;

    log::info!("Reply HTML fetched ({} bytes)", html.len());

    let cid = extract_input_value(&html, "_cid")
        .ok_or_else(|| {
            let has_form = html.contains("<form");
            let has_login = html.contains("linkCommonLogin") && html.contains("login-body");
            format!("_cid トークンが見つかりません (len={}, has_form={}, login_page={})",
                html.len(), has_form, has_login)
        })?;
    let csrf = extract_input_value(&html, "_csrf")
        .ok_or("_csrf トークンが見つかりません")?;
    let idnumber = extract_input_value(&html, "idnumber")
        .or_else(|| extract_url_param(&url, "idnumber"))
        .ok_or("idnumber が見つかりません")?;
    let forum_id = extract_input_value(&html, "forumId")
        .or_else(|| extract_url_param(&url, "forumId"))
        .ok_or("forumId が見つかりません")?;
    let thread_id = extract_input_value(&html, "threadId")
        .or_else(|| extract_url_param(&url, "threadId"))
        .ok_or("threadId が見つかりません")?;

    log::info!("Reply: idnumber={}, forumId={}, threadId={}", idnumber, forum_id, thread_id);

    // Extract additional hidden fields from the actual form
    let current_thread = extract_input_value(&html, "currentThread")
        .unwrap_or_else(|| "0".to_string());
    let address_type = extract_input_value(&html, "forum.addressType")
        .unwrap_or_else(|| "0".to_string());
    let group_id = extract_input_value(&html, "forum.groupId")
        .unwrap_or_default();
    let time_start = extract_input_value(&html, "forum.timeStart")
        .unwrap_or_default();

    let content_json = serde_json::json!({
        "ops": [{"insert": format!("{}\n", content)}]
    }).to_string();

    // Build multipart form matching the actual thread page form (enctype="multipart/form-data")
    let form = reqwest::multipart::Form::new()
        .text("_cid", cid)
        .text("_csrf", csrf)
        .text("idnumber", idnumber)
        .text("forumId", forum_id)
        .text("threadId", thread_id)
        .text("forum.addressType", address_type)
        .text("forum.groupId", group_id)
        .text("forum.timeStart", time_start)
        .text("currentThread", current_thread)
        .text("postContentsText", content_json)
        .text("postContentsHtml", format!("<p>{}</p>", html_escape(&content)))
        .text("postContents", content.clone())
        .text("postSendFlag", "false")
        .text("postId", "")
        .text("parentPostId", "")
        .text("editFlag", "1")
        .text("editAuthority", "");

    let resp = luna_post_multipart(&http, "/lms/course/forums/thread", form).await?;

    if resp.contains("\"success\":false") {
        return Err(format!("投稿失敗: {}", crate::client::safe_truncate(&resp, 200)));
    }

    log::info!("Reply submitted successfully");
    Ok("返信しました".to_string())
}

/// Fetch thread posts (the posts within a specific thread)
/// The thread page has a #threadPostList area loaded via form submit
#[tauri::command]
pub async fn luna_fetch_thread_posts(
    state: State<'_, AppState>,
    db: State<'_, crate::db::Database>,
    url: String,
) -> Result<luna_parser::LunaDiscussionThread, String> {
    if url.starts_with("http") || !url.starts_with('/') {
        return Err("許可されていないパスです".into());
    }
    let cache_key = format!("luna_thread:{}", url);
    match luna_http(&state).await {
        Ok(http) => match luna_get(&http, &url).await {
            Ok(html) => {
                #[cfg(debug_assertions)]
                {
                    let dump_path = std::env::temp_dir().join(format!("luna_thread_{}.html",
                        url.replace(['/', '?', '&'], "_")));
                    let _ = std::fs::write(&dump_path, &html);
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
    let region_end = crate::client::ceil_char_boundary(html, (pos + pattern.len() + 200).min(html.len()));
    let region = &html[region_start..region_end];
    let val_marker = "value=\"";
    let val_pos = region.find(val_marker)?;
    let rest = &region[val_pos + val_marker.len()..];
    let end = rest.find('"')?;
    let val = rest[..end].to_string();
    if !val.is_empty() { Some(val) } else { None }
}
