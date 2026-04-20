//! macOS: extract cookies from WKWebView's WKHTTPCookieStore via ObjC API.

use std::ptr::NonNull;

use objc2::MainThreadMarker;
use objc2_foundation::{NSArray, NSHTTPCookie};
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
                });
            }
            if let Some(sender) = tx.lock().unwrap_or_else(|e| e.into_inner()).take() {
                let _ = sender.send(result);
            }
        });

        unsafe { http_cookie_store.getAllCookies(&block) };
    })
    .map_err(|e| format!("Main thread dispatch failed: {}", e))?;

    rx.await
        .map_err(|_| "Cookie extraction failed: channel closed".to_string())
}
