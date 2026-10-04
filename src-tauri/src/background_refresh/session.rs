//! Session status, recovery, and keep-alive.

use std::sync::atomic::Ordering;

use crate::client;
use crate::{KgcState, KwicState, LunaState, MailState};

use super::*;

pub async fn sync_backend_session_status(
    app: &AppHandle,
    attempt_recovery: bool,
) -> Result<BackendSessionStatusPayload, String> {
    sync_session_status(app, attempt_recovery, true).await
}

pub(super) async fn sync_backend_session_status_for_refresh(
    app: &AppHandle,
) -> Result<BackendSessionStatusPayload, String> {
    // The refresh tick is about to fetch Luna/KWIC itself. Skip a second
    // validation page when one of those fetches just succeeded.
    sync_session_status(app, true, false).await
}

async fn sync_session_status(
    app: &AppHandle,
    attempt_recovery: bool,
    force_probe: bool,
) -> Result<BackendSessionStatusPayload, String> {
    let state = app.state::<BackendRefreshState>();
    let owns_recovery =
        attempt_recovery && !state.session_sync_running.swap(true, Ordering::SeqCst);
    struct RecoveryGuard<'a>(Option<&'a AtomicBool>);
    impl Drop for RecoveryGuard<'_> {
        fn drop(&mut self) {
            if let Some(running) = self.0 {
                running.store(false, Ordering::SeqCst);
            }
        }
    }
    let _guard = RecoveryGuard(owns_recovery.then_some(&state.session_sync_running));

    let payload = sync_backend_session_status_inner(app, owns_recovery, force_probe).await?;
    if state.session_status_unchanged(&payload) {
        return Ok(payload);
    }
    emit_session_status(app, &payload);
    state.store_session_status(payload.clone());
    Ok(payload)
}

async fn sync_backend_session_status_inner(
    app: &AppHandle,
    attempt_recovery: bool,
    force_probe: bool,
) -> Result<BackendSessionStatusPayload, String> {
    let kgc_had_session = is_kgc_authenticated(app).await;
    let luna_had_session = is_luna_authenticated(app).await;
    let kwic_had_session = is_kwic_authenticated(app).await;

    // KGC is a short-lived, request-driven session. Background status sync must
    // never touch its server; actual KGC requests and explicit user checks are
    // responsible for confirming whether the stored session still works.
    let kgc_status = kgc_status_from_memory(app, kgc_had_session).await;
    let mut luna_valid = if should_probe_service(
        app,
        luna_had_session,
        attempt_recovery,
        force_probe,
        &["luna_updates", "luna_todo"],
    ) {
        match crate::luna_commands::luna_check_session(app.state::<LunaState>()).await {
            Ok(valid) => valid,
            Err(e) => {
                log::warn!(
                    "session status: Luna validation transient failure, retaining session: {}",
                    e
                );
                true
            }
        }
    } else {
        luna_had_session
    };
    let mut kwic_valid = if should_probe_service(
        app,
        kwic_had_session,
        attempt_recovery,
        force_probe,
        &["kwic_home"],
    ) {
        match crate::kwic_commands::kwic_check_session(app.state::<KwicState>()).await {
            Ok(valid) => valid,
            Err(e) => {
                log::warn!(
                    "session status: KWIC validation transient failure, retaining session: {}",
                    e
                );
                true
            }
        }
    } else {
        kwic_had_session
    };
    if attempt_recovery {
        // Proactive recovery is restricted to the core services. KGC is only
        // recovered after an actual KGC request fails or a user asks for it.
        let has_session_evidence = kgc_had_session || luna_had_session || kwic_had_session;
        if has_session_evidence
            && !luna_valid
            && attempt_service_recovery(app, SessionService::Luna).await
        {
            luna_valid = crate::luna_commands::luna_check_session(app.state::<LunaState>())
                .await
                .unwrap_or(true);
        }
        if has_session_evidence
            && !kwic_valid
            && attempt_service_recovery(app, SessionService::Kwic).await
        {
            kwic_valid = crate::kwic_commands::kwic_check_session(app.state::<KwicState>())
                .await
                .unwrap_or(true);
        }
    }

    let mail_status = crate::mail_commands::mail_check_session(app.state::<MailState>())
        .await
        .unwrap_or(crate::mail_commands::MailSessionStatus {
            authenticated: false,
            email: String::new(),
            display_name: String::new(),
        });

    // Luna and KWIC are the core app sessions. KGC enriches timetable and
    // academic-record features, but its isolated failure must not put the
    // whole app into the user-facing re-authentication state.
    let core_session_expired = !(luna_valid && kwic_valid);

    Ok(BackendSessionStatusPayload {
        kgc_session_present: kgc_status.valid,
        session_expired: core_session_expired,
        username: if kgc_status.valid {
            kgc_status.username
        } else {
            String::new()
        },
        display_name: if kgc_status.valid {
            kgc_status.display_name
        } else {
            String::new()
        },
        student_id: if kgc_status.valid {
            kgc_status.student_id
        } else {
            String::new()
        },
        faculty: if kgc_status.valid {
            kgc_status.faculty
        } else {
            String::new()
        },
        department: if kgc_status.valid {
            kgc_status.department
        } else {
            String::new()
        },
        luna_authenticated: luna_valid,
        kwic_authenticated: kwic_valid,
        mail_authenticated: mail_status.authenticated,
        mail_email: mail_status.email,
        mail_display_name: mail_status.display_name,
    })
}

