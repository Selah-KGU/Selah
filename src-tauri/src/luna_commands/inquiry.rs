use tauri::State;

use crate::luna_parser;
use crate::LunaState;

use super::forum::validate_forum_file_name;
use super::{
    add_text_fields, extract_url_param, html_escape, looks_like_luna_home_redirect, luna_get,
    luna_get_with_referer, luna_http, luna_post_multipart, LunaDiscussionUploadFile,
    LUNA_FORUM_FILE_MAX_BYTES,
};

/// Fetch and parse a Luna inquiry (お問い合わせ / メッセージ) detail page.
///
/// Inquiry URLs come in two flavours:
///   /lms/course/inquiry/post?idnumber=X&inquiryId=Y         — direct
///   /lms/course/inquiry/firstSet?idnumber=X&inquiryId=Y     — landing page that
///     auto-redirects via JS. We can request `post` directly with the same params.
#[tauri::command]
pub async fn luna_fetch_inquiry_detail(
    state: State<'_, LunaState>,
    db: State<'_, crate::db::Database>,
    path: String,
) -> Result<luna_parser::LunaInquiryDetail, String> {
    if path.starts_with("http") || !path.starts_with('/') {
        return Err("許可されていないパスです".into());
    }
    let cache_key = format!("luna_inquiry:{}", path);
    // Same redirect trap as the forum endpoints: the inquiry page bounces to
    // /lms/home unless Referer points at the owning course top.
    let referer_path = {
        let idn = extract_url_param(&path, "idnumber").unwrap_or_default();
        if !idn.is_empty() {
            format!("/lms/course?idnumber={}", idn)
        } else {
            "/lms/home".to_string()
        }
    };
    match luna_http(&state).await {
        Ok(http) => match luna_get_with_referer(&http, &path, &referer_path).await {
            Ok(html) => {
                if looks_like_luna_home_redirect(&html) {
                    return Err(
                        "Lunaがホーム画面にリダイレクトしました。メッセージページが見つかりません。"
                            .into(),
                    );
                }
                #[cfg(debug_assertions)]
                {
                    if crate::should_dump_debug_html() {
                        let filename = path.replace(['/', '?', '&'], "_");
                        let dump_path =
                            std::env::temp_dir().join(format!("luna_inquiry{}.html", filename));
                        let _ = std::fs::write(&dump_path, &html);
                        log::info!(
                            "Luna inquiry detail dumped to {} ({} bytes)",
                            dump_path.display(),
                            html.len()
                        );
                    }
                }
                let data = luna_parser::parse_luna_inquiry_detail(&html);
                if let Ok(json) = serde_json::to_string(&data) {
                    let _ = db.save_data_cache(&cache_key, &json);
                }
                Ok(data)
            }
            Err(e) => {
                if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                    if let Ok(cached) = serde_json::from_str(&json) {
                        log::info!("luna_inquiry: cache fallback ({})", e);
                        return Ok(cached);
                    }
                }
                Err(e)
            }
        },
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("luna_inquiry: cache fallback ({})", e);
                    return Ok(cached);
                }
            }
            Err(e)
        }
    }
}

/// Upload a single attachment for an inquiry reply.
///
/// Inquiry pages bundle ONE file slot per post (unlike forum threads which take
/// `files[N]`), so attachments are uploaded individually and the resulting
/// fileId / fileName are echoed back to be inserted into the reply form.
async fn upload_inquiry_attachment(
    http: &reqwest::Client,
    upload_action: &str,
    csrf: &str,
    idnumber: &str,
    file: &LunaDiscussionUploadFile,
) -> Result<(String, String, String), String> {
    use base64::Engine;

    let file_name = file.file_name.trim().to_string();
    validate_forum_file_name(&file_name)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&file.file_base64)
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

    let form = reqwest::multipart::Form::new()
        .text("_csrf", csrf.to_string())
        .text("idnumber", idnumber.to_string())
        .part(
            "uploadFile",
            reqwest::multipart::Part::bytes(bytes)
                .file_name(file_name.clone())
                .mime_str("application/octet-stream")
                .map_err(|e| format!("MIME error: {}", e))?,
        );

    let resp = luna_post_multipart(http, upload_action, form).await?;

    // Luna echoes JSON like { "success": true, "fileId": "...", "fileName": "...", "scanStatus": "..." }.
    // Shape inferred from the forum/report upload responses — adjust if Luna's real reply differs.
    let json: serde_json::Value = serde_json::from_str(&resp).map_err(|e| {
        format!(
            "添付ファイルアップロード応答の解析失敗: {} — body: {}",
            e,
            crate::client::safe_truncate(&resp, 200)
        )
    })?;

    if json.get("success").and_then(|v| v.as_bool()) == Some(false) {
        let msg = json
            .get("message")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| crate::client::safe_truncate(&resp, 200).to_string());
        return Err(format!("アップロード失敗: {}", msg));
    }

    let file_id = json
        .get("fileId")
        .and_then(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        })
        .ok_or("fileId が見つかりません")?;
    let returned_name = json
        .get("fileName")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| file_name.clone());
    let scan_status = json
        .get("scanStatus")
        .and_then(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        })
        .unwrap_or_default();

    Ok((file_id, returned_name, scan_status))
}

