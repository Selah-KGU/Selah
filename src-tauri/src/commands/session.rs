use crate::auth;
use crate::client;
use crate::config;
use crate::cookie_bridge;
use crate::kwic_client;
use crate::luna_client;
use crate::parser;
use crate::session_coordinator::{
    Health, RecoveryOutcome, RecoveryReport, RecoveryTrigger, Service, ServiceRecovery,
    SessionError, SESSIONS,
};
use crate::{KgcState, KwicState, LunaState};
use serde::Serialize;
use tauri::{Manager, State};

pub(super) async fn lock_session_sync() -> tokio::sync::OwnedMutexGuard<()> {
    SESSIONS.lock().await
}

pub(crate) async fn ensure_kgc_session_for_automatic_request(app: &tauri::AppHandle) -> bool {
    match recover_sessions(app, &[Service::Kgc], RecoveryTrigger::AutomaticRequest).await {
        Ok(report) => {
            report.verified(Service::Kgc) || SESSIONS.lease(Service::Kgc).has_credentials()
        }
        Err(error) => {
            log::warn!("KGC automatic session check: {error}");
            false
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SessionStates {
    pub sso: bool,
    pub kgc: bool,
    pub luna: bool,
    pub kwic: bool,
}

#[tauri::command]
pub async fn get_session_states(
    app: tauri::AppHandle,
    _state: State<'_, KgcState>,
    _luna_state: State<'_, LunaState>,
    _kwic_state: State<'_, KwicState>,
) -> Result<SessionStates, String> {
    let snapshot = SESSIONS.snapshot();
    let kgc = snapshot.services[Service::Kgc.index()].credentials_present;
    let luna = snapshot.services[Service::Luna.index()].credentials_present;
    let kwic = snapshot.services[Service::Kwic.index()].credentials_present;
    let sso = !crate::session_coordinator::SESSIONS.signed_out()
        && cookie_bridge::has_sso_evidence(&app).await?;
    SESSIONS.ensure_current(snapshot.generation)?;
    Ok(SessionStates {
        kgc,
        luna,
        kwic,
        sso,
    })
}

#[tauri::command]
pub fn get_saved_cookie_summaries(
) -> Result<Vec<client::SavedCookieSummary>, crate::keychain::StoreError> {
    [
        ("kgc", client::KGC_COOKIES_KEY),
        ("luna", luna_client::LUNA_COOKIES_KEY),
        ("kwic", kwic_client::KWIC_COOKIES_KEY),
    ]
    .into_iter()
    .map(|(service, key)| client::saved_cookie_summary(service, key))
    .collect()
}

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
    is_session_expired: fn(&str) -> bool,
) -> Result<bool, SessionError> {
    log::info!("headless_{}: starting (Cookie Bridge)", label);

    let win = match cookie_bridge::headless_saml_window(app, label, saml_url, sp_domain, 20)
        .await
        .map_err(SessionError::Unavailable)?
    {
        Some(w) => w,
        None => return Ok(false),
    };

    cookie_bridge::extract_and_inject(app, sp_domain, cookie_store, base_url)
        .await
        .map_err(SessionError::Unavailable)?;

    let result = client::fetch_session_page(http, verify_url, base_url, is_session_expired).await;
    let _ = win.close();

    match result {
        Ok(_) => {
            log::info!("headless_{}: succeeded (verified)", label);
            Ok(true)
        }
        Err(e) => {
            log::warn!(
                "headless_{}: cookie injection succeeded but session invalid: {}",
                label,
                e
            );
            Err(e)
        }
    }
}

async fn headless_kgc_refresh(
    app: &tauri::AppHandle,
    state: &KgcState,
    generation: u64,
) -> Result<bool, SessionError> {
    log::info!("headless_kgc_refresh: starting (Cookie Bridge)");
    let _kgc_gate = state.gate.lock().await;

    let entry_url = format!("{}/uniasv2/UnSSOLoginControl2", config::KG_COURSE_BASE);
    let win = match cookie_bridge::headless_saml_window(
        app,
        "kgc-headless",
        &entry_url,
        "kg-course.kwansei.ac.jp",
        20,
    )
    .await
    .map_err(SessionError::Unavailable)?
    {
        Some(w) => w,
        None => return Ok(false),
    };

    let (cookie_store, http) = client::new_cookie_client();
    cookie_bridge::extract_and_inject(
        app,
        "kg-course.kwansei.ac.jp",
        &cookie_store,
        config::KG_COURSE_BASE,
    )
    .await
    .map_err(SessionError::Unavailable)?;

    let verify_url = format!(
        "{}/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014",
        config::KG_COURSE_BASE
    );
    match crate::client::fetch_session_page(
        &http,
        &verify_url,
        config::KG_COURSE_BASE,
        client::is_session_expired_body,
    )
    .await
    {
        Ok(html) => {
            let info = parser::parse_student_info(&html);
            if info.student_id.is_empty() && info.name.is_empty() {
                log::warn!("headless_kgc_refresh: unrecognized verification page");
                let _ = win.close();
                return Err(SessionError::Unavailable(
                    "KGC returned an unrecognized verification page".into(),
                ));
            }
            let identity = auth::AuthSession {
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
            crate::session_coordinator::SESSIONS
                .accept_verified(
                    generation,
                    crate::session_coordinator::Service::Kgc,
                    client::CookieClientParts { http, cookie_store },
                    Some(identity),
                )
                .map_err(|_| SessionError::Cancelled)?;
            log::info!("headless_kgc_refresh: succeeded");
            let _ = win.close();
            Ok(true)
        }
        Err(e) => {
            log::warn!(
                "headless_kgc_refresh: candidate session verification failed: {}",
                e
            );
            let _ = win.close();
            Err(e)
        }
    }
}

async fn headless_luna_refresh(
    app: &tauri::AppHandle,
    _state: &LunaState,
    generation: u64,
) -> Result<bool, SessionError> {
    let (cookie_store, http) = client::new_cookie_client();

    let verify_url = format!("{}/lms/timetable", config::LUNA_BASE);
    let ok = headless_saml_refresh(
        app,
        "luna-headless",
        config::LUNA_SAML_URL,
        "luna.kwansei.ac.jp",
        config::LUNA_BASE,
        &verify_url,
        &cookie_store,
        &http,
        luna_client::is_luna_session_expired,
    )
    .await?;
    if ok {
        crate::session_coordinator::SESSIONS
            .accept_verified(
                generation,
                crate::session_coordinator::Service::Luna,
                client::CookieClientParts { http, cookie_store },
                None,
            )
            .map_err(|_| SessionError::Cancelled)?;
    }
    Ok(ok)
}

async fn headless_kwic_refresh(
    app: &tauri::AppHandle,
    _state: &KwicState,
    generation: u64,
) -> Result<bool, SessionError> {
    let (cookie_store, http) = client::new_cookie_client();

    let verify_url = format!("{}/portal/home", config::KWIC_BASE);
    let ok = headless_saml_refresh(
        app,
        "kwic-headless",
        config::KWIC_SAML_URL,
        "kwic.kwansei.ac.jp",
        config::KWIC_BASE,
        &verify_url,
        &cookie_store,
        &http,
        kwic_client::is_kwic_session_expired,
    )
    .await?;
    if ok {
        crate::session_coordinator::SESSIONS
            .accept_verified(
                generation,
                crate::session_coordinator::Service::Kwic,
                client::CookieClientParts { http, cookie_store },
                None,
            )
            .map_err(|_| SessionError::Cancelled)?;
    }
    Ok(ok)
}

#[tauri::command]
pub(crate) async fn sync_session(
    app: tauri::AppHandle,
    service: String,
    trigger: Option<RecoveryTrigger>,
) -> Result<RecoveryReport, SessionError> {
    let services = if service == "all" {
        vec![Service::Luna, Service::Kwic]
    } else {
        vec![Service::parse(&service)
            .ok_or_else(|| SessionError::InvalidService(service.clone()))?]
    };
    recover_sessions(&app, &services, trigger.unwrap_or(RecoveryTrigger::Manual)).await
}

/// All automatic/manual callers submit a trigger. This application boundary
/// owns validation, evidence gathering, candidate flows, and partial results.
pub(crate) async fn recover_sessions(
    app: &tauri::AppHandle,
    services: &[Service],
    trigger: RecoveryTrigger,
) -> Result<RecoveryReport, SessionError> {
    let generation = SESSIONS.generation();
    let requested = std::time::Instant::now();
    let _sync_gate = SESSIONS.lock().await;
    SESSIONS
        .ensure_current(generation)
        .map_err(|_| SessionError::Cancelled)?;
    if SESSIONS.signed_out() {
        return Ok(RecoveryReport {
            results: services
                .iter()
                .map(|service| ServiceRecovery::new(*service, RecoveryOutcome::SignedOut))
                .collect(),
            snapshot: SESSIONS.snapshot(),
            identity: None,
        });
    }
    let work = async {
        let mut results = Vec::new();
        let mut evidence = None;
        for &target in services {
            if target == Service::Kgc && !trigger.may_recover_kgc() {
                continue;
            }
            if let Some(result) = SESSIONS.completed_since(generation, target, requested) {
                results.push(result);
                continue;
            }
            let before = SESSIONS.lease(target);
            if trigger.probes() && before.has_credentials() {
                if !SESSIONS.probe_due(target, trigger) {
                    results.push(ServiceRecovery::new(
                        target,
                        if before.is_verified() {
                            RecoveryOutcome::Verified
                        } else {
                            RecoveryOutcome::Unavailable
                        },
                    ));
                    continue;
                }
                let probe = match target {
                    Service::Kgc => {
                        let state = app.state::<KgcState>();
                        let _gate = state.gate.lock().await;
                        crate::session_coordinator::verify(target).await
                    }
                    _ => crate::session_coordinator::verify(target).await,
                };
                match probe {
                    Ok(lease) if lease.is_verified() => {
                        results.push(ServiceRecovery::new(target, RecoveryOutcome::Verified));
                        continue;
                    }
                    Err(SessionError::Cancelled) => return Err(SessionError::Cancelled),
                    Err(error) => {
                        results.push(ServiceRecovery::unavailable(target, error));
                        continue;
                    }
                    _ => {}
                }
            }
            if let Some(result) = SESSIONS.recovery_result(generation, target, requested, trigger) {
                results.push(result);
                continue;
            }
            let has_evidence = if let Some(value) = evidence {
                value
            } else {
                let value = match cookie_bridge::has_sso_evidence(app).await {
                    Ok(value) => value,
                    Err(error) => {
                        SESSIONS.record(generation, target, Health::Unavailable);
                        results.push(ServiceRecovery::unavailable(target, error));
                        continue;
                    }
                };
                evidence = Some(value);
                value
            };
            if !has_evidence {
                let outcome = if crate::keychain::get_secret_store_status().state == "ready" {
                    RecoveryOutcome::NeedsLogin
                } else {
                    RecoveryOutcome::Unavailable
                };
                if !SESSIONS.lease(target).has_credentials() {
                    SESSIONS.record(
                        generation,
                        target,
                        if outcome == RecoveryOutcome::NeedsLogin {
                            Health::NeedsLogin
                        } else {
                            Health::Unavailable
                        },
                    );
                }
                results.push(ServiceRecovery::new(target, outcome));
                continue;
            }
            if let Err(error) = cookie_bridge::restore_sso_cookies(app).await {
                log::warn!("SSO backup unavailable; checking native SSO: {error}");
            }
            SESSIONS.pace_flow().await;
            SESSIONS.record(generation, target, Health::Refreshing);
            let attempt = match target {
                Service::Kgc => {
                    headless_kgc_refresh(app, app.state::<KgcState>().inner(), generation).await
                }
                Service::Luna => {
                    headless_luna_refresh(app, app.state::<LunaState>().inner(), generation).await
                }
                Service::Kwic => {
                    headless_kwic_refresh(app, app.state::<KwicState>().inner(), generation).await
                }
            };
            SESSIONS
                .ensure_current(generation)
                .map_err(|_| SessionError::Cancelled)?;
            let result = match attempt {
                Ok(true) => ServiceRecovery {
                    recovered: true,
                    ..ServiceRecovery::new(target, RecoveryOutcome::Verified)
                },
                Ok(false) | Err(SessionError::NeedsLogin) => {
                    ServiceRecovery::new(target, RecoveryOutcome::NeedsLogin)
                }
                Err(SessionError::Cancelled) => return Err(SessionError::Cancelled),
                Err(error) => ServiceRecovery::unavailable(target, error),
            };
            if result.outcome != RecoveryOutcome::Verified {
                let health = if result.outcome == RecoveryOutcome::NeedsLogin
                    && !SESSIONS.lease(target).has_credentials()
                {
                    Health::NeedsLogin
                } else {
                    Health::Unavailable
                };
                SESSIONS.record(generation, target, health);
            }
            SESSIONS.finish_recovery(generation, target, &result);
            results.push(result);
        }
        if results.iter().any(|result| result.recovered) {
            cookie_bridge::persist_sso_cookies(app).await;
        }
        SESSIONS
            .ensure_current(generation)
            .map_err(|_| SessionError::Cancelled)?;
        let (snapshot, identity) = SESSIONS.overview();
        Ok(RecoveryReport {
            results,
            snapshot,
            identity,
        })
    };
    tokio::select! {
        biased;
        _ = SESSIONS.cancelled(generation) => Err(SessionError::Cancelled),
        result = work => result,
    }
}

#[tauri::command]
pub(crate) async fn restore_university_sessions(
    app: tauri::AppHandle,
) -> Result<RecoveryReport, SessionError> {
    recover_sessions(
        &app,
        &[Service::Luna, Service::Kwic],
        RecoveryTrigger::Startup,
    )
    .await
}
