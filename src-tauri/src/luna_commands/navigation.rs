use super::{luna_http, UNIVERSITY_DETAIL_COUNTER};
use crate::{config, luna_client, LunaState};
use std::sync::atomic::Ordering;
use tauri::{Manager, State};

/// Infer `(mode, idnumber, info_id)` from a Luna detail URL when the caller
/// did not pass an explicit `mode`.
///
/// We are deliberately conservative here: only routes where the existing
/// backend parser + renderer pipeline already exists are auto-detected. For
/// other module types (e.g. お問い合わせ / inquiry) the default branch keeps
/// rendering the generic detail until a proper parser exists, instead of
/// pointing at a specialised renderer that would just bail with a parameter
/// error.
fn infer_luna_window_target(
    path: &str,
    mode: Option<&str>,
    idnumber: Option<&str>,
    info_id: Option<&str>,
) -> (Option<String>, Option<String>, Option<String>) {
    let explicit_mode = mode.map(ToString::to_string);
    let explicit_idnumber = idnumber.map(ToString::to_string);
    let explicit_info_id = info_id.map(ToString::to_string);

    if explicit_mode.is_some() {
        return (explicit_mode, explicit_idnumber, explicit_info_id);
    }

    let raw = path.trim();
    if raw.is_empty() {
        return (None, explicit_idnumber, explicit_info_id);
    }

    let full = if raw.starts_with("http://") || raw.starts_with("https://") {
        raw.to_string()
    } else {
        format!("{}{}", config::LUNA_BASE, raw)
    };
    let Ok(url) = url::Url::parse(&full) else {
        return (None, explicit_idnumber, explicit_info_id);
    };

    let query_param = |key: &str| -> Option<String> {
        url.query_pairs()
            .find_map(|(k, v)| (k == key).then(|| v.into_owned()))
            .filter(|s| !s.is_empty())
    };

    let inferred_idnumber = explicit_idnumber
        .clone()
        .or_else(|| query_param("idnumber"));
    let path_name = url.path();

    // Course top / attendance share the same path; the fragment disambiguates.
    if path_name == "/lms/course" || path_name == "/lms/contents" {
        let hash = url.fragment().unwrap_or_default();
        let inferred_mode = if hash == "attendance" {
            "attendance"
        } else {
            "course"
        };
        return (
            Some(inferred_mode.to_string()),
            inferred_idnumber,
            explicit_info_id,
        );
    }

    // Announcement detail discards the original `path` in the URL builder and
    // only re-uses idnumber + informationId, so we only infer it when both are
    // present — otherwise the renderer would error with "パラメータが不足".
    if path_name == "/lms/coursetop/information/listdetail" {
        let info_from_query = query_param("informationId");
        if inferred_idnumber.is_some() && (explicit_info_id.is_some() || info_from_query.is_some())
        {
            return (
                Some("announcement".to_string()),
                inferred_idnumber,
                explicit_info_id.or(info_from_query),
            );
        }
    }

    // Forum thread — needed so a "new comment" notification can land directly
    // on the thread post-list view (not the generic detail page, which omits
    // the post stream). `thread_postfile` is a download endpoint, exclude it.
    if path_name == "/lms/course/forums/thread" {
        return (
            Some("thread".to_string()),
            inferred_idnumber,
            explicit_info_id.or_else(|| query_param("threadId")),
        );
    }

    // Forum theme top — "new thread created" notifications point here. Falls
    // through to the discussion list renderer.
    if path_name == "/lms/course/forums/themetop" {
        return (
            Some("discussion".to_string()),
            inferred_idnumber,
            explicit_info_id.or_else(|| query_param("forumId")),
        );
    }

    // Inquiry (メッセージ / お問い合わせ) — message thread between student and
    // teacher. Both `/post` and `/firstSet` land on the same renderable page.
    if path_name.starts_with("/lms/course/inquiry/") {
        return (
            Some("inquiry".to_string()),
            inferred_idnumber,
            explicit_info_id.or_else(|| query_param("inquiryId")),
        );
    }

    (None, inferred_idnumber, explicit_info_id)
}

