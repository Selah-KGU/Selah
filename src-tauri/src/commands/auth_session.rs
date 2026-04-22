use crate::auth;
use crate::client;
use crate::config;
use crate::cookie_bridge;
use crate::kwic_client;
use crate::luna_client;
use crate::parser;
use crate::{KgcState, KwicState, LunaState};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use tauri::{Emitter, Manager};

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionStatus {
    pub valid: bool,
    pub username: String,
    pub display_name: String,
    pub student_id: String,
    pub faculty: String,
    pub department: String,
}

#[tauri::command]
pub async fn open_login_window(app: tauri::AppHandle) -> Result<(), String> {
    let kgc_entry = format!("{}/uniasv2/UnSSOLoginControl2", config::KG_COURSE_BASE);
    log::info!("Cookie Bridge: opening login webview to {}", &kgc_entry);

    if let Some(existing) = app.get_webview_window("login") {
        let _ = existing.close();
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(4);

    let parsed_url: url::Url = kgc_entry
        .parse()
        .map_err(|e| format!("URL parse error: {}", e))?;

    let current_sp_host = Arc::new(std::sync::Mutex::new("kg-course.kwansei.ac.jp".to_string()));
    let sp_host_for_load = current_sp_host.clone();

    let _login_window =
        tauri::WebviewWindowBuilder::new(&app, "login", tauri::WebviewUrl::External(parsed_url))
            .title("関西学院 - サインイン")
            .inner_size(480.0, 700.0)
            .resizable(true)
            .on_navigation(|_| true)
            .on_page_load(move |_win, payload| {
                use tauri::webview::PageLoadEvent;
                if !matches!(payload.event(), PageLoadEvent::Finished) {
                    return;
                }
                let url = payload.url();
                let expected_host = sp_host_for_load
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
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
            .map_err(|e| format!("ログインウィンドウ作成失敗: {}", e))?;

    let app_clone = app.clone();
    tokio::spawn(async move {
        match tokio::time::timeout(std::time::Duration::from_secs(120), rx.recv()).await {
            Ok(Some(_host)) => {
                log::info!(
                    "Cookie Bridge: Phase 1 - KG-Course SAML complete, extracting cookies..."
                );
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;

                let kgc_state = app_clone.state::<KgcState>();
                let cookie_store = kgc_state.client.lock().await.cookie_store.clone();
                let inject_result = cookie_bridge::extract_and_inject(
                    &app_clone,
                    "kg-course.kwansei.ac.jp",
                    &cookie_store,
                    config::KG_COURSE_BASE,
                )
                .await;
                if let Err(e) = inject_result {
                    log::warn!("Cookie Bridge: cookie extraction failed: {}", e);
                    let _ = app_clone.emit("login-error", &e);
                    if let Some(win) = app_clone.get_webview_window("login") {
                        let _ = win.close();
                    }
                    return;
                }

                let http = kgc_state.client.lock().await.http.clone();
                let verify_url = format!(
                    "{}/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014",
                    config::KG_COURSE_BASE
                );
                match client::fetch_page_with(&http, &verify_url).await {
                    Ok(html) => {
                        let info = parser::parse_student_info(&html);
                        log::info!(
                            "Cookie Bridge: student info: id={}, name={}",
                            info.student_id,
                            info.name
                        );
                        let session = auth::AuthSession {
                            username: info.student_id.clone(),
                            display_name: if info.name.is_empty() {
                                "ユーザー".to_string()
                            } else {
                                info.name
                            },
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
                let _ = app_clone.emit("login-cancelled", "ログインがキャンセルされました");
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

        log::info!("=== Cookie Bridge Phase 2: Luna SAML ===");

        if let Some(win) = app_clone.get_webview_window("login") {
            {
                let mut host = current_sp_host.lock().unwrap_or_else(|e| e.into_inner());
                *host = "luna.kwansei.ac.jp".to_string();
            }
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
                    )
                    .await;
                    match result {
                        Ok(()) => {
                            let http = luna_state.client.lock().await.http.clone();
                            let verify_url = format!("{}/lms/timetable", config::LUNA_BASE);
                            match client::fetch_with_redirect(
                                &http,
                                &verify_url,
                                config::LUNA_BASE,
                                luna_client::LUNA_SESSION_EXPIRED_MSG,
                                luna_client::is_luna_session_expired,
                            )
                            .await
                            {
                                Ok(_) => {
                                    let mut luna = luna_state.client.lock().await;
                                    luna.authenticated = true;
                                    luna.save_session();
                                    drop(luna);
                                    log::info!("Cookie Bridge: Luna login successful (verified)");
                                    let _ = app_clone.emit("luna-login-success", ());
                                }
                                Err(e) => {
                                    log::warn!(
                                        "Cookie Bridge: Luna session verification failed: {}",
                                        e
                                    );
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

        log::info!("=== Cookie Bridge Phase 3: KWIC Portal SAML ===");

        if let Some(win) = app_clone.get_webview_window("login") {
            {
                let mut host = current_sp_host.lock().unwrap_or_else(|e| e.into_inner());
                *host = "kwic.kwansei.ac.jp".to_string();
            }
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
                    )
                    .await;
                    match result {
                        Ok(()) => {
                            let http = kwic_state.client.lock().await.http.clone();
                            let verify_url = format!("{}/portal/home", config::KWIC_BASE);
                            match client::fetch_with_redirect(
                                &http,
                                &verify_url,
                                config::KWIC_BASE,
                                kwic_client::KWIC_SESSION_EXPIRED_MSG,
                                kwic_client::is_kwic_session_expired,
                            )
                            .await
                            {
                                Ok(_) => {
                                    let mut kwic = kwic_state.client.lock().await;
                                    kwic.authenticated = true;
                                    kwic.save_session();
                                    drop(kwic);
                                    log::info!(
                                        "Cookie Bridge: KWIC Portal login successful (verified)"
                                    );
                                    let _ = app_clone.emit("kwic-login-success", ());
                                }
                                Err(e) => {
                                    log::warn!(
                                        "Cookie Bridge: KWIC session verification failed: {}",
                                        e
                                    );
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

        if let Err(e) = crate::notifier::notification_sync_now(app_clone.clone()).await {
            log::warn!("notification sync after login failed: {}", e);
        }
        if let Err(e) = crate::background_refresh::refresh_backend_data_now(&app_clone).await {
            log::warn!("background refresh after login failed: {}", e);
        }

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
pub async fn delete_all_local_data(
    app: tauri::AppHandle,
    state: State<'_, KgcState>,
    luna_state: State<'_, LunaState>,
    kwic_state: State<'_, KwicState>,
) -> Result<(), String> {
    {
        let mut c = state.client.lock().await;
        c.clear_session();
    }
    {
        let mut l = luna_state.client.lock().await;
        l.clear();
    }
    {
        let mut k = kwic_state.client.lock().await;
        k.clear();
    }

    let keychain_keys = [
        "ai_api_key",
        "gcal_token",
        "gcal_client_secret",
        "ms_mail_token",
    ];
    for key in &keychain_keys {
        crate::keychain::delete_secret(key);
    }

    let dir = client::data_dir();
    if dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
    }

    let _ = app.emit("logout", ());

    Ok(())
}

#[tauri::command]
pub async fn check_session(state: State<'_, KgcState>) -> Result<SessionStatus, String> {
    log::info!("check_session: called");
    let _kgc_gate = state.gate.lock().await;
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
            log::info!(
                "check_session: validating user={} sid={}",
                s.username,
                s.student_id
            );
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

    let verify_url = format!(
        "{}/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014",
        config::KG_COURSE_BASE
    );
    log::info!("check_session: fetching verify page...");
    match client::fetch_page_with(&http, &verify_url).await {
        Ok(html) => {
            let info = parser::parse_student_info(&html);
            if info.student_id.is_empty() && info.name.is_empty() {
                log::warn!(
                    "check_session: server returned empty page (stale session), disk user={} sid={}",
                    session.username,
                    session.student_id
                );
                let mut client = state.client.lock().await;
                client.clear_session();
                return Ok(SessionStatus {
                    valid: false,
                    username: session.username,
                    display_name: session.display_name,
                    student_id: session.student_id,
                    faculty: session.faculty,
                    department: session.department,
                });
            }

            let needs_update = session.student_id.is_empty() || session.display_name == "ユーザー";
            let mut client = state.client.lock().await;
            if needs_update {
                log::info!(
                    "Reparsed student info: id={}, name={}, faculty={}, dept={}",
                    info.student_id,
                    info.name,
                    info.faculty,
                    info.department
                );
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
                log::info!(
                    "check_session: session expired (server confirmed), disk user={} sid={}",
                    session.username,
                    session.student_id
                );
                let mut client = state.client.lock().await;
                client.clear_session();
            } else {
                log::warn!(
                    "check_session: validation failed (transient?): {} -- keeping disk state, user={}",
                    e,
                    session.username
                );
            }
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

#[tauri::command]
pub async fn validate_session(state: State<'_, KgcState>) -> Result<SessionStatus, String> {
    let (http, is_auth, session_snapshot) = {
        let client = state.client.lock().await;
        (
            client.http.clone(),
            client.is_authenticated(),
            client.session.clone(),
        )
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

    let verify_url = format!(
        "{}/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014",
        config::KG_COURSE_BASE
    );
    match client::fetch_page_with(&http, &verify_url).await {
        Ok(html) => {
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
            let client = state.client.lock().await;
            client.save_session();
            let session = session_snapshot
                .as_ref()
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
