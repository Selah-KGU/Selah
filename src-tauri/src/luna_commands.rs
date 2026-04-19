use tauri::{State, Manager};
use crate::LunaState;
use crate::config;
use crate::client;
use crate::luna_client;
use crate::luna_parser;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::LazyLock;

static LUNA_DETAIL_COUNTER: AtomicU32 = AtomicU32::new(0);

// ── Cached selectors (compiled once, reused across all calls) ──
macro_rules! sel {
    ($name:ident, $s:expr) => {
        static $name: LazyLock<scraper::Selector> =
            LazyLock::new(|| scraper::Selector::parse($s).expect(concat!("bad selector: ", $s)));
    };
}
sel!(SEL_META_REFRESH,  "meta[http-equiv='refresh']");
sel!(SEL_IFRAME_SRC,    "iframe[src]");
sel!(SEL_SCRIPT,        "script");
sel!(SEL_A_HREF,        "a[href]");
sel!(SEL_BODY,          "body");
sel!(SEL_FORM,          "form");
sel!(SEL_REPORT_FORM,   "form#reportSubmissionForm");
sel!(SEL_HIDDEN_INPUT,  "input[type='hidden']");

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

/// Save bytes to the download folder with conflict avoidance (appends " (N)" if the file exists).
/// If course_name is provided and classify_by_course is enabled, saves into a course subfolder.
fn save_to_downloads(filename: &str, bytes: &[u8], course_name: Option<&str>) -> Result<String, String> {
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

    std::fs::write(&final_path, bytes)
        .map_err(|e| format!("ファイル保存失敗: {}", e))?;

    let path_str = final_path.to_string_lossy().to_string();
    crate::commands::record_download(filename, &path_str, course_name, "luna", bytes.len() as u64);

    Ok(path_str)
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

/// Open a Luna detail page in a separate native window
#[allow(clippy::too_many_arguments)]
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
    course_name: Option<String>,
) -> Result<(), String> {
    let existing = app.webview_windows().keys()
        .filter(|k| k.starts_with("luna-detail-")).count();
    if existing >= 10 {
        return Err(config::TOO_MANY_WINDOWS_MSG.into());
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
            let mut parts = format!(
                "luna-detail.html?mode=announcement&title={}&idnumber={}&infoId={}",
                urlencoding::encode(&title),
                urlencoding::encode(idnumber.as_deref().unwrap_or("")),
                urlencoding::encode(info_id.as_deref().unwrap_or(""))
            );
            if let Some(cn) = &course_name {
                parts.push_str(&format!("&courseName={}", urlencoding::encode(cn)));
            }
            parts
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
            if let Some(cn) = &course_name {
                parts.push_str(&format!("&courseName={}", urlencoding::encode(cn)));
            }
            parts
        }
        Some("survey") | Some("questionnaire") => {
            let mut parts = format!(
                "luna-detail.html?mode=survey&path={}&title={}",
                urlencoding::encode(&path),
                urlencoding::encode(&title)
            );
            if let Some(cn) = &course_name {
                parts.push_str(&format!("&courseName={}", urlencoding::encode(cn)));
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
        Some("attendance") => {
            format!(
                "luna-detail.html?mode=attendance&idnumber={}&title={}",
                urlencoding::encode(idnumber.as_deref().unwrap_or("")),
                urlencoding::encode(&title)
            )
        }
        _ => {
            let mut parts = format!("luna-detail.html?path={}&title={}", urlencoding::encode(&path), urlencoding::encode(&title));
            if let Some(cn) = &course_name {
                parts.push_str(&format!("&courseName={}", urlencoding::encode(cn)));
            }
            parts
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
pub async fn luna_launch_lti(app: tauri::AppHandle, state: State<'_, LunaState>, path: String) -> Result<(), String> {
    let http = luna_http(&state).await?;
    let final_url = luna_client::launch_lti(&http, &path).await?;
    crate::commands::open_external_url(app, final_url, None).await
}

/// Reveal a file in Finder (restricted to app download directory)
#[tauri::command]
pub async fn luna_reveal_file(app: tauri::AppHandle, path: String) -> Result<(), String> {
    // Restrict to files under the user's Downloads or configured download directory
    let p = std::path::Path::new(&path);
    let canonical = p.canonicalize().map_err(|e| format!("パスが無効です: {}", e))?;
    let sys_downloads = crate::commands::default_download_dir();
    let dl_config = crate::commands::load_download_config();
    let custom_dir = if dl_config.download_dir.is_empty() { None } else {
        std::path::Path::new(&dl_config.download_dir).canonicalize().ok()
    };
    let sys_dl = dirs::download_dir().unwrap_or_else(|| {
        dirs::home_dir().map(|h| h.join("Downloads")).unwrap_or_else(std::env::temp_dir)
    });
    let allowed = canonical.starts_with(&sys_downloads)
        || canonical.starts_with(&sys_dl)
        || custom_dir.as_ref().is_some_and(|d| canonical.starts_with(d));
    if !allowed {
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
    state: State<'_, LunaState>,
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
    state: State<'_, LunaState>,
) -> Result<bool, String> {
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
        &http, &url, crate::config::LUNA_BASE,
        crate::luna_client::LUNA_SESSION_EXPIRED_MSG, crate::luna_client::is_luna_session_expired,
    ).await {
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
    luna_fetch_cached(&state, &db, "/lms/todo", "luna_todo", luna_parser::parse_luna_todo).await
}

/// Fetch parsed notifications
#[tauri::command]
pub async fn luna_fetch_updates(
    state: State<'_, LunaState>,
    db: State<'_, crate::db::Database>,
) -> Result<Vec<luna_parser::LunaNotification>, String> {
    luna_fetch_cached(&state, &db, "/updateinfo", "luna_updates", luna_parser::parse_luna_notifications).await
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
    state: State<'_, LunaState>,
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
                    let filename = path.replace(['/', '?', '&'], "_");
                    let dump_path = std::env::temp_dir().join(format!("luna_survey{}.html", filename));
                    let _ = std::fs::write(&dump_path, &html);
                    log::info!("Luna survey detail dumped to {} ({} bytes)", dump_path.display(), html.len());
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

/// Submit survey answers to Luna
#[tauri::command]
pub async fn luna_submit_survey(
    state: State<'_, LunaState>,
    form_fields: Vec<(String, String)>,
    answers: std::collections::HashMap<String, String>,
) -> Result<(), String> {
    // Build the full POST params: hidden fields + user answers
    let mut params: Vec<(String, String)> = Vec::new();

    // Add all hidden form fields (includes _cid, _csrf, idnumber, surveyId, takeFlag,
    // answer[N].surveyNo, answer[N].surveyNoSub, answerDetail[N].*, enableSurveyItems[N])
    for (k, v) in &form_fields {
        params.push((k.clone(), v.clone()));
    }

    // Merge user answers: answers map is {questionIndex: selectedValue}
    // The form field is answer[N].answerItem[0].answer = selectedValue
    for (idx_str, value) in &answers {
        let idx: usize = idx_str.parse().map_err(|_| "無効な質問インデックスです")?;
        let field_name = format!("answer[{}].answerItem[0].answer", idx);
        // Replace existing empty field or add new one
        let mut found = false;
        for p in &mut params {
            if p.0 == field_name {
                p.1 = value.clone();
                found = true;
                break;
            }
        }
        if !found {
            params.push((field_name, value.clone()));
        }
    }

    let http = luna_http(&state).await?;
    let response = luna_post(&http, "/lms/course/surveys/take", &params).await?;

    // Check for error indicators in the response
    if response.contains("エラー") && response.contains("回答期間を過ぎている") {
        return Err("回答期間を過ぎています".into());
    }

    Ok(())
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
        idnumber,
        attendance_id
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
        if html.contains("完了") || html.contains("登録しました") || html.contains("登録済") {
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
        let builder = http.post(&submit_url)
            .header("Referer", &referer)
            .form(&fields);
        html = client::send_and_follow_redirect(
            &http,
            builder,
            config::LUNA_BASE,
            luna_client::LUNA_SESSION_EXPIRED_MSG,
            luna_client::is_luna_session_expired,
        ).await?;
    }

    if html.contains("完了") || html.contains("登録しました") || html.contains("登録済") {
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
    let (materials, reports, examinations, discussions, surveys) = luna_parser::parse_luna_contents_page(&contents_html);
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
    // For external URLs (SharePoint etc.), just return the URL for the frontend to open
    if url.starts_with("http") {
        return Ok(url);
    }

    let http = luna_http(&state).await?;

    // Mode 2: Structured attachment — GET form submit (mirrors browser form.submit())
    // Luna JS modifies the form action to: {action}/{makeDownFileName(name)}
    // then submits as GET with form fields as query params.
    // download_params contains ALL query fields (static + per-file dynamic, merged by parser)
    let bytes = if url.is_empty() {
        let action = download_action.as_deref().unwrap_or("");

        if action.is_empty() {
            return Err("ダウンロードURLが見つかりません".into());
        }

        // Build query string from pre-merged form fields
        let mut params: Vec<String> = Vec::new();
        if let Some(ref fields) = download_params {
            for (k, v) in fields {
                params.push(format!("{}={}", form_encode(k), form_encode(v)));
            }
        }

        // Action URL path includes makeDownFileName(filename) — set by JS before submit
        let path_name = make_down_file_name(&filename);
        let download_url = format!("{}/{}?{}", action, path_name, params.join("&"));

        log::info!("Attachment GET: url='{}'", download_url);
        luna_download(&http, &download_url).await?
    } else {
        log::info!("Attachment GET download: url='{}', filename='{}'", url, filename);
        luna_download(&http, &url).await?
    };

    log::info!("Attachment downloaded {} bytes for '{}'", bytes.len(), filename);

    if bytes.is_empty() {
        return Err("ダウンロードされたファイルが空です".into());
    }

    // Check if we got an HTML error page instead of the actual file
    if bytes.len() < 2000 {
        if let Ok(text) = std::str::from_utf8(&bytes) {
            if text.contains("<!DOCTYPE") || text.contains("<html") || text.contains("<HTML") {
                log::error!("Attachment download returned HTML instead of file: {}", crate::client::safe_truncate(text, 500));
                return Err("サーバーがファイルではなくエラーページを返しました".into());
            }
        }
    }

    save_to_downloads(&filename, &bytes, course_name.as_deref())
}

/// Replicate Luna's CommonUtil.makeDownFileName JS function:
/// replace fullwidth/halfwidth spaces with _, collapse multiple _, then encodeURI
pub(crate) fn make_down_file_name(file_name: &str) -> String {
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

    log::info!("Material link resolve: file='{}', resource='{}', type='{}'",
        file_name, resource_id, file_type);

    let course_url = format!("/lms/course?idnumber={}", idnumber);
    let _ = luna_get(&http, &course_url).await;

    // Step 1: Prepare tempfile
    let tempfile_query = format!(
        "fileName={}&objectName={}&id={}&idnumber={}",
        urlencoding::encode(&file_name),
        urlencoding::encode(&object_name),
        urlencoding::encode(&resource_id),
        urlencoding::encode(&idnumber),
    );
    let tempfile_url = format!("/lms/course/make/tempfile?{}", tempfile_query);
    let file_id = luna_get(&http, &tempfile_url).await
        .map_err(|e| format!("Failed to prepare tempfile: {}", e))?;
    let file_id = file_id.trim().to_string();

    if file_id.contains('<') || file_id.is_empty() {
        return Err(format!("tempfile returned unexpected response (len={})", file_id.len()));
    }

    // Step 2: Fetch HTML via sethtmlfiledown
    let path_encoded_name = make_down_file_name(&file_name);
    let base_path = format!("/lms/course/materialref/sethtmlfiledown/{}", path_encoded_name);
    let dl_title = display_name.unwrap_or_default();
    let content_id = material_id.unwrap_or_default();
    let end_date_val = end_date.unwrap_or_default();

    let query_string = format!(
        "fileName={}&fileId={}&idnumber={}&resourceId={}&screen=1&contentId={}&endDate={}&title={}",
        form_encode(&file_name),
        form_encode(&file_id),
        form_encode(&idnumber),
        form_encode(&resource_id),
        form_encode(&content_id),
        form_encode(&end_date_val),
        form_encode(&dl_title),
    );
    let full_url = format!("{}{}?{}", config::LUNA_BASE, base_path, query_string);

    let resp = http.get(&full_url).send().await
        .map_err(|e| format!("Request failed: {}", e))?;
    let final_url = resp.url().to_string();
    let html = resp.text().await.unwrap_or_default();

    log::info!("Material link HTML (len={}, final_url={}): {}", html.len(), final_url, crate::client::safe_truncate(&html, 1000));

    // If we were redirected to an external URL, return that
    if !final_url.contains("luna.kwansei.ac.jp") {
        return Ok(final_url);
    }

    // Try to extract URL from the HTML content
    // 1) meta refresh: <meta http-equiv="refresh" content="0;url=...">
    // 2) window.location / location.href in script
    // 3) iframe src
    // 4) anchor href
    let doc = scraper::Html::parse_document(&html);

    // meta refresh
    if let Some(meta) = doc.select(&SEL_META_REFRESH).next() {
        if let Some(content) = meta.value().attr("content") {
            if let Some(idx) = content.to_lowercase().find("url=") {
                let url = content[idx + 4..].trim().trim_matches(|c| c == '\'' || c == '"');
                if !url.is_empty() {
                    return Ok(url.to_string());
                }
            }
        }
    }

    // iframe src
    if let Some(iframe) = doc.select(&SEL_IFRAME_SRC).next() {
        if let Some(src) = iframe.value().attr("src") {
            if src.starts_with("http") {
                return Ok(src.to_string());
            }
        }
    }

    // window.location or location.href in script
    for script in doc.select(&SEL_SCRIPT) {
        let text = script.text().collect::<String>();
        for pattern in &["window.location.href", "window.location", "location.href", "window.open("] {
            if let Some(idx) = text.find(pattern) {
                let after = &text[idx + pattern.len()..];
                // Find URL in quotes after = or (
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

    // <a> with external href
    for a in doc.select(&SEL_A_HREF) {
        if let Some(href) = a.value().attr("href") {
            if href.starts_with("http") && !href.contains("luna.kwansei.ac.jp") {
                return Ok(href.to_string());
            }
        }
    }

    // Fallback: if the HTML body itself looks like a plain URL
    let body_text = doc.select(&SEL_BODY).next()
        .map(|b| b.text().collect::<String>().trim().to_string())
        .unwrap_or_default();
    if body_text.starts_with("http") && !body_text.contains(' ') {
        return Ok(body_text);
    }

    Err("リンク先のURLを抽出できませんでした".into())
}

/// Detect report submission type by fetching the submission page
/// Returns "text", "file", or "both"
#[tauri::command]
pub async fn luna_check_report_type(
    state: State<'_, LunaState>,
    idnumber: String,
    report_id: String,
) -> Result<String, String> {
    let http = luna_http(&state).await?;
    let url = format!(
        "/lms/course/report/submission?idnumber={}&reportId={}",
        idnumber, report_id
    );
    let html = luna_get(&http, &url).await?;

    let has_textarea = html.contains("id=\"submissionText\"") || html.contains("name=\"submissionText\"");
    // File upload: look for file input or drag-and-drop area
    let has_file = html.contains("id=\"uploadFile\"")
        || html.contains("name=\"uploadFile\"")
        || html.contains("type=\"file\"")
        || html.contains("dragAndDrop");

    let result = match (has_textarea, has_file) {
        (true, true) => "both",
        (true, false) => "text",
        (false, true) => "file",
        (false, false) => "file", // default fallback
    };
    log::info!("Report type detection: idnumber={}, reportId={}, textarea={}, file={} → {}", idnumber, report_id, has_textarea, has_file, result);
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

    #[cfg(debug_assertions)]
    {
        let dump_path = std::env::temp_dir().join("luna_report_confirm.html");
        let _ = std::fs::write(&dump_path, &confirm_html);
        log::info!("Report confirm page dumped to {} ({} bytes)", dump_path.display(), confirm_html.len());
    }

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

    #[cfg(debug_assertions)]
    {
        let dump_path2 = std::env::temp_dir().join("luna_report_register_result.html");
        let _ = std::fs::write(&dump_path2, &register_resp);
        log::info!("Report register response dumped to {} ({} bytes)", dump_path2.display(), register_resp.len());
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
            let dump_path3 = std::env::temp_dir().join("luna_report_verify.html");
            let _ = std::fs::write(&dump_path3, &verify_html);
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
        log::info!("Report registration completed (response {} bytes)", register_resp.len());
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
    submission_text: String,
) -> Result<String, String> {
    let http = luna_http(&state).await?;

    if submission_text.trim().is_empty() {
        return Err("提出テキストが空です".into());
    }

    log::info!("Text report submission: idnumber={}, reportId={}, text_len={}", idnumber, report_id, submission_text.len());

    // Step 1: Fetch the submission page for tokens
    let submission_url = format!(
        "/lms/course/report/submission?idnumber={}&reportId={}",
        idnumber, report_id
    );
    let page_html = luna_get(&http, &submission_url).await?;

    let cid = extract_input_value(&page_html, "_cid")
        .ok_or("_cid トークンが見つかりません")?;
    let csrf = extract_input_value(&page_html, "_csrf")
        .ok_or("_csrf トークンが見つかりません")?;

    log::info!("Text report tokens: _cid={}..., _csrf={}...", crate::client::safe_truncate(&cid, 8), crate::client::safe_truncate(&csrf, 8));

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
    let raw_resp = http.post(&submit_url)
        .form(&submit_params)
        .send().await
        .map_err(|e| format!("提出リクエスト失敗: {}", e))?;

    let step2_status = raw_resp.status();
    log::info!("Text report step 2: status={}", step2_status);

    let confirm_html = if step2_status.is_redirection() {
        if let Some(loc) = raw_resp.headers().get("location").and_then(|v| v.to_str().ok()) {
            let next_url = if loc.starts_with('/') {
                format!("{}{}", config::LUNA_BASE, loc)
            } else {
                loc.to_string()
            };
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

    #[cfg(debug_assertions)]
    {
        let dump_path = std::env::temp_dir().join("luna_report_text_confirm.html");
        let _ = std::fs::write(&dump_path, &confirm_html);
        log::info!("Text report confirm page dumped to {} ({} bytes)", dump_path.display(), confirm_html.len());
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
        let confirm_cid = extract_input_value(&confirm_html, "_cid")
            .unwrap_or_else(|| cid.clone());
        let confirm_csrf = extract_input_value(&confirm_html, "_csrf")
            .unwrap_or(csrf);

        let mut action = String::new();
        let mut fields: Vec<(String, String)> = Vec::new();

        if let Some(form_el) = confirm_doc.select(&SEL_REPORT_FORM).next()
            .or_else(|| {
                confirm_doc.select(&SEL_FORM).find(|f| {
                    f.value().attr("action")
                        .map(|a| a.contains("/report/submission") && !a.contains("download"))
                        .unwrap_or(false)
                })
            })
        {
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

        log::info!("Text report step 3 fields: {:?}", fields.iter().map(|(k,v)| format!("{}={}", k, client::safe_truncate(v, 20))).collect::<Vec<_>>());

        (action, fields)
    };

    if register_action.is_empty() {
        if confirm_html.contains("提出が完了") || confirm_html.contains("完了しました") || confirm_html.contains("提出済") {
            return Ok("テキストを提出しました".into());
        }
        return Err("確認画面に登録フォームが見つかりません".into());
    }

    let register_url = format!("{}{}", config::LUNA_BASE, register_action);
    let raw_resp3 = http.post(&register_url)
        .form(&register_fields)
        .send().await
        .map_err(|e| format!("登録リクエスト失敗: {}", e))?;

    let step3_status = raw_resp3.status();
    log::info!("Text report step 3: status={}", step3_status);

    let _register_resp = if step3_status.is_redirection() {
        if let Some(loc) = raw_resp3.headers().get("location").and_then(|v| v.to_str().ok()) {
            let next_url = if loc.starts_with('/') {
                format!("{}{}", config::LUNA_BASE, loc)
            } else {
                loc.to_string()
            };
            client::fetch_with_redirect(
                &http, &next_url, config::LUNA_BASE,
                luna_client::LUNA_SESSION_EXPIRED_MSG, luna_client::is_luna_session_expired,
            ).await?
        } else {
            raw_resp3.text().await.map_err(|e| format!("レスポンス読取失敗: {}", e))?
        }
    } else {
        raw_resp3.text().await.map_err(|e| format!("レスポンス読取失敗: {}", e))?
    };

    #[cfg(debug_assertions)]
    {
        let dump_path2 = std::env::temp_dir().join("luna_report_text_result.html");
        let _ = std::fs::write(&dump_path2, &_register_resp);
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
    state: State<'_, LunaState>,
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
    state: State<'_, LunaState>,
    url: String,
    content: String,
    parent_post_id: Option<String>,
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
        .text("parentPostId", parent_post_id.unwrap_or_default())
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
    state: State<'_, LunaState>,
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

/// Extract first matching form action + fields (hidden/text/textarea/select).
fn extract_form_fields(html: &str, action_hint: &str) -> Option<(String, Vec<(String, String)>)> {
    sel!(SEL_INPUT_NAME,    "input[name]");
    sel!(SEL_TEXTAREA_NAME, "textarea[name]");
    sel!(SEL_SELECT_NAME,   "select[name]");
    sel!(SEL_OPT_SELECTED,  "option[selected]");
    sel!(SEL_OPTION,        "option");

    let doc = scraper::Html::parse_document(html);

    let mut fallback: Option<(String, Vec<(String, String)>)> = None;
    for form in doc.select(&SEL_FORM) {
        let action = form.value().attr("action").unwrap_or_default().to_string();
        let mut fields = Vec::new();

        for input in form.select(&SEL_INPUT_NAME) {
            let name = input.value().attr("name").unwrap_or_default();
            let typ = input.value().attr("type").unwrap_or("text").to_ascii_lowercase();
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
            let value = se.select(&SEL_OPT_SELECTED).next()
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
    fields.iter().find_map(|(k, v)| if k == key { Some(v.as_str()) } else { None })
}
