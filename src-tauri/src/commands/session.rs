use crate::auth;
use crate::client;
use crate::config;
use crate::cookie_bridge;
use crate::kwic_client;
use crate::luna_client;
use crate::parser;
use crate::{KgcState, KwicState, LunaState};
use serde::Serialize;
use tauri::State;

#[derive(Debug, Serialize)]
pub struct SessionStates {
    pub kgc: bool,
    pub luna: bool,
    pub kwic: bool,
}

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

#[tauri::command]
pub async fn get_session_expiry(
    state: State<'_, KgcState>,
    luna_state: State<'_, LunaState>,
    kwic_state: State<'_, KwicState>,
) -> Result<Option<i64>, String> {
    let kgc_exp = state.client.lock().await.soonest_cookie_expiry_secs();
    let luna_exp = client::soonest_cookie_expiry(&luna_state.client.lock().await.cookie_store);
    let kwic_exp = client::soonest_cookie_expiry(&kwic_state.client.lock().await.cookie_store);
    let min = [kgc_exp, luna_exp, kwic_exp].into_iter().flatten().min();
    Ok(min)
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
    session_expired_msg: &str,
    is_session_expired: fn(&str) -> bool,
) -> Result<bool, String> {
    log::info!("headless_{}: starting (Cookie Bridge)", label);

    let win = match cookie_bridge::headless_saml_window(app, label, saml_url, sp_domain, 20).await?
    {
        Some(w) => w,
        None => return Ok(false),
    };

    cookie_bridge::extract_and_inject(app, sp_domain, cookie_store, base_url).await?;

    let result = client::fetch_with_redirect(
        http,
        verify_url,
        base_url,
        session_expired_msg,
        is_session_expired,
    )
    .await;
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

async fn headless_kgc_refresh(app: &tauri::AppHandle, state: &KgcState) -> Result<bool, String> {
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
    .await?
    {
        Some(w) => w,
        None => return Ok(false),
    };

    let cookie_store = state.client.lock().await.cookie_store.clone();
    cookie_bridge::extract_and_inject(
        app,
        "kg-course.kwansei.ac.jp",
        &cookie_store,
        config::KG_COURSE_BASE,
    )
    .await?;

    let http = state.client.lock().await.http.clone();
    let verify_url = format!(
        "{}/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014",
        config::KG_COURSE_BASE
    );
    match crate::client::fetch_page_with(&http, &verify_url).await {
        Ok(html) => {
            let info = parser::parse_student_info(&html);
            if info.student_id.is_empty() && info.name.is_empty() {
                log::warn!(
                    "headless_kgc_refresh: page returned empty student info (stale session)"
                );
                let _ = win.close();
                return Ok(false);
            }
            let mut client = state.client.lock().await;
            client.session = Some(auth::AuthSession {
                username: info.student_id.clone(),
                display_name: if info.name.is_empty() {
                    "ユーザー".to_string()
                } else {
                    info.name
                },
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

async fn headless_luna_refresh(app: &tauri::AppHandle, state: &LunaState) -> Result<bool, String> {
    let luna = state.client.lock().await;
    let cookie_store = luna.cookie_store.clone();
    let http = luna.http.clone();
    drop(luna);

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
        luna_client::LUNA_SESSION_EXPIRED_MSG,
        luna_client::is_luna_session_expired,
    )
    .await?;
    if ok {
        let mut luna = state.client.lock().await;
        luna.authenticated = true;
        luna.save_session();
    }
    Ok(ok)
}

async fn headless_kwic_refresh(app: &tauri::AppHandle, state: &KwicState) -> Result<bool, String> {
    let kwic = state.client.lock().await;
    let cookie_store = kwic.cookie_store.clone();
    let http = kwic.http.clone();
    drop(kwic);

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
        kwic_client::KWIC_SESSION_EXPIRED_MSG,
        kwic_client::is_kwic_session_expired,
    )
    .await?;
    if ok {
        let mut kwic = state.client.lock().await;
        kwic.authenticated = true;
        kwic.save_session();
    }
    Ok(ok)
}

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
            let (kgc_res, luna_res, kwic_res) = tokio::join!(
                headless_kgc_refresh(&app, kgc_state.inner()),
                headless_luna_refresh(&app, luna_state.inner()),
                headless_kwic_refresh(&app, kwic_state.inner()),
            );
            let kgc_ok = kgc_res.unwrap_or(false);
            let luna_ok = luna_res.unwrap_or(false);
            let kwic_ok = kwic_res.unwrap_or(false);
            log::info!(
                "sync_session(all): kgc={}, luna={}, kwic={}",
                kgc_ok,
                luna_ok,
                kwic_ok
            );
            Ok(kgc_ok || luna_ok || kwic_ok)
        }
        _ => Err(format!("Unknown service: {}", service)),
    }
}
