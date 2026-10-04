//! Luna detail and announcement fetches.
//!
//! Validates cached HTML before it is shown, and retries when Luna redirects
//! back to the home page. HTTP helpers stay in the parent.

use super::*;

fn normalize_detail_title(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !c.is_whitespace()
                && !matches!(
                    c,
                    '|' | '｜' | '【' | '】' | '[' | ']' | '(' | ')' | '（' | '）' | ':' | '：'
                )
        })
        .collect::<String>()
        .to_lowercase()
}

fn title_matches_expected(actual: &str, expected: Option<&str>) -> bool {
    let Some(expected) = expected.map(str::trim).filter(|s| !s.is_empty()) else {
        return true;
    };
    let actual = normalize_detail_title(actual);
    let expected = normalize_detail_title(expected);
    !actual.is_empty()
        && !expected.is_empty()
        && (actual.contains(&expected) || expected.contains(&actual))
}

fn has_detail_payload(data: &luna_parser::LunaDetailPage) -> bool {
    !data.sections.is_empty() || !data.meta.is_empty() || !data.attachments.is_empty()
}

fn is_report_detail_path(path: &str) -> bool {
    path.contains("/lms/course/report/submission")
}

fn is_course_top_path(path: &str) -> bool {
    let raw = path.split('#').next().unwrap_or(path);
    raw == "/lms/course"
        || raw.starts_with("/lms/course?")
        || raw == "/lms/contents"
        || raw.starts_with("/lms/contents?")
}

fn normalize_luna_detail_path(path: &str) -> String {
    path.replace("&amp;", "&")
}

fn finalize_report_detail(
    mut data: luna_parser::LunaDetailPage,
    expected_title: Option<&str>,
) -> luna_parser::LunaDetailPage {
    if data.title.trim().is_empty() {
        if let Some(expected) = expected_title.filter(|s| !s.trim().is_empty()) {
            data.title = expected.to_string();
        }
    }
    data
}

fn has_blacklisted_cached_sections(data: &luna_parser::LunaDetailPage) -> bool {
    data.sections
        .iter()
        .any(|section| crate::luna_parser::is_blacklisted_system_notice_text(&section.body))
}

fn finalize_generic_detail(
    mut data: luna_parser::LunaDetailPage,
    expected_title: Option<&str>,
) -> luna_parser::LunaDetailPage {
    if data.title.trim().is_empty() {
        if let Some(expected) = expected_title.filter(|s| !s.trim().is_empty()) {
            data.title = expected.to_string();
        }
    }
    data
}

fn has_generic_detail_structure(html: &str) -> bool {
    let doc = scraper::Html::parse_document(html);
    let has_title =
        doc.select(&SEL_DETAIL_TITLE).next().is_some() || html.contains("course-title-txt");
    let has_detail_rows = doc.select(&SEL_DETAIL_VERT).next().is_some();
    let has_report_form = doc.select(&SEL_REPORT_FORM).next().is_some();
    let has_forum_post = doc.select(&SEL_THREAD_POST_MARKER).next().is_some();
    let has_downloads = html.contains("downloadFile");
    let has_updates = doc.select(&SEL_UPDATE_INFO_LIST).next().is_some();

    (has_detail_rows || has_report_form || has_forum_post || has_downloads)
        && (has_title || has_report_form || has_forum_post)
        && !(has_updates && !has_detail_rows && !has_report_form && !has_forum_post)
}

fn has_announcement_detail_structure(html: &str) -> bool {
    let doc = scraper::Html::parse_document(html);
    let has_title = doc.select(&SEL_DETAIL_TITLE).next().is_some();
    let has_detail_rows = doc.select(&SEL_DETAIL_VERT).next().is_some();
    has_title && has_detail_rows
}

fn is_valid_generic_detail_response(
    html: &str,
    data: &luna_parser::LunaDetailPage,
    expected_title: Option<&str>,
) -> bool {
    title_matches_expected(&data.title, expected_title)
        && has_detail_payload(data)
        && has_generic_detail_structure(html)
}

