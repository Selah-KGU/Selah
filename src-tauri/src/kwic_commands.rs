use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::auth;
use crate::AppState;

const KWIC_SAML_CALLBACK_HOST: &str = "kwic-saml-callback.localhost";

static KWIC_DETAIL_COUNTER: AtomicU32 = AtomicU32::new(0);

// ============ Types ============

/// A notification/information entry from the KWIC Portal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KwicPortalNotification {
    pub id: String,
    pub title: String,
    pub date: String,
    pub category: String,
    pub important: bool,
    /// data2: informationType (e.g. "10")
    pub information_type: String,
    /// data3: personCategoryCd (e.g. "0")
    pub person_category_cd: String,
    /// data4: categoryCd (e.g. "02")
    pub category_cd: String,
}

/// The home page data from KWIC Portal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KwicPortalHome {
    /// Category sections on the home page
    pub sections: Vec<KwicPortalSection>,
    /// Raw HTML for debug/exploration (only in debug mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_html_debug: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KwicPortalSection {
    pub title: String,
    pub items: Vec<KwicPortalItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KwicPortalItem {
    pub id: String,
    pub title: String,
    pub date: String,
    pub category: String,
    pub url: String,
    pub important: bool,
    #[serde(default)]
    pub information_type: String,
    #[serde(default)]
    pub person_category_cd: String,
    #[serde(default)]
    pub category_cd: String,
}

// ============ Commands ============

