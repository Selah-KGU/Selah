//! Cookie Bridge: extract cookies from the platform's native webview cookie store
//! and inject them into reqwest cookie jars.
//!
//! - macOS: WKHTTPCookieStore (ObjC API via objc2)
//! - Windows: WebView2 Chrome DevTools Protocol (CDP)

use tauri::Manager;

#[cfg(any(target_os = "windows", test))]
mod cdp_cookie;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos::{delete_university_cookies, extract_all_cookies, set_all_cookies};

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use self::windows::{delete_university_cookies, extract_all_cookies, set_all_cookies};

/// Keychain key holding the JSON backup of the SSO/Okta session cookies. These
/// normally live only in the OS webview store; backing them up lets headless
/// re-auth survive the webview store being cleared (OS cleanup / reinstall),
/// keeping the long-lived device token usable for far longer.
const SSO_COOKIE_BACKUP_KEY: &str = "sso_cookie_backup";
static SSO_RESTORE_GATE: tokio::sync::Mutex<bool> = tokio::sync::Mutex::const_new(false);

/// All university cookies are deleted by a complete login reset.
fn is_university_cookie(domain: &str) -> bool {
    let d = domain.trim_start_matches('.');
    d == "kwansei.ac.jp" || d.ends_with(".kwansei.ac.jp")
}

/// Only identity-provider cookies are backed up. Service-provider cookies,
/// especially KGC's short-lived cookies, must retain their natural lifetime.
fn is_backupable_sso_cookie(domain: &str) -> bool {
    let d = domain.trim_start_matches('.');
    d == "kwansei.ac.jp" || OKTA_HOSTS.contains(&d)
}

/// Remove all university/SSO cookies from the native webview store and delete
/// the keychain backup so startup restoration cannot silently log back in.
pub async fn clear_university_cookies(app: &tauri::AppHandle) -> Result<usize, String> {
    // Attempt both cleanup operations, even if the vault is currently locked.
    // The durable sign-out marker prevents residual data from being restored.
    let backup = crate::keychain::delete_secrets(&["cookie.sso_cookie_backup"]);
    let native = clear_native_university_cookies(app).await;
    backup?;
    native
}

pub(crate) async fn clear_native_university_cookies(
    app: &tauri::AppHandle,
) -> Result<usize, String> {
    let mut restored = SSO_RESTORE_GATE.lock().await;
    let deleted = delete_university_cookies(app).await?;
    *restored = true;
    log::info!("clear_university_cookies: removed {deleted} university cookies");
    Ok(deleted)
}

/// Plain cookie data extracted from the webview (Send + Sync safe).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct CookieData {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub expires_unix: Option<f64>,
    #[serde(default)]
    pub same_site: Option<String>,
    #[serde(default)]
    pub host_only: Option<bool>,
}

impl CookieData {
    fn is_host_only(&self) -> bool {
        self.host_only.unwrap_or(!self.domain.starts_with('.'))
    }

    fn normalized_same_site(&self) -> Option<&'static str> {
        match self.same_site.as_deref()?.to_ascii_lowercase().as_str() {
            "none" => Some("None"),
            "lax" => Some("Lax"),
            "strict" => Some("Strict"),
            _ => None,
        }
    }

    fn live(&self, now: f64) -> bool {
        self.expires_unix
            .is_none_or(|expiry| expiry.is_finite() && expiry > now)
    }
}

/// Extract cookies matching a specific domain from the webview.
async fn extract_cookies_for_domain(
    app: &tauri::AppHandle,
    domain: &str,
) -> Result<Vec<CookieData>, String> {
    let all = extract_all_cookies(app).await?;
    let domain_owned = domain.to_string();
    Ok(all
        .into_iter()
        .filter(|c| {
            let cookie_domain = c.domain.trim_start_matches('.');
            cookie_domain == domain_owned || domain_owned.ends_with(&format!(".{}", cookie_domain))
        })
        .collect())
}

