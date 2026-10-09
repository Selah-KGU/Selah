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
    let requested_generation = crate::session_coordinator::SESSIONS.generation();
    let sync_guard = super::session::lock_session_sync().await;
    crate::session_coordinator::SESSIONS.ensure_current(requested_generation)?;
    let generation = crate::session_coordinator::begin_interactive(&app);
    if let Err(error) = cookie_bridge::restore_sso_cookies(&app).await {
        log::warn!("Interactive login will use the native cookie store: {error}");
    }
    let kgc_entry = format!("{}/uniasv2/UnSSOLoginControl2", config::KG_COURSE_BASE);
    log::info!("Cookie Bridge: opening login webview to {}", &kgc_entry);

    if let Some(existing) = app.get_webview_window("login") {
        let _ = existing.close();
    }

    let (tx, rx) = tokio::sync::mpsc::channel::<String>(4);

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
        let window = cookie_bridge::AuthWindow(_login_window);
        let completed = tokio::select! {
            biased;
            _ = crate::session_coordinator::SESSIONS.cancelled(generation) => {
                let _ = app_clone.emit("login-cancelled", "ログインがキャンセルされました");
                false
            },
            result = run_visible_login(app_clone.clone(), rx, current_sp_host, generation) => result,
        };
        drop(window);
        drop(sync_guard);
        if !completed {
            return;
        }
        // Post-login work can itself request recovery, so release the gate first.
        if let Err(e) = crate::notifier::notification_sync_now(app_clone.clone()).await {
            log::warn!("notification sync after login failed: {e}");
        }
        if let Err(e) = crate::background_refresh::refresh_backend_data_now(&app_clone).await {
            log::warn!("background refresh after login failed: {e}");
        }
    });

    Ok(())
}