fn is_valid_announcement_detail_response(
    html: &str,
    data: &luna_parser::LunaDetailPage,
    expected_title: Option<&str>,
) -> bool {
    title_matches_expected(&data.title, expected_title)
        && has_detail_payload(data)
        && has_announcement_detail_structure(html)
}

fn is_usable_cached_detail_response(
    path: &str,
    data: &luna_parser::LunaDetailPage,
    expected_title: Option<&str>,
) -> bool {
    if has_blacklisted_cached_sections(data) {
        return false;
    }
    if is_report_detail_path(path) {
        return true;
    }
    let _ = expected_title;
    has_detail_payload(data)
}

fn is_soft_usable_generic_detail_response(
    html: &str,
    data: &luna_parser::LunaDetailPage,
    expected_title: Option<&str>,
) -> bool {
    has_detail_payload(data)
        && has_generic_detail_structure(html)
        && (title_matches_expected(&data.title, expected_title) || data.title.trim().is_empty())
}

fn is_soft_usable_announcement_detail_response(
    html: &str,
    data: &luna_parser::LunaDetailPage,
    expected_title: Option<&str>,
) -> bool {
    has_detail_payload(data)
        && has_announcement_detail_structure(html)
        && (title_matches_expected(&data.title, expected_title) || data.title.trim().is_empty())
}

async fn refresh_luna_detail_context(http: &reqwest::Client) {
    let _ = luna_get(http, "/lms/home").await;
}

/// Luna redirects unauthorised / context-less requests to the home page
/// (`<title>時間割</title>` with the timetable grid). Detect that response so
/// the caller can surface a meaningful error instead of parsing zero posts and
/// leaving the renderer stuck on a loading spinner forever.
pub(super) fn looks_like_luna_home_redirect(html: &str) -> bool {
    let head = html.get(..2048).unwrap_or(html);
    head.contains("<title>時間割</title>")
        || (head.contains("<title>Luna") && html.contains("div-table-data-row"))
}

fn unstable_detail_error_message(kind: &str) -> String {
    match kind {
        "announcement" => {
            "Luna お知らせ詳細の読込が一時的に不安定でした。自動で再取得できなかったため、少し待ってから再度お試しください。".into()
        }
        _ => {
            "Luna 詳細ページの読込が一時的に不安定でした。自動で再取得できなかったため、少し待ってから再度お試しください。".into()
        }
    }
}

