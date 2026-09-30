//! Windows: extract cookies from WebView2 via Chrome DevTools Protocol.
//!
//! Uses `Network.getAllCookies` CDP method through `ICoreWebView2::CallDevToolsProtocolMethod`.
//! The callback plumbing relies on `webview2-com`'s handler helper types.
//!
//! NOTE: The exact `webview2-com` version must be compatible with the version
//! that Tauri's `wry` uses internally.  If a version mismatch causes a build
//! error, adjust the version in `Cargo.toml` to match `wry`'s dependency.

use super::CookieData;
use std::sync::{Arc, Mutex};
use tauri::Manager;

type CdpResultSender = Arc<Mutex<Option<tokio::sync::oneshot::Sender<Result<String, String>>>>>;

fn complete_cdp_call(sender: &CdpResultSender, result: Result<String, String>) {
    if let Some(sender) = sender
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
    {
        let _ = sender.send(result);
    }
}

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
    let tx = Arc::new(Mutex::new(Some(tx)));

    win.with_webview(move |webview| {
        unsafe {
            use webview2_com::CallDevToolsProtocolMethodCompletedHandler;

            let core_webview = match webview.controller().CoreWebView2() {
                Ok(core_webview) => core_webview,
                Err(error) => {
                    complete_cdp_call(
                        &tx,
                        Err(format!(
                            "CoreWebView2 is unavailable after SAML loading: {error}"
                        )),
                    );
                    return;
                }
            };

            // Build wide-string parameters for the CDP call.
            let method: Vec<u16> = "Network.getAllCookies\0".encode_utf16().collect();
            let params: Vec<u16> = "{}\0".encode_utf16().collect();

            let handler_tx = tx.clone();
            let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
                move |error_code, return_json| {
                    let result = if error_code.is_ok() {
                        Ok(return_json)
                    } else {
                        Err(format!("CDP call failed: {:?}", error_code))
                    };
                    complete_cdp_call(&handler_tx, result);
                    Ok(())
                },
            ));

            // PCWSTR from windows-core 0.61 matches webview2-com-sys 0.38's expected types.
            if let Err(error) = core_webview.CallDevToolsProtocolMethod(
                windows_core::PCWSTR(method.as_ptr()),
                windows_core::PCWSTR(params.as_ptr()),
                &handler,
            ) {
                complete_cdp_call(
                    &tx,
                    Err(format!(
                        "CallDevToolsProtocolMethod dispatch failed: {error}"
                    )),
                );
            }
        }
    })
    .map_err(|e| format!("with_webview failed: {}", e))?;

    let json = tokio::time::timeout(std::time::Duration::from_secs(10), rx)
        .await
        .map_err(|_| "CDP cookie extraction timed out".to_string())?
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

/// Delete every cookie under the Kwansei Gakuin domain family via CDP.
pub(super) async fn delete_university_cookies(app: &tauri::AppHandle) -> Result<usize, String> {
    let cookies: Vec<CookieData> = extract_all_cookies(app)
        .await?
        .into_iter()
        .filter(|c| super::is_university_cookie(&c.domain))
        .collect();
    let count = cookies.len();

    let win = app
        .get_webview_window("login")
        .or_else(|| app.get_webview_window("kgc-headless"))
        .or_else(|| app.get_webview_window("luna-headless"))
        .or_else(|| app.get_webview_window("kwic-headless"))
        .or_else(|| app.get_webview_window("main"))
        .ok_or("No webview window available for cookie deletion")?;

    win.with_webview(move |webview| unsafe {
        use webview2_com::CallDevToolsProtocolMethodCompletedHandler;

        let core_webview = match webview.controller().CoreWebView2() {
            Ok(core_webview) => core_webview,
            Err(error) => {
                log::warn!("cookie deletion skipped because CoreWebView2 is unavailable: {error}");
                return;
            }
        };

        for cookie in &cookies {
            let params = serde_json::json!({
                "name": cookie.name,
                "domain": cookie.domain,
                "path": cookie.path,
            });
            let params_str = format!("{}\0", params);
            let method: Vec<u16> = "Network.deleteCookies\0".encode_utf16().collect();
            let params_w: Vec<u16> = params_str.encode_utf16().collect();
            let handler =
                CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|_code, _json| Ok(())));

            if let Err(error) = core_webview.CallDevToolsProtocolMethod(
                windows_core::PCWSTR(method.as_ptr()),
                windows_core::PCWSTR(params_w.as_ptr()),
                &handler,
            ) {
                log::warn!("failed to dispatch Network.deleteCookies: {error}");
            }
        }
    })
    .map_err(|e| format!("with_webview failed: {}", e))?;

    Ok(count)
}

/// Write cookies into the WebView2 cookie store via CDP `Network.setCookie`.
/// Best-effort, fire-and-forget per cookie. Returns the count attempted.
pub(super) async fn set_all_cookies(
    app: &tauri::AppHandle,
    cookies: &[CookieData],
) -> Result<usize, String> {
    let win = app
        .get_webview_window("login")
        .or_else(|| app.get_webview_window("kgc-headless"))
        .or_else(|| app.get_webview_window("luna-headless"))
        .or_else(|| app.get_webview_window("kwic-headless"))
        .or_else(|| app.get_webview_window("main"))
        .ok_or("No webview window available for cookie injection")?;

    let cookies = cookies.to_vec();
    let count = cookies.len();

    win.with_webview(move |webview| unsafe {
        use webview2_com::CallDevToolsProtocolMethodCompletedHandler;

        let core_webview = match webview.controller().CoreWebView2() {
            Ok(core_webview) => core_webview,
            Err(error) => {
                log::warn!("cookie injection skipped because CoreWebView2 is unavailable: {error}");
                return;
            }
        };

        for c in &cookies {
            let mut params = serde_json::json!({
                "name": c.name,
                "value": c.value,
                "domain": c.domain.trim_start_matches('.'),
                "path": c.path,
                "secure": c.secure,
                "httpOnly": c.http_only,
            });
            if let Some(exp) = c.expires_unix {
                params["expires"] = serde_json::json!(exp);
            }
            let params_str = format!("{}\0", params);
            let method: Vec<u16> = "Network.setCookie\0".encode_utf16().collect();
            let params_w: Vec<u16> = params_str.encode_utf16().collect();

            let handler =
                CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|_code, _json| Ok(())));

            if let Err(error) = core_webview.CallDevToolsProtocolMethod(
                windows_core::PCWSTR(method.as_ptr()),
                windows_core::PCWSTR(params_w.as_ptr()),
                &handler,
            ) {
                log::warn!("failed to dispatch Network.setCookie: {error}");
            }
        }
    })
    .map_err(|e| format!("with_webview failed: {}", e))?;

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::{complete_cdp_call, Arc, Mutex};

    #[test]
    fn cdp_completion_uses_only_the_first_result() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let sender = Arc::new(Mutex::new(Some(tx)));

        complete_cdp_call(&sender, Ok("cookies".to_string()));
        complete_cdp_call(&sender, Err("late failure".to_string()));

        assert_eq!(
            rx.blocking_recv().expect("CDP result"),
            Ok("cookies".to_string())
        );
    }
}
