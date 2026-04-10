//! Cookie Bridge: extract cookies from WKWebView's native cookie store
//! and inject them into reqwest cookie jars.
//!
//! This replaces the fragile SAMLResponse interception approach by letting
//! the webview complete SAML authentication natively, then extracting
//! the resulting session cookies via the WKHTTPCookieStore ObjC API.

use std::ptr::NonNull;

use objc2::MainThreadMarker;
use objc2_foundation::{NSArray, NSHTTPCookie};
use objc2_web_kit::WKWebsiteDataStore;
use tauri::Manager;

/// Plain cookie data extracted from the webview (Send + Sync safe).
#[derive(Debug, Clone)]
struct CookieData {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    /// Unix timestamp (seconds since epoch) when the cookie expires.
    /// None for session cookies.
    pub expires_unix: Option<f64>,
}

/// Extract all cookies from the default WKWebsiteDataStore.
/// Dispatches to the main thread since WKWebKit APIs are main-thread-only.
async fn extract_all_cookies(app: &tauri::AppHandle) -> Result<Vec<CookieData>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Vec<CookieData>>();
    let tx = std::sync::Mutex::new(Some(tx));

    app.run_on_main_thread(move || {
        // SAFETY: run_on_main_thread guarantees we're on the main thread
        let mtm = unsafe { MainThreadMarker::new_unchecked() };
        let data_store = unsafe { WKWebsiteDataStore::defaultDataStore(mtm) };
        let http_cookie_store = unsafe { data_store.httpCookieStore() };

        let block = block2::RcBlock::new(
            move |cookies_ptr: NonNull<NSArray<NSHTTPCookie>>| {
                let cookies = unsafe { cookies_ptr.as_ref() };
                let count = cookies.count();
                let mut result = Vec::with_capacity(count);
                for i in 0..count {
                    let c = cookies.objectAtIndex(i);
                    let expires_unix = c.expiresDate()
                        .map(|d| d.timeIntervalSince1970());
                    result.push(CookieData {
                        name: c.name().to_string(),
                        value: c.value().to_string(),
                        domain: c.domain().to_string(),
                        path: c.path().to_string(),
                        secure: c.isSecure(),
                        http_only: c.isHTTPOnly(),
                        expires_unix,
                    });
                }
                if let Some(sender) = tx.lock().unwrap_or_else(|e| e.into_inner()).take() {
                    let _ = sender.send(result);
                }
            },
        );

        unsafe { http_cookie_store.getAllCookies(&block) };
    })
    .map_err(|e| format!("Main thread dispatch failed: {}", e))?;

    rx.await
        .map_err(|_| "Cookie extraction failed: channel closed".to_string())
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
            // Match exact domain or parent domain cookie (e.g. ".kwansei.ac.jp" matches "kg-course.kwansei.ac.jp")
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
/// Returns true if the URL is on the SP domain and not at an auth-related path.
pub fn is_post_saml_sp_url(url: &url::Url, sp_host: &str) -> bool {
    let host = url.host_str().unwrap_or("");
    if host != sp_host {
        return false;
    }
    let path = url.path();
    // Filter out SAML/SSO/Shibboleth paths that are part of the auth flow
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
///
/// This is the standard cookie bridge flow used after every SAML authentication:
/// 1. Extract cookies matching the SP subdomain (e.g. "luna.kwansei.ac.jp")
/// 2. Extract parent domain SSO cookies ("kwansei.ac.jp")
/// 3. Inject both sets into the reqwest cookie jar
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

/// Create a hidden WebView that navigates to a SAML entry URL and waits for the
/// SP page to finish loading. Returns `Ok(Some(window))` when SAML completes
/// (the caller should extract cookies then close the window), `Ok(None)` when
/// the Okta session has expired (timeout), or `Err` on build failure.
///
/// This is the shared core of all headless refresh flows.
/// Okta/SSO domains — if the headless WebView lands here, the session is expired
/// and we can abort immediately instead of waiting for the full timeout.
const OKTA_HOSTS: &[&str] = &["sso.kwansei.ac.jp", "idp.kwansei.ac.jp", "sts.kwansei.ac.jp"];

fn is_okta_login_page(url: &url::Url) -> bool {
    let host = url.host_str().unwrap_or("");
    OKTA_HOSTS.iter().any(|h| host == *h)
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

    // Channel sends `true` = SAML complete (SP page loaded),
    //                `false` = Okta login page detected (session expired).
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
            // Okta login page detected — session is expired, abort immediately
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