/// Open the shared university detail shell in a separate native window.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn university_open_detail_window(
    app: tauri::AppHandle,
    path: String,
    title: String,
    mode: Option<String>,
    period: Option<String>,
    status: Option<String>,
    idnumber: Option<String>,
    info_id: Option<String>,
    kgc_path: Option<String>,
    course_name: Option<String>,
) -> Result<(), String> {
    let existing = app
        .webview_windows()
        .keys()
        .filter(|k| k.starts_with("university-detail-"))
        .count();
    if existing >= 10 {
        return Err(config::TOO_MANY_WINDOWS_MSG.into());
    }
    let id = UNIVERSITY_DETAIL_COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = format!("university-detail-{}", id);
    let (mode, idnumber, info_id) = infer_luna_window_target(
        &path,
        mode.as_deref(),
        idnumber.as_deref(),
        info_id.as_deref(),
    );

    let url_str = match mode.as_deref() {
        Some("material") => {
            let mut parts = format!(
                "university-detail.html?mode=material&title={}",
                urlencoding::encode(&title)
            );
            if let Some(p) = &period {
                parts.push_str(&format!("&period={}", urlencoding::encode(p)));
            }
            if let Some(s) = &status {
                parts.push_str(&format!("&status={}", urlencoding::encode(s)));
            }
            if let Some(id) = &idnumber {
                parts.push_str(&format!("&idnumber={}", urlencoding::encode(id)));
            }
            if let Some(info) = &info_id {
                parts.push_str(&format!("&infoId={}", urlencoding::encode(info)));
            }
            if let Some(cn) = &course_name {
                parts.push_str(&format!("&courseName={}", urlencoding::encode(cn)));
            }
            parts
        }
        Some("announcement") => {
            let mut parts = format!(
                "university-detail.html?mode=announcement&title={}&idnumber={}&infoId={}",
                urlencoding::encode(&title),
                urlencoding::encode(idnumber.as_deref().unwrap_or("")),
                urlencoding::encode(info_id.as_deref().unwrap_or(""))
            );
            if let Some(cn) = &course_name {
                parts.push_str(&format!("&courseName={}", urlencoding::encode(cn)));
            }
            parts
        }
        Some("discussion") => {
            let mut parts = format!(
                "university-detail.html?mode=discussion&path={}&title={}",
                urlencoding::encode(&path),
                urlencoding::encode(&title)
            );
            if let Some(cn) = &course_name {
                parts.push_str(&format!("&courseName={}", urlencoding::encode(cn)));
            }
            parts
        }
        Some("inquiry") => {
            let mut parts = format!(
                "university-detail.html?mode=inquiry&path={}&title={}",
                urlencoding::encode(&path),
                urlencoding::encode(&title)
            );
            if let Some(cn) = &course_name {
                parts.push_str(&format!("&courseName={}", urlencoding::encode(cn)));
            }
            parts
        }
        Some("report") => {
            let mut parts = format!(
                "university-detail.html?mode=report&path={}&title={}",
                urlencoding::encode(&path),
                urlencoding::encode(&title)
            );
            if let Some(id) = &idnumber {
                parts.push_str(&format!("&idnumber={}", urlencoding::encode(id)));
            }
            if let Some(info) = &info_id {
                parts.push_str(&format!("&reportId={}", urlencoding::encode(info)));
            }
            if let Some(cn) = &course_name {
                parts.push_str(&format!("&courseName={}", urlencoding::encode(cn)));
            }
            parts
        }
        Some("survey") | Some("questionnaire") => {
            let mut parts = format!(
                "university-detail.html?mode=survey&path={}&title={}",
                urlencoding::encode(&path),
                urlencoding::encode(&title)
            );
            if let Some(cn) = &course_name {
                parts.push_str(&format!("&courseName={}", urlencoding::encode(cn)));
            }
            parts
        }
        Some("thread") => {
            let mut parts = format!(
                "university-detail.html?mode=thread&path={}&title={}",
                urlencoding::encode(&path),
                urlencoding::encode(&title)
            );
            if let Some(cn) = &course_name {
                parts.push_str(&format!("&courseName={}", urlencoding::encode(cn)));
            }
            parts
        }
        Some("course") => {
            let mut parts = format!(
                "university-detail.html?mode=course&idnumber={}&title={}",
                urlencoding::encode(idnumber.as_deref().unwrap_or("")),
                urlencoding::encode(&title)
            );
            if let Some(kp) = &kgc_path {
                parts.push_str(&format!("&kgcPath={}", urlencoding::encode(kp)));
            }
            if let Some(cn) = &course_name {
                parts.push_str(&format!("&courseName={}", urlencoding::encode(cn)));
            }
            parts
        }
        Some("attendance") => {
            let mut parts = format!(
                "university-detail.html?mode=attendance&idnumber={}&title={}",
                urlencoding::encode(idnumber.as_deref().unwrap_or("")),
                urlencoding::encode(&title)
            );
            if let Some(cn) = &course_name {
                parts.push_str(&format!("&courseName={}", urlencoding::encode(cn)));
            }
            parts
        }
        _ => {
            let mut parts = format!(
                "university-detail.html?path={}&title={}",
                urlencoding::encode(&path),
                urlencoding::encode(&title)
            );
            if let Some(cn) = &course_name {
                parts.push_str(&format!("&courseName={}", urlencoding::encode(cn)));
            }
            parts
        }
    };

    let builder =
        tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App(url_str.into()))
            .title(&title)
            .inner_size(720.0, 780.0)
            .min_inner_size(560.0, 480.0)
            .resizable(true);

    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);

    builder
        .build()
        .map_err(|e| format!("ウィンドウ作成失敗: {}", e))?;

    Ok(())
}

