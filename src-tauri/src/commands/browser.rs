use crate::config;
use tauri::Manager;

/// Open an external URL in a new webview window with browser toolbar
#[tauri::command]
pub async fn open_external_url(
    app: tauri::AppHandle,
    url: String,
    title: Option<String>,
) -> Result<(), String> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    let parsed_url: url::Url = url.parse().map_err(|e| format!("URL parse error: {}", e))?;

    let scheme = parsed_url.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(format!("Unsupported URL scheme: {}", scheme));
    }

    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = format!("ext-{}", id);
    let win_title = title.unwrap_or_else(|| parsed_url.host_str().unwrap_or("Web").to_string());

    crate::webview_toolbar::create_browser_window(
        &app,
        &label,
        tauri::WebviewUrl::External(parsed_url),
        &win_title,
        900.0,
        640.0,
        &[],
    )?;

    Ok(())
}

/// Open a URL in the system default browser (Safari, Chrome, etc.)
#[tauri::command]
pub async fn open_in_system_browser(app: tauri::AppHandle, url: String) -> Result<(), String> {
    let parsed: url::Url = url.parse().map_err(|e| format!("URL parse error: {}", e))?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(format!("Unsupported URL scheme: {}", scheme));
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(&url, None::<&str>)
        .map_err(|e| format!("ブラウザを開けませんでした: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn open_profile_edit_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_window("profile-edit") {
        let _ = win.set_focus();
        return Ok(());
    }

    let url: url::Url = format!("{}/uniasv2/UnSSOLoginControl2?REQ_LOGIN_NO=2&REQ_ACTION_DO=/GGA110.do&REQ_PRFR_MNU_ID=MNUIDSTD0104011", config::KG_COURSE_BASE)
        .parse()
        .map_err(|e| format!("URL parse error: {}", e))?;

    crate::webview_toolbar::create_browser_window(
        &app,
        "profile-edit",
        tauri::WebviewUrl::External(url),
        "個人情報編集",
        1000.0,
        720.0,
        &[],
    )?;

    Ok(())
}

#[tauri::command]
pub async fn open_facility_reservation(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_window("facility-rsv") {
        let _ = win.set_focus();
        return Ok(());
    }

    let url: url::Url = "https://facility-rsv.kwansei.ac.jp/ss/top"
        .parse()
        .map_err(|e| format!("URL parse error: {}", e))?;

    crate::webview_toolbar::create_browser_window(
        &app,
        "facility-rsv",
        tauri::WebviewUrl::External(url),
        "施設予約",
        1100.0,
        780.0,
        &[],
    )?;

    Ok(())
}

#[tauri::command]
pub async fn open_registration_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_window("registration") {
        let _ = win.set_focus();
        return Ok(());
    }

    let url: url::Url = format!("{}/uniasv2/UnSSOLoginControl2?REQ_LOGIN_NO=2&REQ_ACTION_DO=/ARD010.do&REQ_PRFR_MNU_ID=MNUIDSTD0102012&SE_LANGUAGE=", config::KG_COURSE_BASE)
        .parse()
        .map_err(|e| format!("URL parse error: {}", e))?;

    crate::webview_toolbar::create_browser_window(
        &app,
        "registration",
        tauri::WebviewUrl::External(url),
        "履修登録",
        1100.0,
        780.0,
        &[],
    )?;

    Ok(())
}

#[tauri::command]
pub async fn open_downloads_window(
    app: tauri::AppHandle,
    focus_course: Option<String>,
) -> Result<(), String> {
    use tauri::Emitter;
    let course = focus_course
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| super::downloads::simplify_course_name(s).trim().to_string())
        .filter(|s| !s.is_empty());

    if let Some(win) = app.get_webview_window("downloads") {
        let _ = win.set_focus();
        if let Some(c) = &course {
            let _ = win.emit_to("downloads", "focus-course", c);
        } else {
            let _ = win.emit_to("downloads", "focus-course", "");
        }
        return Ok(());
    }

    // Encode the course name into the URL hash so the page can read it
    // synchronously on first paint without waiting on an event.
    let url_with_hash = if let Some(c) = &course {
        let encoded = url::form_urlencoded::byte_serialize(c.as_bytes()).collect::<String>();
        format!("downloads.html#course={}", encoded)
    } else {
        "downloads.html".to_string()
    };

    tauri::WebviewWindowBuilder::new(
        &app,
        "downloads",
        tauri::WebviewUrl::App(url_with_hash.into()),
    )
    .title("資料管理")
    .inner_size(900.0, 600.0)
    .min_inner_size(640.0, 380.0)
    .resizable(true)
    .build()
    .map_err(|e| format!("Failed to open downloads window: {}", e))?;

    Ok(())
}
