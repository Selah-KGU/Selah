//! WebView2 cookie operations. Every CDP response is awaited and checked.
use super::CookieData;
use std::sync::{Arc, Mutex};
use tauri::Manager;

type CdpResultSender = Arc<Mutex<Option<tokio::sync::oneshot::Sender<Result<String, String>>>>>;

fn complete_cdp_call(sender: &CdpResultSender, result: Result<String, String>) {
    if let Some(sender) = sender.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = sender.send(result);
    }
}

async fn call_cdp(
    app: &tauri::AppHandle,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let win = [
        "login",
        "kgc-headless",
        "luna-headless",
        "kwic-headless",
        "main",
    ]
    .into_iter()
    .find_map(|label| app.get_webview_window(label))
    .ok_or("No webview window available for cookie operation")?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let tx = Arc::new(Mutex::new(Some(tx)));
    let method: Vec<u16> = format!("{method}\0").encode_utf16().collect();
    let params: Vec<u16> = format!("{params}\0").encode_utf16().collect();
    win.with_webview(move |webview| unsafe {
        use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
        let core = match webview.controller().CoreWebView2() {
            Ok(core) => core,
            Err(e) => {
                complete_cdp_call(&tx, Err(format!("CoreWebView2 unavailable: {e}")));
                return;
            }
        };
        let handler_tx = tx.clone();
        let handler =
            CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |code, json| {
                complete_cdp_call(
                    &handler_tx,
                    if code.is_ok() {
                        Ok(json)
                    } else {
                        Err(format!("Cookie operation failed: {code:?}"))
                    },
                );
                Ok(())
            }));
        if let Err(e) = core.CallDevToolsProtocolMethod(
            windows_core::PCWSTR(method.as_ptr()),
            windows_core::PCWSTR(params.as_ptr()),
            &handler,
        ) {
            complete_cdp_call(&tx, Err(format!("Cookie operation dispatch failed: {e}")));
        }
    })
    .map_err(|e| format!("Cookie webview dispatch failed: {e}"))?;
    let json = tokio::time::timeout(std::time::Duration::from_secs(10), rx)
        .await
        .map_err(|_| "Cookie operation timed out")?
        .map_err(|_| "Cookie operation channel closed")??;
    super::cdp_cookie::parse_response(&json)
}

use super::cdp_cookie::{set_cookie_params, CdpCookie};

async fn read_cdp_cookies(app: &tauri::AppHandle) -> Result<Vec<CdpCookie>, String> {
    let response = call_cdp(app, "Network.getAllCookies", serde_json::json!({})).await?;
    serde_json::from_value(response["cookies"].clone())
        .map_err(|_| "Invalid cookie collection".into())
}

pub(super) async fn extract_all_cookies(app: &tauri::AppHandle) -> Result<Vec<CookieData>, String> {
    let cookies = read_cdp_cookies(app).await?;
    // Partitioned cookies require a partition-aware store; never flatten them
    // into unpartitioned cookies with broader access.
    Ok(cookies
        .into_iter()
        .filter(|c| c.partition_key.is_none() && !c.partition_key_opaque)
        .map(|c| CookieData {
            host_only: Some(!c.domain.starts_with('.')),
            name: c.name,
            value: c.value,
            domain: c.domain,
            path: c.path,
            secure: c.secure,
            http_only: c.http_only,
            expires_unix: if c.session { None } else { Some(c.expires) },
            same_site: c.same_site,
        })
        .collect())
}

pub(super) async fn delete_university_cookies(app: &tauri::AppHandle) -> Result<usize, String> {
    let cookies = read_cdp_cookies(app).await?;
    let mut deleted = 0;
    for c in cookies
        .iter()
        .filter(|c| super::is_university_cookie(&c.domain))
    {
        let mut params = serde_json::json!({
            "name": c.name, "domain": c.domain, "path": c.path,
        });
        if let Some(partition) = &c.partition_key {
            params["partitionKey"] = partition.clone();
        }
        call_cdp(app, "Network.deleteCookies", params).await?;
        deleted += 1;
    }
    Ok(deleted)
}

pub(super) async fn set_all_cookies(
    app: &tauri::AppHandle,
    cookies: &[CookieData],
) -> Result<usize, String> {
    let mut stored = 0;
    for c in cookies {
        let params = set_cookie_params(c);
        let response = call_cdp(app, "Network.setCookie", params).await?;
        if response["success"].as_bool() != Some(true) {
            return Err("WebView2 rejected an SSO cookie".into());
        }
        stored += 1;
    }
    Ok(stored)
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
