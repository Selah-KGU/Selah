use crate::LunaState;

use super::super::luna_http;
use super::http::luna_download;
use super::prepare::{build_material_download_query, prepare_material_tempfile};
use super::save::{make_down_file_name, save_to_downloads};

pub(crate) async fn download_luna_material_file(
    state: &LunaState,
    idnumber: &str,
    file: &crate::luna_parser::LunaMaterialFile,
    course_name: Option<&str>,
) -> Result<String, String> {
    let http = luna_http(state).await?;
    let file_name = if file.file_name.trim().is_empty() {
        file.display_name.trim()
    } else {
        file.file_name.trim()
    };
    if file_name.is_empty() {
        return Err("資料ファイル名が空です".into());
    }

    log::info!(
        "Agent material download: idnumber='{}', file='{}', object='{}', resource='{}', type='{}'",
        idnumber,
        file_name,
        file.object_name,
        file.resource_id,
        file.file_type
    );

    let file_id = prepare_material_tempfile(
        &http,
        idnumber,
        file_name,
        &file.object_name,
        &file.resource_id,
        "資料ファイル準備失敗",
    )
    .await?;

    let path_encoded_name = make_down_file_name(file_name);
    let base_path = if file.file_type == "0" {
        format!("/lms/course/materialref/setfiledown/{}", path_encoded_name)
    } else {
        format!(
            "/lms/course/materialref/sethtmlfiledown/{}",
            path_encoded_name
        )
    };
    let title_val = if file.file_type != "0" {
        file.display_name.as_str()
    } else {
        ""
    };
    let query_string = build_material_download_query(
        file_name,
        &file_id,
        idnumber,
        &file.resource_id,
        &file.material_id,
        &file.end_date,
        title_val,
    );
    let full_download_url = format!("{}?{}", base_path, query_string);
    log::info!("Agent material download URL: {}", full_download_url);

    let bytes = luna_download(&http, &full_download_url).await?;
    if bytes.is_empty() {
        return Err("ダウンロードされたファイルが空です".into());
    }
    if bytes.len() < 1000 {
        if let Ok(text) = std::str::from_utf8(&bytes) {
            if text.contains("<!DOCTYPE") || text.contains("<html") {
                log::error!(
                    "Agent material download returned HTML instead of file: {}",
                    crate::client::safe_truncate(text, 500)
                );
                return Err("サーバーがファイルではなくエラーページを返しました".into());
            }
        }
    }

    save_to_downloads(file_name, &bytes, course_name)
}
