use tauri::State;

use super::super::{luna_get, luna_http, report_submission_unavailable_message};
use crate::LunaState;

/// Detect report submission type by fetching the submission page
/// Returns "text", "file", or "both"
#[tauri::command]
pub async fn luna_check_report_type(
    state: State<'_, LunaState>,
    idnumber: String,
    report_id: String,
    period: Option<String>,
) -> Result<String, String> {
    let http = luna_http(&state).await?;
    let url = format!(
        "/lms/course/report/submission?idnumber={}&reportId={}",
        idnumber, report_id
    );
    let html = luna_get(&http, &url).await?;

    let has_textarea =
        html.contains("id=\"submissionText\"") || html.contains("name=\"submissionText\"");
    // File upload: look for file input or drag-and-drop area
    let has_file = html.contains("id=\"uploadFile\"")
        || html.contains("name=\"uploadFile\"")
        || html.contains("type=\"file\"")
        || html.contains("dragAndDrop");

    if !has_textarea && !has_file {
        if let Some(message) = report_submission_unavailable_message(&html, period.as_deref()) {
            return Err(message);
        }
    }

    let result = match (has_textarea, has_file) {
        (true, true) => "both",
        (true, false) => "text",
        (false, true) => "file",
        (false, false) => "file", // default fallback
    };
    log::info!(
        "Report type detection: idnumber={}, reportId={}, textarea={}, file={} → {}",
        idnumber,
        report_id,
        has_textarea,
        has_file,
        result
    );
    Ok(result.into())
}
