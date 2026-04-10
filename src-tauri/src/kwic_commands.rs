use serde::{Deserialize, Serialize};
use tauri::{State};
use std::sync::{atomic::{AtomicU32, Ordering}, LazyLock};

use crate::client;
use crate::config;
use crate::kwic_client;
use crate::AppState;

static KWIC_DETAIL_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Briefly lock KWIC client, check auth and clone http. Releases lock immediately.
async fn kwic_http(state: &AppState) -> Result<reqwest::Client, String> {
    let kwic = state.kwic.lock().await;
    if !kwic.authenticated {
        return Err(kwic_client::KWIC_AUTH_REQUIRED_MSG.into());
    }
    Ok(kwic.http.clone())
}

/// KWIC GET: fetch a page without holding the lock.
async fn kwic_get(http: &reqwest::Client, path: &str) -> Result<String, String> {
    let url = format!("{}{}", config::KWIC_BASE, path);
    client::fetch_with_redirect(
        http, &url, config::KWIC_BASE,
        kwic_client::KWIC_SESSION_EXPIRED_MSG, kwic_client::is_kwic_session_expired,
    ).await
}

/// KWIC POST: submit a form without holding the lock.
async fn kwic_post(http: &reqwest::Client, path: &str, params: &[(&str, &str)]) -> Result<String, String> {
    let url = format!("{}{}", config::KWIC_BASE, path);
    client::post_form_with_redirect(
        http, &url, config::KWIC_BASE,
        kwic_client::KWIC_SESSION_EXPIRED_MSG, kwic_client::is_kwic_session_expired,
        params.iter().copied(),
        &[],
    ).await
}

// ============ Cached Selectors ============

macro_rules! sel {
    ($name:ident, $s:expr) => {
        static $name: LazyLock<scraper::Selector> =
            LazyLock::new(|| scraper::Selector::parse($s).expect(concat!("bad selector: ", $s)));
    };
}

sel!(SEL_NOTICE_A, ".portal-notice-li a.portal-notice-li-a");
sel!(SEL_MAINLINK_A, ".portal-mainlink-li a");
sel!(SEL_INFO_A, "a.portal-info-content-li-a");
sel!(SEL_INFO_DATE, ".portal-subblock-infolist-left-item2 > div");
sel!(SEL_INFO_TITLE, ".portal-subblock-infolist-left-item2 > span");
sel!(SEL_INFO_CATEGORY, ".portal-subblock-infolist-right");

