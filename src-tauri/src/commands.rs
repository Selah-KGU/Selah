use crate::client;
use crate::config;
use crate::notifier;
use crate::stt;
use serde::Serialize;
#[cfg(debug_assertions)]
use std::sync::LazyLock;
#[cfg(debug_assertions)]
use std::time::Instant;
use tauri::Manager;
use tauri::State;

use crate::parser;
use crate::KgcState;

#[path = "commands/app_config.rs"]
mod app_config;
#[path = "commands/auth_session.rs"]
mod auth_session;
#[path = "commands/browser.rs"]
mod browser;
#[path = "commands/downloads.rs"]
mod downloads;
#[path = "commands/session.rs"]
mod session;
#[path = "commands/syllabus.rs"]
mod syllabus;
#[path = "commands/weather.rs"]
mod weather;

pub use app_config::*;
pub use auth_session::*;
pub use browser::*;
pub use downloads::*;
pub use session::*;
pub use syllabus::*;
pub use weather::*;

/// Briefly lock KGC client, check auth and clone http. Releases lock immediately.
async fn kgc_http(state: &KgcState) -> Result<reqwest::Client, String> {
    let client = state.client.lock().await;
    if !client.is_authenticated() {
        return Err(config::KGC_AUTH_REQUIRED_MSG.into());
    }
    Ok(client.http.clone())
}

/// KGC GET: fetch a page without holding the lock.
pub(crate) async fn kgc_get(http: &reqwest::Client, path: &str) -> Result<String, String> {
    let url = format!("{}{}", config::KG_COURSE_BASE, path);
    client::fetch_page_with(http, &url).await
}

/// KGC POST: submit a form without holding the lock.
pub(crate) async fn kgc_post(
    http: &reqwest::Client,
    path: &str,
    params: &[(String, String)],
) -> Result<String, String> {
    let url = format!("{}{}", config::KG_COURSE_BASE, path);
    client::post_form_with_redirect(
        http,
        &url,
        config::KG_COURSE_BASE,
        client::SESSION_EXPIRED_MSG,
        client::is_session_expired_body,
        params.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        &[
            (
                "Referer",
                &format!("{}/uniasv2/ARF010.do", config::KG_COURSE_BASE),
            ),
            ("Origin", config::KG_COURSE_BASE),
        ],
    )
    .await
}

/// KGC fetch with gate + auth check, returning raw HTML (no early-return on error).
async fn kgc_try_fetch(state: &KgcState, path: &str) -> Result<String, String> {
    let _kgc_gate = state.gate.lock().await;
    let (http, is_auth) = {
        let client = state.client.lock().await;
        (client.http.clone(), client.is_authenticated())
    };
    if !is_auth {
        return Err(config::KGC_AUTH_REQUIRED_MSG.into());
    }
    let url = format!("{}{}", config::KG_COURSE_BASE, path);
    client::fetch_with_redirect(
        &http,
        &url,
        config::KG_COURSE_BASE,
        client::SESSION_EXPIRED_MSG,
        client::is_session_expired_body,
    )
    .await
}

/// Fetch from KGC with DB cache fallback.
/// On success: parse, save to cache, return.
/// On failure: try returning cached data; if no cache, propagate error.
macro_rules! kgc_fetch_cached {
    ($state:expr, $db:expr, $cache_key:expr, $path:expr, $parser:expr) => {{
        match kgc_try_fetch(&$state, $path).await {
            Ok(html) => {
                let data = $parser(&html);
                if let Ok(json) = serde_json::to_string(&data) {
                    let _ = $db.save_data_cache($cache_key, &json);
                }
                Ok(data)
            }
            Err(e) => {
                if let Ok(Some((json, _))) = $db.get_data_cache($cache_key) {
                    if let Ok(cached) = serde_json::from_str(&json) {
                        log::info!("{}: cache fallback ({})", $cache_key, e);
                        return Ok(cached);
                    }
                }
                Err(e)
            }
        }
    }};
    ($state:expr, $db:expr, $cache_key:expr, $path:expr, $parser:expr, $dump:expr) => {{
        match kgc_try_fetch(&$state, $path).await {
            Ok(html) => {
                #[cfg(debug_assertions)]
                {
                    let _ = std::fs::write(std::env::temp_dir().join($dump), &html);
                }
                let data = $parser(&html);
                if let Ok(json) = serde_json::to_string(&data) {
                    let _ = $db.save_data_cache($cache_key, &json);
                }
                Ok(data)
            }
            Err(e) => {
                if let Ok(Some((json, _))) = $db.get_data_cache($cache_key) {
                    if let Ok(cached) = serde_json::from_str(&json) {
                        log::info!("{}: cache fallback ({})", $cache_key, e);
                        return Ok(cached);
                    }
                }
                Err(e)
            }
        }
    }};
}

