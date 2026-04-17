use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::{Emitter, Manager};
#[cfg(debug_assertions)]
use std::time::Instant;
use std::sync::LazyLock;
use std::sync::Arc;
use crate::config;
use crate::client;
use crate::cookie_bridge;
use crate::luna_client;
use crate::kwic_client;
use regex::Regex;

use crate::auth;
use crate::parser;
use crate::{KgcState, LunaState, KwicState};

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
pub(crate) async fn kgc_post(http: &reqwest::Client, path: &str, params: &[(String, String)]) -> Result<String, String> {
    let url = format!("{}{}", config::KG_COURSE_BASE, path);
    client::post_form_with_redirect(
        http, &url, config::KG_COURSE_BASE,
        client::SESSION_EXPIRED_MSG, client::is_session_expired_body,
        params.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        &[
            ("Referer", &format!("{}/uniasv2/ARF010.do", config::KG_COURSE_BASE)),
            ("Origin", config::KG_COURSE_BASE),
        ],
    ).await
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
        &http, &url, config::KG_COURSE_BASE,
        client::SESSION_EXPIRED_MSG, client::is_session_expired_body,
    ).await
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
                { let _ = std::fs::write(std::env::temp_dir().join($dump), &html); }
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

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionStatus {
    pub valid: bool,
    pub username: String,
    pub display_name: String,
    pub student_id: String,
    pub faculty: String,
    pub department: String,
}