/// Fetch and parse a Luna detail page (any path)
#[tauri::command]
pub async fn luna_fetch_detail(
    state: State<'_, LunaState>,
    db: State<'_, crate::db::Database>,
    path: String,
    expected_title: Option<String>,
) -> Result<luna_parser::LunaDetailPage, String> {
    let path = normalize_luna_detail_path(&path);
    // Reject absolute URLs and enforce known Luna path prefixes
    if path.starts_with("http") || !path.starts_with('/') {
        return Err("許可されていないパスです".into());
    }
    if is_course_top_path(&path) {
        return Err(
            "Lunaの授業トップURLです。詳細ページではなくコース画面として開いてください。".into(),
        );
    }
    let is_report = is_report_detail_path(&path);
    let cache_key = if is_report {
        format!(
            "luna_report_detail:{}:{}",
            LUNA_REPORT_DETAIL_CACHE_VERSION, path
        )
    } else {
        format!("luna_detail:{}:{}", LUNA_DETAIL_CACHE_VERSION, path)
    };
    let expected_title = expected_title.as_deref();
    match luna_http(&state).await {
        Ok(http) => {
            let fetch = async {
                let mut html = luna_get(&http, &path).await?;
                let mut data = luna_parser::parse_luna_detail_page(&html);
                if is_report {
                    data = finalize_report_detail(data, expected_title);
                    #[cfg(debug_assertions)]
                    {
                        if crate::should_dump_debug_html() {
                            let filename = path.replace(['/', '?', '&'], "_");
                            let dump_path = std::env::temp_dir()
                                .join(format!("luna_report_detail{}.html", filename));
                            let _ = std::fs::write(&dump_path, &html);
                            log::info!(
                                "Luna report detail HTML dumped to {} ({} bytes)",
                                dump_path.display(),
                                html.len()
                            );
                        }
                    }
                    return Ok(data);
                }
                let mut accepted_soft = false;
                if !is_valid_generic_detail_response(&html, &data, expected_title) {
                    for attempt in 1..LUNA_DETAIL_RETRY_ATTEMPTS {
                        log::warn!(
                            "Luna detail page for '{}' looked unstable on attempt {}/{} (title='{}', sections={}, meta={}, attachments={}), refreshing and retrying",
                            path,
                            attempt,
                            LUNA_DETAIL_RETRY_ATTEMPTS,
                            data.title,
                            data.sections.len(),
                            data.meta.len(),
                            data.attachments.len()
                        );
                        refresh_luna_detail_context(&http).await;
                        tokio::time::sleep(std::time::Duration::from_millis(250 * attempt as u64))
                            .await;
                        html = luna_get(&http, &path).await?;
                        data = luna_parser::parse_luna_detail_page(&html);
                        if is_valid_generic_detail_response(&html, &data, expected_title) {
                            break;
                        }
                    }
                    if !is_valid_generic_detail_response(&html, &data, expected_title) {
                        if is_soft_usable_generic_detail_response(&html, &data, expected_title) {
                            log::warn!(
                                "Luna detail page for '{}' still looked atypical after retries; accepting soft-valid payload",
                                path
                            );
                            accepted_soft = true;
                            data = finalize_generic_detail(data, expected_title);
                        } else {
                            return Err(unstable_detail_error_message("detail"));
                        }
                    }
                }
                if !accepted_soft {
                    data = finalize_generic_detail(data, expected_title);
                }

                #[cfg(debug_assertions)]
                {
                    if crate::should_dump_debug_html() {
                        let filename = path.replace(['/', '?', '&'], "_");
                        let dump_path =
                            std::env::temp_dir().join(format!("luna_detail{}.html", filename));
                        let _ = std::fs::write(&dump_path, &html);
                        log::info!(
                            "Luna detail HTML dumped to {} ({} bytes)",
                            dump_path.display(),
                            html.len()
                        );
                    }
                }
                Ok(data)
            };

            match fetch.await {
                Ok(data) => {
                    if let Ok(json) = serde_json::to_string(&data) {
                        let _ = db.save_data_cache(&cache_key, &json);
                    }
                    Ok(data)
                }
                Err(e) => {
                    if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                        if let Ok(cached) = serde_json::from_str(&json) {
                            if is_report {
                                log::info!("luna_report_detail: cache fallback ({})", e);
                                return Ok(finalize_report_detail(cached, expected_title));
                            }
                            if is_usable_cached_detail_response(&path, &cached, expected_title) {
                                log::info!("luna_detail: cache fallback ({})", e);
                                return Ok(cached);
                            }
                            log::warn!("luna_detail: ignored stale/invalid cache fallback");
                        }
                    }
                    Err(e)
                }
            }
        }
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    if is_report {
                        log::info!("luna_report_detail: cache fallback ({})", e);
                        return Ok(finalize_report_detail(cached, expected_title));
                    }
                    if is_usable_cached_detail_response(&path, &cached, expected_title) {
                        log::info!("luna_detail: cache fallback ({})", e);
                        return Ok(cached);
                    }
                    log::warn!("luna_detail: ignored stale/invalid cache fallback");
                }
            }
            Err(e)
        }
    }
}

