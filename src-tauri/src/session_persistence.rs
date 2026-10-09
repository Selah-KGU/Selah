//! Checkpoint current, accepted service jars after ordinary Set-Cookie traffic.
//! Candidate/retired HTTP clients can notify us but cannot publish their jars.
use reqwest::{cookie::CookieStore, header::HeaderValue, Url};
use std::{sync::Arc, time::Duration};

static DIRTY: tokio::sync::Notify = tokio::sync::Notify::const_new();

pub(crate) struct RotatingCookies(pub Arc<reqwest_cookie_store::CookieStoreMutex>);

impl CookieStore for RotatingCookies {
    fn set_cookies(&self, headers: &mut dyn Iterator<Item = &HeaderValue>, url: &Url) {
        self.0.set_cookies(headers, url);
        DIRTY.notify_one();
    }
    fn cookies(&self, url: &Url) -> Option<HeaderValue> {
        self.0.cookies(url)
    }
}

/// Checkpoint only accepted records, serialized with manager transitions.
pub(crate) async fn checkpoint(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::Emitter;
    let pending = crate::session_coordinator::SESSIONS
        .snapshot()
        .login_persistence_pending;
    let result = tokio::task::spawn_blocking(|| {
        crate::session_coordinator::SESSIONS
            .checkpoint_and_commit(&crate::session_coordinator::signout_marker())
    })
    .await
    .map_err(|e| e.to_string())?;
    let snapshot = crate::session_coordinator::SESSIONS.snapshot();
    if pending != snapshot.login_persistence_pending {
        let _ = app.emit("university-login-persistence", snapshot);
    }
    result
}

pub(crate) fn start(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            // Periodic retry covers failed writes and passive cookie expiry.
            let _ = tokio::time::timeout(Duration::from_secs(30), DIRTY.notified()).await;
            tokio::time::sleep(Duration::from_millis(250)).await;
            if let Err(error) = checkpoint(&app).await {
                log::warn!("Session checkpoint deferred: {error}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_response_rotation_and_deletion_update_the_shared_jar() {
        let jar = Arc::new(reqwest_cookie_store::CookieStoreMutex::default());
        let provider = RotatingCookies(jar.clone());
        let url = Url::parse("https://example.test/").unwrap();
        for value in ["sid=old; Path=/; HttpOnly", "sid=new; Path=/; HttpOnly"] {
            provider.set_cookies(
                &mut [&HeaderValue::from_str(value).unwrap()].into_iter(),
                &url,
            );
        }
        assert_eq!(provider.cookies(&url).unwrap(), "sid=new");
        provider.set_cookies(
            &mut [&HeaderValue::from_static("sid=; Path=/; Max-Age=0")].into_iter(),
            &url,
        );
        assert!(provider.cookies(&url).is_none());
        assert_eq!(jar.lock().unwrap().iter_unexpired().count(), 0);
    }
}
