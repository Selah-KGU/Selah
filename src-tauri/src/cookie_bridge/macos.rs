//! macOS: extract cookies from WKWebView's WKHTTPCookieStore via ObjC API.

use std::ptr::NonNull;

use objc2::MainThreadMarker;
use objc2_foundation::{NSArray, NSDictionary, NSHTTPCookie, NSString, NSURL};
use objc2_web_kit::WKWebsiteDataStore;

use super::CookieData;

/// Extract all cookies from the default WKWebsiteDataStore.
/// Dispatches to the main thread since WKWebKit APIs are main-thread-only.
pub(super) async fn extract_all_cookies(app: &tauri::AppHandle) -> Result<Vec<CookieData>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Vec<CookieData>>();
    let tx = std::sync::Mutex::new(Some(tx));

    app.run_on_main_thread(move || {
        // SAFETY: run_on_main_thread guarantees we're on the main thread
        let mtm = unsafe { MainThreadMarker::new_unchecked() };
        let data_store = unsafe { WKWebsiteDataStore::defaultDataStore(mtm) };
        let http_cookie_store = unsafe { data_store.httpCookieStore() };

        let block = block2::RcBlock::new(move |cookies_ptr: NonNull<NSArray<NSHTTPCookie>>| {
            let cookies = unsafe { cookies_ptr.as_ref() };
            let count = cookies.count();
            let mut result = Vec::with_capacity(count);
            for i in 0..count {
                let c = cookies.objectAtIndex(i);
                let expires_unix = c.expiresDate().map(|d| d.timeIntervalSince1970());
                result.push(CookieData {
                    name: c.name().to_string(),
                    value: c.value().to_string(),
                    domain: c.domain().to_string(),
                    path: c.path().to_string(),
                    secure: c.isSecure(),
                    http_only: c.isHTTPOnly(),
                    expires_unix,
                    same_site: c.sameSitePolicy().map(|site| site.to_string()),
                    host_only: Some(!c.domain().to_string().starts_with('.')),
                });
            }
            if let Some(sender) = tx.lock().unwrap_or_else(|e| e.into_inner()).take() {
                let _ = sender.send(result);
            }
        });

        unsafe { http_cookie_store.getAllCookies(&block) };
    })
    .map_err(|e| format!("Main thread dispatch failed: {}", e))?;

    tokio::time::timeout(std::time::Duration::from_secs(10), rx)
        .await
        .map_err(|_| "Cookie extraction timed out".to_string())?
        .map_err(|_| "Cookie extraction failed: channel closed".to_string())
}

/// Delete all university cookies and await each native completion callback.
pub(super) async fn delete_university_cookies(app: &tauri::AppHandle) -> Result<usize, String> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    app.run_on_main_thread(move || {
        let mtm = unsafe { MainThreadMarker::new_unchecked() };
        let data_store = unsafe { WKWebsiteDataStore::defaultDataStore(mtm) };
        let store = unsafe { data_store.httpCookieStore() };
        let delete_store = store.clone();
        let block = block2::RcBlock::new(move |cookies_ptr: NonNull<NSArray<NSHTTPCookie>>| {
            let cookies = unsafe { cookies_ptr.as_ref() };
            for i in 0..cookies.count() {
                let cookie = cookies.objectAtIndex(i);
                if super::is_university_cookie(&cookie.domain().to_string()) {
                    let done = tx.clone();
                    let completion = block2::RcBlock::new(move || {
                        let _ = done.send(Ok(()));
                    });
                    unsafe {
                        delete_store.deleteCookie_completionHandler(&cookie, Some(&completion))
                    };
                }
            }
            // WebKit releases this block after the callback, then only the
            // per-cookie completions retain senders.
        });
        unsafe { store.getAllCookies(&block) };
    })
    .map_err(|e| format!("Main thread dispatch failed: {e}"))?;
    super::await_cookie_completions(rx).await
}

/// Write cookies into the default WKHTTPCookieStore. Each `CookieData` is turned
/// into a `Set-Cookie` header and parsed back into an `NSHTTPCookie` (avoids
/// hand-building the property dictionary). Returns the number stored.
pub(super) async fn set_all_cookies(
    app: &tauri::AppHandle,
    cookies: &[CookieData],
) -> Result<usize, String> {
    let cookies = cookies.to_vec();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    app.run_on_main_thread(move || {
        // SAFETY: run_on_main_thread guarantees we're on the main thread
        let mtm = unsafe { MainThreadMarker::new_unchecked() };
        let data_store = unsafe { WKWebsiteDataStore::defaultDataStore(mtm) };
        let store = unsafe { data_store.httpCookieStore() };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let key = NSString::from_str("Set-Cookie");

        for c in &cookies {
            let bare_domain = c.domain.trim_start_matches('.');
            let mut header = format!("{}={}; Path={}", c.name, c.value, c.path);
            if !c.is_host_only() {
                header.push_str(&format!("; Domain={}", c.domain));
            }
            if let Some(site) = c.normalized_same_site() {
                header.push_str(&format!("; SameSite={site}"));
            }
            if c.secure {
                header.push_str("; Secure");
            }
            if c.http_only {
                header.push_str("; HttpOnly");
            }
            if let Some(exp) = c.expires_unix {
                let max_age = (exp - now).max(0.0) as i64;
                header.push_str(&format!("; Max-Age={}", max_age));
            }
            let header_ns = NSString::from_str(&header);
            let dict = NSDictionary::<NSString, NSString>::from_slices(&[&*key], &[&*header_ns]);
            let url_ns = NSString::from_str(&format!("https://{}/", bare_domain));
            let Some(url) = NSURL::URLWithString(&url_ns) else {
                let _ = tx.send(Err("Invalid cookie origin".into()));
                continue;
            };
            let parsed = NSHTTPCookie::cookiesWithResponseHeaderFields_forURL(&dict, &url);
            if parsed.count() == 0 {
                let _ = tx.send(Err("WebKit rejected an SSO cookie".into()));
            }
            for i in 0..parsed.count() {
                let cookie = parsed.objectAtIndex(i);
                let done = tx.clone();
                let completion = block2::RcBlock::new(move || {
                    let _ = done.send(Ok(()));
                });
                unsafe { store.setCookie_completionHandler(&cookie, Some(&completion)) };
            }
        }
    })
    .map_err(|e| format!("Main thread dispatch failed: {}", e))?;

    super::await_cookie_completions(rx).await
}
