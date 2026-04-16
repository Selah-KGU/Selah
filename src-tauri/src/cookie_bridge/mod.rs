//! Cookie Bridge: extract cookies from the platform's native webview cookie store
//! and inject them into reqwest cookie jars.
//!
//! - macOS: WKHTTPCookieStore (ObjC API via objc2)
//! - Windows: WebView2 Chrome DevTools Protocol (CDP)

use tauri::Manager;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos::extract_all_cookies;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use self::windows::extract_all_cookies;

/// Plain cookie data extracted from the webview (Send + Sync safe).
#[derive(Debug, Clone)]
pub(crate) struct CookieData {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub expires_unix: Option<f64>,
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
            cookie_domain == domain_owned
                || domain_owned.ends_with(&format!(".{}", cookie_domain))
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
        let mut builder = cookie_store::RawCookie::build((&*c.name, &*c.value))
            .domain(&*c.domain)
            .path(&*c.path);
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
        match jar.insert_raw(&raw, &url) {
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

const OKTA_HOSTS: &[&str] = &["sso.kwansei.ac.jp", "idp.kwansei.ac.jp", "sts.kwansei.ac.jp"];

fn is_okta_login_page(url: &url::Url) -> bool {
    let host = url.host_str().unwrap_or("");
    OKTA_HOSTS.contains(&host)
}

pub async fn headless_saml_window(
    app: &tauri::AppHandle,
    window_label: &str,
    saml_url: &str,
    sp_domain: &str,
    timeout_secs: u64,
) -> Result<Option<tauri::WebviewWindow>, String> {
    if let Some(w) = app.get_webview_window(window_label) {
        let _ = w.close();
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<bool>(1);

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
        if is_post_saml_sp_url(url, &sp_domain_owned) {
            log::info!("{}: page loaded on SP domain", label_for_log);
            let _ = tx.try_send(true);
        } else if is_okta_login_page(url) {
            log::info!("{}: Okta login page detected - session expired", label_for_log);
            let _ = tx.try_send(false);
        }
    })
    .build()
    .map_err(|e| format!("Failed to build headless window '{}': {}", window_label, e))?;

    match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), rx.recv()).await {
        Ok(Some(true)) => {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            Ok(Some(win))
        }
        Ok(Some(false)) => {
            let _ = win.close();
            Ok(None)
        }
        Ok(None) => {
            log::info!("{}: window closed without completing", window_label);
            Ok(None)
        }
        Err(_) => {
            log::info!("{}: timed out - Okta session likely expired", window_label);
            let _ = win.close();
            Ok(None)
        }
    }
}