#[tauri::command]
pub async fn fetch_grades(
    state: State<'_, KgcState>,
    db: State<'_, crate::db::Database>,
) -> Result<parser::GradesData, String> {
    kgc_fetch_cached!(
        state,
        db,
        "grades",
        "/uniasv2/ARF140.do?REQ_PRFR_MNU_ID=MNUIDSTD0102020",
        parser::parse_grades,
        "kgc-grades.html"
    )
}

#[tauri::command]
pub async fn fetch_cancellations(
    state: State<'_, KgcState>,
    db: State<'_, crate::db::Database>,
) -> Result<parser::CancellationsData, String> {
    kgc_fetch_cached!(
        state,
        db,
        "cancellations",
        "/uniasv2/APB020PLS01Action.do?REQ_PRFR_MNU_ID=MNUIDSTD0101011",
        parser::parse_cancellations,
        "kgc-cancellations.html"
    )
}

#[tauri::command]
pub async fn fetch_makeup_classes(
    state: State<'_, KgcState>,
    db: State<'_, crate::db::Database>,
) -> Result<parser::MakeupData, String> {
    kgc_fetch_cached!(
        state,
        db,
        "makeup",
        "/uniasv2/APC020PLS01Action.do?REQ_PRFR_MNU_ID=MNUIDSTD0101012",
        parser::parse_makeup_classes,
        "kgc-makeup.html"
    )
}

#[tauri::command]
pub async fn fetch_room_changes(
    state: State<'_, KgcState>,
    db: State<'_, crate::db::Database>,
) -> Result<parser::RoomChangesData, String> {
    kgc_fetch_cached!(
        state,
        db,
        "rooms",
        "/uniasv2/APA960.do?REQ_PRFR_MNU_ID=MNUIDSTD0101013",
        parser::parse_room_changes,
        "kgc-roomchanges.html"
    )
}

#[tauri::command]
pub async fn fetch_registration(
    state: State<'_, KgcState>,
    db: State<'_, crate::db::Database>,
) -> Result<parser::RegistrationData, String> {
    kgc_fetch_cached!(
        state,
        db,
        "registration",
        "/uniasv2/ARD010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102012",
        parser::parse_registration,
        "kgc-registration.html"
    )
}

#[tauri::command]
pub async fn fetch_exam_timetable(
    state: State<'_, KgcState>,
    db: State<'_, crate::db::Database>,
) -> Result<parser::ExamTimetableData, String> {
    kgc_fetch_cached!(
        state,
        db,
        "exam_timetable",
        "/uniasv2/ARF010PVL01Action.do?REQ_PRFR_MNU_ID=MNUIDSTD0102019",
        parser::parse_exam_timetable
    )
}

#[tauri::command]
pub async fn fetch_notifications(
    state: State<'_, KgcState>,
    db: State<'_, crate::db::Database>,
) -> Result<parser::NotificationsData, String> {
    kgc_fetch_cached!(state, db, "notifications", "/uniasv2/CPA010PLS01Action.do?REQ_FUNCTION_JUMP_START_FLG=1&PRD_FLG=1&REQ_PRFR_FUNC_ID=CPA010", parser::parse_notifications, "kgc-notifications.html")
}

#[tauri::command]
pub async fn fetch_page(state: State<'_, KgcState>, path: String) -> Result<String, String> {
    // Only allow paths under the university system
    if !path.starts_with("/uniasv2/") {
        return Err("許可されていないパスです".into());
    }
    let _kgc_gate = state.gate.lock().await;
    let http = kgc_http(&state).await?;
    kgc_get(&http, &path).await
}