async fn run_visible_login(
    app_clone: tauri::AppHandle,
    mut rx: tokio::sync::mpsc::Receiver<String>,
    current_sp_host: Arc<std::sync::Mutex<String>>,
    generation: u64,
) -> bool {
    match tokio::time::timeout(std::time::Duration::from_secs(120), rx.recv()).await {
        Ok(Some(_host)) => {
            log::info!("Cookie Bridge: Phase 1 - KG-Course SAML complete, extracting cookies...");
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;

            let kgc_state = app_clone.state::<KgcState>();
            let _kgc_gate = kgc_state.gate.lock().await;
            let (cookie_store, http) = client::new_cookie_client();
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
                return false;
            }

            let verify_url = format!(
                "{}/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014",
                config::KG_COURSE_BASE
            );
            match client::fetch_page_with(&http, &verify_url).await {
                Ok(html) => {
                    let info = parser::parse_student_info(&html);
                    if info.student_id.is_empty() && info.name.is_empty() {
                        let _ = app_clone.emit("login-error", "学生情報を確認できませんでした");
                        return false;
                    }
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
                    if crate::session_coordinator::SESSIONS
                        .accept_verified(
                            generation,
                            crate::session_coordinator::Service::Kgc,
                            client::CookieClientParts { http, cookie_store },
                            Some(session.clone()),
                        )
                        .is_err()
                    {
                        return false;
                    }
                    if crate::session_coordinator::SESSIONS
                        .mark_login_pending(generation)
                        .is_err()
                    {
                        return false;
                    }
                    if let Err(error) =
                        crate::session_coordinator::confirm_interactive(generation).await
                    {
                        if crate::session_coordinator::SESSIONS
                            .ensure_current(generation)
                            .is_err()
                        {
                            return false;
                        }
                        log::warn!("University login verified; persistence pending: {error}");
                    }
                    let mut identity =
                        serde_json::to_value(&session).expect("serializable identity");
                    identity["generation"] = serde_json::json!(generation);
                    identity["persistence_pending"] = serde_json::json!(
                        crate::session_coordinator::SESSIONS
                            .snapshot()
                            .login_persistence_pending
                    );
                    let _ = app_clone.emit("login-success", identity);
                }
                Err(e) => {
                    log::warn!("Cookie Bridge: KGC session verification failed: {}", e);
                    let _ = app_clone.emit("login-error", &e);
                    if let Some(win) = app_clone.get_webview_window("login") {
                        let _ = win.close();
                    }
                    return false;
                }
            }

            log::info!("Cookie Bridge: KG-Course login successful, proceeding to Luna");
        }
        Ok(None) => {
            log::info!("Login window closed without completing login");
            let _ = app_clone.emit("login-cancelled", "ログインがキャンセルされました");
            return false;
        }
        Err(_) => {
            log::warn!("Login timed out (120s)");
            let _ = app_clone.emit("login-error", "Login timed out");
            if let Some(win) = app_clone.get_webview_window("login") {
                let _ = win.close();
            }
            return false;
        }
    }

    log::info!("=== Cookie Bridge Phase 2: Luna SAML ===");
    let mut luna_authenticated = false;
    let mut kwic_authenticated = false;

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
                let (cookie_store, http) = client::new_cookie_client();
                let result = cookie_bridge::extract_and_inject(
                    &app_clone,
                    "luna.kwansei.ac.jp",
                    &cookie_store,
                    config::LUNA_BASE,
                )
                .await;
                match result {
                    Ok(()) => {
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
                                if crate::session_coordinator::SESSIONS
                                    .accept_verified(
                                        generation,
                                        crate::session_coordinator::Service::Luna,
                                        client::CookieClientParts { http, cookie_store },
                                        None,
                                    )
                                    .is_err()
                                {
                                    return false;
                                }
                                luna_authenticated = true;
                                log::info!("Cookie Bridge: Luna login successful (verified)");
                                let _ = app_clone.emit(
                                    "luna-login-success",
                                    serde_json::json!({"generation": generation}),
                                );
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
                let (cookie_store, http) = client::new_cookie_client();
                let result = cookie_bridge::extract_and_inject(
                    &app_clone,
                    "kwic.kwansei.ac.jp",
                    &cookie_store,
                    config::KWIC_BASE,
                )
                .await;
                match result {
                    Ok(()) => {
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
                                if crate::session_coordinator::SESSIONS
                                    .accept_verified(
                                        generation,
                                        crate::session_coordinator::Service::Kwic,
                                        client::CookieClientParts { http, cookie_store },
                                        None,
                                    )
                                    .is_err()
                                {
                                    return false;
                                }
                                kwic_authenticated = true;
                                log::info!(
                                    "Cookie Bridge: KWIC Portal login successful (verified)"
                                );
                                let _ = app_clone.emit(
                                    "kwic-login-success",
                                    serde_json::json!({"generation": generation}),
                                );
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

    // All SAML phases done: back up the fresh SSO cookies (incl. device
    // token) before the login window — and its webview store — go away.
    cookie_bridge::persist_sso_cookies(&app_clone).await;
    for (service, valid) in [
        (
            crate::session_coordinator::Service::Luna,
            luna_authenticated,
        ),
        (
            crate::session_coordinator::Service::Kwic,
            kwic_authenticated,
        ),
    ] {
        if !valid {
            crate::session_coordinator::SESSIONS.record(
                generation,
                service,
                crate::session_coordinator::Health::Unavailable,
            );
        }
    }
    let _ = app_clone.emit(
        "university-login-complete",
        serde_json::json!({
            "generation": generation,
            "luna_authenticated": luna_authenticated,
            "kwic_authenticated": kwic_authenticated,
        }),
    );

    if let Some(win) = app_clone.get_webview_window("login") {
        let _ = win.close();
    }

    true
}

#[tauri::command]
pub async fn logout(
    app: tauri::AppHandle,
    state: State<'_, KgcState>,
    luna_state: State<'_, LunaState>,
    kwic_state: State<'_, KwicState>,
) -> Result<(), String> {
    reset_university_login(app.clone(), state, luna_state, kwic_state).await?;
    let _ = app.emit("logout", ());
    Ok(())
}

/// Clear only university authentication state, including native webview SSO
/// cookies, so the next visible login starts from a genuinely signed-out state.
#[tauri::command]
pub async fn reset_university_login(
    app: tauri::AppHandle,
    _state: State<'_, KgcState>,
    _luna_state: State<'_, LunaState>,
    _kwic_state: State<'_, KwicState>,
) -> Result<usize, String> {
    crate::session_coordinator::sign_out(&app)?;
    // Invalidate in-flight work before waiting for its authentication gate.
    let _sync_gate = super::session::lock_session_sync().await;

    for label in ["login", "kgc-headless", "luna-headless", "kwic-headless"] {
        if let Some(window) = app.get_webview_window(label) {
            let _ = window.close();
        }
    }

    crate::session_coordinator::SESSIONS.clear_all();

    cookie_bridge::clear_university_cookies(&app).await
}

#[tauri::command]
pub async fn delete_all_local_data(
    app: tauri::AppHandle,
    _state: State<'_, KgcState>,
    _luna_state: State<'_, LunaState>,
    _kwic_state: State<'_, KwicState>,
) -> Result<(), String> {
    crate::session_coordinator::sign_out(&app)?;
    let _sync_gate = super::session::lock_session_sync().await;
    crate::session_coordinator::SESSIONS.clear_all();

    // Clear native cookies before erasing the vault; never write secrets again
    // in this process after erase, including delayed OAuth results or exit hooks.
    let native_cleanup = cookie_bridge::clear_native_university_cookies(&app).await;
    // Record the purge before erasure so legacy OAuth files cannot repopulate
    // an empty vault after an interruption between these steps.
    let purge_receipt = if native_cleanup.is_ok() {
        crate::data_reset::schedule()
    } else {
        Ok(())
    };
    app.state::<crate::MailState>().cancellation.cancel();
    app.state::<crate::GCalState>().cancellation.cancel();
    let vault_cleanup = crate::keychain::erase_all();
    app.state::<crate::MailState>().client.lock().await.retire();
    app.state::<crate::GCalState>().client.lock().await.retire();
    native_cleanup?;
    purge_receipt?;
    vault_cleanup?;

    let _ = app.emit("logout", ());

    Ok(())
}

impl SessionStatus {
    pub(crate) fn from_lease(lease: &crate::session_coordinator::SessionLease) -> Self {
        let identity = lease.identity();
        Self {
            valid: lease.is_verified(),
            username: identity.map(|s| s.username.clone()).unwrap_or_default(),
            display_name: identity.map(|s| s.display_name.clone()).unwrap_or_default(),
            student_id: identity.map(|s| s.student_id.clone()).unwrap_or_default(),
            faculty: identity.map(|s| s.faculty.clone()).unwrap_or_default(),
            department: identity.map(|s| s.department.clone()).unwrap_or_default(),
        }
    }
}

#[tauri::command]
pub async fn get_kgc_session_snapshot(state: State<'_, KgcState>) -> Result<SessionStatus, String> {
    Ok(SessionStatus::from_lease(&state.session()))
}

#[tauri::command]
pub(crate) async fn check_session(
    state: State<'_, KgcState>,
) -> Result<SessionStatus, crate::session_coordinator::SessionError> {
    let _kgc_gate = state.gate.lock().await;
    let lease =
        crate::session_coordinator::verify(crate::session_coordinator::Service::Kgc).await?;
    Ok(SessionStatus::from_lease(&lease))
}
