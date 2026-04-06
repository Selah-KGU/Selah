use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::{Emitter, Manager};
use std::time::Instant;
use std::sync::LazyLock;
use regex::Regex;

use crate::auth;
use crate::parser;
use crate::AppState;

const SAML_CALLBACK_HOST: &str = "kgc-saml-callback.localhost";
const AUTH_REQUIRED_MSG: &str = "ログインしてください";

/// Helper: lock client, check auth, fetch page, optionally dump to /tmp, parse.
macro_rules! kgc_fetch {
    ($state:expr, $path:expr, $parser:expr) => {{
        let client = $state.client.lock().await;
        if !client.is_authenticated() {
            return Err(AUTH_REQUIRED_MSG.into());
        }
        let html = client.fetch_page($path).await?;
        Ok($parser(&html))
    }};
    ($state:expr, $path:expr, $parser:expr, $dump:expr) => {{
        let client = $state.client.lock().await;
        if !client.is_authenticated() {
            return Err(AUTH_REQUIRED_MSG.into());
        }
        let html = client.fetch_page($path).await?;
        #[cfg(debug_assertions)]
        { let _ = std::fs::write($dump, &html); }
        Ok($parser(&html))
    }};
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionStatus {
    pub valid: bool,
    pub username: String,
    pub display_name: String,
    pub student_id: String,
    pub faculty: String,
    pub department: String,
}

/// Open a login webview window.
/// 1. Use reqwest to initiate SP auth and get the Okta SAML URL
/// 2. Open a webview window to the Okta URL
/// 3. The initialization_script intercepts SAMLResponse form submission
/// 4. on_navigation catches the callback URL and extracts SAML data
/// 5. Background task submits SAMLResponse to SP via reqwest
/// 6. Emits "login-success" or "login-error" event to frontend
#[tauri::command]
pub async fn open_login_window(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Step 1: Initiate SP auth to get the Okta SAML URL
    // Clear any expired session first so we start with a fresh cookie jar
    let saml_url: String = {
        let mut client = state.client.lock().await;
        if client.session.is_some() {
            client.clear_session();
        }
        auth::initiate_sp_auth(&client.http).await?
    };
    log::info!("Opening login webview with SAML URL: {}", &saml_url[..120.min(saml_url.len())]);

    // Close any existing login window before opening a new one
    if let Some(existing) = app.get_webview_window("login") {
        let _ = existing.close();
    }

    // Channel to pass SAML data from on_navigation to the background task
    let (tx, mut rx) = tokio::sync::mpsc::channel::<auth::SamlCallbackData>(2);

    // Step 2: Create the login webview window
    let parsed_url: url::Url = saml_url.parse()
        .map_err(|e| format!("URL parse error: {}", e))?;

    let _login_window = tauri::WebviewWindowBuilder::new(
        &app,
        "login",
        tauri::WebviewUrl::External(parsed_url),
    )
    .title("関西学院 - サインイン")
    .inner_size(480.0, 700.0)
    .resizable(true)
    .initialization_script(&auth::saml_intercept_script(SAML_CALLBACK_HOST))
    .on_navigation(move |url| {
        // Intercept our special callback URL
        if url.host_str() == Some("kgc-saml-callback.localhost") {
            let pairs: std::collections::HashMap<String, String> =
                url.query_pairs().into_owned().collect();

            if let Some(saml_response) = pairs.get("saml_response") {
                let data = auth::SamlCallbackData {
                    saml_response: saml_response.clone(),
                    relay_state: pairs.get("relay_state").cloned().unwrap_or_default(),
                    acs_url: pairs.get("acs_url").cloned().unwrap_or_default(),
                };
                log::info!(
                    "Intercepted SAMLResponse (len={}), ACS={}",
                    data.saml_response.len(),
                    &data.acs_url[..80.min(data.acs_url.len())]
                );
                let _ = tx.try_send(data);
            }
            return false; // Block navigation to the fake URL
        }
        true // Allow all other navigation
    })
    .build()
    .map_err(|e| format!("ログインウィンドウ作成失敗: {}", e))?;

    // Step 3: Spawn a background task to wait for SAML data and complete login
    let app_clone = app.clone();
    tokio::spawn(async move {
        match rx.recv().await {
            Some(data) => {
                log::info!("Processing SAML callback (ACS: {})...", &data.acs_url);

                // Get the client and complete the login
                let app_state = app_clone.state::<AppState>();
                let mut client = app_state.client.lock().await;

                match auth::complete_saml_login(&mut client, &data).await {
                    Ok(session) => {
                        log::info!("Selah login successful: {}", session.display_name);
                        let _ = app_clone.emit("login-success", &session);
                        drop(client); // release the lock

                        // Phase 2: Navigate the login webview to Luna's SAML entry
                        // The Okta session is still active in the webview, so Luna should auto-authenticate
                        log::info!("=== Phase 2: Luna SAML login ===");
                        if let Some(win) = app_clone.get_webview_window("login") {
                            let luna_saml_url = "https://luna.kwansei.ac.jp/saml/login?disco=true";
                            log::info!("Navigating login webview to Luna SAML: {}", luna_saml_url);
                            let luna_url: url::Url = luna_saml_url.parse().unwrap();
                            let _ = win.navigate(luna_url);

                            // Wait for Luna's SAMLResponse
                            match tokio::time::timeout(
                                std::time::Duration::from_secs(15),
                                rx.recv(),
                            ).await {
                                Ok(Some(luna_data)) => {
                                    log::info!("Luna SAML callback received (ACS: {})", &luna_data.acs_url);
                                    let mut luna = app_state.luna.lock().await;
                                    match luna.complete_saml_login(
                                        &luna_data.saml_response,
                                        &luna_data.relay_state,
                                    ).await {
                                        Ok(()) => {
                                            log::info!("Luna login successful");
                                            luna.save_session();
                                            let _ = app_clone.emit("luna-login-success", ());
                                        }
                                        Err(e) => {
                                            log::warn!("Luna SAML login failed: {}", e);
                                            let _ = app_clone.emit("luna-login-error", &e);
                                        }
                                    }
                                }
                                Ok(None) => {
                                    log::warn!("Luna login window closed before completion");
                                }
                                Err(_) => {
                                    log::warn!("Luna SAML login timed out (15s)");
                                    let _ = app_clone.emit("luna-login-error", "Luna login timed out");
                                }
                            }

                            // Phase 3: Navigate the login webview to KWIC Portal's SAML entry
                            // Like Luna Phase 2, navigate the webview directly — the Okta session
                            // in the webview will auto-authenticate KWIC Portal.
                            log::info!("=== Phase 3: KWIC Portal SAML login ===");
                            if let Some(win) = app_clone.get_webview_window("login") {
                                let kwic_saml_url = "https://kwic.kwansei.ac.jp/saml/login?disco=true";
                                log::info!("Navigating login webview to KWIC Portal SAML: {}", kwic_saml_url);
                                let kwic_url: url::Url = kwic_saml_url.parse().unwrap();
                                let _ = win.navigate(kwic_url);

                                match tokio::time::timeout(
                                    std::time::Duration::from_secs(15),
                                    rx.recv(),
                                ).await {
                                    Ok(Some(kwic_data)) => {
                                        log::info!("KWIC Portal SAML callback received (ACS: {})", &kwic_data.acs_url[..80.min(kwic_data.acs_url.len())]);
                                        let mut kwic = app_state.kwic.lock().await;
                                        match kwic.complete_saml_login(
                                            &kwic_data.saml_response,
                                            &kwic_data.relay_state,
                                            &kwic_data.acs_url,
                                        ).await {
                                            Ok(()) => {
                                                log::info!("KWIC Portal login successful");
                                                kwic.save_session();
                                                let _ = app_clone.emit("kwic-login-success", ());
                                            }
                                            Err(e) => {
                                                log::warn!("KWIC Portal SAML login failed: {}", e);
                                                let _ = app_clone.emit("kwic-login-error", &e);
                                            }
                                        }
                                    }
                                    Ok(None) => {
                                        log::warn!("KWIC Portal login window closed before completion");
                                    }
                                    Err(_) => {
                                        log::warn!("KWIC Portal SAML login timed out (15s)");
                                        let _ = app_clone.emit("kwic-login-error", "KWIC Portal login timed out");
                                    }
                                }
                            }

                            // Close the login window
                            if let Some(win) = app_clone.get_webview_window("login") {
                                let _ = win.close();
                            }
                        } else {
                            log::warn!("Login window not found for Luna phase");
                        }
                    }
                    Err(e) => {
                        log::error!("SAML login completion failed: {}", e);
                        let _ = app_clone.emit("login-error", &e);

                        // Close the login window
                        if let Some(win) = app_clone.get_webview_window("login") {
                            let _ = win.close();
                        }
                    }
                }
            }
            None => {
                log::info!("Login window closed without completing login");
                let _ = app_clone.emit("login-cancelled", "ログインがキャンセルされました");
            }
        }
    });

    Ok(())
}

#[tauri::command]
pub async fn logout(state: State<'_, AppState>) -> Result<(), String> {
    let mut client = state.client.lock().await;
    client.clear_session();
    drop(client);
    let mut luna = state.luna.lock().await;
    luna.clear();
    drop(luna);
    let mut kwic = state.kwic.lock().await;
    kwic.clear();
    Ok(())
}

#[tauri::command]
pub async fn check_session(
    state: State<'_, AppState>,
) -> Result<SessionStatus, String> {
    let mut client = state.client.lock().await;

    // If no in-memory session, try to restore from disk
    if client.session.is_none() {
        if client.try_restore_session() {
            log::info!("Restored session from disk, validating...");
            // Validate by trying to fetch a page
            match client.fetch_page("/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014").await {
                Ok(html) => {
                    log::info!("Restored session is valid");
                    // If user info is missing, parse it from the student info page
                    let needs_update = client.session.as_ref()
                        .map(|s| s.student_id.is_empty() || s.display_name == "ユーザー")
                        .unwrap_or(false);
                    if needs_update {
                        let info = parser::parse_student_info(&html);
                        log::info!("Reparsed student info: id={}, name={}, faculty={}, dept={}", info.student_id, info.name, info.faculty, info.department);
                        if let Some(session) = &mut client.session {
                            if !info.student_id.is_empty() {
                                session.username = info.student_id.clone();
                                session.student_id = info.student_id;
                            }
                            if !info.name.is_empty() {
                                session.display_name = info.name;
                            }
                            session.faculty = info.faculty;
                            session.department = info.department;
                        }
                        client.save_session();
                    }
                }
                Err(e) => {
                    log::info!("Restored session is expired: {}", e);
                    client.clear_session();
                }
            }
        }
    }

    if let Some(session) = &client.session {
        Ok(SessionStatus {
            valid: true,
            username: session.username.clone(),
            display_name: session.display_name.clone(),
            student_id: session.student_id.clone(),
            faculty: session.faculty.clone(),
            department: session.department.clone(),
        })
    } else {
        Ok(SessionStatus {
            valid: false,
            username: String::new(),
            display_name: String::new(),
            student_id: String::new(),
            faculty: String::new(),
            department: String::new(),
        })
    }
}

#[tauri::command]
pub async fn fetch_timetable(state: State<'_, AppState>) -> Result<parser::TimetableData, String> {
    kgc_fetch!(state, "/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014", parser::parse_timetable)
}

/// Validate the session by actually hitting the server.
/// Returns valid=false if session has expired on the server.
#[tauri::command]
pub async fn validate_session(state: State<'_, AppState>) -> Result<SessionStatus, String> {
    let client = state.client.lock().await;
    if !client.is_authenticated() {
        return Ok(SessionStatus {
            valid: false,
            username: String::new(),
            display_name: String::new(),
            student_id: String::new(),
            faculty: String::new(),
            department: String::new(),
        });
    }

    // Actually try to fetch a page to check if server session is still valid
    match client.fetch_page("/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014").await {
        Ok(_) => {
            let session = client.session.as_ref().unwrap();
            Ok(SessionStatus {
                valid: true,
                username: session.username.clone(),
                display_name: session.display_name.clone(),
                student_id: session.student_id.clone(),
                faculty: session.faculty.clone(),
                department: session.department.clone(),
            })
        }
        Err(e) => {
            log::info!("Session validation failed: {}", e);
            // Don't clear session here — let open_login_window handle cleanup
            // so concurrent API calls can still detect session expiry properly
            Ok(SessionStatus {
                valid: false,
                username: String::new(),
                display_name: String::new(),
                student_id: String::new(),
                faculty: String::new(),
                department: String::new(),
            })
        }
    }
}

/// Navigate timetable to previous/next week.
/// direction: "prev" or "next"
#[tauri::command]
pub async fn fetch_timetable_week(state: State<'_, AppState>, direction: String) -> Result<parser::TimetableData, String> {
    let client = state.client.lock().await;
    if !client.is_authenticated() {
        return Err(AUTH_REQUIRED_MSG.into());
    }

    // First, do a fresh GET to ARF010 to get a valid Struts token
    // (other commands like fetch_student_profile may have invalidated the old token)
    let fresh_html = client.fetch_page("/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014").await?;
    let fresh_data = parser::parse_timetable(&fresh_html);

    // Use the fresh token and form fields from the server
    let mut params: Vec<(String, String)> = fresh_data.form_fields.into_iter().collect();

    // Add the navigation button parameter (simulates image submit)
    match direction.as_str() {
        "prev" => {
            params.push(("EPrevious.x".into(), "1".into()));
            params.push(("EPrevious.y".into(), "1".into()));
        }
        "next" => {
            params.push(("ENext.x".into(), "1".into()));
            params.push(("ENext.y".into(), "1".into()));
        }
        _ => return Err("Invalid direction".into()),
    }

    let html = client.post_form("/uniasv2/ARF010PCT01EventAction.do", &params).await?;
    #[cfg(debug_assertions)]
    { let _ = std::fs::write("/tmp/kgc-week-response.html", &html); }
    Ok(parser::parse_timetable(&html))
}

#[tauri::command]
pub async fn fetch_grades(state: State<'_, AppState>) -> Result<parser::GradesData, String> {
    kgc_fetch!(state, "/uniasv2/ARF140.do?REQ_PRFR_MNU_ID=MNUIDSTD0102020", parser::parse_grades, "/tmp/kgc-grades.html")
}

#[tauri::command]
pub async fn fetch_cancellations(state: State<'_, AppState>) -> Result<parser::CancellationsData, String> {
    kgc_fetch!(state, "/uniasv2/APB020PLS01Action.do?REQ_PRFR_MNU_ID=MNUIDSTD0101011", parser::parse_cancellations, "/tmp/kgc-cancellations.html")
}

#[tauri::command]
pub async fn fetch_makeup_classes(state: State<'_, AppState>) -> Result<parser::MakeupData, String> {
    kgc_fetch!(state, "/uniasv2/APC020PLS01Action.do?REQ_PRFR_MNU_ID=MNUIDSTD0101012", parser::parse_makeup_classes, "/tmp/kgc-makeup.html")
}

#[tauri::command]
pub async fn fetch_room_changes(state: State<'_, AppState>) -> Result<parser::RoomChangesData, String> {
    kgc_fetch!(state, "/uniasv2/APA960.do?REQ_PRFR_MNU_ID=MNUIDSTD0101013", parser::parse_room_changes, "/tmp/kgc-roomchanges.html")
}

#[tauri::command]
pub async fn fetch_registration(state: State<'_, AppState>) -> Result<parser::RegistrationData, String> {
    kgc_fetch!(state, "/uniasv2/ARD010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102012", parser::parse_registration, "/tmp/kgc-registration.html")
}

#[tauri::command]
pub async fn fetch_exam_timetable(state: State<'_, AppState>) -> Result<parser::ExamTimetableData, String> {
    kgc_fetch!(state, "/uniasv2/ARF010PVL01Action.do?REQ_PRFR_MNU_ID=MNUIDSTD0102019", parser::parse_exam_timetable)
}

#[tauri::command]
pub async fn fetch_notifications(
    state: State<'_, AppState>,
) -> Result<parser::NotificationsData, String> {
    kgc_fetch!(state, "/uniasv2/CPA010PLS01Action.do?REQ_FUNCTION_JUMP_START_FLG=1&PRD_FLG=1&REQ_PRFR_FUNC_ID=CPA010", parser::parse_notifications, "/tmp/kgc-notifications.html")
}

/// 関西学院大学 period → (start_hour, start_min, end_hour, end_min)
/// 6限以降は通常使わないため None を返す（カレンダーに同期しない）
fn period_to_time(period: i32) -> Option<(u32, u32, u32, u32)> {
    match period {
        1 => Some((9, 0, 10, 30)),
        2 => Some((11, 0, 12, 30)),
        3 => Some((13, 30, 15, 0)),
        4 => Some((15, 10, 16, 40)),
        5 => Some((16, 50, 18, 20)),
        _ => None, // 6限以降は同期しない
    }
}

static WEEK_MONDAY_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d{4})/(\d{2})/(\d{2})\(月\)").unwrap());
static WEEK_DATE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d{4})/(\d{2})/(\d{2})").unwrap());

