use tauri::State;

use super::super::{
    extract_input_value, extract_report_token, luna_get, luna_http, luna_post_multipart_with_cid,
    SEL_FORM, SEL_HIDDEN_INPUT, SEL_REPORT_FORM,
};
use crate::client;
use crate::config;
use crate::luna_client;
use crate::LunaState;

/// Submit a report (課題提出) to Luna
/// Flow: 1) GET submission page → extract _cid, _csrf
///       2) POST /lms/course/report/upload (multipart) → get fileId
///       3) POST /lms/course/report/submission → confirm
#[tauri::command]
pub async fn luna_submit_report(
    state: State<'_, LunaState>,
    idnumber: String,
    report_id: String,
    period: Option<String>,
    file_name: String,
    file_base64: String,
) -> Result<String, String> {
    use base64::Engine;
    let http = luna_http(&state).await?;

    // Decode base64 file data
    let file_bytes = base64::engine::general_purpose::STANDARD
        .decode(&file_base64)
        .map_err(|e| format!("Base64デコード失敗: {}", e))?;

    log::info!(
        "Report submission: idnumber={}, reportId={}, file={} ({}B)",
        idnumber,
        report_id,
        file_name,
        file_bytes.len()
    );

    // Step 1: Fetch the submission page to get _cid and _csrf tokens
    let submission_url = format!(
        "/lms/course/report/submission?idnumber={}&reportId={}",
        idnumber, report_id
    );
    let page_html = luna_get(&http, &submission_url).await?;

    let cid = extract_report_token(&page_html, "_cid", period.as_deref())?;
    let csrf = extract_report_token(&page_html, "_csrf", period.as_deref())?;

    log::info!(
        "Report tokens: _cid={}..., _csrf={}...",
        crate::client::safe_truncate(&cid, 8),
        crate::client::safe_truncate(&csrf, 8)
    );

    // Step 2: Upload file via multipart POST (AJAX endpoint — _cid goes in URL)
    let upload_form = reqwest::multipart::Form::new()
        .text("_cid", cid.clone())
        .text("_csrf", csrf.clone())
        .text("method", "0".to_string())
        .text("idnumber", idnumber.clone())
        .text("reportId", report_id.clone())
        .part(
            "uploadFile",
            reqwest::multipart::Part::bytes(file_bytes)
                .file_name(file_name.clone())
                .mime_str("application/octet-stream")
                .map_err(|e| format!("MIME error: {}", e))?,
        );

    let upload_resp =
        luna_post_multipart_with_cid(&http, "/lms/course/report/upload", &cid, upload_form).await?;

    let upload_json: serde_json::Value = serde_json::from_str(&upload_resp).map_err(|e| {
        format!(
            "アップロード応答の解析失敗: {} — body: {}",
            e,
            crate::client::safe_truncate(&upload_resp, 200)
        )
    })?;

    if upload_json.get("success").and_then(|v| v.as_bool()) != Some(true) {
        let msg = upload_json
            .get("message")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_else(|| crate::client::safe_truncate(&upload_resp, 200).to_string());
        return Err(format!("アップロード失敗: {}", msg));
    }

    let file_id = upload_json
        .get("fileId")
        .and_then(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        })
        .ok_or("fileId が見つかりません")?;

    log::info!("Report file uploaded: fileId={}", file_id);

    // Step 3: Submit to confirmation page (url-encoded form POST)
    // The browser JS clears file inputs before submit, so only text fields are sent.
    // Include fileName (comment field) as empty since the original form has it.
    let submit_params = [
        ("_cid", cid.clone()),
        ("_csrf", csrf.clone()),
        ("method", "0".to_string()),
        ("idnumber", idnumber.clone()),
        ("reportId", report_id.clone()),
        ("fileId[0]", file_id.clone()),
        ("originalFileName[0]", file_name.clone()),
        ("deleteFlag[0]", "0".to_string()),
        ("rowCounter", "1".to_string()),
        ("fileName", "".to_string()),
    ];

    let submit_url = format!("{}/lms/course/report/submission", config::LUNA_BASE);
    let raw_resp = http
        .post(&submit_url)
        .form(&submit_params)
        .send()
        .await
        .map_err(|e| format!("確認画面リクエスト失敗: {}", e))?;

    let step3_status = raw_resp.status();
    let step3_url = raw_resp.url().to_string();
    let step3_location = raw_resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let step3_content_type = raw_resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    log::info!(
        "Step 3 raw: status={}, url={}, location={:?}, content-type={:?}",
        step3_status,
        client::safe_truncate(&step3_url, 120),
        step3_location,
        step3_content_type
    );

    let confirm_html = if step3_status.is_redirection() {
        if let Some(loc) = &step3_location {
            let next_url = if loc.starts_with('/') {
                format!("{}{}", config::LUNA_BASE, loc)
            } else {
                loc.clone()
            };
            log::info!(
                "Step 3 redirect -> {}",
                client::safe_truncate(&next_url, 120)
            );
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
            let dump_path = std::env::temp_dir().join("luna_report_confirm.html");
            let _ = std::fs::write(&dump_path, &confirm_html);
            log::info!(
                "Report confirm page dumped to {} ({} bytes)",
                dump_path.display(),
                confirm_html.len()
            );
        }
    }

    if confirm_html.is_empty() {
        return Err("確認画面が空です。セッションが切れている可能性があります。".into());
    }

    // Step 4: Parse confirmation page and submit the registration form
    let (register_action, register_fields) = {
        let confirm_doc = scraper::Html::parse_document(&confirm_html);
        let confirm_cid = extract_input_value(&confirm_html, "_cid").unwrap_or_else(|| cid.clone());
        let confirm_csrf = extract_input_value(&confirm_html, "_csrf").unwrap_or(csrf);

        // Find the reportSubmissionForm specifically (not other forms on the page)
        let mut action = String::new();
        let mut fields: Vec<(String, String)> = Vec::new();

        if let Some(form_el) = confirm_doc.select(&SEL_REPORT_FORM).next() {
            if let Some(a) = form_el.value().attr("action") {
                action = a.to_string();
            }
            for input_el in form_el.select(&SEL_HIDDEN_INPUT) {
                let name = input_el.value().attr("name").unwrap_or_default();
                let value = input_el.value().attr("value").unwrap_or_default();
                if !name.is_empty() {
                    // JS changes _method from "post" to "put" when clicking "登録する"
                    if name == "_method" {
                        fields.push(("_method".to_string(), "put".to_string()));
                    } else {
                        fields.push((name.to_string(), value.to_string()));
                    }
                }
            }
        }

        // Fallback: find form by action
        if action.is_empty() {
            for form_el in confirm_doc.select(&SEL_FORM) {
                if let Some(a) = form_el.value().attr("action") {
                    if a.contains("/report/submission") && !a.contains("download") {
                        action = a.to_string();
                        break;
                    }
                }
            }
        }

        if fields.is_empty() {
            fields = vec![
                ("_cid".into(), confirm_cid),
                ("_csrf".into(), confirm_csrf),
                ("_method".into(), "put".into()),
                ("method".into(), "0".into()),
                ("idnumber".into(), idnumber.clone()),
                ("reportId".into(), report_id.clone()),
                ("submissionText".into(), "".into()),
                ("dragAndDrop".into(), "false".into()),
            ];
        }

        log::info!(
            "Step 4 fields: {:?}",
            fields
                .iter()
                .map(|(k, v)| format!("{}={}", k, client::safe_truncate(v, 20)))
                .collect::<Vec<_>>()
        );

        (action, fields)
    }; // confirm_doc dropped here

    if register_action.is_empty() {
        // Maybe the confirmation page submitted directly — check for success indicators
        if confirm_html.contains("提出が完了") || confirm_html.contains("提出済") {
            log::info!("Report submitted directly (no confirmation step)");
            return Ok(format!("「{}」を提出しました", file_name));
        }
        log::warn!("No registration form found on confirmation page");
        return Err("確認画面に登録フォームが見つかりません。dump を確認してください。".into());
    }

    log::info!(
        "Report confirm form action: {}, fields: {}",
        register_action,
        register_fields.len()
    );

    let register_url = format!("{}{}", config::LUNA_BASE, register_action);
    let raw_resp4 = http
        .post(&register_url)
        .form(&register_fields)
        .send()
        .await
        .map_err(|e| format!("登録リクエスト失敗: {}", e))?;

    let step4_status = raw_resp4.status();
    let step4_url = raw_resp4.url().to_string();
    let step4_location = raw_resp4
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let step4_content_type = raw_resp4
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    log::info!(
        "Step 4 raw: status={}, url={}, location={:?}, content-type={:?}",
        step4_status,
        client::safe_truncate(&step4_url, 120),
        step4_location,
        step4_content_type
    );

    let register_resp = if step4_status.is_redirection() {
        if let Some(loc) = &step4_location {
            let next_url = if loc.starts_with('/') {
                format!("{}{}", config::LUNA_BASE, loc)
            } else {
                loc.clone()
            };
            log::info!(
                "Step 4 redirect -> {}",
                client::safe_truncate(&next_url, 120)
            );
            client::fetch_with_redirect(
                &http,
                &next_url,
                config::LUNA_BASE,
                luna_client::LUNA_SESSION_EXPIRED_MSG,
                luna_client::is_luna_session_expired,
            )
            .await?
        } else {
            raw_resp4
                .text()
                .await
                .map_err(|e| format!("レスポンス読取失敗: {}", e))?
        }
    } else {
        raw_resp4
            .text()
            .await
            .map_err(|e| format!("レスポンス読取失敗: {}", e))?
    };

    #[cfg(debug_assertions)]
    {
        if crate::should_dump_debug_html() {
            let dump_path2 = std::env::temp_dir().join("luna_report_register_result.html");
            let _ = std::fs::write(&dump_path2, &register_resp);
            log::info!(
                "Report register response dumped to {} ({} bytes)",
                dump_path2.display(),
                register_resp.len()
            );
        }
    }

    // Verify: the result page should show completion
    if register_resp.contains("提出が完了") || register_resp.contains("完了しました") {
        log::info!("Report registration confirmed by response content");
        Ok(format!("「{}」を提出しました", file_name))
    } else if register_resp.is_empty() {
        // Some Luna actions return empty on success redirect
        log::info!("Report registration response empty, verifying...");

        // Re-fetch the original page to verify
        let verify_html = luna_get(&http, &submission_url).await?;
        #[cfg(debug_assertions)]
        {
            if crate::should_dump_debug_html() {
                let dump_path3 = std::env::temp_dir().join("luna_report_verify.html");
                let _ = std::fs::write(&dump_path3, &verify_html);
            }
        }

        // Check for "既に提出済みの成果物" section containing actual files
        // NOT just "提出済" in comments or the file_name which might match the user's name
        let has_submitted_section = verify_html.contains("submittedFile")
            || verify_html.contains("既に提出済みの成果物</")  // closed tag means content follows
            || {
                // Check if the submitted artifacts section has content (not just empty comments)
                if let Some(pos) = verify_html.find("既に提出済みの成果物") {
                    let after = &verify_html[pos..std::cmp::min(pos + 500, verify_html.len())];
                    after.contains("downloadFile") || after.contains("file-name")
                } else {
                    false
                }
            };

        if has_submitted_section {
            log::info!("Report submitted and verified via re-fetch");
            Ok(format!("「{}」を提出しました", file_name))
        } else {
            log::warn!("Report submission verification failed — no submitted files found in re-fetched page");
            Ok(format!("「{}」を提出しました（未確認）", file_name))
        }
    } else {
        log::info!(
            "Report registration completed (response {} bytes)",
            register_resp.len()
        );
        Ok(format!("「{}」を提出しました", file_name))
    }
}
