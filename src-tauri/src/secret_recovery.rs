//! Application recovery reports each independent component; one corrupt record
//! must not prevent other services from becoming usable after unlock.
use crate::keychain::{SecretStoreStatus, StoreError};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(crate) struct ComponentRecovery {
    component: &'static str,
    outcome: &'static str,
    error: Option<StoreError>,
}
impl ComponentRecovery {
    fn result(component: &'static str, result: Result<bool, StoreError>) -> Self {
        match result {
            Ok(restored) => Self {
                component,
                outcome: if restored { "restored" } else { "unchanged" },
                error: None,
            },
            Err(error) => Self {
                component,
                outcome: "failed",
                error: Some(error),
            },
        }
    }
}
#[derive(Serialize)]
pub(crate) struct SecretRecoveryReport {
    status: SecretStoreStatus,
    results: Vec<ComponentRecovery>,
}

fn recover_services(
    mut restore: impl FnMut(crate::session_coordinator::Service) -> Result<bool, StoreError>,
) -> Vec<ComponentRecovery> {
    use crate::session_coordinator::Service;
    [Service::Kgc, Service::Luna, Service::Kwic]
        .into_iter()
        .map(|service| ComponentRecovery::result(service.name(), restore(service)))
        .collect()
}

#[tauri::command]
pub async fn retry_secret_store(app: tauri::AppHandle) -> Result<SecretRecoveryReport, String> {
    use tauri::Manager;
    let _gate = crate::session_coordinator::SESSIONS.lock().await;
    tokio::task::spawn_blocking(crate::keychain::retry_store)
        .await
        .map_err(|e| e.to_string())??;
    let mut results = Vec::new();
    if !crate::session_coordinator::SESSIONS.signed_out() {
        results.extend(
            tokio::task::spawn_blocking(|| {
                recover_services(|service| crate::session_coordinator::SESSIONS.restore(service))
            })
            .await
            .map_err(|e| e.to_string())?,
        );
        results.push(ComponentRecovery::result(
            "session_checkpoint",
            crate::session_persistence::checkpoint(&app)
                .await
                .map(|()| true)
                .map_err(|e| StoreError::new("write_failed", e)),
        ));
        results.push(ComponentRecovery::result(
            "sso_cookies",
            crate::cookie_bridge::restore_sso_cookies(&app)
                .await
                .map(|_| true)
                .map_err(|e| StoreError::new("restore_failed", e)),
        ));
    }
    {
        let state = app.state::<crate::MailState>();
        let mut client = state.client.lock().await;
        let restore = if client.token.is_none() {
            client.try_restore_token()
        } else {
            Ok(false)
        };
        results.push(ComponentRecovery::result(
            "mail",
            restore.and_then(|restored| client.save_token().map(|()| restored)),
        ));
    }
    {
        let state = app.state::<crate::GCalState>();
        let mut client = state.client.lock().await;
        results.push(ComponentRecovery::result(
            "gcal_config",
            client.reload_config().map(|()| true),
        ));
        let restore = if client.token.is_none() {
            client.try_restore_token()
        } else {
            Ok(false)
        };
        results.push(ComponentRecovery::result(
            "gcal",
            restore.and_then(|restored| client.save_token().map(|()| restored)),
        ));
    }
    Ok(SecretRecoveryReport {
        status: crate::keychain::get_secret_store_status(),
        results,
    })
}

#[tauri::command]
pub fn get_secret_store_status() -> SecretStoreStatus {
    crate::keychain::get_secret_store_status()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn broken_first_service_does_not_block_independent_restoration() {
        let mut attempted = Vec::new();
        let results = recover_services(|service| {
            attempted.push(service.name());
            if service == crate::session_coordinator::Service::Kgc {
                Err(StoreError::new("corrupt_record", "corrupt"))
            } else {
                Ok(true)
            }
        });
        assert_eq!(attempted, ["kgc", "luna", "kwic"]);
        assert_eq!(results[0].outcome, "failed");
        assert_eq!(results[1].outcome, "restored");
        assert_eq!(results[2].outcome, "restored");
    }
}