/// Open a login webview window using the Cookie Bridge approach.
/// 1. Navigate webview to KG-Course login entry (triggers SAML redirect to Okta)
/// 2. User authenticates at Okta (or Okta auto-submits if session alive)
/// 3. Webview completes SAML POST natively — SP sets cookies in WKWebView
/// 4. on_navigation detects return to SP domain → extract cookies → inject to reqwest
/// 5. Navigate to Luna SAML entry → same cookie extraction
/// 6. Navigate to KWIC SAML entry → same cookie extraction
#[tauri::command]
pub async fn open_login_window(
    app: tauri::AppHandle,
) -> Result<(), String> {
    let kgc_entry = format!("{}/uniasv2/UnSSOLoginControl2", config::KG_COURSE_BASE);
    log::info!("Cookie Bridge: opening login webview to {}", &kgc_entry);

    if let Some(existing) = app.get_webview_window("login") {
        let _ = existing.close();
    }

    // Channel to signal when a SAML phase completes (page loaded on SP domain)
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(4);

    let parsed_url: url::Url = kgc_entry
        .parse()
        .map_err(|e| format!("URL parse error: {}", e))?;

    // Track current expected SP host (changes between phases)
    let current_sp_host = Arc::new(std::sync::Mutex::new("kg-course.kwansei.ac.jp".to_string()));
    let sp_host_for_load = current_sp_host.clone();

    let _login_window = tauri::WebviewWindowBuilder::new(
        &app,
        "login",
        tauri::WebviewUrl::External(parsed_url),
    )
    .title("\u{95a2}\u{897f}\u{5b66}\u{9662} - \u{30b5}\u{30a4}\u{30f3}\u{30a4}\u{30f3}")
    .inner_size(480.0, 700.0)
    .resizable(true)
    .on_navigation(|_| true) // Allow all navigations (SSO, SP, Shibboleth)
    .on_page_load(move |_win, payload| {
        use tauri::webview::PageLoadEvent;
        if !matches!(payload.event(), PageLoadEvent::Finished) {
            return;
        }
        let url = payload.url();
        let expected_host = sp_host_for_load.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if cookie_bridge::is_post_saml_sp_url(url, &expected_host) {
            log::info!(
                "Cookie Bridge: page loaded on SP domain: {}{}",
                url.host_str().unwrap_or(""),
                url.path()
            );
            let _ = tx.try_send(expected_host);
        }
    })
    .build()
    .map_err(|e| format!("\u{30ed}\u{30b0}\u{30a4}\u{30f3}\u{30a6}\u{30a3}\u{30f3}\u{30c9}\u{30a6}\u{4f5c}\u{6210}\u{5931}\u{6557}: {}", e))?;

    // Background task: wait for each SAML phase, extract cookies, move to next SP
    let app_clone = app.clone();
    tokio::spawn(async move {
        // ===== Phase 1: KG-Course =====
        match tokio::time::timeout(std::time::Duration::from_secs(120), rx.recv()).await {
            Ok(Some(_host)) => {
                log::info!("Cookie Bridge: Phase 1 - KG-Course SAML complete, extracting cookies...");
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;

                let kgc_state = app_clone.state::<KgcState>();
                // Extract and inject cookies
                let cookie_store = kgc_state.client.lock().await.cookie_store.clone();
                let inject_result = cookie_bridge::extract_and_inject(
                    &app_clone,
                    "kg-course.kwansei.ac.jp",
                    &cookie_store,
                    config::KG_COURSE_BASE,
                ).await;
                if let Err(e) = inject_result {
                    log::warn!("Cookie Bridge: cookie extraction failed: {}", e);
                    let _ = app_clone.emit("login-error", &e);
                    if let Some(win) = app_clone.get_webview_window("login") {
                        let _ = win.close();
                    }
                    return;
                }

                // Verify session without holding mutex across network call
                let http = kgc_state.client.lock().await.http.clone();
                let verify_url = format!("{}/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014", config::KG_COURSE_BASE);
                match client::fetch_page_with(&http, &verify_url).await {
                    Ok(html) => {
                        let info = parser::parse_student_info(&html);
                        log::info!("Cookie Bridge: student info: id={}, name={}", info.student_id, info.name);
                        let session = auth::AuthSession {
                            username: info.student_id.clone(),
                            display_name: if info.name.is_empty() { "\u{30e6}\u{30fc}\u{30b6}\u{30fc}".to_string() } else { info.name },
                            student_id: info.student_id,
                            faculty: info.faculty,
                            department: info.department,
                        };
                        let mut client = kgc_state.client.lock().await;
                        client.session = Some(session.clone());
                        client.save_session();
                        drop(client);
                        let _ = app_clone.emit("login-success", &session);
                    }
                    Err(e) => {
                        log::warn!("Cookie Bridge: KGC session verification failed: {}", e);
                        let mut client = kgc_state.client.lock().await;
                        client.clear_session();
                        drop(client);
                        let _ = app_clone.emit("login-error", &e);
                        if let Some(win) = app_clone.get_webview_window("login") {
                            let _ = win.close();
                        }
                        return;
                    }
                }

                log::info!("Cookie Bridge: KG-Course login successful, proceeding to Luna");
            }
            Ok(None) => {
                log::info!("Login window closed without completing login");
                let _ = app_clone.emit("login-cancelled", "\u{30ed}\u{30b0}\u{30a4}\u{30f3}\u{304c}\u{30ad}\u{30e3}\u{30f3}\u{30bb}\u{30eb}\u{3055}\u{308c}\u{307e}\u{3057}\u{305f}");
                return;
            }
            Err(_) => {
                log::warn!("Login timed out (120s)");
                let _ = app_clone.emit("login-error", "Login timed out");
                if let Some(win) = app_clone.get_webview_window("login") {
                    let _ = win.close();
                }
                return;
            }
        }

        // ===== Phase 2: Luna =====
        log::info!("=== Cookie Bridge Phase 2: Luna SAML ===");

        if let Some(win) = app_clone.get_webview_window("login") {
            {
                let mut host = current_sp_host.lock().unwrap_or_else(|e| e.into_inner());
                *host = "luna.kwansei.ac.jp".to_string();
            }
            // Drain any stale page-load signals buffered from Phase 1.
            // Must happen AFTER changing sp_host so no new stale signals can be sent.
            while rx.try_recv().is_ok() {}

            let luna_url: url::Url = config::LUNA_SAML_URL
                .parse()
                .expect("hardcoded Luna SAML URL is valid");
            let _ = win.navigate(luna_url);

            match tokio::time::timeout(std::time::Duration::from_secs(15), rx.recv()).await {
                Ok(Some(_host)) => {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    let luna_state = app_clone.state::<LunaState>();
                    let cookie_store = luna_state.client.lock().await.cookie_store.clone();
                    let result = cookie_bridge::extract_and_inject(
                        &app_clone,
                        "luna.kwansei.ac.jp",
                        &cookie_store,
                        config::LUNA_BASE,
                    ).await;
                    match result {
                        Ok(()) => {
                            // Verify session against server before marking authenticated
                            let http = luna_state.client.lock().await.http.clone();
                            let verify_url = format!("{}/lms/timetable", config::LUNA_BASE);
                            match client::fetch_with_redirect(
                                &http, &verify_url, config::LUNA_BASE,
                                luna_client::LUNA_SESSION_EXPIRED_MSG, luna_client::is_luna_session_expired,
                            ).await {
                                Ok(_) => {
                                    let mut luna = luna_state.client.lock().await;
                                    luna.authenticated = true;
                                    luna.save_session();
                                    drop(luna);
                                    log::info!("Cookie Bridge: Luna login successful (verified)");
                                    let _ = app_clone.emit("luna-login-success", ());
                                }
                                Err(e) => {
                                    log::warn!("Cookie Bridge: Luna session verification failed: {}", e);
                                    let _ = app_clone.emit("luna-login-error", &e);
                                }
                            }
                        }
                        Err(e) => {
                            log::warn!("Cookie Bridge: Luna cookie extraction failed: {}", e);
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
        }

        // ===== Phase 3: KWIC Portal =====
        log::info!("=== Cookie Bridge Phase 3: KWIC Portal SAML ===");

        if let Some(win) = app_clone.get_webview_window("login") {
            {
                let mut host = current_sp_host.lock().unwrap_or_else(|e| e.into_inner());
                *host = "kwic.kwansei.ac.jp".to_string();
            }
            // Drain stale signals from Phase 2
            while rx.try_recv().is_ok() {}
            let kwic_url: url::Url = config::KWIC_SAML_URL
                .parse()
                .expect("hardcoded KWIC SAML URL is valid");
            let _ = win.navigate(kwic_url);

            match tokio::time::timeout(std::time::Duration::from_secs(15), rx.recv()).await {
                Ok(Some(_host)) => {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    let kwic_state = app_clone.state::<KwicState>();
                    let cookie_store = kwic_state.client.lock().await.cookie_store.clone();
                    let result = cookie_bridge::extract_and_inject(
                        &app_clone,
                        "kwic.kwansei.ac.jp",
                        &cookie_store,
                        config::KWIC_BASE,
                    ).await;
                    match result {
                        Ok(()) => {
                            // Verify session against server before marking authenticated
                            let http = kwic_state.client.lock().await.http.clone();
                            let verify_url = format!("{}/portal/home", config::KWIC_BASE);
                            match client::fetch_with_redirect(
                                &http, &verify_url, config::KWIC_BASE,
                                kwic_client::KWIC_SESSION_EXPIRED_MSG, kwic_client::is_kwic_session_expired,
                            ).await {
                                Ok(_) => {
                                    let mut kwic = kwic_state.client.lock().await;
                                    kwic.authenticated = true;
                                    kwic.save_session();
                                    drop(kwic);
                                    log::info!("Cookie Bridge: KWIC Portal login successful (verified)");
                                    let _ = app_clone.emit("kwic-login-success", ());
                                }
                                Err(e) => {
                                    log::warn!("Cookie Bridge: KWIC session verification failed: {}", e);
                                    let _ = app_clone.emit("kwic-login-error", &e);
                                }
                            }
                        }
                        Err(e) => {
                            log::warn!("Cookie Bridge: KWIC cookie extraction failed: {}", e);
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
    });

    Ok(())
}

#[tauri::command]
pub async fn logout(
    app: tauri::AppHandle,
    state: State<'_, KgcState>,
    luna_state: State<'_, LunaState>,
    kwic_state: State<'_, KwicState>,
) -> Result<(), String> {
    let mut client = state.client.lock().await;
    client.clear_session();
    drop(client);
    let mut luna = luna_state.client.lock().await;
    luna.clear();
    drop(luna);
    let mut kwic = kwic_state.client.lock().await;
    kwic.clear();
    drop(kwic);
    let _ = app.emit("logout", ());
    Ok(())
}

#[tauri::command]
pub async fn check_session(
    state: State<'_, KgcState>,
) -> Result<SessionStatus, String> {
    log::info!("check_session: called");
    let _kgc_gate = state.gate.lock().await;
    // Single lock: try restore if needed, then clone snapshot
    let (http, session_snapshot) = {
        let mut client = state.client.lock().await;
        if client.session.is_none() && client.try_restore_session() {
            log::info!("check_session: restored session from disk, will validate...");
        }
        let has_session = client.session.is_some();
        log::info!("check_session: has_session={}", has_session);
        (client.http.clone(), client.session.clone())
    };

    let session = match session_snapshot {
        Some(s) => {
            log::info!("check_session: validating user={} sid={}", s.username, s.student_id);
            s
        }
        None => {
            log::info!("check_session: no session found, returning invalid");
            return Ok(SessionStatus {
                valid: false,
                username: String::new(),
                display_name: String::new(),
                student_id: String::new(),
                faculty: String::new(),
                department: String::new(),
            });
        }
    };

    // Validate session against the server without holding the lock
    let verify_url = format!("{}/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014", config::KG_COURSE_BASE);
    log::info!("check_session: fetching verify page...");
    match client::fetch_page_with(&http, &verify_url).await {
        Ok(html) => {
            // Parse the page to verify it actually contains user data.
            // When the server-side session is stale, KG-Course returns a 200
            // page with empty hidden inputs instead of an SSO redirect.
            let info = parser::parse_student_info(&html);
            if info.student_id.is_empty() && info.name.is_empty() {
                log::warn!("check_session: server returned empty page (stale session), disk user={} sid={}", session.username, session.student_id);
                let mut client = state.client.lock().await;
                client.clear_session();
                // Return the disk-saved user info so frontend can show cached data
                return Ok(SessionStatus {
                    valid: false,
                    username: session.username,
                    display_name: session.display_name,
                    student_id: session.student_id,
                    faculty: session.faculty,
                    department: session.department,
                });
            }

            // Update user info if needed, then always persist cookies
            // (server may have rotated session cookies during validation)
            let needs_update = session.student_id.is_empty() || session.display_name == "\u{30e6}\u{30fc}\u{30b6}\u{30fc}";
            let mut client = state.client.lock().await;
            if needs_update {
                log::info!("Reparsed student info: id={}, name={}, faculty={}, dept={}", info.student_id, info.name, info.faculty, info.department);
                if let Some(s) = &mut client.session {
                    if !info.student_id.is_empty() {
                        s.username = info.student_id.clone();
                        s.student_id = info.student_id;
                    }
                    if !info.name.is_empty() {
                        s.display_name = info.name;
                    }
                    s.faculty = info.faculty;
                    s.department = info.department;
                }
            }
            client.save_session();
            // Return session info -- prefer live state, but if session was
            // concurrently cleared (logout), report invalid instead of stale data.
            match client.session.as_ref() {
                Some(s) => Ok(SessionStatus {
                    valid: true,
                    username: s.username.clone(),
                    display_name: s.display_name.clone(),
                    student_id: s.student_id.clone(),
                    faculty: s.faculty.clone(),
                    department: s.department.clone(),
                }),
                None => {
                    log::warn!("Session cleared by concurrent logout after successful validation");
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
        Err(e) => {
            if e == client::SESSION_EXPIRED_MSG {
                log::info!("check_session: session expired (server confirmed), disk user={} sid={}", session.username, session.student_id);
                let mut client = state.client.lock().await;
                client.clear_session();
            } else {
                log::warn!("check_session: validation failed (transient?): {} -- keeping disk state, user={}", e, session.username);
            }
            // Return disk-saved user info so frontend can display cached data
            Ok(SessionStatus {
                valid: false,
                username: session.username,
                display_name: session.display_name,
                student_id: session.student_id,
                faculty: session.faculty,
                department: session.department,
            })
        }
    }
}

/// Validate the session by actually hitting the server.
/// Returns valid=false if session has expired on the server.
#[tauri::command]
pub async fn validate_session(state: State<'_, KgcState>) -> Result<SessionStatus, String> {
    // Clone what we need, then release the lock before network I/O
    let (http, is_auth, session_snapshot) = {
        let client = state.client.lock().await;
        (client.http.clone(), client.is_authenticated(), client.session.clone())
    };

    if !is_auth {
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
    let verify_url = format!("{}/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014", config::KG_COURSE_BASE);
    match client::fetch_page_with(&http, &verify_url).await {
        Ok(html) => {
            // Verify the page actually contains user data.
            // When the server-side session is stale, KG-Course returns a 200
            // page with empty hidden inputs instead of an SSO redirect.
            let info = parser::parse_student_info(&html);
            if info.student_id.is_empty() && info.name.is_empty() {
                log::warn!("validate_session: server returned empty page (stale session)");
                let snap = session_snapshot.as_ref();
                return Ok(SessionStatus {
                    valid: false,
                    username: snap.map_or(String::new(), |s| s.username.clone()),
                    display_name: snap.map_or(String::new(), |s| s.display_name.clone()),
                    student_id: snap.map_or(String::new(), |s| s.student_id.clone()),
                    faculty: snap.map_or(String::new(), |s| s.faculty.clone()),
                    department: snap.map_or(String::new(), |s| s.department.clone()),
                });
            }
            // Persist cookies — the server may have rotated/renewed session cookies
            let client = state.client.lock().await;
            client.save_session();
            let session = session_snapshot.as_ref()
                .ok_or_else(|| "session lost after fetch".to_string())?;
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
            let snap = session_snapshot.as_ref();
            Ok(SessionStatus {
                valid: false,
                username: snap.map_or(String::new(), |s| s.username.clone()),
                display_name: snap.map_or(String::new(), |s| s.display_name.clone()),
                student_id: snap.map_or(String::new(), |s| s.student_id.clone()),
                faculty: snap.map_or(String::new(), |s| s.faculty.clone()),
                department: snap.map_or(String::new(), |s| s.department.clone()),
            })
        }
    }
}

#[tauri::command]
pub async fn fetch_grades(state: State<'_, KgcState>, db: State<'_, crate::db::Database>) -> Result<parser::GradesData, String> {
    kgc_fetch_cached!(state, db, "grades", "/uniasv2/ARF140.do?REQ_PRFR_MNU_ID=MNUIDSTD0102020", parser::parse_grades, "kgc-grades.html")
}

#[tauri::command]
pub async fn fetch_cancellations(state: State<'_, KgcState>, db: State<'_, crate::db::Database>) -> Result<parser::CancellationsData, String> {
    kgc_fetch_cached!(state, db, "cancellations", "/uniasv2/APB020PLS01Action.do?REQ_PRFR_MNU_ID=MNUIDSTD0101011", parser::parse_cancellations, "kgc-cancellations.html")
}

#[tauri::command]
pub async fn fetch_makeup_classes(state: State<'_, KgcState>, db: State<'_, crate::db::Database>) -> Result<parser::MakeupData, String> {
    kgc_fetch_cached!(state, db, "makeup", "/uniasv2/APC020PLS01Action.do?REQ_PRFR_MNU_ID=MNUIDSTD0101012", parser::parse_makeup_classes, "kgc-makeup.html")
}

#[tauri::command]
pub async fn fetch_room_changes(state: State<'_, KgcState>, db: State<'_, crate::db::Database>) -> Result<parser::RoomChangesData, String> {
    kgc_fetch_cached!(state, db, "rooms", "/uniasv2/APA960.do?REQ_PRFR_MNU_ID=MNUIDSTD0101013", parser::parse_room_changes, "kgc-roomchanges.html")
}

#[tauri::command]
pub async fn fetch_registration(state: State<'_, KgcState>, db: State<'_, crate::db::Database>) -> Result<parser::RegistrationData, String> {
    kgc_fetch_cached!(state, db, "registration", "/uniasv2/ARD010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102012", parser::parse_registration, "kgc-registration.html")
}

#[tauri::command]
pub async fn fetch_exam_timetable(state: State<'_, KgcState>, db: State<'_, crate::db::Database>) -> Result<parser::ExamTimetableData, String> {
    kgc_fetch_cached!(state, db, "exam_timetable", "/uniasv2/ARF010PVL01Action.do?REQ_PRFR_MNU_ID=MNUIDSTD0102019", parser::parse_exam_timetable)
}

#[tauri::command]
pub async fn fetch_notifications(
    state: State<'_, KgcState>,
    db: State<'_, crate::db::Database>,
) -> Result<parser::NotificationsData, String> {
    kgc_fetch_cached!(state, db, "notifications", "/uniasv2/CPA010PLS01Action.do?REQ_FUNCTION_JUMP_START_FLG=1&PRD_FLG=1&REQ_PRFR_FUNC_ID=CPA010", parser::parse_notifications, "kgc-notifications.html")
}

// ── Calendar.app JXA sync helpers (dev-only) ──

#[cfg(debug_assertions)]
fn period_to_time(period: i32) -> Option<(u32, u32, u32, u32)> {
    if (1..=5).contains(&period) {
        Some(config::PERIOD_TIMES[(period - 1) as usize])
    } else {
        None
    }
}

#[cfg(debug_assertions)]
static WEEK_MONDAY_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d{4})/(\d{2})/(\d{2})\(月\)").expect("valid regex"));
#[cfg(debug_assertions)]
static WEEK_DATE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d{4})/(\d{2})/(\d{2})").expect("valid regex"));

#[cfg(debug_assertions)]
fn parse_week_start(week_label: &str) -> Result<(i32, u32, u32), String> {
    let caps = WEEK_MONDAY_RE.captures(week_label)
        .or_else(|| WEEK_DATE_RE.captures(week_label));
    if let Some(caps) = caps {
        let y: i32 = caps[1].parse().map_err(|e| format!("year parse error: {}", e))?;
        let m: u32 = caps[2].parse().map_err(|e| format!("month parse error: {}", e))?;
        let d: u32 = caps[3].parse().map_err(|e| format!("day parse error: {}", e))?;
        return Ok((y, m, d));
    }
    Err(format!("週ラベルを解析できません: {}", week_label))
}

#[cfg(debug_assertions)]
fn add_days(year: i32, month: u32, day: u32, offset: i32) -> Result<(i32, u32, u32), String> {
    use chrono::{Datelike, NaiveDate};
    let date = NaiveDate::from_ymd_opt(year, month, day)
        .ok_or_else(|| format!("無効な日付: {}/{}/{}", year, month, day))?
        + chrono::Duration::days(offset as i64);
    Ok((date.year(), date.month(), date.day()))
}

/// Only available in dev builds (App Store forbids apple-events entitlement).
#[cfg(debug_assertions)]
async fn run_jxa(script: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let output = std::process::Command::new("osascript")
            .arg("-l").arg("JavaScript")
            .arg("-e").arg(&script)
            .output()
            .map_err(|e| format!("osascript 実行失敗: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("osascript エラー: {}", stderr.trim()));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct CalendarSyncEntry {
    pub day: String,
    pub period: i32,
    pub course_name: String,
    pub room: String,
    pub is_cancelled: bool,
}

#[cfg(debug_assertions)]
fn escape_js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\'' => out.push_str("\\'"),
            '`' => out.push('`'),
            _ => out.push(c),
        }
    }
    out
}

/// Sync timetable entries to macOS Calendar.app (dev-only; release returns error).
#[tauri::command]
pub async fn sync_calendar(
    entries: Vec<CalendarSyncEntry>,
    week_label: String,
) -> Result<String, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (&entries, &week_label);
        return Err("Apple Calendar は macOS でのみ利用可能です".into());
    }
    #[cfg(all(target_os = "macos", not(debug_assertions)))]
    {
        let _ = (&entries, &week_label);
        return Err("Calendar.app 連携は開発ビルドのみ対応です。Google カレンダーをご利用ください。".into());
    }
    #[cfg(all(target_os = "macos", debug_assertions))]
    {
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
        let (y, m, d) = add_days(base_year, base_month, base_day, offset)?;
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

    let end_sat = add_days(base_year, base_month, base_day, 5)?;
    let script = format!(
        r#"
var Calendar = Application("Calendar");
Calendar.includeStandardAdditions = true;

// Find or create the KWIC calendar
var calName = "Selah 時間割";
var cal = null;
var calendars = Calendar.calendars();
for (var i = 0; i < calendars.length; i++) {{
  try {{
    if (calendars[i].name() === calName) {{
      cal = calendars[i];
      break;
    }}
  }} catch(e) {{}}
}}
if (!cal) {{
  cal = Calendar.Calendar({{ name: calName }});
  Calendar.calendars.push(cal);
}}

// Delete events within this week's range only
var weekStart = new Date({wy},{wmi},{wd},0,0,0);
var weekEnd = new Date({wey},{wemi},{wed},23,59,59);
try {{
  var events = cal.events();
  for (var i = events.length - 1; i >= 0; i--) {{
    try {{
      var sd = events[i].startDate();
      if (sd >= weekStart && sd <= weekEnd) {{
        Calendar.delete(events[i]);
      }}
    }} catch(e) {{}}
  }}
}} catch(e) {{}}

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
        wey = end_sat.0,
        wemi = end_sat.1 - 1,
        wed = end_sat.2
    );

    let count = run_jxa(script).await
        .map_err(|e| format!("カレンダー同期失敗: {}", e))?;
    log::info!("Calendar sync: {} events added", count);
    Ok(format!("{}件のイベントを同期しました", count))
    }
}

/// Get info about the KG-Course calendar (exists, event count)
#[tauri::command]
pub async fn get_calendar_info() -> Result<serde_json::Value, String> {
    #[cfg(not(target_os = "macos"))]
    { return Ok(serde_json::json!({ "exists": false, "count": 0 })); }
    #[cfg(all(target_os = "macos", not(debug_assertions)))]
    { return Ok(serde_json::json!({ "exists": false, "count": 0 })); }
    #[cfg(all(target_os = "macos", debug_assertions))]
    {
    let script = r#"
var Calendar = Application("Calendar");
var calName = "Selah 時間割";
var cal = null;
try {
  var calendars = Calendar.calendars();
  for (var i = 0; i < calendars.length; i++) {
    try {
      if (calendars[i].name() === calName) {
        cal = calendars[i];
        break;
      }
    } catch(e) {}
  }
} catch(e) {}
if (!cal) {
  JSON.stringify({ exists: false, count: 0 });
} else {
  var c = 0;
  try { c = cal.events().length; } catch(e) {}
  JSON.stringify({ exists: true, count: c });
}
"#;
    match run_jxa(script.to_string()).await {
        Ok(stdout) => serde_json::from_str(&stdout).map_err(|e| format!("JSON parse error: {}", e)),
        Err(_) => Ok(serde_json::json!({ "exists": false, "count": 0 })),
    }
    }
}

/// Clear all events from the KG-Course calendar, or delete the calendar entirely
#[tauri::command]
pub async fn clear_calendar(delete_calendar: bool) -> Result<String, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = delete_calendar;
        return Err("Apple Calendar は macOS でのみ利用可能です".into());
    }
    #[cfg(all(target_os = "macos", not(debug_assertions)))]
    {
        let _ = delete_calendar;
        return Err("Calendar.app 連携は開発ビルドのみ対応です".into());
    }
    #[cfg(all(target_os = "macos", debug_assertions))]
    {
    let script = if delete_calendar {
        r#"
var Calendar = Application("Calendar");
var calName = "Selah 時間割";
var found = false;
try {
  var calendars = Calendar.calendars();
  for (var i = 0; i < calendars.length; i++) {
    try {
      if (calendars[i].name() === calName) {
        found = true;
        Calendar.delete(calendars[i]);
        break;
      }
    } catch(e) {}
  }
} catch(e) {}
found ? "deleted" : "not_found";
"#.to_string()
    } else {
        r#"
var Calendar = Application("Calendar");
var calName = "Selah 時間割";
var cal = null;
try {
  var calendars = Calendar.calendars();
  for (var i = 0; i < calendars.length; i++) {
    try {
      if (calendars[i].name() === calName) {
        cal = calendars[i];
        break;
      }
    } catch(e) {}
  }
} catch(e) {}
var count = 0;
if (cal) {
  try {
    var events = cal.events();
    count = events.length;
    for (var i = events.length - 1; i >= 0; i--) {
      try { Calendar.delete(events[i]); } catch(e) {}
    }
  } catch(e) {}
}
count + "";
"#.to_string()
    };

    let result = run_jxa(script).await
        .map_err(|e| format!("カレンダー操作失敗: {}", e))?;

    if delete_calendar {
        if result.contains("not_found") {
            Ok("カレンダーが見つかりません".into())
        } else {
            Ok("カレンダーを削除しました".into())
        }
    } else {
        Ok(format!("{}件のイベントを削除しました", result))
    }
    }
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
pub async fn fetch_course_detail(state: State<'_, KgcState>, db: State<'_, crate::db::Database>, path: String) -> Result<parser::CourseDetail, String> {
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
    let existing = app.webview_windows().keys()
        .filter(|k| k.starts_with("detail-")).count();
    if existing >= 10 {
        return Err(config::TOO_MANY_WINDOWS_MSG.into());
    }

    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = format!("detail-{}", id);

    let encoded_path = urlencoding::encode(&path);
    let encoded_name = urlencoding::encode(&course_name);
    let url_str = format!("luna-detail.html?mode=kgc&path={}&name={}", encoded_path, encoded_name);

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

/// Open an external URL in a new webview window with browser toolbar
#[tauri::command]
pub async fn open_external_url(
    app: tauri::AppHandle,
    url: String,
    title: Option<String>,
) -> Result<(), String> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    let parsed_url: url::Url = url.parse()
        .map_err(|e| format!("URL parse error: {}", e))?;

    // Only allow http/https URLs
    let scheme = parsed_url.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(format!("Unsupported URL scheme: {}", scheme));
    }

    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = format!("ext-{}", id);
    let win_title = title.unwrap_or_else(|| parsed_url.host_str().unwrap_or("Web").to_string());

    crate::webview_toolbar::create_browser_window(
        &app,
        &label,
        tauri::WebviewUrl::External(parsed_url),
        &win_title,
        900.0, 640.0,
        &[],
    )?;

    Ok(())
}

/// Open a URL in the system default browser (Safari, Chrome, etc.)
#[tauri::command]
pub async fn open_in_system_browser(app: tauri::AppHandle, url: String) -> Result<(), String> {
    let parsed: url::Url = url.parse()
        .map_err(|e| format!("URL parse error: {}", e))?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(format!("Unsupported URL scheme: {}", scheme));
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_url(&url, None::<&str>)
        .map_err(|e| format!("ブラウザを開けませんでした: {}", e))?;
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

    let url: url::Url = format!("{}/uniasv2/UnSSOLoginControl2?REQ_LOGIN_NO=2&REQ_ACTION_DO=/GGA110.do&REQ_PRFR_MNU_ID=MNUIDSTD0104011", config::KG_COURSE_BASE)
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
    let url: url::Url = format!("{}/uniasv2/UnSSOLoginControl2?REQ_LOGIN_NO=2&REQ_ACTION_DO=/ARD010.do&REQ_PRFR_MNU_ID=MNUIDSTD0102012&SE_LANGUAGE=", config::KG_COURSE_BASE)
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
pub async fn fetch_student_profile(state: State<'_, KgcState>, db: State<'_, crate::db::Database>) -> Result<parser::StudentInfo, String> {
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
    let url1 = format!("{}/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014", config::KG_COURSE_BASE);
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
    let url2 = format!("{}/uniasv2/GGA110.do?REQ_PRFR_MNU_ID=MNUIDSTD0104011", config::KG_COURSE_BASE);
    if let Ok(html) = client::fetch_page_with(&http, &url2).await {
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
}

#[cfg(debug_assertions)]
#[tauri::command]
pub async fn debug_info(state: State<'_, KgcState>) -> Result<DebugInfo, String> {
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
pub async fn debug_info() -> Result<DebugInfo, String> {
    Err("debug commands are not available in release builds".into())
}

#[cfg(not(debug_assertions))]
#[tauri::command]
pub async fn debug_ping() -> Result<PingResult, String> {
    Err("debug commands are not available in release builds".into())
}

// ============ Syllabus ============

#[cfg(debug_assertions)]
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
    kgc_state: State<'_, KgcState>,
) -> Result<crate::syllabus::SyllabusSearchResult, String> {
    let _kgc_gate = kgc_state.gate.lock().await;
    let http = kgc_http(kgc_state.inner()).await?;

    let search_html = kgc_get(&http,
        "/uniasv2/UnSSOLoginControl2?REQ_LOGIN_NO=2&REQ_ACTION_DO=/AGA030.do&REQ_PRFR_MNU_ID=MNUIDSTD0103011",
    ).await?;
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

    let html = kgc_post(&http, "/uniasv2/AGA030PSC01EventAction.do", &form_params).await?;

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
        let mut form_params = extract_all_form_inputs(&current_html);
        form_params.retain(|(k, _)| {
            !k.starts_with("ESearch") && !k.starts_with("ENarrowSearch")
            && !k.starts_with("EBack") && !k.starts_with("ENext")
            && !k.starts_with("EPrev") && !k.starts_with("ERefer")
            && !k.starts_with("ERegister") && !k.starts_with("EPageSet")
        });
        form_params.push(("ENext.x".into(), "10".into()));
        form_params.push(("ENext.y".into(), "10".into()));

        log::info!("Fetching page {} with {} form params", page, form_params.len());

        let next_html = kgc_post(&http, "/uniasv2/AGA030PLS01EventAction.do", &form_params).await?;

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
    kgc_state: State<'_, KgcState>,
    db: State<'_, crate::db::Database>,
) -> Result<crate::syllabus::SyllabusSearchResult, String> {
    let _kgc_gate = kgc_state.gate.lock().await;
    let http = match kgc_http(kgc_state.inner()).await {
        Ok(h) => h,
        Err(e) => {
            // Network/session error → fall back to cache
            if let Ok(Some((json, _))) = db.get_data_cache("syllabus_favorites") {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("syllabus_favorites: cache fallback ({})", e);
                    return Ok(cached);
                }
            }
            return Err(e);
        }
    };

    let main_terms = ["02", "03", "01"];
    let sub_terms = ["04", "05", "06", "07"];
    let mut all_entries = Vec::new();
    let mut seen_codes = std::collections::HashSet::new();

    for term_code in main_terms.iter().chain(sub_terms.iter()) {
        let search_html = kgc_get(&http,
            "/uniasv2/UnSSOLoginControl2?REQ_LOGIN_NO=2&REQ_ACTION_DO=/AGA030.do&REQ_PRFR_MNU_ID=MNUIDSTD0103011",
        ).await?;
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
        let html = kgc_post(&http, "/uniasv2/AGA030PSC01EventAction.do", &params).await?;

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

    let result = crate::syllabus::SyllabusSearchResult {
        entries: all_entries,
        total_count: 0,
        current_page: 1,
        total_pages: 1,
    };

    // Cache to DB
    if let Ok(json) = serde_json::to_string(&result) {
        let _ = db.save_data_cache("syllabus_favorites", &json);
    }

    Ok(result)
}

/// Search for a specific class_code across terms, returning the results HTML page.
pub(crate) async fn find_syllabus_results_by_class_code(
    http: &reqwest::Client,
    class_code: &str,
) -> Result<String, String> {
    let terms = ["02", "03", "01", "04", "05", "06", "07"];
    for term_code in &terms {
        let search_html = kgc_get(http,
            "/uniasv2/UnSSOLoginControl2?REQ_LOGIN_NO=2&REQ_ACTION_DO=/AGA030.do&REQ_PRFR_MNU_ID=MNUIDSTD0103011",
        ).await?;
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
        let html = kgc_post(http, "/uniasv2/AGA030PSC01EventAction.do", &search_params).await?;

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
    kgc_state: State<'_, KgcState>,
    class_code: String,
) -> Result<bool, String> {
    let _kgc_gate = kgc_state.gate.lock().await;
    let http = kgc_http(kgc_state.inner()).await?;

    let html = find_syllabus_results_by_class_code(&http, &class_code).await?;

    // Find the target row's register index
    let parsed = crate::syllabus::parse_search_results_public(&html)?;
    let target_entry = parsed.entries.iter()
        .find(|e| e.class_code == class_code)
        .ok_or_else(|| format!("科目コード {} が見つかりません", class_code))?;
    let target_index = target_entry.register_index.clone();

    // Extract ALL form fields from results page (same approach as pagination fix)
    let mut form_params = extract_all_form_inputs(&html);

    // Remove action buttons and search-dispatch flag
    form_params.retain(|(k, _)| {
        !k.starts_with("ESearch") && !k.starts_with("ENarrowSearch")
        && !k.starts_with("EBack") && !k.starts_with("ENext")
        && !k.starts_with("EPrev") && !k.starts_with("ERefer")
        && !k.starts_with("ERegister") && !k.starts_with("EPageSet")
        && k != "hdnEsearch"
    });

    // Set the target register index and add ERegister action
    form_params.retain(|(k, _)| k != "eregisterIndex");
    form_params.push(("eregisterIndex".into(), target_index.clone()));
    form_params.push(("ERegister.x".into(), "10".into()));
    form_params.push(("ERegister.y".into(), "10".into()));

    log::info!("Bookmark toggle: class_code={}, eregisterIndex={}, params_count={}",
        class_code, target_index, form_params.len());

    let toggle_html = kgc_post(&http, "/uniasv2/AGA030PLS01EventAction.do", &form_params).await?;

    let success = !toggle_html.contains("UNM000480E") && !toggle_html.contains("不正アクセス");
    log::info!("Bookmark toggle result: success={}, len={}", success, toggle_html.len());

    Ok(success)
}

#[tauri::command]
pub async fn open_syllabus_detail(
    app: tauri::AppHandle,
    kgc_state: State<'_, KgcState>,
    class_code: String,
    course_name: String,
) -> Result<(), String> {
    let _kgc_gate = kgc_state.gate.lock().await;
    let http = kgc_http(kgc_state.inner()).await?;

    // Search by class_code across terms to find the course
    let html = find_syllabus_results_by_class_code(&http, &class_code).await?;

    // Parse results to obtain the fresh ereferIndex for this course
    let results = crate::syllabus::parse_search_results_public(&html)
        .map_err(|e| format!("検索結果の解析に失敗: {}", e))?;
    let target_entry = results.entries.iter()
        .find(|e| e.class_code == class_code)
        .ok_or("授業が見つかりませんでした")?;
    let fresh_refer_index = target_entry.refer_index.clone();

    // Extract ALL form fields from results page (same approach as pagination/bookmark fix)
    let mut form_params = extract_all_form_inputs(&html);

    // Deduplicate Struts tokens (multiple forms on page) — keep only the LAST one
    let token_key = "org.apache.struts.taglib.html.TOKEN";
    let token_count = form_params.iter().filter(|(k, _)| k == token_key).count();
    if token_count > 1 {
        let last_token = form_params.iter().rev()
            .find(|(k, _)| k == token_key).map(|(_, v)| v.clone());
        form_params.retain(|(k, _)| k != token_key);
        if let Some(tok) = last_token {
            form_params.insert(0, (token_key.into(), tok));
        }
        log::warn!("open_syllabus_detail: deduped Struts tokens: {} -> 1", token_count);
    }

    // Remove action buttons and search-dispatch flag
    form_params.retain(|(k, _)| {
        !k.starts_with("ESearch") && !k.starts_with("ENarrowSearch")
        && !k.starts_with("EBack") && !k.starts_with("ENext")
        && !k.starts_with("EPrev") && !k.starts_with("ERefer")
        && !k.starts_with("ERegister") && !k.starts_with("EPageSet")
        && k != "hdnEsearch"
    });

    // Set the target refer index and add ERefer action
    form_params.retain(|(k, _)| k != "ereferIndex");
    form_params.push(("ereferIndex".into(), fresh_refer_index.clone()));
    form_params.push(("ERefer.x".into(), "10".into()));
    form_params.push(("ERefer.y".into(), "10".into()));

    log::info!("Syllabus detail: ereferIndex={}, params_count={}", fresh_refer_index, form_params.len());

    let detail_html = kgc_post(&http, "/uniasv2/AGA030PLS01EventAction.do", &form_params).await?;

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
    let url_str = format!("luna-detail.html?mode=syllabus&name={}&wlabel={}", encoded_name, encoded_label);

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

/// Get cached KGC syllabus fields for a course (textbook, references, etc.)
#[tauri::command]
pub async fn get_kgc_syllabus_fields(
    db: State<'_, crate::db::Database>,
    kgc_code: String,
) -> Result<Option<serde_json::Value>, String> {
    Ok(db.get_kgc_course_detail(&kgc_code)?.map(|d| {
        serde_json::json!({
            "fields": d.fields,
            "textbooks": d.textbooks,
        })
    }))
}

static STRUTS_TOKEN_RE1: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"name="org\.apache\.struts\.taglib\.html\.TOKEN"[^>]*value="([^"]+)""#).expect("valid regex"));
static STRUTS_TOKEN_RE2: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"value="([^"]+)"[^>]*name="org\.apache\.struts\.taglib\.html\.TOKEN""#).expect("valid regex"));

pub(crate) fn extract_struts_token(html: &str) -> Result<String, String> {
    STRUTS_TOKEN_RE1.captures(html)
        .or_else(|| STRUTS_TOKEN_RE2.captures(html))
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| "Strutsトークンが見つかりません".into())
}

static YEAR_RE1: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"name="txtLsnOpcFcy"[^>]*value="(\d{4})""#).expect("valid regex"));
static YEAR_RE2: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"value="(\d{4})"[^>]*name="txtLsnOpcFcy""#).expect("valid regex"));

pub(crate) fn extract_year_from_search_page(html: &str) -> Option<String> {
    YEAR_RE1.captures(html)
        .or_else(|| YEAR_RE2.captures(html))
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Extract ALL form inputs from the results page HTML for pagination replay.
/// Collects hidden inputs, text inputs, and selected option values from the main form.
pub(crate) fn extract_all_form_inputs(html: &str) -> Vec<(String, String)> {
    extract_form_inputs_impl(html, "form")
}

/// Extract inputs only from the form with the given name attribute.
/// Falls back to all forms if the named form is not found.
pub(crate) fn extract_named_form_inputs(html: &str, form_name: &str) -> Vec<(String, String)> {
    let selector = format!("form[name=\"{}\"]", form_name);
    let params = extract_form_inputs_impl(html, &selector);
    if params.is_empty() {
        log::warn!("extract_named_form_inputs: form '{}' not found, falling back to all forms", form_name);
        extract_form_inputs_impl(html, "form")
    } else {
        params
    }
}

fn extract_form_inputs_impl(html: &str, form_selector: &str) -> Vec<(String, String)> {
    use scraper::{Html, Selector};

    let document = Html::parse_document(html);
    let mut params: Vec<(String, String)> = Vec::new();

    // Collect all <input> elements (hidden, text, etc.)
    let input_sel = Selector::parse(&format!("{} input", form_selector)).expect("valid selector");
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
        if (input_type == "checkbox" || input_type == "radio")
            && el.value().attr("checked").is_none() {
                continue;
            }
        let value = el.value().attr("value").unwrap_or("").to_string();
        params.push((name, value));
    }

    // Collect <select> elements with their selected <option> value
    let select_sel = Selector::parse(&format!("{} select", form_selector)).expect("valid selector");
    let option_sel = Selector::parse("option[selected]").expect("valid selector");
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
pub async fn get_session_states(
    state: State<'_, KgcState>,
    luna_state: State<'_, LunaState>,
    kwic_state: State<'_, KwicState>,
) -> Result<SessionStates, String> {
    let kgc = state.client.lock().await.is_authenticated();
    let luna = luna_state.client.lock().await.authenticated;
    let kwic = kwic_state.client.lock().await.authenticated;
    Ok(SessionStates { kgc, luna, kwic })
}

/// Return seconds until soonest cookie expiry across all services.
/// Returns null/None if no time-limited cookies exist.
#[tauri::command]
pub async fn get_session_expiry(
    state: State<'_, KgcState>,
    luna_state: State<'_, LunaState>,
    kwic_state: State<'_, KwicState>,
) -> Result<Option<i64>, String> {
    let kgc_exp = state.client.lock().await.soonest_cookie_expiry_secs();
    let luna_exp = client::soonest_cookie_expiry(&luna_state.client.lock().await.cookie_store);
    let kwic_exp = client::soonest_cookie_expiry(&kwic_state.client.lock().await.cookie_store);
    let min = [kgc_exp, luna_exp, kwic_exp]
        .into_iter()
        .flatten()
        .min();
    Ok(min)
}

/// Attempt a silent (headless) KG-Course session refresh via an invisible WebView.
/// Uses the Cookie Bridge approach: navigate to the SP's login entry, let the
/// webview's persisted Okta cookies auto-authenticate, then extract session cookies.
/// Returns true on success, false when Okta has also expired.
///
/// Shared helper for headless SAML refresh (Luna / KWIC pattern).
/// Handles: headless window -> cookie injection -> verify with redirect detection.
/// Returns `Ok(true)` on verified success, `Ok(false)` if Okta expired, `Err` on failure.
#[allow(clippy::too_many_arguments)]
async fn headless_saml_refresh(
    app: &tauri::AppHandle,
    label: &str,
    saml_url: &str,
    sp_domain: &str,
    base_url: &str,
    verify_url: &str,
    cookie_store: &reqwest_cookie_store::CookieStoreMutex,
    http: &reqwest::Client,
    session_expired_msg: &str,
    is_session_expired: fn(&str) -> bool,
) -> Result<bool, String> {
    log::info!("headless_{}: starting (Cookie Bridge)", label);

    let win = match cookie_bridge::headless_saml_window(
        app, label, saml_url, sp_domain, 20,
    ).await? {
        Some(w) => w,
        None => return Ok(false),
    };

    cookie_bridge::extract_and_inject(app, sp_domain, cookie_store, base_url).await?;

    let result = client::fetch_with_redirect(
        http, verify_url, base_url, session_expired_msg, is_session_expired,
    ).await;
    let _ = win.close();

    match result {
        Ok(_) => {
            log::info!("headless_{}: succeeded (verified)", label);
            Ok(true)
        }
        Err(e) => {
            log::warn!("headless_{}: cookie injection succeeded but session invalid: {}", label, e);
            Err(e)
        }
    }
}

async fn headless_kgc_refresh(
    app: &tauri::AppHandle,
    state: &KgcState,
) -> Result<bool, String> {
    log::info!("headless_kgc_refresh: starting (Cookie Bridge)");
    let _kgc_gate = state.gate.lock().await;

    let entry_url = format!("{}/uniasv2/UnSSOLoginControl2", config::KG_COURSE_BASE);
    let win = match cookie_bridge::headless_saml_window(
        app, "kgc-headless", &entry_url, "kg-course.kwansei.ac.jp", 20,
    ).await? {
        Some(w) => w,
        None => return Ok(false),
    };

    let cookie_store = state.client.lock().await.cookie_store.clone();
    cookie_bridge::extract_and_inject(
        app, "kg-course.kwansei.ac.jp", &cookie_store, config::KG_COURSE_BASE,
    ).await?;

    let http = state.client.lock().await.http.clone();
    let verify_url = format!("{}/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014", config::KG_COURSE_BASE);
    match crate::client::fetch_page_with(&http, &verify_url).await {
        Ok(html) => {
            let info = parser::parse_student_info(&html);
            // Empty student info means the page came back without real data
            // (server-side session stale despite cookie accepted)
            if info.student_id.is_empty() && info.name.is_empty() {
                log::warn!("headless_kgc_refresh: page returned empty student info (stale session)");
                let _ = win.close();
                return Ok(false);
            }
            let mut client = state.client.lock().await;
            client.session = Some(auth::AuthSession {
                username: info.student_id.clone(),
                display_name: if info.name.is_empty() { "\u{30e6}\u{30fc}\u{30b6}\u{30fc}".to_string() } else { info.name },
                student_id: info.student_id,
                faculty: info.faculty,
                department: info.department,
            });
            client.save_session();
            log::info!("headless_kgc_refresh: succeeded");
            let _ = win.close();
            Ok(true)
        }
        Err(e) => {
            let mut client = state.client.lock().await;
            client.clear_session();
            log::warn!("headless_kgc_refresh: session verification failed: {}", e);
            let _ = win.close();
            Err(e)
        }
    }
}

/// Attempt a silent (headless) Luna session refresh via an invisible WebView.
async fn headless_luna_refresh(
    app: &tauri::AppHandle,
    state: &LunaState,
) -> Result<bool, String> {
    let luna = state.client.lock().await;
    let cookie_store = luna.cookie_store.clone();
    let http = luna.http.clone();
    drop(luna);

    let verify_url = format!("{}/lms/timetable", config::LUNA_BASE);
    let ok = headless_saml_refresh(
        app, "luna-headless", config::LUNA_SAML_URL, "luna.kwansei.ac.jp",
        config::LUNA_BASE, &verify_url, &cookie_store, &http,
        luna_client::LUNA_SESSION_EXPIRED_MSG, luna_client::is_luna_session_expired,
    ).await?;
    if ok {
        let mut luna = state.client.lock().await;
        luna.authenticated = true;
        luna.save_session();
    }
    Ok(ok)
}

/// Attempt a silent (headless) KWIC Portal session refresh via an invisible WebView.
async fn headless_kwic_refresh(
    app: &tauri::AppHandle,
    state: &KwicState,
) -> Result<bool, String> {
    let kwic = state.client.lock().await;
    let cookie_store = kwic.cookie_store.clone();
    let http = kwic.http.clone();
    drop(kwic);

    let verify_url = format!("{}/portal/home", config::KWIC_BASE);
    let ok = headless_saml_refresh(
        app, "kwic-headless", config::KWIC_SAML_URL, "kwic.kwansei.ac.jp",
        config::KWIC_BASE, &verify_url, &cookie_store, &http,
        kwic_client::KWIC_SESSION_EXPIRED_MSG, kwic_client::is_kwic_session_expired,
    ).await?;
    if ok {
        let mut kwic = state.client.lock().await;
        kwic.authenticated = true;
        kwic.save_session();
    }
    Ok(ok)
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
    kgc_state: State<'_, KgcState>,
    luna_state: State<'_, LunaState>,
    kwic_state: State<'_, KwicState>,
    service: String,
) -> Result<bool, String> {
    log::info!("sync_session: service={}", service);
    match service.as_str() {
        "kgc" => headless_kgc_refresh(&app, kgc_state.inner()).await,
        "luna" => headless_luna_refresh(&app, luna_state.inner()).await,
        "kwic" => headless_kwic_refresh(&app, kwic_state.inner()).await,
        "all" => {
            // All three services share Okta SSO — refresh independently in parallel.
            // Any success proves Okta is alive; don't let one failure block others.
            let (kgc_res, luna_res, kwic_res) = tokio::join!(
                headless_kgc_refresh(&app, kgc_state.inner()),
                headless_luna_refresh(&app, luna_state.inner()),
                headless_kwic_refresh(&app, kwic_state.inner()),
            );
            let kgc_ok = kgc_res.unwrap_or(false);
            let luna_ok = luna_res.unwrap_or(false);
            let kwic_ok = kwic_res.unwrap_or(false);
            log::info!("sync_session(all): kgc={}, luna={}, kwic={}", kgc_ok, luna_ok, kwic_ok);
            // Return true if ANY service succeeded (Okta is alive, app is usable)
            Ok(kgc_ok || luna_ok || kwic_ok)
        }
        _ => Err(format!("Unknown service: {}", service)),
    }
}

// ───────── Weather (fetched server-side to avoid CSP issues) ─────────

#[derive(Debug, Serialize, Deserialize)]
pub struct WeatherData {
    pub temperature: i32,
    #[serde(rename = "weatherCode")]
    pub weather_code: i32,
    pub humidity: i32,
    #[serde(rename = "windSpeed")]
    pub wind_speed: i32,
    pub tomorrow: Option<WeatherTomorrow>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WeatherTomorrow {
    #[serde(rename = "tempMax")]
    pub temp_max: i32,
    #[serde(rename = "tempMin")]
    pub temp_min: i32,
    #[serde(rename = "weatherCode")]
    pub weather_code: i32,
}

#[tauri::command]
pub async fn fetch_weather() -> Result<WeatherData, String> {
    let url = "https://api.open-meteo.com/v1/forecast?latitude=34.7383&longitude=135.3416&current=temperature_2m,weather_code,relative_humidity_2m,wind_speed_10m&daily=weather_code,temperature_2m_max,temperature_2m_min&timezone=Asia%2FTokyo&forecast_days=2";
    let resp: serde_json::Value = reqwest::get(url)
        .await
        .map_err(|e| format!("天気API接続失敗: {}", e))?
        .json()
        .await
        .map_err(|e| format!("天気API解析失敗: {}", e))?;

    let current = &resp["current"];
    let daily = &resp["daily"];

    let tomorrow = if daily["time"].as_array().is_some_and(|a| a.len() >= 2) {
        Some(WeatherTomorrow {
            temp_max: daily["temperature_2m_max"][1].as_f64().unwrap_or(0.0).round() as i32,
            temp_min: daily["temperature_2m_min"][1].as_f64().unwrap_or(0.0).round() as i32,
            weather_code: daily["weather_code"][1].as_i64().unwrap_or(0) as i32,
        })
    } else {
        None
    };

    Ok(WeatherData {
        temperature: current["temperature_2m"].as_f64().unwrap_or(0.0).round() as i32,
        weather_code: current["weather_code"].as_i64().unwrap_or(0) as i32,
        humidity: current["relative_humidity_2m"].as_i64().unwrap_or(0) as i32,
        wind_speed: current["wind_speed_10m"].as_f64().unwrap_or(0.0).round() as i32,
        tomorrow,
    })
}

// ============ Download Config ============

#[tauri::command]
pub async fn open_downloads_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("downloads") {
        let _ = win.set_focus();
        return Ok(());
    }

    tauri::WebviewWindowBuilder::new(
        &app,
        "downloads",
        tauri::WebviewUrl::App("downloads.html".into()),
    )
    .title("ダウンロード")
    .inner_size(780.0, 520.0)
    .min_inner_size(560.0, 360.0)
    .resizable(true)
    .build()
    .map_err(|e| format!("Failed to open downloads window: {}", e))?;

    Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadConfig {
    pub download_dir: String,
    pub classify_by_course: bool,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            download_dir: String::new(),
            classify_by_course: true,
        }
    }
}

fn download_config_path() -> std::path::PathBuf {
    client::data_dir().join("download_config.json")
}

pub fn load_download_config() -> DownloadConfig {
    let path = download_config_path();
    if path.exists() {
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str(&data) {
                return cfg;
            }
        }
    }
    DownloadConfig::default()
}

fn save_download_config_to_disk(config: &DownloadConfig) -> Result<(), String> {
    let path = download_config_path();
    let data = serde_json::to_string_pretty(config)
        .map_err(|e| format!("JSON serialization error: {}", e))?;
    std::fs::write(&path, &data)
        .map_err(|e| format!("Failed to write download config: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn get_download_config() -> DownloadConfig {
    load_download_config()
}

#[tauri::command]
pub fn save_download_config(config: DownloadConfig) -> Result<(), String> {
    // Validate download_dir if set
    if !config.download_dir.is_empty() {
        let p = std::path::Path::new(&config.download_dir);
        if !p.is_absolute() {
            return Err("ダウンロードディレクトリは絶対パスで指定してください".into());
        }
        // Create if it doesn't exist
        std::fs::create_dir_all(p)
            .map_err(|e| format!("ディレクトリの作成に失敗しました: {}", e))?;
    }
    save_download_config_to_disk(&config)
}

#[tauri::command]
pub async fn select_download_dir() -> Result<String, String> {
    let result = rfd::AsyncFileDialog::new()
        .set_title("ダウンロードフォルダを選択")
        .pick_folder()
        .await;

    match result {
        Some(handle) => Ok(handle.path().to_string_lossy().to_string()),
        None => Err("cancelled".into()),
    }
}

// ─── Notification Config ───────────────────────────────────────

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationConfig {
    pub notify_important: bool,
    pub notify_faculty: bool,
    pub notify_class: bool,
    pub notify_other: bool,
    pub notify_mail: bool,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            notify_important: true,
            notify_faculty: true,
            notify_class: true,
            notify_other: true,
            notify_mail: true,
        }
    }
}

fn notification_config_path() -> std::path::PathBuf {
    client::data_dir().join("notification_config.json")
}

pub fn load_notification_config() -> NotificationConfig {
    let path = notification_config_path();
    if path.exists() {
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str(&data) {
                return cfg;
            }
        }
    }
    NotificationConfig::default()
}

fn save_notification_config_to_disk(config: &NotificationConfig) -> Result<(), String> {
    let path = notification_config_path();
    let data = serde_json::to_string_pretty(config)
        .map_err(|e| format!("JSON serialization error: {}", e))?;
    std::fs::write(&path, &data)
        .map_err(|e| format!("Failed to write notification config: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn get_notification_config() -> NotificationConfig {
    load_notification_config()
}

#[tauri::command]
pub fn save_notification_config(config: NotificationConfig) -> Result<(), String> {
    save_notification_config_to_disk(&config)
}

/// Sanitize a string to be safe as a directory/file name component.
fn sanitize_path_component(name: &str) -> String {
    let s: String = name.chars().map(|c| match c {
        '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
        _ => c,
    }).collect();
    let trimmed = s.trim().trim_matches('.');
    if trimmed.is_empty() { "_".into() } else { trimmed.to_string() }
}

/// Simplify a course name for use as a folder name.
/// Luna course names often look like:
///   "日本語教育センター 51001004 日本語I ４"
///   "国際学部_International Studies 34001001 キリスト教学Ａ　１"
/// We strip the leading department + numeric code prefix, bracket sections, and
/// trailing parenthesized scheduling info to get just the core course name.
fn simplify_course_name(name: &str) -> String {
    static RE_DEPT_CODE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        // Match leading "department text + 8-digit code + space" prefix
        // e.g. "日本語教育センター 51001004 " or "国際学部_International Studies 34001001 "
        regex::Regex::new(r"^.+\s\d{7,8}\s+").unwrap()
    });
    static RE_BRACKET: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"[\[［]\d+[\]］]").unwrap()
    });
    static RE_PAREN_SUFFIX: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"[（(][^)）]*(?:学期|限|クラス|組|セメスター|Quarter|Semester)[^)）]*[)）]\s*$").unwrap()
    });
    // Strip department + course code prefix first
    let s = RE_DEPT_CODE.replace(name, "");
    let s = RE_BRACKET.replace_all(&s, "");
    let s = RE_PAREN_SUFFIX.replace_all(&s, "");
    // Collapse whitespace
    let s: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = s.trim().to_string();
    if s.is_empty() { name.trim().to_string() } else { s }
}

/// Default download base directory: ~/Documents/Selah (created if needed).
pub fn default_download_dir() -> std::path::PathBuf {
    let doc = dirs::document_dir().unwrap_or_else(|| {
        dirs::home_dir().map(|h| h.join("Documents")).unwrap_or_else(std::env::temp_dir)
    });
    let dir = doc.join("Selah");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Resolve the download directory with optional course classification.
/// Returns the target directory (created if needed) for saving a file.
pub fn resolve_download_dir(course_name: Option<&str>) -> std::path::PathBuf {
    let config = load_download_config();
    let base = if config.download_dir.is_empty() {
        default_download_dir()
    } else {
        std::path::PathBuf::from(&config.download_dir)
    };

    if config.classify_by_course {
        if let Some(course) = course_name {
            let simplified = simplify_course_name(course);
            let safe_course = sanitize_path_component(&simplified);
            let dir = base.join(&safe_course);
            let _ = std::fs::create_dir_all(&dir);
            return dir;
        }
    }

    base
}

// ───────── Download History ─────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRecord {
    pub id: String,
    pub filename: String,
    pub path: String,
    pub course_name: String,
    pub source: String,   // "luna", "mail", etc.
    pub size_bytes: u64,
    pub downloaded_at: i64,  // unix millis
    #[serde(default)]
    pub file_exists: bool,   // populated at query time, not persisted
}

fn download_history_path() -> std::path::PathBuf {
    client::data_dir().join("download_history.json")
}

pub fn load_download_history() -> Vec<DownloadRecord> {
    let path = download_history_path();
    if path.exists() {
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(records) = serde_json::from_str(&data) {
                return records;
            }
        }
    }
    Vec::new()
}

fn save_download_history(records: &[DownloadRecord]) -> Result<(), String> {
    let path = download_history_path();
    let data = serde_json::to_string(records)
        .map_err(|e| format!("JSON serialization error: {}", e))?;
    std::fs::write(&path, &data)
        .map_err(|e| format!("Failed to write download history: {}", e))?;
    Ok(())
}

/// Record a new download in the history. Called from save_to_downloads.
pub fn record_download(filename: &str, path: &str, course_name: Option<&str>, source: &str, size_bytes: u64) {
    let mut records = load_download_history();
    let record = DownloadRecord {
        id: format!("{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()),
        filename: filename.to_string(),
        path: path.to_string(),
        course_name: course_name.unwrap_or("").to_string(),
        source: source.to_string(),
        size_bytes,
        downloaded_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64,
        file_exists: true,
    };
    records.push(record);
    // Keep at most 500 records
    if records.len() > 500 {
        records.drain(0..records.len() - 500);
    }
    let _ = save_download_history(&records);
}

#[tauri::command]
pub fn list_downloads() -> Vec<DownloadRecord> {
    let mut records = load_download_history();
    records.retain(|r| !r.path.is_empty());
    // Check file existence on disk
    for r in &mut records {
        r.file_exists = std::path::Path::new(&r.path).exists();
    }
    records.reverse(); // newest first
    records
}

/// Scan the download directories for files not in the history and add them.
/// Returns the updated full list.
#[tauri::command]
pub fn scan_download_dir() -> Vec<DownloadRecord> {
    let config = load_download_config();
    let base = if config.download_dir.is_empty() {
        dirs::download_dir().unwrap_or_else(std::env::temp_dir)
    } else {
        std::path::PathBuf::from(&config.download_dir)
    };

    let mut records = load_download_history();
    // Build set of known paths for O(1) lookup
    let known_paths: std::collections::HashSet<String> = records.iter()
        .map(|r| r.path.clone())
        .collect();

    // Walk the base directory (max 2 levels deep for course subfolders)
    let mut discovered: Vec<DownloadRecord> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(rec) = try_discover_file(&path, "", &known_paths) {
                    discovered.push(rec);
                }
            } else if path.is_dir() {
                // Course subfolder
                let folder_name = path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                if let Ok(sub_entries) = std::fs::read_dir(&path) {
                    for sub in sub_entries.flatten() {
                        let sub_path = sub.path();
                        if sub_path.is_file() {
                            if let Some(rec) = try_discover_file(&sub_path, &folder_name, &known_paths) {
                                discovered.push(rec);
                            }
                        }
                    }
                }
            }
        }
    }

    if !discovered.is_empty() {
        records.extend(discovered);
        if records.len() > 500 {
            records.drain(0..records.len() - 500);
        }
        let _ = save_download_history(&records);
    }

    // Return with file_exists populated
    records.retain(|r| !r.path.is_empty());
    for r in &mut records {
        r.file_exists = std::path::Path::new(&r.path).exists();
    }
    records.reverse();
    records
}

fn try_discover_file(
    path: &std::path::Path,
    course_folder: &str,
    known: &std::collections::HashSet<String>,
) -> Option<DownloadRecord> {
    let path_str = path.to_string_lossy().to_string();
    if known.contains(&path_str) {
        return None;
    }
    // Skip hidden files and system files
    let filename = path.file_name()?.to_str()?;
    if filename.starts_with('.') || filename == "desktop.ini" || filename == "Thumbs.db" {
        return None;
    }
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?
        .duration_since(std::time::UNIX_EPOCH).ok()?
        .as_millis() as i64;

    Some(DownloadRecord {
        id: format!("scan_{}", modified),
        filename: filename.to_string(),
        path: path_str,
        course_name: course_folder.to_string(),
        source: "scan".to_string(),
        size_bytes: metadata.len(),
        downloaded_at: modified,
        file_exists: true,
    })
}

#[tauri::command]
pub fn check_file_downloaded(filename: String, course_name: Option<String>) -> Option<DownloadRecord> {
    let records = load_download_history();
    let target = filename.to_lowercase();
    let mut found: Option<DownloadRecord> = None;
    for r in records.iter().rev() {
        let rname = r.filename.to_lowercase();
        if rname == target {
            if let Some(ref cn) = course_name {
                if !cn.is_empty() && !r.course_name.is_empty() && r.course_name != *cn {
                    continue;
                }
            }
            let mut rec = r.clone();
            rec.file_exists = std::path::Path::new(&rec.path).exists();
            if rec.file_exists {
                return Some(rec);
            }
            if found.is_none() {
                found = Some(rec);
            }
        }
    }
    found
}

#[tauri::command]
pub fn open_downloaded_file(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    if !p.exists() {
        return Err("ファイルが見つかりません".into());
    }
    // Security: restrict to Downloads or configured download directory
    let canonical = p.canonicalize().map_err(|e| format!("パスが無効です: {}", e))?;
    let sys_downloads = dirs::download_dir().unwrap_or_else(|| {
        dirs::home_dir().map(|h| h.join("Downloads")).unwrap_or_else(std::env::temp_dir)
    });
    let dl_config = load_download_config();
    let custom_dir = if dl_config.download_dir.is_empty() { None } else {
        std::path::Path::new(&dl_config.download_dir).canonicalize().ok()
    };
    let allowed = canonical.starts_with(&sys_downloads)
        || custom_dir.as_ref().is_some_and(|d| canonical.starts_with(d));
    if !allowed {
        return Err("ダウンロードフォルダ外のファイルは開けません".into());
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_path(canonical.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("ファイルを開けませんでした: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn remove_download_record(id: String) -> Result<(), String> {
    let mut records = load_download_history();
    records.retain(|r| r.id != id);
    save_download_history(&records)
}

#[tauri::command]
pub fn clear_download_history() -> Result<(), String> {
    save_download_history(&[])
}
