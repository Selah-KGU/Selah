use tauri::State;

use super::super::{
    extract_input_value, extract_report_token, luna_get, luna_http, SEL_FORM, SEL_HIDDEN_INPUT,
    SEL_REPORT_FORM,
};
use crate::client;
use crate::config;
use crate::luna_client;
use crate::LunaState;

/// Submit a text-based report (テキスト入力課題) to Luna
/// Flow: 1) GET submission page → extract _cid, _csrf
///       2) POST /lms/course/report/submission with submissionText → confirm
///       3) POST confirmation form → register
#[tauri::command]
pub async fn luna_submit_report_text(
    state: State<'_, LunaState>,
    idnumber: String,
    report_id: String,
    period: Option<String>,
    submission_text: String,
) -> Result<String, String> {
    let http = luna_http(&state).await?;

    if submission_text.trim().is_empty() {
        return Err("提出テキストが空です".into());
    }

    log::info!(
        "Text report submission: idnumber={}, reportId={}, text_len={}",
        idnumber,
        report_id,
        submission_text.len()
    );

    // Step 1: Fetch the submission page for tokens
    let submission_url = format!(
        "/lms/course/report/submission?idnumber={}&reportId={}",
        idnumber, report_id
    );
    let page_html = luna_get(&http, &submission_url).await?;

    let cid = extract_report_token(&page_html, "_cid", period.as_deref())?;
    let csrf = extract_report_token(&page_html, "_csrf", period.as_deref())?;

    log::info!(
        "Text report tokens: _cid={}..., _csrf={}...",
        crate::client::safe_truncate(&cid, 8),
        crate::client::safe_truncate(&csrf, 8)
    );

    // Step 2: POST submission with text content (no file upload needed)
    let submit_params = [
        ("_cid", cid.clone()),
        ("_csrf", csrf.clone()),
        ("method", "1".to_string()),
        ("idnumber", idnumber.clone()),
        ("reportId", report_id.clone()),
        ("submissionText", submission_text.clone()),
        ("rowCounter", "0".to_string()),
        ("dragAndDrop", "false".to_string()),
    ];

    let submit_url = format!("{}/lms/course/report/submission", config::LUNA_BASE);
    let raw_resp = http
        .post(&submit_url)
        .form(&submit_params)
        .send()
        .await
        .map_err(|e| format!("提出リクエスト失敗: {}", e))?;

    let step2_status = raw_resp.status();
    log::info!("Text report step 2: status={}", step2_status);

    let confirm_html = if step2_status.is_redirection() {
        if let Some(loc) = raw_resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
        {
            let next_url = if loc.starts_with('/') {
                format!("{}{}", config::LUNA_BASE, loc)
            } else {
                loc.to_string()
            };
            client::fetch_with_redirect(
                &http,
                &next_url,
                config::LUNA_BASE,
                luna_client::LUNA_SESSION_EXPIRED_MSG,
                luna_client::is_luna_session_expired,
            )
            .await?
        } else {
            raw_resp
                .text()
                .await
                .map_err(|e| format!("レスポンス読取失敗: {}", e))?
        }
    } else {
        raw_resp
            .text()
            .await
            .map_err(|e| format!("レスポンス読取失敗: {}", e))?
    };

    #[cfg(debug_assertions)]
    {
        if crate::should_dump_debug_html() {
            let dump_path = std::env::temp_dir().join("luna_report_text_confirm.html");
            let _ = std::fs::write(&dump_path, &confirm_html);
            log::info!(
                "Text report confirm page dumped to {} ({} bytes)",
                dump_path.display(),
                confirm_html.len()
            );
        }
    }

    if confirm_html.is_empty() {
        return Err("確認画面が空です。セッションが切れている可能性があります。".into());
    }

    // Check for direct success
    if confirm_html.contains("提出が完了") || confirm_html.contains("提出済") {
        log::info!("Text report submitted directly (no confirmation step)");
        return Ok("テキストを提出しました".into());
    }

    // Step 3: Parse confirmation page and submit registration form
    let (register_action, register_fields) = {
        let confirm_doc = scraper::Html::parse_document(&confirm_html);
        let confirm_cid = extract_input_value(&confirm_html, "_cid").unwrap_or_else(|| cid.clone());
        let confirm_csrf = extract_input_value(&confirm_html, "_csrf").unwrap_or(csrf);

        let mut action = String::new();
        let mut fields: Vec<(String, String)> = Vec::new();

        if let Some(form_el) = confirm_doc.select(&SEL_REPORT_FORM).next().or_else(|| {
            confirm_doc.select(&SEL_FORM).find(|f| {
                f.value()
                    .attr("action")
                    .map(|a| a.contains("/report/submission") && !a.contains("download"))
                    .unwrap_or(false)
            })
        }) {
            if let Some(a) = form_el.value().attr("action") {
                action = a.to_string();
            }
            for input_el in form_el.select(&SEL_HIDDEN_INPUT) {
                let name = input_el.value().attr("name").unwrap_or_default();
                let value = input_el.value().attr("value").unwrap_or_default();
                if !name.is_empty() {
                    if name == "_method" {
                        fields.push(("_method".to_string(), "put".to_string()));
                    } else {
                        fields.push((name.to_string(), value.to_string()));
                    }
                }
            }
        }

        if fields.is_empty() {
            fields = vec![
                ("_cid".into(), confirm_cid),
                ("_csrf".into(), confirm_csrf),
                ("_method".into(), "put".into()),
                ("method".into(), "1".into()),
                ("idnumber".into(), idnumber.clone()),
                ("reportId".into(), report_id.clone()),
                ("submissionText".into(), submission_text.clone()),
                ("dragAndDrop".into(), "false".into()),
            ];
        }

        log::info!(
            "Text report step 3 fields: {:?}",
            fields
                .iter()
                .map(|(k, v)| format!("{}={}", k, client::safe_truncate(v, 20)))
                .collect::<Vec<_>>()
        );

        (action, fields)
    };

    if register_action.is_empty() {
        if confirm_html.contains("提出が完了")
            || confirm_html.contains("完了しました")
            || confirm_html.contains("提出済")
        {
            return Ok("テキストを提出しました".into());
        }
        return Err("確認画面に登録フォームが見つかりません".into());
    }

    let register_url = format!("{}{}", config::LUNA_BASE, register_action);
    let raw_resp3 = http
        .post(&register_url)
        .form(&register_fields)
        .send()
        .await
        .map_err(|e| format!("登録リクエスト失敗: {}", e))?;

    let step3_status = raw_resp3.status();
    log::info!("Text report step 3: status={}", step3_status);

    let _register_resp = if step3_status.is_redirection() {
        if let Some(loc) = raw_resp3
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
        {
            let next_url = if loc.starts_with('/') {
                format!("{}{}", config::LUNA_BASE, loc)
            } else {
                loc.to_string()
            };
            client::fetch_with_redirect(
                &http,
                &next_url,
                config::LUNA_BASE,
                luna_client::LUNA_SESSION_EXPIRED_MSG,
                luna_client::is_luna_session_expired,
            )
            .await?
        } else {
            raw_resp3
                .text()
                .await
                .map_err(|e| format!("レスポンス読取失敗: {}", e))?
        }
    } else {
        raw_resp3
            .text()
            .await
            .map_err(|e| format!("レスポンス読取失敗: {}", e))?
    };

    #[cfg(debug_assertions)]
    {
        if crate::should_dump_debug_html() {
            let dump_path2 = std::env::temp_dir().join("luna_report_text_result.html");
            let _ = std::fs::write(&dump_path2, &_register_resp);
        }
    }

    Ok("テキストを提出しました".into())
}
