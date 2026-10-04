use tauri::State;

use crate::LunaState;

use super::super::luna_http;
use super::http::luna_download;
use super::prepare::{build_material_download_query, prepare_material_tempfile};
use super::save::{make_down_file_name, save_to_downloads};

/// Download a Luna material file (requires tempfile preparation + form-based download)
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn luna_download_material(
    state: State<'_, LunaState>,
    idnumber: String,
    file_name: String,
    object_name: String,
    resource_id: String,
    file_type: String,
    material_id: Option<String>,
    display_name: Option<String>,
    end_date: Option<String>,
    course_name: Option<String>,
    _material_title: Option<String>,
) -> Result<String, String> {
    let http = luna_http(&state).await?;

    log::info!(
        "Material download: file='{}', object='{}', resource='{}', type='{}', matId={:?}",
        file_name,
        object_name,
        resource_id,
        file_type,
        material_id
    );

    let file_id = prepare_material_tempfile(
        &http,
        &idnumber,
        &file_name,
        &object_name,
        &resource_id,
        "ファイル準備失敗",
    )
    .await?;

    let path_encoded_name = make_down_file_name(&file_name);
    let base_path = if file_type == "0" {
        format!("/lms/course/materialref/setfiledown/{}", path_encoded_name)
    } else {
        format!(
            "/lms/course/materialref/sethtmlfiledown/{}",
            path_encoded_name
        )
    };
    let dl_title = display_name.unwrap_or_default();
    let content_id = material_id.unwrap_or_default();
    let title_val = if file_type != "0" { &dl_title } else { "" };
    let end_date_val = end_date.unwrap_or_default();
    let query_string = build_material_download_query(
        &file_name,
        &file_id,
        &idnumber,
        &resource_id,
        &content_id,
        &end_date_val,
        title_val,
    );
    let full_download_url = format!("{}?{}", base_path, query_string);

    log::info!("Material download full URL: {}", full_download_url);

    let bytes = luna_download(&http, &full_download_url).await?;

    log::info!("Material downloaded {} bytes", bytes.len());

    if bytes.len() < 1000 {
        if let Ok(text) = std::str::from_utf8(&bytes) {
            if text.contains("<!DOCTYPE") || text.contains("<html") {
                log::error!(
                    "Download returned HTML instead of file: {}",
                    crate::client::safe_truncate(text, 500)
                );
                return Err("サーバーがファイルではなくエラーページを返しました".into());
            }
        }
    }

    if bytes.is_empty() {
        return Err("ダウンロードされたファイルが空です".into());
    }

    save_to_downloads(&file_name, &bytes, course_name.as_deref())
}