/// Inject extracted cookies into a reqwest cookie store.
fn inject_cookies(
    store: &reqwest_cookie_store::CookieStoreMutex,
    cookies: &[CookieData],
    base_url: &str,
) {
    let url = match url::Url::parse(base_url) {
        Ok(u) => u,
        Err(e) => {
            log::warn!("inject_cookies: invalid base URL {}: {}", base_url, e);
            return;
        }
    };

    let mut jar = store.lock().unwrap_or_else(|e| e.into_inner());
    let mut count = 0;
    for c in cookies {
        let mut builder = cookie_store::RawCookie::build((&*c.name, &*c.value)).path(&*c.path);
        if !c.is_host_only() {
            builder = builder.domain(&*c.domain);
        }
        // Host-only cookies must be inserted relative to their own origin,
        // never re-scoped to an unrelated service's base URL.
        let cookie_url = if c.is_host_only() {
            match url::Url::parse(&format!("https://{}/", c.domain.trim_start_matches('.'))) {
                Ok(url) => url,
                Err(_) => continue,
            }
        } else {
            url.clone()
        };
        if c.secure {
            builder = builder.secure(true);
        }
        if c.http_only {
            builder = builder.http_only(true);
        }
        if let Some(ts) = c.expires_unix {
            if let Ok(odt) = time::OffsetDateTime::from_unix_timestamp(ts as i64) {
                builder = builder.expires(odt);
            }
        }
        let raw = builder.build();
        match jar.insert_raw(&raw, &cookie_url) {
            Ok(_) => count += 1,
            Err(e) => log::warn!("inject_cookies: failed to insert '{}': {}", c.name, e),
        }
    }
    log::info!(
        "inject_cookies: injected {}/{} cookies for {}",
        count,
        cookies.len(),
        base_url
    );
}

/// Check if a URL indicates we've arrived at an SP domain after SAML.
pub fn is_post_saml_sp_url(url: &url::Url, sp_host: &str) -> bool {
    let host = url.host_str().unwrap_or("");
    if host != sp_host {
        return false;
    }
    let path = url.path();
    if path.contains("Shibboleth.sso")
        || path.starts_with("/saml/")
        || path.starts_with("/Shibboleth.sso")
    {
        return false;
    }
    true
}

/// Extract cookies for a specific SP domain (+ parent SSO cookies) from the webview
/// and inject them into a reqwest cookie store.
pub async fn extract_and_inject(
    app: &tauri::AppHandle,
    sp_domain: &str,
    cookie_store: &reqwest_cookie_store::CookieStoreMutex,
    base_url: &str,
) -> Result<(), String> {
    let sp_cookies = extract_cookies_for_domain(app, sp_domain).await?;
    let sso_cookies = match extract_cookies_for_domain(app, "kwansei.ac.jp").await {
        Ok(cookies) => cookies,
        Err(e) => {
            log::warn!("Failed to extract SSO parent domain cookies: {e}");
            Vec::new()
        }
    };
    let all: Vec<_> = sp_cookies
        .iter()
        .chain(sso_cookies.iter())
        .cloned()
        .collect();
    inject_cookies(cookie_store, &all, base_url);
    Ok(())
}

const OKTA_HOSTS: &[&str] = &[
    "sso.kwansei.ac.jp",
    "idp.kwansei.ac.jp",
    "sts.kwansei.ac.jp",
];

fn is_okta_login_page(url: &url::Url) -> bool {
    let host = url.host_str().unwrap_or("");
    OKTA_HOSTS.contains(&host)
}

#[derive(Debug, PartialEq, Eq)]
enum HeadlessSamlPageState {
    Complete,
    Continue,
}