/// Fetch announcement detail from Luna course page
#[tauri::command]
pub async fn luna_fetch_announcement_detail(
    state: State<'_, LunaState>,
    db: State<'_, crate::db::Database>,
    idnumber: String,
    info_id: String,
    expected_title: Option<String>,
) -> Result<luna_parser::LunaDetailPage, String> {
    if !is_safe_param(&idnumber) || !is_safe_param(&info_id) {
        return Err("無効なパラメータです".into());
    }
    let cache_key = format!(
        "luna_announce:{}:{}:{}",
        LUNA_ANNOUNCEMENT_CACHE_VERSION, idnumber, info_id
    );
    let expected_title = expected_title.as_deref();
    let path = format!(
        "/lms/coursetop/information/listdetail?idnumber={}&informationId={}",
        idnumber, info_id
    );
    match luna_http(&state).await {
        Ok(http) => {
            let fetch = async {
                let mut html = luna_get(&http, &path).await?;
                let mut data = luna_parser::parse_luna_announcement_detail(&html);

                if !is_valid_announcement_detail_response(&html, &data, expected_title) {
                    for attempt in 1..LUNA_DETAIL_RETRY_ATTEMPTS {
                        log::warn!(
                            "Luna announcement detail for '{}:{}' looked unstable on attempt {}/{} (title='{}', sections={}, meta={}, attachments={}), refreshing and retrying",
                            idnumber,
                            info_id,
                            attempt,
                            LUNA_DETAIL_RETRY_ATTEMPTS,
                            data.title,
                            data.sections.len(),
                            data.meta.len(),
                            data.attachments.len()
                        );
                        refresh_luna_detail_context(&http).await;
                        tokio::time::sleep(std::time::Duration::from_millis(250 * attempt as u64))
                            .await;
                        html = luna_get(&http, &path).await?;
                        data = luna_parser::parse_luna_announcement_detail(&html);
                        if is_valid_announcement_detail_response(&html, &data, expected_title) {
                            break;
                        }
                    }
                    if !is_valid_announcement_detail_response(&html, &data, expected_title) {
                        if is_soft_usable_announcement_detail_response(&html, &data, expected_title)
                        {
                            log::warn!(
                                "Luna announcement detail for '{}:{}' still looked atypical after retries; accepting soft-valid payload",
                                idnumber,
                                info_id
                            );
                            data = finalize_generic_detail(data, expected_title);
                        } else {
                            return Err(unstable_detail_error_message("announcement"));
                        }
                    }
                }
                data = finalize_generic_detail(data, expected_title);

                #[cfg(debug_assertions)]
                {
                    if crate::should_dump_debug_html() {
                        let dump_path = std::env::temp_dir()
                            .join(format!("luna_announcement_{}_{}.html", idnumber, info_id));
                        let _ = std::fs::write(&dump_path, &html);
                        log::info!("Luna announcement detail dumped ({} bytes)", html.len());
                    }
                }
                Ok(data)
            };

            match fetch.await {
                Ok(data) => {
                    if let Ok(json) = serde_json::to_string(&data) {
                        let _ = db.save_data_cache(&cache_key, &json);
                    }
                    Ok(data)
                }
                Err(e) => {
                    if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                        if let Ok(cached) = serde_json::from_str(&json) {
                            let cache_path = format!(
                                "/lms/coursetop/information/listdetail?idnumber={}&informationId={}",
                                idnumber, info_id
                            );
                            if is_usable_cached_detail_response(
                                &cache_path,
                                &cached,
                                expected_title,
                            ) {
                                log::info!("luna_announce: cache fallback ({})", e);
                                return Ok(cached);
                            }
                            log::warn!("luna_announce: ignored stale/invalid cache fallback");
                        }
                    }
                    Err(e)
                }
            }
        }
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    let cache_path = format!(
                        "/lms/coursetop/information/listdetail?idnumber={}&informationId={}",
                        idnumber, info_id
                    );
                    if is_usable_cached_detail_response(&cache_path, &cached, expected_title) {
                        log::info!("luna_announce: cache fallback ({})", e);
                        return Ok(cached);
                    }
                    log::warn!("luna_announce: ignored stale/invalid cache fallback");
                }
            }
            Err(e)
        }
    }
}
