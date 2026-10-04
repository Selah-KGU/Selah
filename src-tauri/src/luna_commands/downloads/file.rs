use tauri::State;

use crate::LunaState;

use super::super::luna_http;
use super::http::luna_download;
use super::save::{form_encode, make_down_file_name, save_to_downloads};

/// Download a Luna file attachment to the Downloads folder and return the saved path.
///
/// Two modes:
///   1. `url` is non-empty (legacy or direct link): download from URL directly
///   2. `url` is empty but `download_action`/`object_name` provided:
///      re-fetch the detail page via `page_path` to get fresh `_cid` token,
///      then construct the proper form-based download URL.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn luna_download_file(
    state: State<'_, LunaState>,
    url: String,
    filename: String,
    _page_path: Option<String>,
    _object_name: Option<String>,
    download_action: Option<String>,
    download_params: Option<Vec<(String, String)>>,
    course_name: Option<String>,
    _detail_title: Option<String>,
) -> Result<String, String> {
    if url.starts_with("http") {
        return Ok(url);
    }

    let http = luna_http(&state).await?;

    let bytes = if url.is_empty() {
        let action = download_action.as_deref().unwrap_or("");
        if action.is_empty() {
            return Err("ダウンロードURLが見つかりません".into());
        }

        let mut params: Vec<String> = Vec::new();
        if let Some(ref fields) = download_params {
            for (k, v) in fields {
                params.push(format!("{}={}", form_encode(k), form_encode(v)));
            }
        }

        let path_name = make_down_file_name(&filename);
        let download_url = format!("{}/{}?{}", action, path_name, params.join("&"));

        log::info!("Attachment GET: url='{}'", download_url);
        luna_download(&http, &download_url).await?
    } else {
        log::info!(
            "Attachment GET download: url='{}', filename='{}'",
            url,
            filename
        );
        luna_download(&http, &url).await?
    };

    log::info!(
        "Attachment downloaded {} bytes for '{}'",
        bytes.len(),
        filename
    );

    if bytes.is_empty() {
        return Err("ダウンロードされたファイルが空です".into());
    }

    if bytes.len() < 2000 {
        if let Ok(text) = std::str::from_utf8(&bytes) {
            if text.contains("<!DOCTYPE") || text.contains("<html") || text.contains("<HTML") {
                log::error!(
                    "Attachment download returned HTML instead of file: {}",
                    crate::client::safe_truncate(text, 500)
                );
                return Err("サーバーがファイルではなくエラーページを返しました".into());
            }
        }
    }

    save_to_downloads(&filename, &bytes, course_name.as_deref())
}