fn headless_saml_page_state(url: &url::Url, sp_host: &str) -> HeadlessSamlPageState {
    if is_post_saml_sp_url(url, sp_host) {
        HeadlessSamlPageState::Complete
    } else {
        HeadlessSamlPageState::Continue
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SsoSnapshot {
    version: u32,
    saved_at: i64,
    cookies: Vec<CookieData>,
}

fn read_backup(json: &str) -> Result<Vec<CookieData>, String> {
    // v1 stored only the array. Preserve it during the v2 transition.
    if json.trim_start().starts_with('[') {
        return serde_json::from_str(json).map_err(|_| "Invalid legacy SSO backup".into());
    }
    let snapshot: SsoSnapshot = serde_json::from_str(json).map_err(|_| "Invalid SSO backup")?;
    if snapshot.version != 2 {
        return Err("Unsupported SSO backup version".into());
    }
    Ok(snapshot.cookies)
}

fn now_unix() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn missing_sso_cookies(
    backup: Vec<CookieData>,
    native: &[CookieData],
    now: f64,
) -> Vec<CookieData> {
    let keys: std::collections::HashSet<_> = native
        .iter()
        .filter(|c| c.live(now))
        .map(|c| {
            (
                c.name.as_str(),
                c.domain.trim_start_matches('.').to_ascii_lowercase(),
                c.path.as_str(),
            )
        })
        .collect();
    backup
        .into_iter()
        .filter(|c| is_backupable_sso_cookie(&c.domain) && c.live(now))
        .filter(|c| {
            !keys.contains(&(
                c.name.as_str(),
                c.domain.trim_start_matches('.').to_ascii_lowercase(),
                c.path.as_str(),
            ))
        })
        .collect()
}

/// Native cookies are authoritative. Snapshot only after successful SAML;
/// an empty successful read replaces old data instead of reviving stale state.
pub async fn persist_sso_cookies(app: &tauri::AppHandle) {
    let mut restored = SSO_RESTORE_GATE.lock().await;
    let result = async {
        let now = now_unix();
        let cookies = extract_all_cookies(app)
            .await?
            .into_iter()
            .filter(|c| is_backupable_sso_cookie(&c.domain) && c.live(now))
            .collect();
        let snapshot = SsoSnapshot {
            version: 2,
            saved_at: now as i64,
            cookies,
        };
        let json = serde_json::to_string(&snapshot).map_err(|_| "Serialize SSO backup")?;
        crate::keychain::set_cookie_secret(SSO_COOKIE_BACKUP_KEY, &json)
    }
    .await;
    *restored = true;
    if let Err(error) = result {
        log::warn!("SSO backup was not saved: {error}");
    }
}

/// Restore only absent cookies, await native completion, and allow a later
/// attempt after temporary failure. Called under the authentication coordinator.
pub async fn restore_sso_cookies(app: &tauri::AppHandle) -> Result<(), String> {
    let mut restored = SSO_RESTORE_GATE.lock().await;
    if crate::session_coordinator::SESSIONS.signed_out() {
        return Ok(());
    }
    if *restored {
        return Ok(());
    }
    if let Some(json) = crate::keychain::get_cookie_secret(SSO_COOKIE_BACKUP_KEY)? {
        let cookies = read_backup(&json)?;
        let native = extract_all_cookies(app).await?;
        let missing = missing_sso_cookies(cookies, &native, now_unix());
        if !missing.is_empty() {
            let count = set_all_cookies(app, &missing).await?;
            log::info!("Restored {count} missing SSO cookies");
        }
    }
    *restored = true;
    Ok(())
}

/// This is recovery evidence, not proof of an authenticated upstream session.
pub async fn has_sso_evidence(
    app: &tauri::AppHandle,
) -> Result<bool, crate::session_coordinator::SessionError> {
    use crate::session_coordinator::SessionError;
    let now = now_unix();
    let saved = crate::keychain::get_cookie_secret(SSO_COOKIE_BACKUP_KEY)
        .map_err(SessionError::Storage)
        .and_then(|json| {
            json.map(|json| read_backup(&json).map_err(SessionError::Unavailable))
                .transpose()
        })
        .map(|cookies| {
            cookies.is_some_and(|cookies| {
                cookies
                    .iter()
                    .any(|c| is_backupable_sso_cookie(&c.domain) && c.live(now))
            })
        });
    if matches!(saved, Ok(true)) {
        return Ok(true);
    }
    let native = extract_all_cookies(app)
        .await
        .map_err(SessionError::Unavailable)?;
    if native
        .iter()
        .any(|c| is_backupable_sso_cookie(&c.domain) && c.live(now))
    {
        return Ok(true);
    }
    saved
}

/// Wait for every native completion callback, including failures. No fixed
/// sleep can prove that a native cookie mutation has completed.
#[cfg(target_os = "macos")]
async fn await_cookie_completions(
    mut receiver: tokio::sync::mpsc::UnboundedReceiver<Result<(), String>>,
) -> Result<usize, String> {
    tokio::time::timeout(std::time::Duration::from_secs(10), async move {
        let mut count = 0;
        let mut error = None;
        while let Some(result) = receiver.recv().await {
            match result {
                Ok(()) => count += 1,
                Err(e) => error = Some(e),
            }
        }
        error.map_or(Ok(count), Err)
    })
    .await
    .map_err(|_| "Native cookie operation timed out".to_string())?
}

pub(crate) struct AuthWindow(pub tauri::WebviewWindow);
impl std::ops::Deref for AuthWindow {
    type Target = tauri::WebviewWindow;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl Drop for AuthWindow {
    fn drop(&mut self) {
        let _ = self.0.close();
    }
}

pub async fn headless_saml_window(
    app: &tauri::AppHandle,
    window_label: &str,
    saml_url: &str,
    sp_domain: &str,
    timeout_secs: u64,
) -> Result<Option<AuthWindow>, String> {
    if let Some(w) = app.get_webview_window(window_label) {
        let _ = w.close();
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);

    let parsed_url: url::Url = saml_url
        .parse()
        .map_err(|e| format!("URL parse error: {}", e))?;

    let sp_domain_owned = sp_domain.to_string();
    let label_for_log = window_label.to_string();
    let win = tauri::WebviewWindowBuilder::new(
        app,
        window_label,
        tauri::WebviewUrl::External(parsed_url),
    )
    .visible(false)
    .on_navigation(|_| true)
    .on_page_load(move |_win, payload| {
        use tauri::webview::PageLoadEvent;
        if !matches!(payload.event(), PageLoadEvent::Finished) {
            return;
        }
        let url = payload.url();
        match headless_saml_page_state(url, &sp_domain_owned) {
            HeadlessSamlPageState::Complete => {
                log::info!("{}: page loaded on SP domain", label_for_log);
                let _ = tx.try_send(());
            }
            HeadlessSamlPageState::Continue if is_okta_login_page(url) => {
                log::info!(
                    "{}: Okta page detected; waiting for automatic SSO continuation",
                    label_for_log
                );
            }
            HeadlessSamlPageState::Continue => {}
        }
    })
    .build()
    .map_err(|e| format!("Failed to build headless window '{}': {}", window_label, e))?;

    let win = AuthWindow(win);
    match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), rx.recv()).await {
        Ok(Some(())) => {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            Ok(Some(win))
        }
        Ok(None) => {
            log::info!("{}: window closed without completing", window_label);
            Err("Automatic SSO window closed before verification".into())
        }
        Err(_) => {
            log::info!(
                "{}: timed out waiting for automatic SSO continuation",
                window_label
            );
            let _ = win.close();
            Err("Automatic SSO timed out; authentication could not be verified".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        headless_saml_page_state, is_backupable_sso_cookie, is_university_cookie,
        HeadlessSamlPageState,
    };

    #[test]
    fn sso_backup_excludes_service_provider_cookies() {
        assert!(is_backupable_sso_cookie(".kwansei.ac.jp"));
        assert!(is_backupable_sso_cookie("sso.kwansei.ac.jp"));
        assert!(is_backupable_sso_cookie("idp.kwansei.ac.jp"));
        assert!(is_backupable_sso_cookie("sts.kwansei.ac.jp"));

        assert!(!is_backupable_sso_cookie("kg-course.kwansei.ac.jp"));
        assert!(!is_backupable_sso_cookie("luna.kwansei.ac.jp"));
        assert!(!is_backupable_sso_cookie("kwic.kwansei.ac.jp"));
        assert!(!is_backupable_sso_cookie("example.com"));
    }

    #[test]
    fn complete_reset_includes_all_university_cookies() {
        assert!(is_university_cookie(".kwansei.ac.jp"));
        assert!(is_university_cookie("sso.kwansei.ac.jp"));
        assert!(is_university_cookie("kg-course.kwansei.ac.jp"));
        assert!(is_university_cookie("luna.kwansei.ac.jp"));
        assert!(is_university_cookie("kwic.kwansei.ac.jp"));
        assert!(!is_university_cookie("example.com"));
    }

    #[test]
    fn headless_saml_waits_through_okta_before_service_redirect() {
        let okta = url::Url::parse("https://sso.kwansei.ac.jp/login").unwrap();
        let kgc =
            url::Url::parse("https://kg-course.kwansei.ac.jp/uniasv2/UnSSOLoginControl2").unwrap();

        assert_eq!(
            headless_saml_page_state(&okta, "kg-course.kwansei.ac.jp"),
            HeadlessSamlPageState::Continue
        );
        assert_eq!(
            headless_saml_page_state(&kgc, "kg-course.kwansei.ac.jp"),
            HeadlessSamlPageState::Complete
        );
    }

    fn cookie(name: &str, value: &str) -> super::CookieData {
        super::CookieData {
            name: name.into(),
            value: value.into(),
            domain: "sso.kwansei.ac.jp".into(),
            path: "/".into(),
            secure: true,
            http_only: true,
            expires_unix: None,
            same_site: Some("None".into()),
            host_only: Some(true),
        }
    }

    #[test]
    fn backup_never_overwrites_native_state_or_restores_expired_and_foreign_cookies() {
        let mut expired = cookie("expired", "x");
        expired.expires_unix = Some(50.0);
        let mut service = cookie("service", "x");
        service.domain = "luna.kwansei.ac.jp".into();
        let backup = vec![
            cookie("session", "old"),
            cookie("device", "d"),
            expired,
            service,
        ];
        let native = vec![cookie("session", "new")];
        let missing = super::missing_sso_cookies(backup, &native, 100.0);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].name, "device");
        assert_eq!(missing[0].normalized_same_site(), Some("None"));
        assert!(missing[0].is_host_only());
    }

    #[test]
    fn legacy_sso_backup_loads_but_unknown_snapshot_version_does_not() {
        let legacy = r#"[{"name":"sid","value":"test","domain":"sso.kwansei.ac.jp","path":"/","secure":true,"http_only":true,"expires_unix":null}]"#;
        assert_eq!(super::read_backup(legacy).unwrap().len(), 1);
        assert!(super::read_backup(r#"{"version":99,"saved_at":1,"cookies":[]}"#).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_mutations_wait_for_all_callbacks_and_propagate_failure() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
            let task = tokio::spawn(super::await_cookie_completions(receiver));
            sender.send(Ok(())).unwrap();
            tokio::task::yield_now().await;
            assert!(!task.is_finished());
            sender.send(Err("rejected".into())).unwrap();
            drop(sender);
            assert_eq!(task.await.unwrap(), Err("rejected".into()));
        });
    }
}