/// Check KWIC Portal session
#[tauri::command]
pub async fn kwic_check_session(state: State<'_, AppState>) -> Result<bool, String> {
    let kwic = state.kwic.lock().await;
    if !kwic.authenticated {
        return Ok(false);
    }
    // Validate by fetching the home page
    match kwic.fetch_page("/portal/home").await {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// Fetch a KWIC Portal page (generic — for exploration)
#[tauri::command]
pub async fn kwic_fetch_page(
    state: State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    // Only allow paths under the KWIC portal
    if !path.starts_with("/portal/") && !path.starts_with("/api/") {
        return Err("許可されていないパスです".into());
    }
    let kwic = state.kwic.lock().await;
    let html = kwic.fetch_page(&path).await?;
    // In debug builds, also dump to /tmp for analysis
    #[cfg(debug_assertions)]
    {
        let safe_name = path.replace('/', "_").replace('?', "_");
        let _ = std::fs::write(format!("/tmp/kwic-portal{}.html", safe_name), &html);
    }
    Ok(html)
}

/// Fetch and parse the KWIC Portal home page
#[tauri::command]
pub async fn kwic_fetch_home(
    state: State<'_, AppState>,
) -> Result<KwicPortalHome, String> {
    let kwic = state.kwic.lock().await;
    let html = kwic.fetch_page("/portal/home").await?;

    #[cfg(debug_assertions)]
    { let _ = std::fs::write("/tmp/kwic-portal-home.html", &html); }

    let sections = parse_portal_home(&html);

    Ok(KwicPortalHome {
        sections,
        #[cfg(debug_assertions)]
        raw_html_debug: Some(html[..5000.min(html.len())].to_string()),
        #[cfg(not(debug_assertions))]
        raw_html_debug: None,
    })
}

/// Fetch KWIC Portal notifications/information list
#[tauri::command]
pub async fn kwic_fetch_notifications(
    state: State<'_, AppState>,
) -> Result<Vec<KwicPortalNotification>, String> {
    let kwic = state.kwic.lock().await;
    // /portal/home/information returns SystemError, so parse notifications from home page
    let html = kwic.fetch_page("/portal/home").await?;

    #[cfg(debug_assertions)]
    { let _ = std::fs::write("/tmp/kwic-portal-notifications-from-home.html", &html); }

    Ok(parse_portal_notifications(&html))
}

/// Parsed detail content of a KWIC Portal notification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KwicNotificationDetail {
    pub title: String,
    pub date: String,
    pub sender: String,
    pub body_html: String,
    /// Attachment file names / links (if any)
    pub attachments: Vec<KwicAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KwicAttachment {
    pub name: String,
    pub url: String,
}

/// Fetch and parse a KWIC Portal notification detail inline (no webview).
/// The detail page is fetched via POST to /portal/home/information/detail
/// using the same form parameters as the portal's #PortalinformationDtl form.
#[tauri::command]
pub async fn kwic_fetch_detail(
    state: State<'_, AppState>,
    information_id: String,
    information_type: String,
    person_category_cd: String,
    category_cd: String,
) -> Result<KwicNotificationDetail, String> {
    let kwic = state.kwic.lock().await;

    // 1. Get home page to extract CSRF token
    let home_html = kwic.fetch_page("/portal/home").await?;
    let csrf = extract_csrf_token(&home_html)
        .ok_or_else(|| "CSRFトークンが取得できませんでした".to_string())?;

    // 2. POST to portal detail endpoint with all required form fields
    //    (mirrors #PortalinformationDtl form + setDetailPortalInfoParam)
    let detail_html = kwic.post_form("/portal/home/information/detail", &[
        ("_csrf", &csrf),
        ("informationId", &information_id),
        ("informationType", &information_type),
        ("personCategoryCd", &person_category_cd),
        ("categoryCd", &category_cd),
        ("selectCategoryCd", &category_cd),
        ("pageViewListNum", "10"),
    ]).await?;

    #[cfg(debug_assertions)]
    { let _ = std::fs::write("/tmp/kwic-portal-detail.html", &detail_html); }

    // 3. Parse the detail HTML page
    Ok(parse_detail_html(&detail_html))
}

/// A link/item from a KWIC Portal subportal page
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KwicSubportalLink {
    pub title: String,
    pub url: String,
    pub icon_url: String,
    pub description: String,
}

/// Subportal page data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KwicSubportalData {
    pub title: String,
    pub links: Vec<KwicSubportalLink>,
    /// Notification items on this subportal
    pub notifications: Vec<KwicPortalNotification>,
}

/// Fetch and parse a KWIC Portal subportal page (e.g. /portal/subportal?tagCd=1)
#[tauri::command]
pub async fn kwic_fetch_subportal(
    state: State<'_, AppState>,
    tag_cd: String,
) -> Result<KwicSubportalData, String> {
    let kwic = state.kwic.lock().await;
    let path = format!("/portal/subportal?tagCd={}", tag_cd);
    let html = kwic.fetch_page(&path).await?;

    #[cfg(debug_assertions)]
    { let _ = std::fs::write(format!("/tmp/kwic-portal-subportal-{}.html", tag_cd), &html); }

    Ok(parse_subportal(&html))
}

/// Open a KWIC Portal notification detail in a native detail window
#[tauri::command]
pub async fn kwic_open_detail_window(
    app: tauri::AppHandle,
    title: String,
    information_id: String,
    information_type: String,
    person_category_cd: String,
    category_cd: String,
) -> Result<(), String> {
    let id = KWIC_DETAIL_COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = format!("kwic-detail-{}", id);

    let encoded_id = urlencoding::encode(&information_id);
    let encoded_type = urlencoding::encode(&information_type);
    let encoded_person = urlencoding::encode(&person_category_cd);
    let encoded_cat = urlencoding::encode(&category_cd);
    let encoded_title = urlencoding::encode(&title);
    let url_str = format!(
        "kwic-detail.html?informationId={}&informationType={}&personCategoryCd={}&categoryCd={}&title={}",
        encoded_id, encoded_type, encoded_person, encoded_cat, encoded_title,
    );

    tauri::WebviewWindowBuilder::new(
        &app,
        &label,
        tauri::WebviewUrl::App(url_str.into()),
    )
    .title(&title)
    .inner_size(520.0, 600.0)
    .resizable(true)
    .build()
    .map_err(|e| format!("ウィンドウ作成失敗: {}", e))?;

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
        let id = KWIC_DETAIL_COUNTER.fetch_add(1, Ordering::Relaxed);
        let label = format!("kwic-detail-{}", id);

        if is_kwic {
            // KWIC Portal: needs special handling because KWIC shows its own login page
            // instead of redirecting to Okta SSO directly.
            // Solution: navigate to KWIC's SAML login URL first (which goes directly to Okta SSO).
            // WKWebView shares Okta SSO cookies from the login flow, so Okta auto-authenticates.
            // After SAML completes, KWIC sets session cookies and redirects to /portal/home.
            // Our initialization_script then redirects to the actual target URL.
            let saml_url: url::Url = "https://kwic.kwansei.ac.jp/saml/login?disco=true"
                .parse().unwrap();

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

            crate::webview_toolbar::create_browser_window(
                &app,
                &label,
                tauri::WebviewUrl::External(saml_url),
                &title,
                1000.0, 750.0,
                &[&redirect_script],
            )?;
        } else {
            // Other kwansei.ac.jp domains (kg-course, library, etc.)
            // These redirect directly to Okta SSO, which auto-authenticates
            // via shared WKWebView cookies. No special handling needed.
            let parsed: url::Url = url.parse()
                .map_err(|e| format!("URL parse error: {}", e))?;

            crate::webview_toolbar::create_browser_window(
                &app,
                &label,
                tauri::WebviewUrl::External(parsed),
                &title,
                1000.0, 750.0,
                &[],
            )?;
        }
    } else {
        // External link → in-app browser webview
        crate::commands::open_external_url(app, url, Some(title)).await?;
    }

    Ok(())
}