/// Parse week_label like "2026/03/30(月)～2026/04/05(日)" to get Monday's date
fn parse_week_start(week_label: &str) -> Result<(i32, u32, u32), String> {
    let caps = WEEK_MONDAY_RE.captures(week_label)
        .or_else(|| WEEK_DATE_RE.captures(week_label));
    if let Some(caps) = caps {
        let y: i32 = caps[1].parse().unwrap();
        let m: u32 = caps[2].parse().unwrap();
        let d: u32 = caps[3].parse().unwrap();
        return Ok((y, m, d));
    }
    Err(format!("週ラベルを解析できません: {}", week_label))
}

/// Calculate actual date by adding day_offset to a base date
fn add_days(year: i32, month: u32, day: u32, offset: i32) -> (i32, u32, u32) {
    // Simple date arithmetic using day-of-year
    let days_in_month = |y: i32, m: u32| -> u32 {
        match m {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 29 } else { 28 },
            _ => 30,
        }
    };
    let mut y = year;
    let mut m = month;
    let mut d = (day as i32) + offset;
    while d > days_in_month(y, m) as i32 {
        d -= days_in_month(y, m) as i32;
        m += 1;
        if m > 12 { m = 1; y += 1; }
    }
    while d < 1 {
        m -= 1;
        if m < 1 { m = 12; y -= 1; }
        d += days_in_month(y, m) as i32;
    }
    (y, m, d as u32)
}

