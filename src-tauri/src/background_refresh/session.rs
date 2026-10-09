//! Background callers submit intent; the university manager owns recovery.
use super::*;
use crate::session_coordinator::{RecoveryTrigger, Service, SESSIONS};
use crate::MailState;

pub async fn sync_backend_session_status(
    app: &AppHandle,
    attempt_recovery: bool,
) -> Result<BackendSessionStatusPayload, String> {
    sync_session_status(app, attempt_recovery.then_some(RecoveryTrigger::Foreground)).await
}
pub(super) async fn sync_backend_session_status_for_refresh(
    app: &AppHandle,
) -> Result<BackendSessionStatusPayload, String> {
    sync_session_status(app, Some(RecoveryTrigger::Background)).await
}
async fn sync_session_status(
    app: &AppHandle,
    trigger: Option<RecoveryTrigger>,
) -> Result<BackendSessionStatusPayload, String> {
    let generation = SESSIONS.generation();
    if let Some(trigger) = trigger {
        crate::commands::recover_sessions(app, &[Service::Luna, Service::Kwic], trigger).await?;
    }
    let payload = session_status_projection(app).await;
    SESSIONS.ensure_current(generation)?;
    let state = app.state::<BackendRefreshState>();
    if !state.session_status_unchanged(&payload) {
        emit_session_status(app, &payload);
        state.store_session_status(payload.clone());
    }
    Ok(payload)
}

pub(crate) async fn session_status_projection(app: &AppHandle) -> BackendSessionStatusPayload {
    let mail_status = crate::mail_commands::mail_check_session(app.state::<MailState>())
        .await
        .unwrap_or_default();
    let (snapshot, identity) = SESSIONS.overview();
    let present = |service: Service| {
        snapshot.services[service.index()].credentials_present && !snapshot.signed_out
    };
    // Legacy UI booleans are projections of proof, never proof themselves.
    let usable = |service: Service| {
        present(service)
            && snapshot.services[service.index()]
                .last_verified_at
                .is_some()
    };
    BackendSessionStatusPayload {
        generation: snapshot.generation,
        signed_out: snapshot.signed_out,
        kgc_session_present: present(Service::Kgc),
        session_expired: [Service::Luna, Service::Kwic].iter().any(|service| {
            snapshot.services[service.index()].state
                == crate::session_coordinator::Health::NeedsLogin
        }),
        username: identity
            .as_ref()
            .map(|s| s.username.clone())
            .unwrap_or_default(),
        display_name: identity
            .as_ref()
            .map(|s| s.display_name.clone())
            .unwrap_or_default(),
        student_id: identity
            .as_ref()
            .map(|s| s.student_id.clone())
            .unwrap_or_default(),
        faculty: identity
            .as_ref()
            .map(|s| s.faculty.clone())
            .unwrap_or_default(),
        department: identity
            .as_ref()
            .map(|s| s.department.clone())
            .unwrap_or_default(),
        luna_authenticated: usable(Service::Luna),
        kwic_authenticated: usable(Service::Kwic),
        mail_authenticated: mail_status.authenticated,
        mail_generation: mail_status.generation,
        mail_connection_id: mail_status.connection_id,
        mail_email: mail_status.email,
        mail_display_name: mail_status.display_name,
        university: Some(snapshot),
    }
}

pub(super) async fn maybe_renew_sessions(app: &AppHandle) {
    if ![Service::Luna, Service::Kwic]
        .iter()
        .any(|service| SESSIONS.lease(*service).has_credentials())
    {
        return;
    }
    if let Err(error) = crate::commands::recover_sessions(
        app,
        &[Service::Luna, Service::Kwic],
        RecoveryTrigger::Keepalive,
    )
    .await
    {
        log::warn!("Background session renewal: {error}");
    }
}