/// Compatibility alias for older frontend/demo code and external callers.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn luna_open_detail_window(
    app: tauri::AppHandle,
    path: String,
    title: String,
    mode: Option<String>,
    period: Option<String>,
    status: Option<String>,
    idnumber: Option<String>,
    info_id: Option<String>,
    kgc_path: Option<String>,
    course_name: Option<String>,
) -> Result<(), String> {
    university_open_detail_window(
        app,
        path,
        title,
        mode,
        period,
        status,
        idnumber,
        info_id,
        kgc_path,
        course_name,
    )
    .await
}

/// Launch an LTI tool (Zoom, Panopto, etc.) and open the final URL in app webview
#[tauri::command]
pub async fn luna_launch_lti(
    app: tauri::AppHandle,
    state: State<'_, LunaState>,
    path: String,
) -> Result<(), String> {
    let http = luna_http(&state).await?;
    let final_url = luna_client::launch_lti(&http, &path).await?;
    crate::commands::open_external_url(app, final_url, None).await
}

/// Reveal a file in Finder (restricted to app download directory)
#[tauri::command]
pub async fn luna_reveal_file(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    let canonical = p
        .canonicalize()
        .map_err(|e| format!("パスが無効です: {}", e))?;
    let sys_downloads = crate::commands::default_download_dir();
    let dl_config = crate::commands::load_download_config();
    let custom_dir = if dl_config.download_dir.is_empty() {
        None
    } else {
        std::path::Path::new(&dl_config.download_dir)
            .canonicalize()
            .ok()
    };
    let sys_dl = dirs::download_dir().unwrap_or_else(|| {
        dirs::home_dir()
            .map(|h| h.join("Downloads"))
            .unwrap_or_else(std::env::temp_dir)
    });
    let allowed = canonical.starts_with(&sys_downloads)
        || canonical.starts_with(&sys_dl)
        || custom_dir
            .as_ref()
            .is_some_and(|d| canonical.starts_with(d));
    if !allowed {
        return Err("ダウンロードフォルダ外のファイルは表示できません".into());
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .reveal_item_in_dir(&canonical)
        .map_err(|e| format!("ファイルを表示できませんでした: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::infer_luna_window_target;

    #[test]
    fn infers_course_mode_from_course_top_url() {
        let (mode, idnumber, info_id) = infer_luna_window_target(
            "/lms/course?idnumber=2026341810090201#information",
            None,
            None,
            None,
        );
        assert_eq!(mode.as_deref(), Some("course"));
        assert_eq!(idnumber.as_deref(), Some("2026341810090201"));
        assert!(info_id.is_none());
    }

    #[test]
    fn infers_attendance_mode_from_attendance_hash() {
        let (mode, idnumber, _) = infer_luna_window_target(
            "/lms/course?idnumber=2026341810090201#attendance",
            None,
            None,
            None,
        );
        assert_eq!(mode.as_deref(), Some("attendance"));
        assert_eq!(idnumber.as_deref(), Some("2026341810090201"));
    }

    #[test]
    fn infers_announcement_mode_with_information_id() {
        let (mode, idnumber, info_id) = infer_luna_window_target(
            "/lms/coursetop/information/listdetail?idnumber=2026341810090201&informationId=7",
            None,
            None,
            None,
        );
        assert_eq!(mode.as_deref(), Some("announcement"));
        assert_eq!(idnumber.as_deref(), Some("2026341810090201"));
        assert_eq!(info_id.as_deref(), Some("7"));
    }

    #[test]
    fn infers_discussion_mode_for_forum_themetop_url() {
        let (mode, idnumber, info_id) = infer_luna_window_target(
            "/lms/course/forums/themetop?idnumber=2026341340000201&forumId=9003",
            None,
            None,
            None,
        );
        assert_eq!(mode.as_deref(), Some("discussion"));
        assert_eq!(idnumber.as_deref(), Some("2026341340000201"));
        assert_eq!(info_id.as_deref(), Some("9003"));
    }

    #[test]
    fn infers_thread_mode_for_forum_thread_url() {
        // 揭示板 new-comment notifications carry a /forums/thread URL — these
        // must land on the thread renderer so the post list shows up.
        let (mode, idnumber, info_id) = infer_luna_window_target(
            "/lms/course/forums/thread?idnumber=2026341810090201&forumId=8375&threadId=53077",
            None,
            None,
            None,
        );
        assert_eq!(mode.as_deref(), Some("thread"));
        assert_eq!(idnumber.as_deref(), Some("2026341810090201"));
        assert_eq!(info_id.as_deref(), Some("53077"));

        // thread_postfile is a download endpoint, not a page.
        let (mode, _, _) = infer_luna_window_target(
            "/lms/course/forums/thread_postfile?fileId=foo",
            None,
            None,
            None,
        );
        assert!(mode.is_none());
    }

    #[test]
    fn infers_inquiry_mode_for_inquiry_paths() {
        for path in [
            "/lms/course/inquiry/post?idnumber=2026510010040201&inquiryId=320411",
            "/lms/course/inquiry/firstSet?idnumber=2026510010040201&inquiryId=320411",
        ] {
            let (mode, idnumber, info_id) = infer_luna_window_target(path, None, None, None);
            assert_eq!(mode.as_deref(), Some("inquiry"), "{path}");
            assert_eq!(idnumber.as_deref(), Some("2026510010040201"), "{path}");
            assert_eq!(info_id.as_deref(), Some("320411"), "{path}");
        }
    }

    #[test]
    fn leaves_unrecognised_paths_to_generic_parser() {
        // Modules without a dedicated parser+renderer pipeline yet (e.g. report
        // / survey via path-only callers) should still fall through to the
        // generic detail. LunaTodo already path-routes these explicitly.
        let (mode, _, _) = infer_luna_window_target(
            "/lms/course/report/submission?idnumber=2026341810090201&reportId=42",
            None,
            None,
            None,
        );
        assert!(mode.is_none());
    }

    #[test]
    fn does_not_infer_announcement_when_information_id_missing() {
        // Without informationId, the announcement renderer can't fetch anything
        // — fall back to the generic detail rather than producing an error
        // window.
        let (mode, _, _) = infer_luna_window_target(
            "/lms/coursetop/information/listdetail?idnumber=2026341810090201",
            None,
            None,
            None,
        );
        assert!(mode.is_none());
    }

    #[test]
    fn explicit_mode_wins_over_inference() {
        let (mode, idnumber, info_id) = infer_luna_window_target(
            "/lms/coursetop/information/listdetail?idnumber=2026341810090201&informationId=7",
            Some("course"),
            None,
            None,
        );
        assert_eq!(mode.as_deref(), Some("course"));
        // Explicit mode keeps caller-provided values verbatim — we don't second-guess by parsing the path.
        assert!(idnumber.is_none());
        assert!(info_id.is_none());
    }
}
