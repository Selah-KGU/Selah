use super::{luna_http, LUNA_DETAIL_COUNTER};
use crate::{config, luna_client, LunaState};
use std::sync::atomic::Ordering;
use tauri::{Manager, State};

fn infer_luna_window_target(
    path: &str,
    mode: Option<&str>,
    idnumber: Option<&str>,
) -> (Option<String>, Option<String>) {
    if mode.is_some() {
        return (
            mode.map(ToString::to_string),
            idnumber.map(ToString::to_string),
        );
    }

    let raw = path.trim();
    if raw.is_empty() {
        return (None, idnumber.map(ToString::to_string));
    }

    let full = if raw.starts_with("http://") || raw.starts_with("https://") {
        raw.to_string()
    } else {
        format!("{}{}", config::LUNA_BASE, raw)
    };
    let Ok(url) = url::Url::parse(&full) else {
        return (None, idnumber.map(ToString::to_string));
    };

    let inferred_idnumber = url
        .query_pairs()
        .find_map(|(k, v)| (k == "idnumber").then(|| v.into_owned()))
        .or_else(|| idnumber.map(ToString::to_string));
    let path_name = url.path();

    if path_name == "/lms/course" || path_name == "/lms/contents" {
        let hash = url.fragment().unwrap_or_default();
        let inferred_mode = if hash == "attendance" {
            "attendance"
        } else {
            "course"
        };
        return (Some(inferred_mode.to_string()), inferred_idnumber);
    }

    (None, inferred_idnumber)
}

/// Open a Luna detail page in a separate native window
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
    let existing = app
        .webview_windows()
        .keys()
        .filter(|k| k.starts_with("luna-detail-"))
        .count();
    if existing >= 10 {
        return Err(config::TOO_MANY_WINDOWS_MSG.into());
    }
    let id = LUNA_DETAIL_COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = format!("luna-detail-{}", id);
    let (mode, idnumber) = infer_luna_window_target(&path, mode.as_deref(), idnumber.as_deref());

    let url_str = match mode.as_deref() {
        Some("material") => {
            let mut parts = format!(
                "luna-detail.html?mode=material&title={}",
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
            parts
        }
        Some("announcement") => {
            let mut parts = format!(
                "luna-detail.html?mode=announcement&title={}&idnumber={}&infoId={}",
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
            format!(
                "luna-detail.html?mode=discussion&path={}&title={}",
                urlencoding::encode(&path),
                urlencoding::encode(&title)
            )
        }
        Some("report") => {
            let mut parts = format!(
                "luna-detail.html?mode=report&path={}&title={}",
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
                "luna-detail.html?mode=survey&path={}&title={}",
                urlencoding::encode(&path),
                urlencoding::encode(&title)
            );
            if let Some(cn) = &course_name {
                parts.push_str(&format!("&courseName={}", urlencoding::encode(cn)));
            }
            parts
        }
        Some("thread") => {
            format!(
                "luna-detail.html?mode=thread&path={}&title={}",
                urlencoding::encode(&path),
                urlencoding::encode(&title)
            )
        }
        Some("course") => {
            let mut parts = format!(
                "luna-detail.html?mode=course&idnumber={}&title={}",
                urlencoding::encode(idnumber.as_deref().unwrap_or("")),
                urlencoding::encode(&title)
            );
            if let Some(kp) = &kgc_path {
                parts.push_str(&format!("&kgcPath={}", urlencoding::encode(kp)));
            }
            parts
        }
        Some("attendance") => {
            format!(
                "luna-detail.html?mode=attendance&idnumber={}&title={}",
                urlencoding::encode(idnumber.as_deref().unwrap_or("")),
                urlencoding::encode(&title)
            )
        }
        _ => {
            let mut parts = format!(
                "luna-detail.html?path={}&title={}",
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
        let (mode, idnumber) = infer_luna_window_target(
            "/lms/course?idnumber=2026341810090201#information",
            None,
            None,
        );
        assert_eq!(mode.as_deref(), Some("course"));
        assert_eq!(idnumber.as_deref(), Some("2026341810090201"));
    }

    #[test]
    fn infers_attendance_mode_from_attendance_hash() {
        let (mode, idnumber) = infer_luna_window_target(
            "/lms/course?idnumber=2026341810090201#attendance",
            None,
            None,
        );
        assert_eq!(mode.as_deref(), Some("attendance"));
        assert_eq!(idnumber.as_deref(), Some("2026341810090201"));
    }

    #[test]
    fn leaves_detail_paths_unmodified() {
        let (mode, idnumber) = infer_luna_window_target(
            "/lms/course/report/submission?idnumber=2026341810090201&reportId=1",
            None,
            None,
        );
        assert!(mode.is_none());
        assert_eq!(idnumber.as_deref(), Some("2026341810090201"));
    }
}