/// Open KWIC Portal login window
#[tauri::command]
pub async fn kwic_open_login(
    app: tauri::AppHandle,
    _state: State<'_, AppState>,
) -> Result<(), String> {
    log::info!("Opening KWIC Portal login webview");

    if let Some(existing) = app.get_webview_window("kwic-login") {
        let _ = existing.close();
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<auth::SamlCallbackData>(1);

    // Navigate directly to KWIC Portal's SAML login URL.
    // The webview shares the WKWebView cookie jar, so if Okta SSO is still alive
    // it will auto-authenticate. Otherwise the user sees the Okta login form.
    let saml_url = "https://kwic.kwansei.ac.jp/saml/login?disco=true";
    let parsed_url: url::Url = saml_url.parse()
        .map_err(|e| format!("URL parse error: {}", e))?;

    let _win = tauri::WebviewWindowBuilder::new(
        &app,
        "kwic-login",
        tauri::WebviewUrl::External(parsed_url),
    )
    .title("KWIC Portal - サインイン")
    .inner_size(480.0, 700.0)
    .resizable(true)
    .initialization_script(&auth::saml_intercept_script(KWIC_SAML_CALLBACK_HOST))
    .on_navigation(move |url| {
        if url.host_str() == Some(KWIC_SAML_CALLBACK_HOST) {
            let pairs: std::collections::HashMap<String, String> =
                url.query_pairs().into_owned().collect();
            if let Some(saml_response) = pairs.get("saml_response") {
                let data = auth::SamlCallbackData {
                    saml_response: saml_response.clone(),
                    relay_state: pairs.get("relay_state").cloned().unwrap_or_default(),
                    acs_url: pairs.get("acs_url").cloned().unwrap_or_default(),
                };
                log::info!("Intercepted KWIC Portal SAMLResponse (len={})", data.saml_response.len());
                let _ = tx.try_send(data);
            }
            return false;
        }
        true
    })
    .build()
    .map_err(|e| format!("KWICログインウィンドウ作成失敗: {}", e))?;

    let app_clone = app.clone();
    tokio::spawn(async move {
        match rx.recv().await {
            Some(data) => {
                let app_state = app_clone.state::<AppState>();
                let mut kwic = app_state.kwic.lock().await;
                match kwic.complete_saml_login(
                    &data.saml_response,
                    &data.relay_state,
                    &data.acs_url,
                ).await {
                    Ok(()) => {
                        log::info!("KWIC Portal login successful");
                        kwic.save_session();
                        let _ = app_clone.emit("kwic-login-success", ());
                    }
                    Err(e) => {
                        log::error!("KWIC Portal login failed: {}", e);
                        let _ = app_clone.emit("kwic-login-error", &e);
                    }
                }
                if let Some(win) = app_clone.get_webview_window("kwic-login") {
                    let _ = win.close();
                }
            }
            None => {
                log::info!("KWIC Portal login cancelled");
            }
        }
    });

    Ok(())
}

// ============ Parsers ============
// Based on actual KWIC Portal HTML structure (kwic.kwansei.ac.jp)
//
// Home page layout:
//   - .portal-notice: pinned important links
//   - .portal-mainlink: 9 category cards (授業・履修・成績, キャンパスライフ, etc.)
//   - .portal-info-tab: 4 notification tabs
//     - #portalinfocontent1: 呼出し・重要なお知らせ
//     - #portalinfocontent2: 学部・研究科からのお知らせ
//     - #portalinfocontent3: 授業のお知らせ
//     - #portalinfocontent4: その他
//   - Each notification item: li.portal-info-content-li
//     - a[data1=informationId]
//     - .portal-subblock-infolist-left-item2 > div (date)
//     - .portal-subblock-infolist-left-item2 > span (title)
//     - .portal-subblock-infolist-right (department/category)
//     - .portal-information-new (NEW badge)

fn parse_portal_home(html: &str) -> Vec<KwicPortalSection> {
    use scraper::{Html, Selector};

    let document = Html::parse_document(html);
    let mut sections = Vec::new();

    // 1. Parse pinned important links (注目コンテンツ)
    if let Ok(sel) = Selector::parse(".portal-notice-li a.portal-notice-li-a") {
        let items: Vec<KwicPortalItem> = document.select(&sel).filter_map(|a| {
            let title: String = a.text().collect::<Vec<_>>().join(" ").trim().to_string();
            let href = a.value().attr("href").unwrap_or_default();
            if title.is_empty() { return None; }
            Some(KwicPortalItem {
                id: String::new(),
                title,
                date: String::new(),
                category: "注目".to_string(),
                url: href.to_string(),
                important: true,
                information_type: String::new(),
                person_category_cd: String::new(),
                category_cd: String::new(),
            })
        }).collect();
        if !items.is_empty() {
            sections.push(KwicPortalSection {
                title: "注目コンテンツ".to_string(),
                items,
            });
        }
    }

    // 2. Parse notification tabs
    let tab_ids = [
        ("portalinfocontent1", "呼出し・重要なお知らせ"),
        ("portalinfocontent2", "学部・研究科からのお知らせ"),
        ("portalinfocontent3", "授業のお知らせ"),
        ("portalinfocontent4", "その他"),
    ];

    for (tab_id, tab_title) in &tab_ids {
        let selector_str = format!("#{} li.portal-info-content-li", tab_id);
        let sel = match Selector::parse(&selector_str) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let items: Vec<KwicPortalItem> = document.select(&sel).filter_map(|li| {
            parse_info_item(&li).map(|(mut item, d2, d3, d4)| {
                item.information_type = d2;
                item.person_category_cd = d3;
                item.category_cd = d4;
                item
            })
        }).collect();
        if !items.is_empty() {
            sections.push(KwicPortalSection {
                title: tab_title.to_string(),
                items,
            });
        }
    }

    // 3. Parse main link categories (メインリンク)
    if let Ok(sel) = Selector::parse(".portal-mainlink-li a") {
        let items: Vec<KwicPortalItem> = document.select(&sel).filter_map(|a| {
            let title: String = a.text().collect::<Vec<_>>().join(" ").trim().to_string();
            let href = a.value().attr("href").unwrap_or_default();
            if title.is_empty() { return None; }
            Some(KwicPortalItem {
                id: String::new(),
                title,
                date: String::new(),
                category: "リンク".to_string(),
                url: if href.starts_with("http") {
                    href.to_string()
                } else {
                    format!("https://kwic.kwansei.ac.jp{}", href)
                },
                important: false,
                information_type: String::new(),
                person_category_cd: String::new(),
                category_cd: String::new(),
            })
        }).collect();
        if !items.is_empty() {
            sections.push(KwicPortalSection {
                title: "メインリンク".to_string(),
                items,
            });
        }
    }

    sections
}

/// Parse a single notification item from li.portal-info-content-li
/// Returns (KwicPortalItem, data2, data3, data4)
fn parse_info_item(li: &scraper::ElementRef) -> Option<(KwicPortalItem, String, String, String)> {
    // Extract informationId and data attributes from `a[data1]`
    let a_sel = scraper::Selector::parse("a.portal-info-content-li-a").ok()?;
    let a = li.select(&a_sel).next()?;
    let id = a.value().attr("data1").unwrap_or_default().to_string();
    let data2 = a.value().attr("data2").unwrap_or_default().to_string();
    let data3 = a.value().attr("data3").unwrap_or_default().to_string();
    let data4 = a.value().attr("data4").unwrap_or_default().to_string();

    // Date: .portal-subblock-infolist-left-item2 > div
    let date = scraper::Selector::parse(".portal-subblock-infolist-left-item2 > div")
        .ok()
        .and_then(|sel| li.select(&sel).next())
        .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
        .unwrap_or_default();

    // Title: .portal-subblock-infolist-left-item2 > span
    let title = scraper::Selector::parse(".portal-subblock-infolist-left-item2 > span")
        .ok()
        .and_then(|sel| li.select(&sel).next())
        .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
        .unwrap_or_default();

    if title.is_empty() { return None; }

    // Category/department: .portal-subblock-infolist-right
    let category = scraper::Selector::parse(".portal-subblock-infolist-right")
        .ok()
        .and_then(|sel| li.select(&sel).next())
        .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
        .unwrap_or_default();

    // NEW badge
    let is_new = scraper::Selector::parse(".portal-information-new")
        .ok()
        .map(|sel| li.select(&sel).next().is_some())
        .unwrap_or(false);

    Some((KwicPortalItem {
        id: id.clone(),
        title,
        date,
        category,
        url: format!("https://kwic.kwansei.ac.jp/portal/home/information/detail?informationId={}&directLink=1", id),
        important: is_new,
        information_type: String::new(),
        person_category_cd: String::new(),
        category_cd: String::new(),
    }, data2, data3, data4))
}

/// Parse notifications from the home page HTML
/// (The standalone /portal/home/information endpoint returns SystemError,
///  so we parse from the home page instead.)
fn parse_portal_notifications(html: &str) -> Vec<KwicPortalNotification> {
    use scraper::{Html, Selector};
    let document = Html::parse_document(html);
    let mut notifications = Vec::new();

    // Parse all notification items across all tabs
    if let Ok(sel) = Selector::parse("li.portal-info-content-li") {
        for li in document.select(&sel) {
            if let Some((item, data2, data3, data4)) = parse_info_item(&li) {
                notifications.push(KwicPortalNotification {
                    id: item.id,
                    title: item.title,
                    date: item.date,
                    category: item.category,
                    important: item.important,
                    information_type: data2,
                    person_category_cd: data3,
                    category_cd: data4,
                });
            }
        }
    }

    notifications
}

/// Extract CSRF token from KWIC Portal HTML
fn extract_csrf_token(html: &str) -> Option<String> {
    use scraper::{Html, Selector};
    let doc = Html::parse_document(html);
    if let Ok(sel) = Selector::parse(r#"input[name="_csrf"]"#) {
        if let Some(el) = doc.select(&sel).next() {
            return el.value().attr("value").map(|v| v.to_string());
        }
    }
    None
}

/// Parse the detail HTML fragment returned by /lms/course/information/listdetail.
/// This is typically a dialog fragment containing info_preview with title, body, sender, date, attachments.
fn parse_detail_html(html: &str) -> KwicNotificationDetail {
    use scraper::{Html, Selector};
    let doc = Html::parse_document(html);

    let text_of = |selector: &str| -> String {
        Selector::parse(selector).ok()
            .and_then(|sel| doc.select(&sel).next())
            .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
            .unwrap_or_default()
    };

    let html_of = |selector: &str| -> String {
        Selector::parse(selector).ok()
            .and_then(|sel| doc.select(&sel).next())
            .map(|el| el.inner_html().trim().to_string())
            .unwrap_or_default()
    };

    // Real KWIC detail structure:
    // Title: .block-title-txt
    // Body:  #contentsHtml (quill editor content)
    // Sender: .portal-information-outgoing-division (contains "配信部署:" + dept name)
    // Date:  掲載期間 section — we extract from the first .contents-input-area with date-like text
    let title = text_of(".block-title-txt");
    let body_html = html_of("#contentsHtml");

    // Sender: extract department from .portal-information-outgoing-division
    let sender = {
        let raw = text_of(".portal-information-outgoing-division");
        raw.replace("配信部署:", "").trim().to_string()
    };

    // Date: look for 掲載期間 section, then get the spans inside its .contents-input-area
    let date = {
        let mut found = String::new();
        if let (Ok(detail_sel), Ok(header_sel), Ok(input_sel)) = (
            Selector::parse(".contents-detail"),
            Selector::parse(".contents-header-txt .bold-txt"),
            Selector::parse(".contents-input-area"),
        ) {
            for detail in doc.select(&detail_sel) {
                if let Some(header) = detail.select(&header_sel).next() {
                    let header_text = header.text().collect::<Vec<_>>().join("");
                    if header_text.contains("掲載期間") {
                        if let Some(input) = detail.select(&input_sel).next() {
                            found = input.text().collect::<Vec<_>>().join("").trim().to_string();
                        }
                        break;
                    }
                }
            }
        }
        found
    };

    // Attachments: .file-object elements → .downloadFile (name), .objectName (object path)
    let mut attachments = Vec::new();
    if let (Ok(fo_sel), Ok(name_sel), Ok(obj_sel)) = (
        Selector::parse(".file-object"),
        Selector::parse(".downloadFile, .fileName"),
        Selector::parse(".objectName"),
    ) {
        for fo in doc.select(&fo_sel) {
            let name = fo.select(&name_sel).next()
                .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
                .unwrap_or_default();
            let object_name = fo.select(&obj_sel).next()
                .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
                .unwrap_or_default();
            if name.is_empty() { continue; }
            let url = format!(
                "https://kwic.kwansei.ac.jp/portal/home/information/detail/download?downloadFileName={}&objectName={}&downloadMode=1",
                urlencoding::encode(&name),
                urlencoding::encode(&object_name),
            );
            attachments.push(KwicAttachment { name, url });
        }
    }

    // Strip <script> tags from body for safety
    let body_clean = {
        use regex::Regex;
        let re = Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
        re.replace_all(&body_html, "").to_string()
    };

    KwicNotificationDetail {
        title,
        date,
        sender,
        body_html: body_clean,
        attachments,
    }
}

/// Parse a KWIC Portal subportal page.
/// Subportal pages contain link lists and notification items similar to the home page.
fn parse_subportal(html: &str) -> KwicSubportalData {
    use scraper::{Html, Selector};
    let doc = Html::parse_document(html);

    // Page title: .subportal-title-txt
    let page_title = Selector::parse(".subportal-title-txt")
        .ok()
        .and_then(|sel| doc.select(&sel).next())
        .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
        .unwrap_or_default();

    // Links: li.subportal-block-relation-list-li a.subportal-block-txtlink-li-b
    // Each <a> contains <img class="systemlink-image"> (icon) + <span> (title)
    let mut links = Vec::new();
    if let (Ok(a_sel), Ok(img_sel)) = (
        Selector::parse("li.subportal-block-relation-list-li a.subportal-block-txtlink-li-b"),
        Selector::parse("img.systemlink-image"),
    ) {
        for a in doc.select(&a_sel) {
            let title: String = a.text().collect::<Vec<_>>().join("").trim().to_string();
            let href = a.value().attr("href").unwrap_or_default();
            if title.is_empty() || href.is_empty() || href == "#" { continue; }
            if href.starts_with("javascript:") { continue; }
            let url = if href.starts_with("http") {
                href.to_string()
            } else {
                format!("https://kwic.kwansei.ac.jp{}", href)
            };
            let icon_url = a.select(&img_sel).next()
                .and_then(|img| img.value().attr("src"))
                .map(|src| if src.starts_with("http") {
                    src.to_string()
                } else {
                    format!("https://kwic.kwansei.ac.jp{}", src)
                })
                .unwrap_or_default();
            if links.iter().any(|l: &KwicSubportalLink| l.url == url) { continue; }
            links.push(KwicSubportalLink {
                title,
                url,
                icon_url,
                description: String::new(),
            });
        }
    }

    // Notifications: li.subportal-block-info-list-li
    // Structure per item:
    //   .subportal-block-list-li-txt-info1 = category
    //   .subportal-block-list-li-txt-info2 span.link-txt[data1][data2] = title + id + type
    //   .subportal-block-list-li-txt-info3 span:first = date
    //   .subportal-block-list-li-txt-info4 = department
    let mut notifications = Vec::new();
    if let Ok(li_sel) = Selector::parse("li.subportal-block-info-list-li") {
        let cat_sel = Selector::parse(".subportal-block-list-li-txt-info1").ok();
        let title_sel = Selector::parse(".subportal-block-list-li-txt-info2 span.link-txt").ok();
        let date_sel = Selector::parse(".subportal-block-list-li-txt-info3 span").ok();
        let dept_sel = Selector::parse(".subportal-block-list-li-txt-info4").ok();
        let new_sel = Selector::parse(".portal-information-priority-urgency-color").ok();

        for li in doc.select(&li_sel) {
            let title_el = title_sel.as_ref().and_then(|s| li.select(s).next());
            let title_el = match title_el { Some(el) => el, None => continue };

            let id = title_el.value().attr("data1").unwrap_or_default().to_string();
            let data2 = title_el.value().attr("data2").unwrap_or_default().to_string();
            let title = title_el.text().collect::<Vec<_>>().join("").trim().to_string();
            if title.is_empty() { continue; }

            let category = cat_sel.as_ref()
                .and_then(|s| li.select(s).next())
                .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
                .unwrap_or_default();

            let date = date_sel.as_ref()
                .and_then(|s| li.select(s).next())
                .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
                .unwrap_or_default();

            let dept = dept_sel.as_ref()
                .and_then(|s| li.select(s).next())
                .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
                .unwrap_or_default();

            let is_new = new_sel.as_ref()
                .map(|s| li.select(s).next().is_some())
                .unwrap_or(false);

            notifications.push(KwicPortalNotification {
                id,
                title,
                date,
                category: if !dept.is_empty() { dept } else { category },
                important: is_new,
                information_type: data2,
                // Subportal notifications only have data1/data2 in onclick
                person_category_cd: String::new(),
                category_cd: String::new(),
            });
        }
    }

    KwicSubportalData {
        title: page_title,
        links,
        notifications,
    }
}
