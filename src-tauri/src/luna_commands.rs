use tauri::{State, Emitter, Manager};
use crate::AppState;
use crate::auth;
use crate::luna_parser;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

static LUNA_DETAIL_COUNTER: AtomicU32 = AtomicU32::new(0);

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

    tauri::WebviewWindowBuilder::new(
        &app,
        &label,
        tauri::WebviewUrl::App(url_str.into()),
    )
    .title(&title)
    .inner_size(480.0, 560.0)
    .resizable(true)
    .build()
    .map_err(|e| format!("ウィンドウ作成失敗: {}", e))?;

    Ok(())
}

/// Open a URL in the default browser
#[tauri::command]
pub async fn luna_open_url(url: String) -> Result<(), String> {
    // Only allow http/https URLs to prevent command injection via custom schemes
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("無効なURLスキームです".into());
    }
    Command::new("open")
        .arg(&url)
        .spawn()
        .map_err(|e| format!("URLを開けませんでした: {}", e))?;
    Ok(())
}

/// Launch an LTI tool (Zoom, Panopto, etc.) and open the final URL in browser
#[tauri::command]
pub async fn luna_launch_lti(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let luna = state.luna.lock().await;
    let final_url = luna.launch_lti(&path).await?;
    drop(luna);
    if !final_url.starts_with("https://") && !final_url.starts_with("http://") {
        return Err("無効なURLスキームです".into());
    }
    Command::new("open")
        .arg(&final_url)
        .spawn()
        .map_err(|e| format!("URLを開けませんでした: {}", e))?;
    Ok(())
}

/// Reveal a file in Finder (restricted to app download directory)
#[tauri::command]
pub async fn luna_reveal_file(path: String) -> Result<(), String> {
    // Restrict to files under the user's Downloads or app data directory
    let p = std::path::Path::new(&path);
    let canonical = p.canonicalize().map_err(|e| format!("パスが無効です: {}", e))?;
    let allowed = dirs::download_dir().unwrap_or_else(|| {
        dirs::home_dir().map(|h| h.join("Downloads")).unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
    });
    if !canonical.starts_with(&allowed) {
        return Err("ダウンロードフォルダ外のファイルは表示できません".into());
    }
    Command::new("open")
        .arg("-R")
        .arg(&path)
        .spawn()
        .map_err(|e| format!("ファイルを表示できませんでした: {}", e))?;
    Ok(())
}

const LUNA_SAML_CALLBACK_HOST: &str = "luna-saml-callback.localhost";

