//! Windows: extract cookies from WebView2 via Chrome DevTools Protocol.
//!
//! Uses `Network.getAllCookies` CDP method through `ICoreWebView2::CallDevToolsProtocolMethod`.
//! The callback plumbing relies on `webview2-com`'s handler helper types.
//!
//! NOTE: The exact `webview2-com` version must be compatible with the version
//! that Tauri's `wry` uses internally.  If a version mismatch causes a build
//! error, adjust the version in `Cargo.toml` to match `wry`'s dependency.

use super::CookieData;
use tauri::Manager;

// ── CDP JSON response structs ───────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct CdpCookiesResponse {
    cookies: Vec<CdpCookie>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CdpCookie {
    name: String,
    value: String,
    domain: String,
    path: String,
    expires: f64,
    http_only: bool,
    secure: bool,
    session: bool,
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Extract all cookies from the WebView2 cookie store using CDP.
///
/// Finds any active `WebviewWindow`, dispatches to its main thread via
/// `with_webview`, calls the DevTools protocol, and parses the JSON result.
pub(super) async fn extract_all_cookies(app: &tauri::AppHandle) -> Result<Vec<CookieData>, String> {
    // Try to find an active webview window (in priority order).
    let win = app
        .get_webview_window("login")
        .or_else(|| app.get_webview_window("kgc-headless"))
        .or_else(|| app.get_webview_window("luna-headless"))
        .or_else(|| app.get_webview_window("kwic-headless"))
        .or_else(|| app.get_webview_window("main"))
        .ok_or("No webview window available for cookie extraction")?;

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    let tx = std::sync::Mutex::new(Some(tx));

    win.with_webview(move |webview| {
        unsafe {
            use webview2_com::Microsoft::Web::WebView2::Win32::*;
            use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
            use webview2_com::string_from_pcwstr;

            let core_webview = webview
                .controller()
                .CoreWebView2()
                .expect("CoreWebView2 must be available after SAML loading");

            // Build wide-string parameters for the CDP call.
            let method: Vec<u16> = "Network.getAllCookies\0".encode_utf16().collect();
            let params: Vec<u16> = "{}\0".encode_utf16().collect();

            let handler = CallDevToolsProtocolMethodCompletedHandler::create(
                Box::new(
                    move |error_code, return_json| {
                        let result = if error_code.is_ok() {
                            Ok(string_from_pcwstr(return_json))
                        } else {
                            Err(format!("CDP call failed: {:#010x}", error_code.0))
                        };
                        if let Some(sender) =
                            tx.lock().unwrap_or_else(|e| e.into_inner()).take()
                        {
                            let _ = sender.send(result);
                        }
                        Ok(())
                    },
                ),
            );

            core_webview
                .CallDevToolsProtocolMethod(
                    windows_core::PCWSTR(method.as_ptr()),
                    windows_core::PCWSTR(params.as_ptr()),
                    &handler,
                )
                .expect("CallDevToolsProtocolMethod dispatch failed");
        }
    })
    .map_err(|e| format!("with_webview failed: {}", e))?;

    let json = rx
        .await
        .map_err(|_| "Cookie extraction channel closed".to_string())?
        .map_err(|e| format!("CDP cookie extraction failed: {}", e))?;

    let response: CdpCookiesResponse = serde_json::from_str(&json)
        .map_err(|e| format!("Failed to parse CDP cookie response: {}", e))?;

    Ok(response
        .cookies
        .into_iter()
        .map(|c| CookieData {
            name: c.name,
            value: c.value,
            domain: c.domain,
            path: c.path,
            secure: c.secure,
            http_only: c.http_only,
            expires_unix: if c.session { None } else { Some(c.expires) },
        })
        .collect())
}
