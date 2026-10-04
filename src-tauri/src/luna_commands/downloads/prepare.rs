use crate::{config, luna_client};

use super::super::luna_get;
use super::save::form_encode;

pub(super) async fn prepare_material_tempfile(
    http: &reqwest::Client,
    idnumber: &str,
    file_name: &str,
    object_name: &str,
    resource_id: &str,
    error_prefix: &str,
) -> Result<String, String> {
    let course_url = format!("/lms/course?idnumber={}", idnumber);
    if let Err(e) = luna_get(http, &course_url).await {
        if e == luna_client::LUNA_SESSION_EXPIRED_MSG {
            return Err(e);
        }
        log::warn!("Material tempfile warmup failed (continuing): {}", e);
    }

    let tempfile_query = format!(
        "fileName={}&objectName={}&id={}&idnumber={}",
        urlencoding::encode(file_name),
        urlencoding::encode(object_name),
        urlencoding::encode(resource_id),
        urlencoding::encode(idnumber),
    );
    let tempfile_path = format!("/lms/course/make/tempfile?{}", tempfile_query);
    let tempfile_full_url = format!("{}{}", config::LUNA_BASE, tempfile_path);
    let referer_full_url = format!("{}{}", config::LUNA_BASE, course_url);
    log::info!("Material tempfile URL: {}", tempfile_path);

    let resp = http
        .get(&tempfile_full_url)
        .header("Referer", &referer_full_url)
        .header("X-Requested-With", "XMLHttpRequest")
        .header("Accept", "text/plain, */*; q=0.01")
        .send()
        .await
        .map_err(|e| format!("{}: リクエスト失敗: {}", error_prefix, e))?;

    let status = resp.status();
    let final_url = resp.url().to_string();
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let content_length = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    log::info!(
        "Material tempfile response: status={}, content-type='{}', content-length='{}', final_url='{}'",
        status,
        content_type,
        content_length,
        crate::client::safe_truncate(&final_url, 200)
    );

    if final_url.contains("sso.kwansei.ac.jp") {
        return Err(luna_client::LUNA_SESSION_EXPIRED_MSG.into());
    }
    if !status.is_success() {
        return Err(format!("{}: HTTP {}", error_prefix, status));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| format!("{}: レスポンス読取失敗: {}", error_prefix, e))?;

    if luna_client::is_luna_session_expired(&body) {
        return Err(luna_client::LUNA_SESSION_EXPIRED_MSG.into());
    }

    let file_id = body.trim().to_string();
    log::info!(
        "Material tempfile returned fileId (len={}): '{}'",
        file_id.len(),
        crate::client::safe_truncate(&file_id, 500)
    );

    if file_id.is_empty() {
        return Err(
            "Luna から応答がありませんでした。少し待ってから再度お試しください。セッションが切れている場合は再ログインしてください。".into()
        );
    }
    if file_id.contains('<') {
        return Err(
            "Luna がエラーページを返しました。再ログインのうえ、もう一度お試しください。".into(),
        );
    }

    Ok(file_id)
}

pub(super) fn build_material_download_query(
    file_name: &str,
    file_id: &str,
    idnumber: &str,
    resource_id: &str,
    content_id: &str,
    end_date: &str,
    title: &str,
) -> String {
    format!(
        "fileName={}&fileId={}&idnumber={}&resourceId={}&screen=1&contentId={}&endDate={}&title={}",
        form_encode(file_name),
        form_encode(file_id),
        form_encode(idnumber),
        form_encode(resource_id),
        form_encode(content_id),
        form_encode(end_date),
        form_encode(title),
    )
}
