//! KWIC detail, cabinet, and external link windows.

use crate::config;

/// Open a KWIC Portal notification detail in a native detail window
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri command arguments map directly to frontend invoke fields.
pub async fn kwic_open_detail_window(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    title: String,
    information_id: String,
    information_type: String,
    person_category_cd: String,
    category_cd: String,
    split: Option<bool>,
) -> Result<(), String> {
    let encoded_id = urlencoding::encode(&information_id);
    let encoded_type = urlencoding::encode(&information_type);
    let encoded_person = urlencoding::encode(&person_category_cd);
    let encoded_cat = urlencoding::encode(&category_cd);
    let encoded_title = urlencoding::encode(&title);
    let params = format!(
        "mode=kwic&informationId={}&informationType={}&personCategoryCd={}&categoryCd={}&title={}",
        encoded_id, encoded_type, encoded_person, encoded_cat, encoded_title,
    );
    if split.unwrap_or(false) {
        crate::document_tabs::open_child_detail(&app, params, title, webview.label())?;
    } else {
        crate::document_tabs::open_university_detail_tab(&app, params, title)?;
    }

    Ok(())
}

#[tauri::command]
pub async fn kwic_open_cabinet_window(
    app: tauri::AppHandle,
    title: Option<String>,
) -> Result<(), String> {
    let title = title.unwrap_or_else(|| "学生キャビネット".to_string());
    let encoded_title = urlencoding::encode(&title);
    let params = format!("mode=kwicCabinet&title={}", encoded_title);
    crate::document_tabs::open_university_detail_tab(&app, params, title)?;

    Ok(())
}

/// Open a link from the KWIC Portal subportal.
/// For kwansei.ac.jp domains, open in a webview window with cookies injected from reqwest.
/// For external domains, open in the system browser.
#[tauri::command]
pub async fn kwic_open_link(
    app: tauri::AppHandle,
    url: String,
    title: String,
) -> Result<(), String> {
    // Only allow http/https
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("無効なURLスキームです".into());
    }

    // Check if this is a kwansei domain → open in webview
    let is_kwansei = url.contains("kwansei.ac.jp");
    let is_kwic = url.contains("kwic.kwansei.ac.jp");

    if is_kwansei {
        if is_kwic {
            // KWIC Portal: needs special handling because KWIC shows its own login page
            // instead of redirecting to Okta SSO directly.
            // Solution: navigate to KWIC's SAML login URL first (which goes directly to Okta SSO).
            // WKWebView shares Okta SSO cookies from the login flow, so Okta auto-authenticates.
            // After SAML completes, KWIC sets session cookies and redirects to /portal/home.
            // Our initialization_script then redirects to the actual target URL.
            let saml_url: url::Url = config::KWIC_SAML_URL
                .parse()
                .expect("hardcoded KWIC SAML URL is valid");

            // Escape the target URL for safe embedding in JS
            let escaped_url = url
                .replace('\\', "\\\\")
                .replace('\'', "\\'")
                .replace('<', "\\x3c")
                .replace('>', "\\x3e");

            // Script runs on every page load in this webview.
            // When we land on a KWIC portal page (= authenticated), redirect to target.
            // sessionStorage prevents infinite redirect loop.
            let redirect_script = format!(
                r#"(function() {{
                    if (window.location.hostname === 'kwic.kwansei.ac.jp'
                        && window.location.pathname.startsWith('/portal/')
                        && !sessionStorage.getItem('__kwic_nav_done')) {{
                        sessionStorage.setItem('__kwic_nav_done', '1');
                        window.location.replace('{}');
                    }}
                }})();"#,
                escaped_url
            );

            crate::document_tabs::open_external_tab_with_scripts(
                &app,
                saml_url.to_string(),
                Some(title),
                &[&redirect_script],
            )?;
        } else {
            // Other kwansei.ac.jp domains (kg-course, library, etc.)
            // These redirect directly to Okta SSO, which auto-authenticates
            // via shared WKWebView cookies. No special handling needed.
            let parsed: url::Url = url.parse().map_err(|e| format!("URL parse error: {}", e))?;

            crate::document_tabs::open_external_tab(&app, parsed.to_string(), Some(title))?;
        }
    } else {
        // External link → in-app browser webview
        crate::commands::open_external_url(app, url, Some(title)).await?;
    }

    Ok(())
}