#[tauri::command]
pub async fn fetch_course_detail(
    state: State<'_, KgcState>,
    db: State<'_, crate::db::Database>,
    path: String,
) -> Result<parser::CourseDetail, String> {
    if !path.starts_with("/uniasv2/") {
        return Err("許可されていないパスです".into());
    }
    let cache_key = format!("course_detail:{}", path);
    match kgc_try_fetch(&state, &path).await {
        Ok(html) => {
            let data = parser::parse_course_detail(&html);
            if let Ok(json) = serde_json::to_string(&data) {
                let _ = db.save_data_cache(&cache_key, &json);
            }
            Ok(data)
        }
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("course_detail: cache fallback ({})", e);
                    return Ok(cached);
                }
            }
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn open_detail_window(
    app: tauri::AppHandle,
    path: String,
    course_name: String,
) -> Result<(), String> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    // Cap concurrent detail windows to prevent resource exhaustion
    let existing = app
        .webview_windows()
        .keys()
        .filter(|k| k.starts_with("detail-"))
        .count();
    if existing >= 10 {
        return Err(config::TOO_MANY_WINDOWS_MSG.into());
    }

    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = format!("detail-{}", id);

    let encoded_path = urlencoding::encode(&path);
    let encoded_name = urlencoding::encode(&course_name);
    let url_str = format!(
        "luna-detail.html?mode=kgc&path={}&name={}",
        encoded_path, encoded_name
    );

    tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App(url_str.into()))
        .title(&course_name)
        .inner_size(480.0, 560.0)
        .resizable(true)
        .build()
        .map_err(|e| format!("ウィンドウ作成失敗: {}", e))?;

    Ok(())
}

#[tauri::command]
pub async fn fetch_student_profile(
    state: State<'_, KgcState>,
    db: State<'_, crate::db::Database>,
) -> Result<parser::StudentInfo, String> {
    let (http, is_auth) = {
        let client = state.client.lock().await;
        (client.http.clone(), client.is_authenticated())
    };
    if !is_auth {
        // Try cache fallback
        if let Ok(Some((json, _))) = db.get_data_cache("student_profile") {
            if let Ok(cached) = serde_json::from_str(&json) {
                log::info!("student_profile: cache fallback (not authenticated)");
                return Ok(cached);
            }
        }
        return Err(config::KGC_AUTH_REQUIRED_MSG.into());
    }
    // Fetch timetable page for basic info (name, id, faculty, department)
    let url1 = format!(
        "{}/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014",
        config::KG_COURSE_BASE
    );
    let mut info = match client::fetch_page_with(&http, &url1).await {
        Ok(html) => parser::parse_student_info(&html),
        Err(e) => {
            // Try cache fallback on network error
            if let Ok(Some((json, _))) = db.get_data_cache("student_profile") {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("student_profile: cache fallback ({})", e);
                    return Ok(cached);
                }
            }
            return Err(e);
        }
    };
    // Fetch student info page for extra fields (student_type, status, etc.)
    let url2 = format!(
        "{}/uniasv2/GGA110.do?REQ_PRFR_MNU_ID=MNUIDSTD0104011",
        config::KG_COURSE_BASE
    );
    if let Ok(html) = client::fetch_page_with(&http, &url2).await {
        let extra = parser::parse_student_info(&html);
        if info.student_id.is_empty() && !extra.student_id.is_empty() {
            info.student_id = extra.student_id;
        }
        if info.name.is_empty() && !extra.name.is_empty() {
            info.name = extra.name;
        }
        if !extra.name_en.is_empty() {
            info.name_en = extra.name_en;
        }
        if info.faculty.is_empty() && !extra.faculty.is_empty() {
            info.faculty = extra.faculty;
        }
        if info.department.is_empty() && !extra.department.is_empty() {
            info.department = extra.department;
        }
        if !extra.student_type.is_empty() {
            info.student_type = extra.student_type;
        }
        if !extra.affiliation_type.is_empty() {
            info.affiliation_type = extra.affiliation_type;
        }
        if !extra.status.is_empty() {
            info.status = extra.status;
        }
        if !extra.class.is_empty() {
            info.class = extra.class;
        }
        if !extra.major.is_empty() {
            info.major = extra.major;
        }
        if !extra.address.is_empty() {
            info.address = extra.address;
        }
    }
    // Cache the result
    if let Ok(json) = serde_json::to_string(&info) {
        let _ = db.save_data_cache("student_profile", &json);
    }
    Ok(info)
}