sel!(SEL_CSRF, r#"input[name="_csrf"]"#);
sel!(SEL_BLOCK_TITLE, ".block-title-txt");
sel!(SEL_CONTENTS_HTML, "#contentsHtml");
sel!(SEL_OUTGOING_DIV, ".portal-information-outgoing-division");
sel!(SEL_CONTENTS_DETAIL, ".contents-detail");
sel!(SEL_HEADER_BOLD, ".contents-header-txt .bold-txt");
sel!(SEL_INPUT_AREA, ".contents-input-area");
sel!(SEL_FILE_OBJECT, ".file-object");
sel!(SEL_FILE_NAME, ".downloadFile, .fileName");
sel!(SEL_OBJECT_NAME, ".objectName");
sel!(SEL_SUBPORTAL_TITLE, ".subportal-title-txt");
sel!(SEL_SUBPORTAL_LINK, "li.subportal-block-relation-list-li a.subportal-block-txtlink-li-b");
sel!(SEL_SYSTEM_IMAGE, "img.systemlink-image");
sel!(SEL_SUBPORTAL_LI, "li.subportal-block-info-list-li");
sel!(SEL_SUBPORTAL_CAT, ".subportal-block-list-li-txt-info1");
sel!(SEL_SUBPORTAL_TITLE_SPAN, ".subportal-block-list-li-txt-info2 span.link-txt");
sel!(SEL_SUBPORTAL_DATE, ".subportal-block-list-li-txt-info3 span");
sel!(SEL_SUBPORTAL_DEPT, ".subportal-block-list-li-txt-info4");


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
    let (http, authenticated) = {
        let kwic = state.kwic.lock().await;
        (kwic.http.clone(), kwic.authenticated)
    };
    if !authenticated {
        return Ok(false);
    }
    // Validate against server without holding the lock
    let url = format!("{}/portal/home", crate::config::KWIC_BASE);
    match crate::client::fetch_with_redirect(
        &http, &url, crate::config::KWIC_BASE,
        crate::kwic_client::KWIC_SESSION_EXPIRED_MSG, crate::kwic_client::is_kwic_session_expired,
    ).await {
        Ok(_) => {
            let kwic = state.kwic.lock().await;
            kwic.save_session();
            Ok(true)
        }
        Err(e) if e == crate::kwic_client::KWIC_SESSION_EXPIRED_MSG => {
            let mut kwic = state.kwic.lock().await;
            kwic.authenticated = false;
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

/// Fetch and parse the KWIC Portal home page
#[tauri::command]
pub async fn kwic_fetch_home(
    state: State<'_, AppState>,
    db: State<'_, crate::db::Database>,
) -> Result<KwicPortalHome, String> {
    match kwic_http(&state).await {
        Ok(http) => match kwic_get(&http, "/portal/home").await {
            Ok(html) => {
                #[cfg(debug_assertions)]
                { let _ = std::fs::write(std::env::temp_dir().join("kwic-portal-home.html"), &html); }

                let sections = parse_portal_home(&html);

                let result = KwicPortalHome {
                    sections,
                    #[cfg(debug_assertions)]
                    raw_html_debug: Some(crate::client::safe_truncate(&html, 5000).to_string()),
                    #[cfg(not(debug_assertions))]
                    raw_html_debug: None,
                };
                if let Ok(json) = serde_json::to_string(&result) {
                    let _ = db.save_data_cache("kwic_home", &json);
                }
                Ok(result)
            }
            Err(e) => {
                if let Ok(Some((json, _))) = db.get_data_cache("kwic_home") {
                    if let Ok(cached) = serde_json::from_str(&json) {
                        log::info!("kwic_home: cache fallback ({})", e);
                        return Ok(cached);
                    }
                }
                Err(e)
            }
        },
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache("kwic_home") {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("kwic_home: cache fallback ({})", e);
                    return Ok(cached);
                }
            }
            Err(e)
        }
    }
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
    db: State<'_, crate::db::Database>,
    information_id: String,
    information_type: String,
    person_category_cd: String,
    category_cd: String,
) -> Result<KwicNotificationDetail, String> {
    let cache_key = format!("kwic_detail:{}", information_id);
    match kwic_http(&state).await {
        Ok(http) => {
            // 1. Get home page to extract CSRF token
            let home_html = match kwic_get(&http, "/portal/home").await {
                Ok(h) => h,
                Err(e) => {
                    if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                        if let Ok(cached) = serde_json::from_str(&json) {
                            log::info!("{}: cache fallback ({})", cache_key, e);
                            return Ok(cached);
                        }
                    }
                    return Err(e);
                }
            };
            let csrf = match extract_csrf_token(&home_html) {
                Some(token) => token,
                None => {
                    if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                        if let Ok(cached) = serde_json::from_str(&json) {
                            log::info!("{}: cache fallback (CSRF extraction failed)", cache_key);
                            return Ok(cached);
                        }
                    }
                    return Err("CSRFトークンが取得できませんでした".to_string());
                }
            };

            // 2. POST to portal detail endpoint
            match kwic_post(&http, "/portal/home/information/detail", &[
                ("_csrf", &csrf),
                ("informationId", &information_id),
                ("informationType", &information_type),
                ("personCategoryCd", &person_category_cd),
                ("categoryCd", &category_cd),
                ("selectCategoryCd", &category_cd),
                ("pageViewListNum", "10"),
            ]).await {
                Ok(detail_html) => {
                    #[cfg(debug_assertions)]
                    { let _ = std::fs::write(std::env::temp_dir().join("kwic-portal-detail.html"), &detail_html); }

                    let data = parse_detail_html(&detail_html);
                    if let Ok(json) = serde_json::to_string(&data) {
                        let _ = db.save_data_cache(&cache_key, &json);
                    }
                    Ok(data)
                }
                Err(e) => {
                    if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                        if let Ok(cached) = serde_json::from_str(&json) {
                            log::info!("{}: cache fallback ({})", cache_key, e);
                            return Ok(cached);
                        }
                    }
                    Err(e)
                }
            }
        }
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("{}: cache fallback ({})", cache_key, e);
                    return Ok(cached);
                }
            }
            Err(e)
        }
    }
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
    db: State<'_, crate::db::Database>,
    tag_cd: String,
) -> Result<KwicSubportalData, String> {
    if !tag_cd.chars().all(|c| c.is_ascii_digit()) {
        return Err("\u{7121}\u{52b9}\u{306a}tagCd\u{3067}\u{3059}".into());
    }
    let cache_key = format!("kwic_subportal:{}", tag_cd);
    match kwic_http(&state).await {
        Ok(http) => {
            let path = format!("/portal/subportal?tagCd={}", tag_cd);
            match kwic_get(&http, &path).await {
                Ok(html) => {
                    #[cfg(debug_assertions)]
                    { let _ = std::fs::write(std::env::temp_dir().join(format!("kwic-portal-subportal-{}.html", tag_cd)), &html); }

                    let data = parse_subportal(&html);
                    if let Ok(json) = serde_json::to_string(&data) {
                        let _ = db.save_data_cache(&cache_key, &json);
                    }
                    Ok(data)
                }
                Err(e) => {
                    if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                        if let Ok(cached) = serde_json::from_str(&json) {
                            log::info!("{}: cache fallback ({})", cache_key, e);
                            return Ok(cached);
                        }
                    }
                    Err(e)
                }
            }
        }
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("{}: cache fallback ({})", cache_key, e);
                    return Ok(cached);
                }
            }
            Err(e)
        }
    }
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
            let saml_url: url::Url = config::KWIC_SAML_URL
                .parse().expect("hardcoded KWIC SAML URL is valid");

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
    {
        let sel = &*SEL_NOTICE_A;
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
    {
        let sel = &*SEL_MAINLINK_A;
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
                    format!("{}{}",config::KWIC_BASE, href)
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
    let a = li.select(&*SEL_INFO_A).next()?;
    let id = a.value().attr("data1").unwrap_or_default().to_string();
    let data2 = a.value().attr("data2").unwrap_or_default().to_string();
    let data3 = a.value().attr("data3").unwrap_or_default().to_string();
    let data4 = a.value().attr("data4").unwrap_or_default().to_string();

    // Date: .portal-subblock-infolist-left-item2 > div
    let date = li.select(&*SEL_INFO_DATE).next()
        .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
        .unwrap_or_default();

    // Title: .portal-subblock-infolist-left-item2 > span
    let title = li.select(&*SEL_INFO_TITLE).next()
        .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
        .unwrap_or_default();

    if title.is_empty() { return None; }

    // Category/department: .portal-subblock-infolist-right
    let category = li.select(&*SEL_INFO_CATEGORY).next()
        .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
        .unwrap_or_default();

    Some((KwicPortalItem {
        id: id.clone(),
        title,
        date,
        category,
        url: format!("{}/portal/home/information/detail?informationId={}&directLink=1", config::KWIC_BASE, id),
        important: false,
        information_type: String::new(),
        person_category_cd: String::new(),
        category_cd: String::new(),
    }, data2, data3, data4))
}

/// Extract CSRF token from KWIC Portal HTML
fn extract_csrf_token(html: &str) -> Option<String> {
    use scraper::Html;
    let doc = Html::parse_document(html);
    if let Some(el) = doc.select(&*SEL_CSRF).next() {
        return el.value().attr("value").map(|v| v.to_string());
    }
    None
}

/// Parse the detail HTML fragment returned by /lms/course/information/listdetail.
/// This is typically a dialog fragment containing info_preview with title, body, sender, date, attachments.
fn parse_detail_html(html: &str) -> KwicNotificationDetail {
    use scraper::Html;
    let doc = Html::parse_document(html);

    let text_of = |sel: &scraper::Selector| -> String {
        doc.select(sel).next()
            .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
            .unwrap_or_default()
    };

    let html_of = |sel: &scraper::Selector| -> String {
        doc.select(sel).next()
            .map(|el| el.inner_html().trim().to_string())
            .unwrap_or_default()
    };

    // Real KWIC detail structure:
    // Title: .block-title-txt
    // Body:  #contentsHtml (quill editor content)
    // Sender: .portal-information-outgoing-division (contains "配信部署:" + dept name)
    // Date:  掲載期間 section — we extract from the first .contents-input-area with date-like text
    let title = text_of(&*SEL_BLOCK_TITLE);
    let body_html = html_of(&*SEL_CONTENTS_HTML);

    // Sender: extract department from .portal-information-outgoing-division
    let sender = {
        let raw = text_of(&*SEL_OUTGOING_DIV);
        raw.replace("配信部署:", "").trim().to_string()
    };

    // Date: look for 掲載期間 section, then get the spans inside its .contents-input-area
    let date = {
        let mut found = String::new();
        for detail in doc.select(&*SEL_CONTENTS_DETAIL) {
            if let Some(header) = detail.select(&*SEL_HEADER_BOLD).next() {
                let header_text = header.text().collect::<Vec<_>>().join("");
                if header_text.contains("掲載期間") {
                    if let Some(input) = detail.select(&*SEL_INPUT_AREA).next() {
                        found = input.text().collect::<Vec<_>>().join("").trim().to_string();
                    }
                    break;
                }
            }
        }
        found
    };

    // Attachments: .file-object elements → .downloadFile (name), .objectName (object path)
    let mut attachments = Vec::new();
    for fo in doc.select(&*SEL_FILE_OBJECT) {
        let name = fo.select(&*SEL_FILE_NAME).next()
            .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
            .unwrap_or_default();
        let object_name = fo.select(&*SEL_OBJECT_NAME).next()
            .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
            .unwrap_or_default();
        if name.is_empty() { continue; }
        let url = format!(
            "{}/portal/home/information/detail/download?downloadFileName={}&objectName={}&downloadMode=1",
            config::KWIC_BASE,
            urlencoding::encode(&name),
            urlencoding::encode(&object_name),
        );
        attachments.push(KwicAttachment { name, url });
    }

    // Strip <script> tags from body for safety
    let body_clean = {
        static RE_SCRIPT: LazyLock<regex::Regex> = LazyLock::new(|| {
            regex::Regex::new(r"(?is)<script[^>]*>.*?</script>").expect("valid regex")
        });
        RE_SCRIPT.replace_all(&body_html, "").to_string()
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
    use scraper::Html;
    let doc = Html::parse_document(html);

    // Page title: .subportal-title-txt
    let page_title = doc.select(&*SEL_SUBPORTAL_TITLE).next()
        .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
        .unwrap_or_default();

    // Links: li.subportal-block-relation-list-li a.subportal-block-txtlink-li-b
    // Each <a> contains <img class="systemlink-image"> (icon) + <span> (title)
    let mut links = Vec::new();
    for a in doc.select(&*SEL_SUBPORTAL_LINK) {
        let title: String = a.text().collect::<Vec<_>>().join("").trim().to_string();
        let href = a.value().attr("href").unwrap_or_default();
        if title.is_empty() || href.is_empty() || href == "#" { continue; }
        if href.starts_with("javascript:") { continue; }
        let url = if href.starts_with("http") {
            href.to_string()
        } else {
            format!("{}{}",config::KWIC_BASE, href)
        };
        let icon_url = a.select(&*SEL_SYSTEM_IMAGE).next()
            .and_then(|img| img.value().attr("src"))
            .map(|src| if src.starts_with("http") {
                src.to_string()
            } else {
                format!("{}{}",config::KWIC_BASE, src)
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

    // Notifications: li.subportal-block-info-list-li
    // Structure per item:
    //   .subportal-block-list-li-txt-info1 = category
    //   .subportal-block-list-li-txt-info2 span.link-txt[data1][data2] = title + id + type
    //   .subportal-block-list-li-txt-info3 span:first = date
    //   .subportal-block-list-li-txt-info4 = department
    let mut notifications = Vec::new();
    for li in doc.select(&*SEL_SUBPORTAL_LI) {
        let title_el = match li.select(&*SEL_SUBPORTAL_TITLE_SPAN).next() {
            Some(el) => el,
            None => continue,
        };

        let id = title_el.value().attr("data1").unwrap_or_default().to_string();
        let data2 = title_el.value().attr("data2").unwrap_or_default().to_string();
        let title = title_el.text().collect::<Vec<_>>().join("").trim().to_string();
        if title.is_empty() { continue; }

        let category = li.select(&*SEL_SUBPORTAL_CAT).next()
            .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
            .unwrap_or_default();

        let date = li.select(&*SEL_SUBPORTAL_DATE).next()
            .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
            .unwrap_or_default();

        let dept = li.select(&*SEL_SUBPORTAL_DEPT).next()
            .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
            .unwrap_or_default();

        notifications.push(KwicPortalNotification {
            id,
            title,
            date,
            category: if !dept.is_empty() { dept } else { category },
            important: false,
            information_type: data2,
            // Subportal notifications only have data1/data2 in onclick
            person_category_cd: String::new(),
            category_cd: String::new(),
        });
    }

    KwicSubportalData {
        title: page_title,
        links,
        notifications,
    }
}