/// Post a reply to a Luna inquiry (お問い合わせ / メッセージ) thread.
///
/// Flow:
///   1. GET the inquiry page → parse hidden form fields (_csrf, idnumber,
///      inquiryId, inquiryPosts.inquiry.title, inquiryPosts.inquiry.authorId).
///   2. If an attachment is present, upload it via `inquiry_upfile` and capture
///      the returned fileId / fileName.
///   3. POST the reply form to `inquirySetForm`'s action (typically
///      `/lms/course/inquiry/postSet`) with the comment text/html and any
///      uploaded file references.
#[tauri::command]
pub async fn luna_reply_inquiry(
    state: State<'_, LunaState>,
    url: String,
    content: String,
    attachment: Option<LunaDiscussionUploadFile>,
) -> Result<String, String> {
    if url.starts_with("http") || !url.starts_with('/') {
        return Err("許可されていないパスです".into());
    }
    let content = content.trim().to_string();
    if content.is_empty() && attachment.is_none() {
        return Err("内容または添付ファイルが必要です".into());
    }

    let http = luna_http(&state).await?;
    let html = luna_get(&http, &url).await?;
    let data = luna_parser::parse_luna_inquiry_detail(&html);

    if data.post_action.is_empty() {
        return Err("メッセージ送信フォームが見つかりません".into());
    }

    let csrf = data
        .post_form_fields
        .iter()
        .find(|(k, _)| k == "_csrf")
        .map(|(_, v)| v.clone())
        .ok_or("_csrf トークンが見つかりません")?;
    let idnumber = if !data.idnumber.is_empty() {
        data.idnumber.clone()
    } else {
        extract_url_param(&url, "idnumber").unwrap_or_default()
    };
    if idnumber.is_empty() {
        return Err("idnumber が見つかりません".into());
    }

    // Optional attachment upload (single file slot).
    let uploaded = if let Some(file) = attachment.as_ref() {
        if data.upload_action.is_empty() {
            return Err("このメッセージは添付ファイル送信に対応していません".into());
        }
        Some(upload_inquiry_attachment(&http, &data.upload_action, &csrf, &idnumber, file).await?)
    } else {
        None
    };

    let content_html = if content.is_empty() {
        String::new()
    } else {
        format!("<p>{}</p>", html_escape(&content))
    };
    let content_delta = if content.is_empty() {
        String::new()
    } else {
        serde_json::json!({ "ops": [{"insert": format!("{}\n", content)}] }).to_string()
    };

    // Start from the parsed hidden fields so anything Luna adds in the future
    // (e.g. a new authorId variant) flows through automatically.
    let mut post_params: Vec<(String, String)> = data.post_form_fields.clone();

    let mut set_field = |key: &str, value: String| {
        if let Some(slot) = post_params.iter_mut().find(|(k, _)| k == key) {
            slot.1 = value;
        } else {
            post_params.push((key.to_string(), value));
        }
    };

    set_field("inquiryCommentText", content_delta);
    set_field("inquiryCommentHtml", content_html);
    set_field("inquiryComment", content.clone());
    set_field("clickedButton", "send".to_string());

    if let Some((file_id, file_name, scan_status)) = uploaded {
        set_field("inputFileId", file_id);
        set_field("inputFileName", file_name.clone());
        set_field("originalFileName", file_name);
        set_field("scanStatus", scan_status);
    }

    let form = add_text_fields(reqwest::multipart::Form::new(), &post_params);
    let resp = luna_post_multipart(&http, &data.post_action, form).await?;

    if resp.contains("\"success\":false") {
        return Err(format!(
            "送信失敗: {}",
            crate::client::safe_truncate(&resp, 200)
        ));
    }

    log::info!("Inquiry reply submitted ({} bytes response)", resp.len());
    Ok("メッセージを送信しました".to_string())
}