fn should_probe_service(
    app: &AppHandle,
    had_session: bool,
    attempt_recovery: bool,
    force_probe: bool,
    proof_keys: &[&str],
) -> bool {
    if !(had_session && attempt_recovery) {
        return false;
    }
    if force_probe {
        return true;
    }
    let db = app.state::<crate::db::Database>();
    let now = epoch_secs();
    let recently_proven = proof_keys
        .iter()
        .any(|key| recent_success_covers_probe(db.cache_updated_at(key), now));
    !recently_proven
}

async fn kgc_status_from_memory(app: &AppHandle, valid: bool) -> crate::commands::SessionStatus {
    let state = app.state::<KgcState>();
    let client = state.client.lock().await;
    match client.session.as_ref() {
        Some(session) => crate::commands::SessionStatus {
            valid,
            username: session.username.clone(),
            display_name: session.display_name.clone(),
            student_id: session.student_id.clone(),
            faculty: session.faculty.clone(),
            department: session.department.clone(),
        },
        None => crate::commands::SessionStatus {
            valid: false,
            username: String::new(),
            display_name: String::new(),
            student_id: String::new(),
            faculty: String::new(),
            department: String::new(),
        },
    }
}

async fn attempt_service_recovery(app: &AppHandle, service: SessionService) -> bool {
    let state = app.state::<BackendRefreshState>();
    let now = epoch_secs();
    if !state.recovery_due(service, now) {
        log::debug!(
            "session recovery: {} skipped during backoff",
            service.name()
        );
        return false;
    }

    log::info!("session recovery: attempting {}", service.name());
    let succeeded = match crate::commands::sync_session(
        app.clone(),
        app.state::<KgcState>(),
        app.state::<LunaState>(),
        app.state::<KwicState>(),
        service.name().to_string(),
    )
    .await
    {
        Ok(true) => true,
        Ok(false) => false,
        Err(e) => {
            log::warn!("session recovery: {} failed: {}", service.name(), e);
            false
        }
    };
    state.record_recovery(service, now, succeeded);
    succeeded
}

pub(super) async fn maybe_renew_sessions(app: &AppHandle) {
    let state = app.state::<BackendRefreshState>();
    if state.session_sync_running.swap(true, Ordering::SeqCst) {
        return;
    }

    struct RunningGuard<'a>(&'a AtomicBool);
    impl Drop for RunningGuard<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }
    let _guard = RunningGuard(&state.session_sync_running);

    if let Err(e) = maybe_renew_sessions_inner(app).await {
        log::warn!("background session renew failed: {}", e);
    }
}

async fn maybe_renew_sessions_inner(app: &AppHandle) -> Result<(), String> {
    // Reactive trigger: a core-service cookie is near its explicit expiry.
    let expiry_due = soonest_core_session_expiry_secs(app)
        .await
        .is_some_and(|secs| secs <= SESSION_RENEW_THRESHOLD_SECS);

    // Time-based keep-alive: only meaningful if we currently believe we're
    // logged in to at least one service (otherwise renewal would just spawn a
    // hidden Okta-login webview for nothing). This covers the session-only
    // cookie case where expiry_due can never fire.
    let any_session = current_core_session_present(app).await;
    let state = app.state::<BackendRefreshState>();
    let now = epoch_secs();
    let last = state.last_session_keepalive.load(Ordering::Relaxed);
    let keepalive_due = any_session && now.saturating_sub(last) >= SESSION_KEEPALIVE_INTERVAL_SECS;

    if !expiry_due && !keepalive_due {
        return Ok(());
    }
    if now.saturating_sub(last) < SESSION_RENEW_MIN_INTERVAL_SECS {
        return Ok(());
    }

    log::info!(
        "background refresh: headless session renew (expiry_due={}, keepalive_due={})",
        expiry_due,
        keepalive_due
    );
    // Record the attempt time up front so a dead Okta session doesn't make us
    // retry every tick — the reactive path handles real usage in the meantime.
    state.last_session_keepalive.store(now, Ordering::Relaxed);
    let _ = crate::commands::sync_session(
        app.clone(),
        app.state::<KgcState>(),
        app.state::<LunaState>(),
        app.state::<KwicState>(),
        "all".to_string(),
    )
    .await?;
    Ok(())
}

/// Whether we currently believe at least one core service is authenticated.
async fn current_core_session_present(app: &AppHandle) -> bool {
    if app.state::<LunaState>().client.lock().await.authenticated {
        return true;
    }
    app.state::<KwicState>().client.lock().await.authenticated
}

async fn soonest_core_session_expiry_secs(app: &AppHandle) -> Option<i64> {
    let luna_exp =
        client::soonest_cookie_expiry(&app.state::<LunaState>().client.lock().await.cookie_store);
    let kwic_exp =
        client::soonest_cookie_expiry(&app.state::<KwicState>().client.lock().await.cookie_store);
    [luna_exp, kwic_exp].into_iter().flatten().min()
}

async fn is_kgc_authenticated(app: &AppHandle) -> bool {
    app.state::<KgcState>()
        .client
        .lock()
        .await
        .is_authenticated()
}

async fn is_luna_authenticated(app: &AppHandle) -> bool {
    app.state::<LunaState>().client.lock().await.authenticated
}

async fn is_kwic_authenticated(app: &AppHandle) -> bool {
    app.state::<KwicState>().client.lock().await.authenticated
}