// ============ Debug Commands ============

#[derive(Debug, Serialize)]
pub struct DebugInfo {
    pub app_version: String,
    pub tauri_version: String,
    pub auth_status: String,
    pub username: String,
    pub cookie_count: usize,
    pub timestamp: String,
    pub os: String,
    pub arch: String,
    pub stt_configured_backend: String,
    pub stt_configured_partial_mode: String,
    pub stt_configured_sensitivity: String,
    pub stt_runtime_backend: String,
    pub stt_runtime_state: String,
    pub stt_active_caller: String,
    pub notification_debug: notifier::NotificationDebugInfo,
}

#[tauri::command]
pub async fn debug_info(
    app: tauri::AppHandle,
    state: State<'_, KgcState>,
) -> Result<DebugInfo, String> {
    let client = state.client.lock().await;
    let (auth_status, username) = if let Some(session) = &client.session {
        ("authenticated".to_string(), session.username.clone())
    } else {
        ("not_authenticated".to_string(), String::new())
    };
    drop(client);
    let stt_debug = stt::stt_runtime_debug_info();
    let notification_debug = notifier::debug_snapshot(&app).await;

    Ok(DebugInfo {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        tauri_version: tauri::VERSION.to_string(),
        auth_status,
        username,
        cookie_count: 0, // cookie jar doesn't expose count
        timestamp: chrono_now(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        stt_configured_backend: stt_debug.configured_backend,
        stt_configured_partial_mode: stt_debug.configured_partial_mode,
        stt_configured_sensitivity: stt_debug.configured_sensitivity,
        stt_runtime_backend: stt_debug.runtime_backend,
        stt_runtime_state: stt_debug.runtime_state,
        stt_active_caller: stt_debug.active_caller,
        notification_debug,
    })
}

#[derive(Debug, Serialize)]
pub struct PingResult {
    pub target: String,
    pub reachable: bool,
    pub status_code: u16,
    pub latency_ms: u64,
    pub error: String,
}

#[cfg(debug_assertions)]
#[tauri::command]
pub async fn debug_ping(target: String) -> Result<PingResult, String> {
    // Restrict to known university hosts
    const ALLOWED_HOSTS: &[&str] = &[
        config::KG_COURSE_BASE,
        config::LUNA_BASE,
        config::KWIC_BASE,
        "https://sts.kwansei.ac.jp",
        "https://idp.kwansei.ac.jp",
        "https://sso.kwansei.ac.jp",
    ];
    if !ALLOWED_HOSTS.iter().any(|h| target.starts_with(h)) {
        return Err("許可されていないホストです".into());
    }

    static PING_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("failed to build ping client")
    });

    let start = Instant::now();
    match PING_CLIENT.head(&target).send().await {
        Ok(resp) => {
            let latency = start.elapsed().as_millis() as u64;
            Ok(PingResult {
                target,
                reachable: true,
                status_code: resp.status().as_u16(),
                latency_ms: latency,
                error: String::new(),
            })
        }
        Err(e) => {
            let latency = start.elapsed().as_millis() as u64;
            Ok(PingResult {
                target,
                reachable: false,
                status_code: 0,
                latency_ms: latency,
                error: e.to_string(),
            })
        }
    }
}

#[cfg(not(debug_assertions))]
#[tauri::command]
pub async fn debug_ping() -> Result<PingResult, String> {
    Err("debug commands are not available in release builds".into())
}

fn chrono_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    // JST = UTC + 9 hours
    let hours = ((secs % 86400) / 3600 + 9) % 24;
    let mins = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", hours, mins, s)
}