/// Open Luna login window — uses the same Okta SSO
/// If user already has an active Okta session from KG-Course login,
/// Luna SAML should auto-authenticate
#[tauri::command]
pub async fn luna_open_login(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Get Luna's Okta SAML URL
    let luna = state.luna.lock().await;
    let saml_url = luna.initiate_saml_auth().await?;
    drop(luna);

    log::info!("Opening Luna login webview: {}", &saml_url[..120.min(saml_url.len())]);

    // Close existing Luna login window
    if let Some(existing) = app.get_webview_window("luna-login") {
        let _ = existing.close();
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<auth::SamlCallbackData>(1);

    let parsed_url: url::Url = saml_url.parse()
        .map_err(|e| format!("URL parse error: {}", e))?;

    let _win = tauri::WebviewWindowBuilder::new(
        &app,
        "luna-login",
        tauri::WebviewUrl::External(parsed_url),
    )
    .title("Luna - サインイン")
    .inner_size(480.0, 700.0)
    .resizable(true)
    .initialization_script(&crate::auth::saml_intercept_script(LUNA_SAML_CALLBACK_HOST))
    .on_navigation(move |url| {
        if url.host_str() == Some("luna-saml-callback.localhost") {
            let pairs: std::collections::HashMap<String, String> =
                url.query_pairs().into_owned().collect();
            if let Some(saml_response) = pairs.get("saml_response") {
                let data = auth::SamlCallbackData {
                    saml_response: saml_response.clone(),
                    relay_state: pairs.get("relay_state").cloned().unwrap_or_default(),
                    acs_url: pairs.get("acs_url").cloned().unwrap_or_default(),
                };
                log::info!("Intercepted Luna SAMLResponse (len={})", data.saml_response.len());
                let _ = tx.try_send(data);
            }
            return false;
        }
        true
    })
    .build()
    .map_err(|e| format!("Lunaログインウィンドウ作成失敗: {}", e))?;

    let app_clone = app.clone();
    tokio::spawn(async move {
        match rx.recv().await {
            Some(data) => {
                let app_state = app_clone.state::<AppState>();
                let mut luna = app_state.luna.lock().await;
                match luna.complete_saml_login(&data.saml_response, &data.relay_state).await {
                    Ok(()) => {
                        log::info!("Luna login successful");
                        let _ = app_clone.emit("luna-login-success", ());
                    }
                    Err(e) => {
                        log::error!("Luna login failed: {}", e);
                        let _ = app_clone.emit("luna-login-error", &e);
                    }
                }
                if let Some(win) = app_clone.get_webview_window("luna-login") {
                    let _ = win.close();
                }
            }
            None => {
                log::info!("Luna login cancelled");
            }
        }
    });

    Ok(())
}

/// Fetch a Luna page (generic)
#[tauri::command]
pub async fn luna_fetch_page(
    state: State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    // Only allow relative paths on Luna
    if path.contains("://") || !path.starts_with('/') {
        return Err("許可されていないパスです".into());
    }
    let luna = state.luna.lock().await;
    luna.fetch_page(&path).await
}

/// Check if Luna session is valid
#[tauri::command]
pub async fn luna_check_session(
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let luna = state.luna.lock().await;
    if !luna.authenticated {
        return Ok(false);
    }
    // Try fetching the top page to verify session
    match luna.fetch_page("/top").await {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// Fetch Luna top/dashboard page
#[tauri::command]
pub async fn luna_fetch_dashboard(
    state: State<'_, AppState>,
) -> Result<String, String> {
    let luna = state.luna.lock().await;
    luna.fetch_page("/top").await
}

/// Fetch Luna course list
#[tauri::command]
pub async fn luna_fetch_courses(
    state: State<'_, AppState>,
) -> Result<String, String> {
    let luna = state.luna.lock().await;
    luna.fetch_page("/course").await
}

/// Fetch Luna notifications/announcements
#[tauri::command]
pub async fn luna_fetch_notifications(
    state: State<'_, AppState>,
) -> Result<String, String> {
    let luna = state.luna.lock().await;
    luna.fetch_page("/notification").await
}

/// Fetch parsed timetable
#[tauri::command]
pub async fn luna_fetch_timetable(
    state: State<'_, AppState>,
    year: Option<String>,
    term: Option<String>,
) -> Result<luna_parser::LunaTimetable, String> {
    let luna = state.luna.lock().await;
    if let (Some(y), Some(t)) = (&year, &term) {
        if !is_safe_param(y) || !is_safe_param(t) {
            return Err("無効なパラメータです".into());
        }
        let path = format!("/lms/timetable?risyunen={}&kikanCd={}", y, t);
        let html = luna.fetch_page(&path).await?;
        return Ok(luna_parser::parse_luna_timetable(&html));
    }
    let html = luna.fetch_page("/lms/timetable").await?;
    Ok(luna_parser::parse_luna_timetable(&html))
}

/// Fetch parsed TODO list
#[tauri::command]
pub async fn luna_fetch_todo(
    state: State<'_, AppState>,
) -> Result<Vec<luna_parser::LunaTodoItem>, String> {
    let luna = state.luna.lock().await;
    let html = luna.fetch_page("/lms/todo").await?;
    Ok(luna_parser::parse_luna_todo(&html))
}

/// Fetch parsed notifications
#[tauri::command]
pub async fn luna_fetch_updates(
    state: State<'_, AppState>,
) -> Result<Vec<luna_parser::LunaNotification>, String> {
    let luna = state.luna.lock().await;
    let html = luna.fetch_page("/updateinfo").await?;
    Ok(luna_parser::parse_luna_notifications(&html))
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
    let luna = state.luna.lock().await;
    let path = format!("/lms/contents?idnumber={}", idnumber);
    luna.fetch_page(&path).await
}

/// Fetch and parse a Luna detail page (any path)
#[tauri::command]
pub async fn luna_fetch_detail(
    state: State<'_, AppState>,
    path: String,
) -> Result<luna_parser::LunaDetailPage, String> {
    let luna = state.luna.lock().await;
    let html = luna.fetch_page(&path).await?;
    // Debug: dump to /tmp for inspection
    #[cfg(debug_assertions)]
    {
        let filename = path.replace('/', "_").replace('?', "_").replace('&', "_");
        let dump_path = format!("/tmp/luna_detail{}.html", filename);
        let _ = std::fs::write(&dump_path, &html);
        log::info!("Luna detail HTML dumped to {} ({} bytes)", dump_path, html.len());
    }
    Ok(luna_parser::parse_luna_detail_page(&html))
}

/// Fetch announcement detail from Luna course page
#[tauri::command]
pub async fn luna_fetch_announcement_detail(
    state: State<'_, AppState>,
    idnumber: String,
    info_id: String,
) -> Result<luna_parser::LunaDetailPage, String> {
    if !is_safe_param(&idnumber) || !is_safe_param(&info_id) {
        return Err("無効なパラメータです".into());
    }
    let luna = state.luna.lock().await;
    let path = format!(
        "/lms/coursetop/information/listdetail?idnumber={}&informationId={}",
        idnumber, info_id
    );
    let html = luna.fetch_page(&path).await?;
    #[cfg(debug_assertions)]
    {
        let dump_path = format!("/tmp/luna_announcement_{}_{}.html", idnumber, info_id);
        let _ = std::fs::write(&dump_path, &html);
        log::info!("Luna announcement detail dumped ({} bytes)", html.len());
    }
    Ok(luna_parser::parse_luna_announcement_detail(&html))
}

/// Fetch and parse course top page (/lms/course?idnumber=XXX)
#[tauri::command]
pub async fn luna_fetch_course_detail(
    state: State<'_, AppState>,
    idnumber: String,
) -> Result<luna_parser::LunaCourseContents, String> {
    if !is_safe_param(&idnumber) {
        return Err("無効なパラメータです".into());
    }
    let luna = state.luna.lock().await;

    let course_path = format!("/lms/course?idnumber={}", idnumber);
    let contents_path = format!("/lms/contents?idnumber={}", idnumber);

    // Fetch course top page — Luna sometimes returns an incomplete/redirect page
    // on the very first access after session restore, so we retry once if menus are empty.
    let course_html = luna.fetch_page(&course_path).await?;
    let mut result = luna_parser::parse_luna_course_contents(&course_html, &idnumber);

    if result.menus.is_empty() {
        log::warn!("Course page for {} returned no menus ({}B), retrying...", idnumber, course_html.len());
        #[cfg(debug_assertions)]
        {
            let dump = format!("/tmp/luna_course_{}_initial.html", idnumber);
            let _ = std::fs::write(&dump, &course_html);
        }
        // Retry: the first request may have warmed up the Luna session/course state
        if let Ok(retry_html) = luna.fetch_page(&course_path).await {
            let retry_result = luna_parser::parse_luna_course_contents(&retry_html, &idnumber);
            if !retry_result.menus.is_empty() {
                log::info!("Retry succeeded for course {}", idnumber);
                result = retry_result;
            }
            #[cfg(debug_assertions)]
            {
                let dump = format!("/tmp/luna_course_{}.html", idnumber);
                let _ = std::fs::write(&dump, &retry_html);
            }
        }
    } else {
        #[cfg(debug_assertions)]
        {
            let dump = format!("/tmp/luna_course_{}.html", idnumber);
            let _ = std::fs::write(&dump, &course_html);
        }
    }

    // Fetch contents top page (actual content items)
    let contents_html = luna.fetch_page(&contents_path).await?;

    #[cfg(debug_assertions)]
    {
        let dump_path = format!("/tmp/luna_contents_{}.html", idnumber);
        let _ = std::fs::write(&dump_path, &contents_html);
    }

    // Merge actual content items from contents page
    let (materials, reports, examinations, discussions) = luna_parser::parse_luna_contents_page(&contents_html);
    result.materials = materials;
    result.reports = reports;
    result.examinations = examinations;
    result.discussions = discussions;

    Ok(result)
}

/// Download a Luna file attachment to the Downloads folder and return the saved path
#[tauri::command]
pub async fn luna_download_file(
    state: State<'_, AppState>,
    url: String,
    filename: String,
) -> Result<String, String> {
    let luna = state.luna.lock().await;

    // For external URLs (SharePoint etc.), just return the URL for the frontend to open
    if url.starts_with("http") {
        return Ok(url);
    }

    let bytes = luna.download_file(&url).await?;

    // Save to Downloads folder
    let downloads = dirs::download_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    let save_path = downloads.join(&filename);

    // Avoid overwriting: if file exists, add a number
    let final_path = if save_path.exists() {
        let stem = std::path::Path::new(&filename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("file");
        let ext = std::path::Path::new(&filename)
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
            if !candidate.exists() || i >= 999 {
                break candidate;
            }
            i += 1;
        }
    } else {
        save_path
    };

    std::fs::write(&final_path, &bytes)
        .map_err(|e| format!("ファイル保存失敗: {}", e))?;

    Ok(final_path.to_string_lossy().to_string())
}

/// Replicate Luna's CommonUtil.makeDownFileName JS function:
/// replace fullwidth/halfwidth spaces with _, collapse multiple _, then encodeURI
fn make_down_file_name(file_name: &str) -> String {
    // Replace fullwidth space (U+3000) and regular space with _
    let mut result = file_name.replace('\u{3000}', "_").replace(' ', "_");
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
    let luna = state.luna.lock().await;

    log::info!("Material download: file='{}', object='{}', resource='{}', type='{}', matId={:?}",
        file_name, object_name, resource_id, file_type, material_id);

    // Step 0: Visit the course contents page first to establish server-side session context
    // (the browser is always on this page when downloading)
    let course_url = format!("/lms/course?idnumber={}", idnumber);
    let _ = luna.fetch_page(&course_url).await;
    let _referer = format!("https://luna.kwansei.ac.jp/lms/course?idnumber={}", idnumber);

    // Step 1: Prepare tempfile (GET /lms/course/make/tempfile)
    // jQuery $.ajax({ type: "GET", data: params }) sends as query string
    // The response is a server-side temp path used as fileId in the download form
    let tempfile_query = format!(
        "fileName={}&objectName={}&id={}&idnumber={}",
        urlencoding::encode(&file_name),
        urlencoding::encode(&object_name),
        urlencoding::encode(&resource_id),
        urlencoding::encode(&idnumber),
    );
    let tempfile_url = format!("/lms/course/make/tempfile?{}", tempfile_query);
    log::info!("Material tempfile URL: {}", tempfile_url);
    let file_id = luna.fetch_page(&tempfile_url).await
        .map_err(|e| format!("ファイル準備失敗: {}", e))?;
    let file_id = file_id.trim().to_string();

    log::info!("Material tempfile returned fileId (len={}): '{}'", file_id.len(), &file_id[..file_id.len().min(500)]);

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

    let bytes = luna.download_file(&full_download_url).await?;

    log::info!("Material downloaded {} bytes", bytes.len());

    // Check if we got an HTML error page instead of the file
    if bytes.len() < 1000 {
        if let Ok(text) = std::str::from_utf8(&bytes) {
            if text.contains("<!DOCTYPE") || text.contains("<html") {
                log::error!("Download returned HTML instead of file: {}", &text[..text.len().min(500)]);
                return Err("サーバーがファイルではなくエラーページを返しました".into());
            }
        }
    }

    if bytes.is_empty() {
        return Err("ダウンロードされたファイルが空です".into());
    }

    // Save to Downloads folder
    let downloads = dirs::download_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    let save_path = downloads.join(&file_name);

    let final_path = if save_path.exists() {
        let stem = std::path::Path::new(&file_name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("file");
        let ext = std::path::Path::new(&file_name)
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
            if !candidate.exists() || i >= 999 {
                break candidate;
            }
            i += 1;
        }
    } else {
        save_path
    };

    std::fs::write(&final_path, &bytes)
        .map_err(|e| format!("ファイル保存失敗: {}", e))?;

    Ok(final_path.to_string_lossy().to_string())
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
    let luna = state.luna.lock().await;

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
    let page_html = luna.fetch_page(&submission_url).await?;

    let cid = extract_input_value(&page_html, "_cid")
        .ok_or("_cid トークンが見つかりません")?;
    let csrf = extract_input_value(&page_html, "_csrf")
        .ok_or("_csrf トークンが見つかりません")?;

    log::info!("Report tokens: _cid={}..., _csrf={}...", &cid[..8.min(cid.len())], &csrf[..8.min(csrf.len())]);

    // Step 2: Upload file via multipart POST
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

    let upload_resp = luna.post_multipart("/lms/course/report/upload", upload_form).await?;

    let upload_json: serde_json::Value = serde_json::from_str(&upload_resp)
        .map_err(|e| format!("アップロード応答の解析失敗: {} — body: {}", e, &upload_resp[..200.min(upload_resp.len())]))?;

    if upload_json.get("success").and_then(|v| v.as_bool()) != Some(true) {
        return Err(format!("アップロード失敗: {}", upload_resp));
    }

    let file_id = upload_json.get("fileId")
        .and_then(|v| v.as_str().map(|s| s.to_string()).or_else(|| v.as_i64().map(|n| n.to_string())))
        .ok_or("fileId が見つかりません")?;

    log::info!("Report file uploaded: fileId={}", file_id);

    // Step 3: Submit the report
    let submit_params = vec![
        ("_cid".to_string(), cid),
        ("_csrf".to_string(), csrf),
        ("method".to_string(), "0".to_string()),
        ("idnumber".to_string(), idnumber),
        ("reportId".to_string(), report_id),
        ("fileId[0]".to_string(), file_id),
        ("originalFileName[0]".to_string(), file_name.clone()),
        ("deleteFlag[0]".to_string(), "0".to_string()),
        ("rowCounter".to_string(), "1".to_string()),
    ];

    let _submit_resp = luna.post_form("/lms/course/report/submission", &submit_params).await?;

    log::info!("Report submitted successfully");
    Ok(format!("「{}」を提出しました", file_name))
}

/// Fetch discussion thread detail (posts list) from Luna
#[tauri::command]
pub async fn luna_fetch_discussion_detail(
    state: State<'_, AppState>,
    url: String,
) -> Result<luna_parser::LunaDiscussionThread, String> {
    let luna = state.luna.lock().await;
    let html = luna.fetch_page(&url).await?;
    #[cfg(debug_assertions)]
    {
        let dump_path = format!("/tmp/luna_discussion_{}.html",
            url.replace('/', "_").replace('?', "_").replace('&', "_"));
        let _ = std::fs::write(&dump_path, &html);
        log::info!("Discussion HTML dumped ({} bytes)", html.len());
    }
    Ok(luna_parser::parse_luna_discussion_thread(&html))
}

/// Post a new thread to a Luna discussion forum
/// Flow: 1) GET setthread page → extract _cid, _csrf
///       2) POST /lms/course/forums/setthread with title + content
#[tauri::command]
pub async fn luna_post_discussion(
    state: State<'_, AppState>,
    url: String,
    title: String,
    content: String,
) -> Result<String, String> {
    let luna = state.luna.lock().await;

    // Extract idnumber and forumId from the themetop URL
    let idnumber = extract_url_param(&url, "idnumber")
        .ok_or("idnumber が見つかりません")?;
    let forum_id = extract_url_param(&url, "forumId")
        .ok_or("forumId が見つかりません")?;

    // Step 1: Fetch the setthread page to get tokens
    let setthread_url = format!(
        "/lms/course/forums/setthread?idnumber={}&forumId={}&threadId=&groupId=",
        idnumber, forum_id
    );
    let html = luna.fetch_page(&setthread_url).await?;

    // Dump for debugging
    #[cfg(debug_assertions)]
    {
        let dump_path = format!("/tmp/luna_setthread_{}.html", forum_id);
        let _ = std::fs::write(&dump_path, &html);
        log::info!("Setthread HTML dumped ({} bytes)", html.len());
    }

    let cid = extract_input_value(&html, "_cid")
        .ok_or("_cid トークンが見つかりません")?;
    let csrf = extract_input_value(&html, "_csrf")
        .ok_or("_csrf トークンが見つかりません")?;

    log::info!("New thread: idnumber={}, forumId={}, title={}", idnumber, forum_id, title);

    // Build Quill Delta JSON for the content
    let content_json = serde_json::json!({
        "ops": [{"insert": format!("{}\n", content)}]
    }).to_string();

    // Step 2: POST the new thread
    // setthread page likely uses multipart like other Luna forms
    let post_params = vec![
        ("_cid".to_string(), cid),
        ("_csrf".to_string(), csrf),
        ("idnumber".to_string(), idnumber),
        ("forumId".to_string(), forum_id),
        ("threadId".to_string(), String::new()),
        ("groupId".to_string(), String::new()),
        ("threadTitle".to_string(), title),
        ("contents".to_string(), content_json.clone()),
        ("contentsText".to_string(), content_json),
        ("contentsHtml".to_string(), format!("<p>{}</p>", html_escape(&content))),
    ];

    let resp = luna.post_form("/lms/course/forums/setthread", &post_params).await?;

    if resp.contains("error") && resp.contains("\"success\":false") {
        return Err(format!("投稿失敗: {}", &resp[..200.min(resp.len())]));
    }

    log::info!("New thread submitted successfully");
    Ok("スレッドを登録しました".to_string())
}

/// Reply to an existing thread
/// Flow: 1) GET thread page → extract _cid, _csrf, hidden fields
///       2) POST /lms/course/forums/thread with postContentsText (Quill JSON)
#[tauri::command]
pub async fn luna_reply_discussion(
    state: State<'_, AppState>,
    url: String,
    content: String,
) -> Result<String, String> {
    let luna = state.luna.lock().await;

    // Fetch thread page to get tokens
    let html = luna.fetch_page(&url).await?;

    let cid = extract_input_value(&html, "_cid")
        .ok_or("_cid トークンが見つかりません")?;
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

    // Extract additional hidden fields
    let time_start = extract_input_value(&html, "forum.timeStart")
        .unwrap_or_default();
    let forum_title = extract_input_value(&html, "forum.title")
        .unwrap_or_default();
    let forum_contents = extract_input_value(&html, "forum.contents")
        .unwrap_or_default();

    let content_json = serde_json::json!({
        "ops": [{"insert": format!("{}\n", content)}]
    }).to_string();

    // Build multipart form (thread page uses enctype="multipart/form-data")
    let form = reqwest::multipart::Form::new()
        .text("_cid", cid)
        .text("_csrf", csrf)
        .text("idnumber", idnumber)
        .text("forumId", forum_id)
        .text("threadId", thread_id)
        .text("postId", "")
        .text("parentPostId", "")
        .text("editFlag", "1")
        .text("editAuthority", "")
        .text("currentThread", "0")
        .text("postContentsText", content_json)
        .text("postContentsHtml", format!("<p>{}</p>", html_escape(&content)))
        .text("postContents", content.clone())
        .text("postSendFlag", "false")
        .text("forum.addressType", "0")
        .text("forum.groupId", "")
        .text("forum.timeStart", time_start)
        .text("forum.title", forum_title)
        .text("forum.contents", forum_contents);

    let resp = luna.post_multipart("/lms/course/forums/thread", form).await?;

    if resp.contains("error") && resp.contains("\"success\":false") {
        return Err(format!("投稿失敗: {}", &resp[..200.min(resp.len())]));
    }

    log::info!("Reply submitted successfully");
    Ok("返信しました".to_string())
}

/// Fetch thread posts (the posts within a specific thread)
/// The thread page has a #threadPostList area loaded via form submit
#[tauri::command]
pub async fn luna_fetch_thread_posts(
    state: State<'_, AppState>,
    url: String,
) -> Result<luna_parser::LunaDiscussionThread, String> {
    let luna = state.luna.lock().await;
    let html = luna.fetch_page(&url).await?;

    #[cfg(debug_assertions)]
    {
        let dump_path = format!("/tmp/luna_thread_{}.html",
            url.replace('/', "_").replace('?', "_").replace('&', "_"));
        let _ = std::fs::write(&dump_path, &html);
    }

    Ok(luna_parser::parse_luna_thread_detail(&html))
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
    let region_start = if pos > 200 { pos - 200 } else { 0 };
    let region_end = (pos + pattern.len() + 200).min(html.len());
    let region = &html[region_start..region_end];
    let val_marker = "value=\"";
    let val_pos = region.find(val_marker)?;
    let rest = &region[val_pos + val_marker.len()..];
    let end = rest.find('"')?;
    let val = rest[..end].to_string();
    if !val.is_empty() { Some(val) } else { None }
}