#[derive(Debug, Deserialize)]
pub struct CalendarSyncEntry {
    pub day: String,
    pub period: i32,
    pub course_name: String,
    pub room: String,
    pub is_cancelled: bool,
}

/// Escape a string for embedding in a JavaScript double-quoted string literal
fn escape_js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\'' => out.push_str("\\'"),
            '`' => out.push('`'), // not special in double quotes, but be safe
            _ => out.push(c),
        }
    }
    out
}

/// Sync timetable entries to macOS Calendar.app
/// Creates a "Selah 時間割" calendar, clears its events, then adds current entries.
#[tauri::command]
pub async fn sync_calendar(
    entries: Vec<CalendarSyncEntry>,
    week_label: String,
) -> Result<String, String> {
    let (base_year, base_month, base_day) = parse_week_start(&week_label)?;

    let day_offset = |d: &str| -> i32 {
        match d { "月" => 0, "火" => 1, "水" => 2, "木" => 3, "金" => 4, "土" => 5, _ => 0 }
    };

    // Build JXA script to sync to Calendar.app
    let mut events_js = String::new();
    for entry in &entries {
        if entry.is_cancelled { continue; }
        let Some((sh, sm, eh, em)) = period_to_time(entry.period) else { continue; };
        let offset = day_offset(&entry.day);
        let (y, m, d) = add_days(base_year, base_month, base_day, offset);
        let title = escape_js_string(&entry.course_name);
        let location = escape_js_string(&entry.room);
        events_js.push_str(&format!(
            r#"  addEvent(cal, "{title}", "{location}", new Date({y},{mIdx},{d},{sh},{sm}), new Date({y},{mIdx},{d},{eh},{em}));
"#,
            title = title,
            location = location,
            y = y, mIdx = m - 1, d = d, sh = sh, sm = sm, eh = eh, em = em
        ));
    }

    let script = format!(
        r#"
var Calendar = Application("Calendar");
Calendar.includeStandardAdditions = true;

// Find or create the KWIC calendar
var calName = "Selah 時間割";
var cal = null;
var calendars = Calendar.calendars();
for (var i = 0; i < calendars.length; i++) {{
  if (calendars[i].name() === calName) {{
    cal = calendars[i];
    break;
  }}
}}
if (!cal) {{
  cal = Calendar.Calendar({{ name: calName }});
  Calendar.calendars.push(cal);
}}

// Delete events within this week's range only
var weekStart = new Date({wy},{wmi},{wd},0,0,0);
var weekEnd = new Date({wey},{wemi},{wed},23,59,59);
var events = cal.events();
for (var i = events.length - 1; i >= 0; i--) {{
  var sd = events[i].startDate();
  if (sd >= weekStart && sd <= weekEnd) {{
    Calendar.delete(events[i]);
  }}
}}

function addEvent(cal, title, location, startDate, endDate) {{
  var e = Calendar.Event({{
    summary: title,
    location: location,
    startDate: startDate,
    endDate: endDate
  }});
  cal.events.push(e);
}}

{events_js}
cal.events().length;
"#,
        wy = base_year, wmi = base_month - 1, wd = base_day,
        wey = {
            let (end_y, _, _) = add_days(base_year, base_month, base_day, 5);
            end_y
        },
        wemi = {
            let (_, end_m, _) = add_days(base_year, base_month, base_day, 5);
            end_m - 1
        },
        wed = {
            let (_, _, end_d) = add_days(base_year, base_month, base_day, 5);
            end_d
        }
    );

    let output = std::process::Command::new("osascript")
        .arg("-l").arg("JavaScript")
        .arg("-e").arg(&script)
        .output()
        .map_err(|e| format!("osascript 実行失敗: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("カレンダー同期失敗: {}", stderr.trim()));
    }

    let count = String::from_utf8_lossy(&output.stdout).trim().to_string();
    log::info!("Calendar sync: {} events added", count);
    Ok(format!("{}件のイベントを同期しました", count))
}

/// Get info about the KG-Course calendar (exists, event count)
#[tauri::command]
pub async fn get_calendar_info() -> Result<serde_json::Value, String> {
    let script = r#"
var Calendar = Application("Calendar");
var calName = "Selah 時間割";
var cal = null;
var calendars = Calendar.calendars();
for (var i = 0; i < calendars.length; i++) {
  if (calendars[i].name() === calName) {
    cal = calendars[i];
    break;
  }
}
if (!cal) {
  JSON.stringify({ exists: false, count: 0 });
} else {
  JSON.stringify({ exists: true, count: cal.events().length });
}
"#;
    let output = std::process::Command::new("osascript")
        .arg("-l").arg("JavaScript")
        .arg("-e").arg(script)
        .output()
        .map_err(|e| format!("osascript 実行失敗: {}", e))?;

    if !output.status.success() {
        return Ok(serde_json::json!({ "exists": false, "count": 0 }));
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    serde_json::from_str(&stdout).map_err(|e| format!("JSON parse error: {}", e))
}

/// Clear all events from the KG-Course calendar, or delete the calendar entirely
#[tauri::command]
pub async fn clear_calendar(delete_calendar: bool) -> Result<String, String> {
    let script = if delete_calendar {
        r#"
var Calendar = Application("Calendar");
var calName = "Selah 時間割";
var calendars = Calendar.calendars();
for (var i = 0; i < calendars.length; i++) {
  if (calendars[i].name() === calName) {
    Calendar.delete(calendars[i]);
    break;
  }
}
"deleted";
"#.to_string()
    } else {
        r#"
var Calendar = Application("Calendar");
var calName = "Selah 時間割";
var cal = null;
var calendars = Calendar.calendars();
for (var i = 0; i < calendars.length; i++) {
  if (calendars[i].name() === calName) {
    cal = calendars[i];
    break;
  }
}
var count = 0;
if (cal) {
  var events = cal.events();
  count = events.length;
  for (var i = events.length - 1; i >= 0; i--) {
    Calendar.delete(events[i]);
  }
}
count + "";
"#.to_string()
    };

    let output = std::process::Command::new("osascript")
        .arg("-l").arg("JavaScript")
        .arg("-e").arg(&script)
        .output()
        .map_err(|e| format!("osascript 実行失敗: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("カレンダー操作失敗: {}", stderr.trim()));
    }

    if delete_calendar {
        Ok("カレンダーを削除しました".into())
    } else {
        let count = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(format!("{}件のイベントを削除しました", count))
    }
}

#[tauri::command]
pub async fn fetch_page(state: State<'_, AppState>, path: String) -> Result<String, String> {
    // Only allow paths under the university system
    if !path.starts_with("/uniasv2/") {
        return Err("許可されていないパスです".into());
    }
    let client = state.client.lock().await;
    client.fetch_page(&path).await
}

#[tauri::command]
pub async fn fetch_course_detail(state: State<'_, AppState>, path: String) -> Result<parser::CourseDetail, String> {
    if !path.starts_with("/uniasv2/") {
        return Err("許可されていないパスです".into());
    }
    let client = state.client.lock().await;
    if !client.is_authenticated() {
        return Err(AUTH_REQUIRED_MSG.into());
    }
    let html = client.fetch_page(&path).await?;
    Ok(parser::parse_course_detail(&html))
}

#[tauri::command]
pub async fn open_detail_window(
    app: tauri::AppHandle,
    path: String,
    course_name: String,
) -> Result<(), String> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = format!("detail-{}", id);

    let encoded_path = urlencoding::encode(&path);
    let encoded_name = urlencoding::encode(&course_name);
    let url_str = format!("detail.html?path={}&name={}", encoded_path, encoded_name);

    tauri::WebviewWindowBuilder::new(
        &app,
        &label,
        tauri::WebviewUrl::App(url_str.into()),
    )
    .title(&course_name)
    .inner_size(480.0, 560.0)
    .resizable(true)
    .build()
    .map_err(|e| format!("ウィンドウ作成失敗: {}", e))?;

    Ok(())
}

#[tauri::command]
pub async fn open_profile_edit_window(
    app: tauri::AppHandle,
) -> Result<(), String> {
    if let Some(win) = app.get_window("profile-edit") {
        let _ = win.set_focus();
        return Ok(());
    }

    let url: url::Url = "https://kg-course.kwansei.ac.jp/uniasv2/UnSSOLoginControl2?REQ_LOGIN_NO=2&REQ_ACTION_DO=/GGA110.do&REQ_PRFR_MNU_ID=MNUIDSTD0104011"
        .parse()
        .map_err(|e| format!("URL parse error: {}", e))?;

    crate::webview_toolbar::create_browser_window(
        &app,
        "profile-edit",
        tauri::WebviewUrl::External(url),
        "個人情報編集",
        1000.0, 720.0,
        &[],
    )?;

    Ok(())
}

#[tauri::command]
pub async fn open_facility_reservation(
    app: tauri::AppHandle,
) -> Result<(), String> {
    if let Some(win) = app.get_window("facility-rsv") {
        let _ = win.set_focus();
        return Ok(());
    }

    let url: url::Url = "https://facility-rsv.kwansei.ac.jp/ss/top"
        .parse()
        .map_err(|e| format!("URL parse error: {}", e))?;

    crate::webview_toolbar::create_browser_window(
        &app,
        "facility-rsv",
        tauri::WebviewUrl::External(url),
        "施設予約",
        1100.0, 780.0,
        &[],
    )?;

    Ok(())
}

#[tauri::command]
pub async fn open_registration_window(
    app: tauri::AppHandle,
) -> Result<(), String> {
    if let Some(win) = app.get_window("registration") {
        let _ = win.set_focus();
        return Ok(());
    }

    // Navigate through SSO entry point so the WebView establishes its own
    // authenticated session using the Okta cookies from the login webview.
    let url: url::Url = "https://kg-course.kwansei.ac.jp/uniasv2/UnSSOLoginControl2?REQ_LOGIN_NO=2&REQ_ACTION_DO=/ARD010.do&REQ_PRFR_MNU_ID=MNUIDSTD0102012&SE_LANGUAGE="
        .parse()
        .map_err(|e| format!("URL parse error: {}", e))?;

    crate::webview_toolbar::create_browser_window(
        &app,
        "registration",
        tauri::WebviewUrl::External(url),
        "履修登録",
        1100.0, 780.0,
        &[],
    )?;

    Ok(())
}

#[tauri::command]
pub async fn fetch_student_profile(state: State<'_, AppState>) -> Result<parser::StudentInfo, String> {
    let client = state.client.lock().await;
    if !client.is_authenticated() {
        return Err(AUTH_REQUIRED_MSG.into());
    }
    // Fetch timetable page for basic info (name, id, faculty, department)
    let mut info = match client.fetch_page("/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014").await {
        Ok(html) => parser::parse_student_info(&html),
        Err(e) => return Err(e),
    };
    // Fetch student info page for extra fields (student_type, status, etc.)
    if let Ok(html) = client.fetch_page("/uniasv2/GGA110.do?REQ_PRFR_MNU_ID=MNUIDSTD0104011").await {
        let extra = parser::parse_student_info(&html);
        if info.student_id.is_empty() && !extra.student_id.is_empty() { info.student_id = extra.student_id; }
        if info.name.is_empty() && !extra.name.is_empty() { info.name = extra.name; }
        if !extra.name_en.is_empty() { info.name_en = extra.name_en; }
        if info.faculty.is_empty() && !extra.faculty.is_empty() { info.faculty = extra.faculty; }
        if info.department.is_empty() && !extra.department.is_empty() { info.department = extra.department; }
        if !extra.student_type.is_empty() { info.student_type = extra.student_type; }
        if !extra.affiliation_type.is_empty() { info.affiliation_type = extra.affiliation_type; }
        if !extra.status.is_empty() { info.status = extra.status; }
        if !extra.class.is_empty() { info.class = extra.class; }
        if !extra.major.is_empty() { info.major = extra.major; }
        if !extra.address.is_empty() { info.address = extra.address; }
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
}

#[tauri::command]
pub async fn debug_info(state: State<'_, AppState>) -> Result<DebugInfo, String> {
    let client = state.client.lock().await;
    let (auth_status, username) = if let Some(session) = &client.session {
        ("authenticated".to_string(), session.username.clone())
    } else {
        ("not_authenticated".to_string(), String::new())
    };

    Ok(DebugInfo {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        tauri_version: tauri::VERSION.to_string(),
        auth_status,
        username,
        cookie_count: 0, // cookie jar doesn't expose count
        timestamp: chrono_now(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
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

#[tauri::command]
pub async fn debug_ping(target: String) -> Result<PingResult, String> {
    // Restrict to known university hosts
    const ALLOWED_HOSTS: &[&str] = &[
        "https://kg-course.kwansei.ac.jp",
        "https://luna.kwansei.ac.jp",
        "https://kwic.kwansei.ac.jp",
        "https://sts.kwansei.ac.jp",
        "https://idp.kwansei.ac.jp",
        "https://sso.kwansei.ac.jp",
    ];
    if !ALLOWED_HOSTS.iter().any(|h| target.starts_with(h)) {
        return Err("許可されていないホストです".into());
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .danger_accept_invalid_certs(false)
        .build()
        .map_err(|e| e.to_string())?;

    let start = Instant::now();
    match client.head(&target).send().await {
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

// ============ Syllabus ============

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


#[tauri::command]
pub async fn search_syllabus(
    params: crate::syllabus::SyllabusSearchParams,
    kgc_state: State<'_, AppState>,
) -> Result<crate::syllabus::SyllabusSearchResult, String> {
    let kgc_client = kgc_state.client.lock().await;
    if !kgc_client.is_authenticated() {
        return Err(AUTH_REQUIRED_MSG.into());
    }

    let search_html = kgc_client
        .fetch_page("/uniasv2/UnSSOLoginControl2?REQ_LOGIN_NO=2&REQ_ACTION_DO=/AGA030.do&REQ_PRFR_MNU_ID=MNUIDSTD0103011")
        .await?;
    let token = extract_struts_token(&search_html)?;
    let year = extract_year_from_search_page(&search_html)
        .unwrap_or_else(|| params.year_from.clone());

    let form_params = vec![
        ("org.apache.struts.taglib.html.TOKEN".into(), token),
        ("selTypeCalLsnOpcFcy".into(), "0".into()),
        ("txtLsnOpcFcy".into(), if params.year_from.is_empty() { year.clone() } else { params.year_from.clone() }),
        ("selTypeCalLsnEndFcy".into(), "0".into()),
        ("txtLsnEndFcy".into(), if params.year_to.is_empty() { year } else { params.year_to.clone() }),
        ("selTacTrmCd".into(), params.term.clone()),
        ("selOpcCmpsCd".into(), params.campus.clone()),
        ("selLsnMngPostCd".into(), params.department.clone()),
        ("txtLsnCd_01".into(), params.class_code.clone()),
        ("txtLsnCd_02".into(), String::new()),
        ("selTmtxCd".into(), params.day_period.clone()),
        ("txtSlbSrchKwd".into(), params.keyword.clone()),
        ("selVolCd1".into(), params.language.clone()),
        ("txtTchKnjfn_01".into(), params.instructor.clone()),
        ("txtTchKnafn_01".into(), String::new()),
        ("txtCbbTchRnmAlpfn_01".into(), String::new()),
        ("hdnClassisyUser".into(), "S".into()),
        ("hdnEsearch".into(), "true".into()),
        ("hdnPhfyPrcFlg".into(), String::new()),
        ("ESearch".into(), "検索/Search".into()),
        ("hdnLoginUrl".into(), String::new()),
    ];

    let html = kgc_client
        .post_form("/uniasv2/AGA030PSC01EventAction.do", &form_params)
        .await?;

    // Check for validation errors
    if html.contains("UNM") {
        if let Some(err) = crate::syllabus::extract_validation_error(&html) {
            return Err(err);
        }
    }
    if !html.contains("結果一覧画面") {
        return Err("検索条件が不足しています。履修期・キャンパス・授業管理部署・曜時のいずれか１つを指定してください。".into());
    }

    let first_page = crate::syllabus::parse_search_results_public(&html)?;
    log::info!("Search page 1: {} entries, page {}/{}", first_page.entries.len(), first_page.current_page, first_page.total_pages);

    if first_page.total_pages <= 1 {
        return Ok(first_page);
    }

    // Fetch remaining pages by replaying the full form state with ENext
    let mut all_entries = first_page.entries;
    let total_pages = first_page.total_pages;
    let mut current_html = html;

    for page in 2..=total_pages {
        // Extract ALL form inputs from the current results page
        let mut form_params = extract_all_form_inputs(&current_html);

        // Remove any existing action buttons (ESearch, ENarrowSearch, EBack, etc.)
        form_params.retain(|(k, _)| {
            !k.starts_with("ESearch") && !k.starts_with("ENarrowSearch")
            && !k.starts_with("EBack") && !k.starts_with("ENext")
            && !k.starts_with("EPrev") && !k.starts_with("ERefer")
            && !k.starts_with("ERegister") && !k.starts_with("EPageSet")
        });

        // Add the ENext button click
        form_params.push(("ENext.x".into(), "10".into()));
        form_params.push(("ENext.y".into(), "10".into()));

        log::info!("Fetching page {} with {} form params", page, form_params.len());

        let next_html = kgc_client
            .post_form("/uniasv2/AGA030PLS01EventAction.do", &form_params)
            .await?;

        match crate::syllabus::parse_search_results_public(&next_html) {
            Ok(page_result) => {
                log::info!("Search page {}: {} entries", page, page_result.entries.len());
                if page_result.entries.is_empty() { break; }
                all_entries.extend(page_result.entries);
                current_html = next_html;
            }
            Err(e) => { log::warn!("Failed to parse page {}: {}", page, e); break; }
        }
    }

    log::info!("Search total: {} entries across {} pages", all_entries.len(), total_pages);
    Ok(crate::syllabus::SyllabusSearchResult {
        total_count: all_entries.len(),
        entries: all_entries,
        current_page: 1,
        total_pages: 1,
    })
}

#[tauri::command]
pub async fn fetch_syllabus_favorites(
    kgc_state: State<'_, AppState>,
) -> Result<crate::syllabus::SyllabusSearchResult, String> {
    let kgc_client = kgc_state.client.lock().await;
    if !kgc_client.is_authenticated() {
        return Err(AUTH_REQUIRED_MSG.into());
    }

    let main_terms = ["02", "03", "01"];
    let sub_terms = ["04", "05", "06", "07"];
    let mut all_entries = Vec::new();
    let mut seen_codes = std::collections::HashSet::new();

    for term_code in main_terms.iter().chain(sub_terms.iter()) {
        let search_html = kgc_client.fetch_page("/uniasv2/UnSSOLoginControl2?REQ_LOGIN_NO=2&REQ_ACTION_DO=/AGA030.do&REQ_PRFR_MNU_ID=MNUIDSTD0103011").await?;
        let token = match extract_struts_token(&search_html) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let year = extract_year_from_search_page(&search_html).unwrap_or_else(|| "2026".into());

        let params = vec![
            ("org.apache.struts.taglib.html.TOKEN".into(), token),
            ("txtLsnOpcFcy".into(), year.clone()),
            ("txtLsnEndFcy".into(), year),
            ("selTypeCalLsnOpcFcy".into(), "0".into()),
            ("selTypeCalLsnEndFcy".into(), "0".into()),
            ("selTacTrmCd".into(), term_code.to_string()),
            ("selOpcCmpsCd".into(), String::new()),
            ("selLsnMngPostCd".into(), String::new()),
            ("hdnClassisyUser".into(), "S".into()),
            ("hdnEsearch".into(), "true".into()),
            ("hdnPhfyPrcFlg".into(), String::new()),
            ("ENarrowSearch".into(), "お気に入り/Bookmark".into()),
        ];
        let html = kgc_client.post_form("/uniasv2/AGA030PSC01EventAction.do", &params).await?;

        if let Ok(result) = crate::syllabus::parse_search_results_public(&html) {
            for entry in result.entries {
                if seen_codes.insert(entry.class_code.clone()) {
                    all_entries.push(entry);
                }
            }
        }
        if *term_code == "01" && !all_entries.is_empty() {
            break;
        }
    }

    Ok(crate::syllabus::SyllabusSearchResult {
        entries: all_entries,
        total_count: 0,
        current_page: 1,
        total_pages: 1,
    })
}

/// Search for a specific class_code across terms, returning the results HTML page.
async fn find_syllabus_results_by_class_code(
    client: &crate::client::KgcClient,
    class_code: &str,
) -> Result<String, String> {
    let terms = ["02", "03", "01", "04", "05", "06", "07"];
    for term_code in &terms {
        let search_html = client
            .fetch_page("/uniasv2/UnSSOLoginControl2?REQ_LOGIN_NO=2&REQ_ACTION_DO=/AGA030.do&REQ_PRFR_MNU_ID=MNUIDSTD0103011")
            .await?;
        let token = match extract_struts_token(&search_html) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let year = extract_year_from_search_page(&search_html).unwrap_or_else(|| "2026".into());

        let search_params = vec![
            ("org.apache.struts.taglib.html.TOKEN".into(), token),
            ("selTypeCalLsnOpcFcy".into(), "0".into()),
            ("txtLsnOpcFcy".into(), year.clone()),
            ("selTypeCalLsnEndFcy".into(), "0".into()),
            ("txtLsnEndFcy".into(), year),
            ("selTacTrmCd".into(), term_code.to_string()),
            ("selOpcCmpsCd".into(), String::new()),
            ("selLsnMngPostCd".into(), String::new()),
            ("txtLsnCd_01".into(), class_code.to_string()),
            ("txtLsnCd_02".into(), String::new()),
            ("selTmtxCd".into(), String::new()),
            ("txtSlbSrchKwd".into(), String::new()),
            ("selVolCd1".into(), String::new()),
            ("txtTchKnjfn_01".into(), String::new()),
            ("txtTchKnafn_01".into(), String::new()),
            ("txtCbbTchRnmAlpfn_01".into(), String::new()),
            ("hdnClassisyUser".into(), "S".into()),
            ("hdnEsearch".into(), "true".into()),
            ("hdnPhfyPrcFlg".into(), String::new()),
            ("ESearch".into(), "検索/Search".into()),
            ("hdnLoginUrl".into(), String::new()),
        ];
        let html = client
            .post_form("/uniasv2/AGA030PSC01EventAction.do", &search_params)
            .await?;

        if html.contains("結果一覧画面") {
            if let Ok(parsed) = crate::syllabus::parse_search_results_public(&html) {
                if parsed.entries.iter().any(|e| e.class_code == class_code) {
                    return Ok(html);
                }
            }
        }
    }
    Err(format!("科目コード {} が見つかりません", class_code))
}

#[tauri::command]
pub async fn toggle_syllabus_bookmark(
    kgc_state: State<'_, AppState>,
    class_code: String,
) -> Result<bool, String> {
    let kgc_client = kgc_state.client.lock().await;
    if !kgc_client.is_authenticated() {
        return Err(AUTH_REQUIRED_MSG.into());
    }

    let html = find_syllabus_results_by_class_code(&kgc_client, &class_code).await?;

    // Find the target row's register index
    let parsed = crate::syllabus::parse_search_results_public(&html)?;
    let target_entry = parsed.entries.iter()
        .find(|e| e.class_code == class_code)
        .ok_or_else(|| format!("科目コード {} が見つかりません", class_code))?;
    let target_index = target_entry.register_index.clone();

    // Extract ALL form fields from results page (same approach as pagination fix)
    let mut form_params = extract_all_form_inputs(&html);

    // Remove action buttons
    form_params.retain(|(k, _)| {
        !k.starts_with("ESearch") && !k.starts_with("ENarrowSearch")
        && !k.starts_with("EBack") && !k.starts_with("ENext")
        && !k.starts_with("EPrev") && !k.starts_with("ERefer")
        && !k.starts_with("ERegister") && !k.starts_with("EPageSet")
    });

    // Set the target register index and add ERegister action
    form_params.retain(|(k, _)| k != "eregisterIndex");
    form_params.push(("eregisterIndex".into(), target_index.clone()));
    form_params.push(("ERegister.x".into(), "10".into()));
    form_params.push(("ERegister.y".into(), "10".into()));

    log::info!("Bookmark toggle: class_code={}, eregisterIndex={}, params_count={}",
        class_code, target_index, form_params.len());

    let toggle_html = kgc_client
        .post_form("/uniasv2/AGA030PLS01EventAction.do", &form_params)
        .await?;

    let success = !toggle_html.contains("UNM000480E") && !toggle_html.contains("不正アクセス");
    log::info!("Bookmark toggle result: success={}, len={}", success, toggle_html.len());

    Ok(success)
}

#[tauri::command]
pub async fn open_syllabus_detail(
    app: tauri::AppHandle,
    kgc_state: State<'_, AppState>,
    class_code: String,
    course_name: String,
) -> Result<(), String> {
    let kgc_client = kgc_state.client.lock().await;
    if !kgc_client.is_authenticated() {
        return Err(AUTH_REQUIRED_MSG.into());
    }

    // Search by class_code across terms to find the course
    let html = find_syllabus_results_by_class_code(&kgc_client, &class_code).await?;

    // Parse results to obtain the fresh ereferIndex for this course
    let results = crate::syllabus::parse_search_results_public(&html)
        .map_err(|e| format!("検索結果の解析に失敗: {}", e))?;
    let target_entry = results.entries.iter()
        .find(|e| e.class_code == class_code)
        .ok_or("授業が見つかりませんでした")?;
    let fresh_refer_index = target_entry.refer_index.clone();

    // Extract ALL form fields from results page (same approach as pagination/bookmark fix)
    let mut form_params = extract_all_form_inputs(&html);

    // Remove action buttons
    form_params.retain(|(k, _)| {
        !k.starts_with("ESearch") && !k.starts_with("ENarrowSearch")
        && !k.starts_with("EBack") && !k.starts_with("ENext")
        && !k.starts_with("EPrev") && !k.starts_with("ERefer")
        && !k.starts_with("ERegister") && !k.starts_with("EPageSet")
    });

    // Set the target refer index and add ERefer action
    form_params.retain(|(k, _)| k != "ereferIndex");
    form_params.push(("ereferIndex".into(), fresh_refer_index.clone()));
    form_params.push(("ERefer.x".into(), "10".into()));
    form_params.push(("ERefer.y".into(), "10".into()));

    log::info!("Syllabus detail: ereferIndex={}, params_count={}", fresh_refer_index, form_params.len());

    let detail_html = kgc_client
        .post_form("/uniasv2/AGA030PLS01EventAction.do", &form_params)
        .await?;

    // Parse as course detail
    let detail = crate::parser::parse_course_detail(&detail_html);
    log::info!("Syllabus detail: {} fields (HTML {} bytes)", detail.fields.len(), detail_html.len());

    // Store detail data in app state for the window to retrieve
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(1000);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = format!("syllabus-detail-{}", id);

    // Store keyed by window label so concurrent opens don't collide
    {
        let state = app.state::<SyllabusDetailData>();
        let mut map = state.0.lock().map_err(|e| e.to_string())?;
        // Evict stale entries if map grows (windows closed without fetching data)
        if map.len() > 20 {
            map.clear();
        }
        map.insert(label.clone(), detail);
    }

    // Open detail window — pass label in URL so the window can retrieve its own data
    let encoded_name = urlencoding::encode(&course_name);
    let encoded_label = urlencoding::encode(&label);
    let url_str = format!("detail.html?syllabus=true&name={}&wlabel={}", encoded_name, encoded_label);

    tauri::WebviewWindowBuilder::new(
        &app,
        &label,
        tauri::WebviewUrl::App(url_str.into()),
    )
    .title(&course_name)
    .inner_size(480.0, 560.0)
    .resizable(true)
    .build()
    .map_err(|e| format!("ウィンドウ作成失敗: {}", e))?;

    Ok(())
}

pub struct SyllabusDetailData(pub std::sync::Mutex<std::collections::HashMap<String, crate::parser::CourseDetail>>);

#[tauri::command]
pub async fn get_syllabus_detail(
    state: State<'_, SyllabusDetailData>,
    label: String,
) -> Result<crate::parser::CourseDetail, String> {
    let mut map = state.0.lock().map_err(|e| e.to_string())?;
    map.remove(&label).ok_or("詳細データがありません".into())
}

static STRUTS_TOKEN_RE1: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"name="org\.apache\.struts\.taglib\.html\.TOKEN"[^>]*value="([^"]+)""#).unwrap());
static STRUTS_TOKEN_RE2: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"value="([^"]+)"[^>]*name="org\.apache\.struts\.taglib\.html\.TOKEN""#).unwrap());

fn extract_struts_token(html: &str) -> Result<String, String> {
    STRUTS_TOKEN_RE1.captures(html)
        .or_else(|| STRUTS_TOKEN_RE2.captures(html))
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| "Strutsトークンが見つかりません".into())
}

static YEAR_RE1: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"name="txtLsnOpcFcy"[^>]*value="(\d{4})""#).unwrap());
static YEAR_RE2: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"value="(\d{4})"[^>]*name="txtLsnOpcFcy""#).unwrap());

fn extract_year_from_search_page(html: &str) -> Option<String> {
    YEAR_RE1.captures(html)
        .or_else(|| YEAR_RE2.captures(html))
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Extract ALL form inputs from the results page HTML for pagination replay.
/// Collects hidden inputs, text inputs, and selected option values from the main form.
fn extract_all_form_inputs(html: &str) -> Vec<(String, String)> {
    use scraper::{Html, Selector};

    let document = Html::parse_document(html);
    let mut params: Vec<(String, String)> = Vec::new();

    // Collect all <input> elements (hidden, text, etc.)
    let input_sel = Selector::parse("form input").unwrap();
    for el in document.select(&input_sel) {
        let name = match el.value().attr("name") {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => continue,
        };
        let input_type = el.value().attr("type").unwrap_or("text").to_lowercase();
        // Skip submit/image/button types (we'll add ENext explicitly)
        if input_type == "submit" || input_type == "image" || input_type == "button" {
            continue;
        }
        // For checkboxes/radios, only include if checked
        if input_type == "checkbox" || input_type == "radio" {
            if el.value().attr("checked").is_none() {
                continue;
            }
        }
        let value = el.value().attr("value").unwrap_or("").to_string();
        params.push((name, value));
    }

    // Collect <select> elements with their selected <option> value
    let select_sel = Selector::parse("form select").unwrap();
    let option_sel = Selector::parse("option[selected]").unwrap();
    for sel_el in document.select(&select_sel) {
        let name = match sel_el.value().attr("name") {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => continue,
        };
        // Get selected option value
        if let Some(opt) = sel_el.select(&option_sel).next() {
            let value = opt.value().attr("value").unwrap_or("").to_string();
            params.push((name, value));
        }
    }

    params
}

// ============ Session Sync ============

/// Current auth state of all services
#[derive(Debug, Serialize)]
pub struct SessionStates {
    pub kgc: bool,
    pub luna: bool,
    pub kwic: bool,
}

/// Returns in-memory auth state for all services.
#[tauri::command]
pub async fn get_session_states(state: State<'_, AppState>) -> Result<SessionStates, String> {
    let kgc = state.client.lock().await.is_authenticated();
    let luna = state.luna.lock().await.authenticated;
    let kwic = state.kwic.lock().await.authenticated;
    Ok(SessionStates { kgc, luna, kwic })
}

/// Attempt a silent (headless) KG-Course session refresh via an invisible WebView.
/// The hidden browser carries the persisted Okta cookies, so if the Okta session
/// is still alive Okta will auto-submit the SAMLResponse without user interaction.
/// Returns true on success, false when Okta has also expired (caller must use
/// the visible login window).
pub async fn headless_kgc_refresh(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<bool, String> {
    log::info!("headless_kgc_refresh: starting");

    // Step 1 – Initiate SP auth via reqwest to get the Okta URL and establish
    // SP pre-session cookies in the reqwest jar (needed for complete_saml_login).
    let saml_url = {
        let mut client = state.client.lock().await;
        client.clear_session();
        auth::initiate_sp_auth(&client.http).await?
    };

    // Step 2 – Close any leftover headless window.
    if let Some(w) = app.get_webview_window("kgc-headless") {
        let _ = w.close();
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<auth::SamlCallbackData>(1);

    let parsed_url: url::Url = saml_url
        .parse()
        .map_err(|e| format!("URL parse error: {}", e))?;

    // Step 3 – Open invisible WebView at the Okta SAML URL.
    // WKWebView automatically sends the persisted Okta session cookies,
    // so Okta will redirect back with a SAMLResponse form which the
    // injected script intercepts.
    let _win = tauri::WebviewWindowBuilder::new(
        app,
        "kgc-headless",
        tauri::WebviewUrl::External(parsed_url),
    )
    .visible(false)
    .initialization_script(&auth::saml_intercept_script(SAML_CALLBACK_HOST))
    .on_navigation(move |url| {
        if url.host_str() == Some("kgc-saml-callback.localhost") {
            let pairs: std::collections::HashMap<String, String> =
                url.query_pairs().into_owned().collect();
            if let Some(saml_response) = pairs.get("saml_response") {
                let data = auth::SamlCallbackData {
                    saml_response: saml_response.clone(),
                    relay_state: pairs.get("relay_state").cloned().unwrap_or_default(),
                    acs_url: pairs.get("acs_url").cloned().unwrap_or_default(),
                };
                log::info!(
                    "headless_kgc_refresh: SAMLResponse intercepted (len={})",
                    data.saml_response.len()
                );
                let _ = tx.try_send(data);
            }
            return false;
        }
        true
    })
    .build()
    .map_err(|e| format!("Failed to build headless window: {}", e))?;

    // Step 4 – Wait up to 20 s for the SAMLResponse.
    // Timeout means Okta showed its login form instead of auto-redirecting.
    match tokio::time::timeout(std::time::Duration::from_secs(20), rx.recv()).await {
        Ok(Some(data)) => {
            let mut client = state.client.lock().await;
            match auth::complete_saml_login(&mut client, &data).await {
                Ok(session) => {
                    log::info!("headless_kgc_refresh: succeeded for {}", session.display_name);
                    let _ = _win.close();
                    Ok(true)
                }
                Err(e) => {
                    log::warn!("headless_kgc_refresh: SAML completion failed: {}", e);
                    let _ = _win.close();
                    Err(e)
                }
            }
        }
        Ok(None) => {
            log::info!("headless_kgc_refresh: window closed without SAMLResponse");
            Ok(false)
        }
        Err(_) => {
            log::info!("headless_kgc_refresh: timed out – Okta session likely expired");
            let _ = _win.close();
            Ok(false)
        }
    }
}

/// Attempt a silent (headless) Luna session refresh via an invisible WebView.
pub async fn headless_luna_refresh(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<bool, String> {
    log::info!("headless_luna_refresh: starting");

    // Step 1 – Get Luna's Okta SAML URL via reqwest.
    let saml_url = {
        let luna = state.luna.lock().await;
        luna.initiate_saml_auth().await?
    };

    if let Some(w) = app.get_webview_window("luna-headless") {
        let _ = w.close();
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<auth::SamlCallbackData>(1);

    let parsed_url: url::Url = saml_url
        .parse()
        .map_err(|e| format!("URL parse error: {}", e))?;

    let _win = tauri::WebviewWindowBuilder::new(
        app,
        "luna-headless",
        tauri::WebviewUrl::External(parsed_url),
    )
    .visible(false)
    .initialization_script(&auth::saml_intercept_script("luna-saml-callback.localhost"))
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
                log::info!(
                    "headless_luna_refresh: SAMLResponse intercepted (len={})",
                    data.saml_response.len()
                );
                let _ = tx.try_send(data);
            }
            return false;
        }
        true
    })
    .build()
    .map_err(|e| format!("Failed to build headless Luna window: {}", e))?;

    match tokio::time::timeout(std::time::Duration::from_secs(20), rx.recv()).await {
        Ok(Some(data)) => {
            let mut luna = state.luna.lock().await;
            match luna.complete_saml_login(&data.saml_response, &data.relay_state).await {
                Ok(()) => {
                    luna.save_session();
                    log::info!("headless_luna_refresh: succeeded");
                    let _ = _win.close();
                    Ok(true)
                }
                Err(e) => {
                    log::warn!("headless_luna_refresh: SAML completion failed: {}", e);
                    let _ = _win.close();
                    Err(e)
                }
            }
        }
        Ok(None) => {
            log::info!("headless_luna_refresh: window closed without SAMLResponse");
            Ok(false)
        }
        Err(_) => {
            log::info!("headless_luna_refresh: timed out – Okta session likely expired");
            let _ = _win.close();
            Ok(false)
        }
    }
}

/// Attempt a silent (headless) KWIC Portal session refresh via an invisible WebView.
pub async fn headless_kwic_refresh(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<bool, String> {
    log::info!("headless_kwic_refresh: starting");

    if let Some(w) = app.get_webview_window("kwic-headless") {
        let _ = w.close();
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<auth::SamlCallbackData>(1);

    // Navigate directly to KWIC Portal's SAML login URL.
    // The invisible WebView shares the Okta session cookies with the visible login window,
    // so Okta will auto-submit the SAMLResponse without user interaction.
    let saml_url = "https://kwic.kwansei.ac.jp/saml/login?disco=true";
    let parsed_url: url::Url = saml_url
        .parse()
        .map_err(|e| format!("URL parse error: {}", e))?;

    let _win = tauri::WebviewWindowBuilder::new(
        app,
        "kwic-headless",
        tauri::WebviewUrl::External(parsed_url),
    )
    .visible(false)
    .initialization_script(&auth::saml_intercept_script("kwic-saml-callback.localhost"))
    .on_navigation(move |url| {
        if url.host_str() == Some("kwic-saml-callback.localhost") {
            let pairs: std::collections::HashMap<String, String> =
                url.query_pairs().into_owned().collect();
            if let Some(saml_response) = pairs.get("saml_response") {
                let data = auth::SamlCallbackData {
                    saml_response: saml_response.clone(),
                    relay_state: pairs.get("relay_state").cloned().unwrap_or_default(),
                    acs_url: pairs.get("acs_url").cloned().unwrap_or_default(),
                };
                log::info!(
                    "headless_kwic_refresh: SAMLResponse intercepted (len={})",
                    data.saml_response.len()
                );
                let _ = tx.try_send(data);
            }
            return false;
        }
        true
    })
    .build()
    .map_err(|e| format!("Failed to build headless KWIC window: {}", e))?;

    match tokio::time::timeout(std::time::Duration::from_secs(20), rx.recv()).await {
        Ok(Some(data)) => {
            let mut kwic = state.kwic.lock().await;
            match kwic.complete_saml_login(&data.saml_response, &data.relay_state, &data.acs_url).await {
                Ok(()) => {
                    kwic.save_session();
                    log::info!("headless_kwic_refresh: succeeded");
                    let _ = _win.close();
                    Ok(true)
                }
                Err(e) => {
                    log::warn!("headless_kwic_refresh: SAML completion failed: {}", e);
                    let _ = _win.close();
                    Err(e)
                }
            }
        }
        Ok(None) => {
            log::info!("headless_kwic_refresh: window closed without SAMLResponse");
            Ok(false)
        }
        Err(_) => {
            log::info!("headless_kwic_refresh: timed out – Okta session likely expired");
            let _ = _win.close();
            Ok(false)
        }
    }
}

/// Silently refresh one or all service sessions using hidden WebViews.
///
/// `service`: `"kgc"` | `"luna"` | `"kwic"` | `"all"`
///
/// Returns `true` if all requested refreshes succeeded (Okta still valid).
/// Returns `false` if the Okta session has also expired — the frontend should
/// then open the visible login window which re-authenticates everything at once.
#[tauri::command]
pub async fn sync_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    service: String,
) -> Result<bool, String> {
    log::info!("sync_session: service={}", service);
    let s = state.inner();
    match service.as_str() {
        "kgc" => headless_kgc_refresh(&app, s).await,
        "luna" => headless_luna_refresh(&app, s).await,
        "kwic" => headless_kwic_refresh(&app, s).await,
        "all" => {
            let kgc_ok = headless_kgc_refresh(&app, s).await?;
            if !kgc_ok {
                return Ok(false); // Okta expired; no point trying Luna/KWIC
            }
            let luna_ok = headless_luna_refresh(&app, s).await?;
            let kwic_ok = headless_kwic_refresh(&app, s).await?;
            Ok(luna_ok && kwic_ok)
        }
        _ => Err(format!("Unknown service: {}", service)),
    }
}
