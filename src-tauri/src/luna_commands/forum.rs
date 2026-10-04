use serde::Deserialize;

use super::{add_text_fields, luna_post_multipart_with_optional_cid, LUNA_FORUM_FILE_MAX_BYTES};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LunaDiscussionUploadFile {
    pub file_name: String,
    pub file_base64: String,
}

pub(super) fn has_forum_file_upload_support(html: &str) -> bool {
    html.contains("/lms/course/forum/thread_file")
        || html.contains("name=\"uploadFiles\"")
        || html.contains("class=\"fileSelectInput")
        || html.contains("files[__index__].fileId")
}

pub(super) fn validate_forum_file_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("ファイル名が空です".into());
    }
    if trimmed.chars().count() > 60 {
        return Err("ファイル名は60文字以下にしてください".into());
    }
    if trimmed.chars().any(|c| {
        matches!(
            c,
            '\\' | '/' | ':' | '*' | '?' | '<' | '>' | '|' | '"' | '%' | '~' | ';'
        )
    }) {
        return Err("ファイル名に使用できない文字が含まれています".into());
    }
    Ok(())
}

pub(super) fn decode_forum_upload_files(
    attachments: Option<Vec<LunaDiscussionUploadFile>>,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    use base64::Engine;

    let Some(attachments) = attachments else {
        return Ok(Vec::new());
    };
    if attachments.len() > 10 {
        return Err("添付ファイルは10個以下にしてください".into());
    }

    let mut files = Vec::new();
    for attachment in attachments {
        let file_name = attachment.file_name.trim().to_string();
        validate_forum_file_name(&file_name)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&attachment.file_base64)
            .map_err(|e| format!("Base64デコード失敗: {}", e))?;
        if bytes.is_empty() {
            return Err("ファイルサイズが0バイトです".into());
        }
        if bytes.len() > LUNA_FORUM_FILE_MAX_BYTES {
            return Err(format!(
                "「{}」は最大サイズ（100MB）を超えています。",
                file_name
            ));
        }
        files.push((file_name, bytes));
    }

    Ok(files)
}

pub(super) async fn upload_forum_files(
    http: &reqwest::Client,
    cid: Option<&str>,
    base_fields: &[(String, String)],
    files: &[(String, Vec<u8>)],
) -> Result<Vec<String>, String> {
    if files.is_empty() {
        return Ok(Vec::new());
    }

    let mut form = add_text_fields(reqwest::multipart::Form::new(), base_fields);
    for (idx, (file_name, bytes)) in files.iter().enumerate() {
        form = form
            .text(format!("files[{}].fileId", idx), "0".to_string())
            .text(format!("files[{}].deleteFlag", idx), "0".to_string())
            .text(format!("files[{}].objectName", idx), String::new())
            .text(format!("files[{}].fileName", idx), file_name.clone())
            .part(
                "uploadFiles",
                reqwest::multipart::Part::bytes(bytes.clone())
                    .file_name(file_name.clone())
                    .mime_str("application/octet-stream")
                    .map_err(|e| format!("MIME error: {}", e))?,
            );
    }

    let upload_resp =
        luna_post_multipart_with_optional_cid(http, "/lms/course/forum/thread_file", cid, form)
            .await?;
    let upload_json: serde_json::Value = serde_json::from_str(&upload_resp).map_err(|e| {
        format!(
            "添付ファイルアップロード応答の解析失敗: {} — body: {}",
            e,
            crate::client::safe_truncate(&upload_resp, 200)
        )
    })?;

    let ids = upload_json
        .as_array()
        .ok_or_else(|| {
            format!(
                "添付ファイルアップロード応答が不正です: {}",
                crate::client::safe_truncate(&upload_resp, 200)
            )
        })?
        .iter()
        .filter_map(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        })
        .collect::<Vec<_>>();

    if ids.len() != files.len() {
        return Err(format!(
            "添付ファイルアップロード数が一致しません (送信={}, 応答={})",
            files.len(),
            ids.len()
        ));
    }

    Ok(ids)
}

pub(super) fn append_forum_file_fields(
    fields: &mut Vec<(String, String)>,
    files: &[(String, Vec<u8>)],
    file_ids: &[String],
) {
    for (idx, (file_name, _)) in files.iter().enumerate() {
        fields.push((format!("files[{}].fileId", idx), file_ids[idx].clone()));
        fields.push((format!("files[{}].deleteFlag", idx), "0".to_string()));
        fields.push((format!("files[{}].objectName", idx), String::new()));
        fields.push((format!("files[{}].fileName", idx), file_name.clone()));
    }
}
