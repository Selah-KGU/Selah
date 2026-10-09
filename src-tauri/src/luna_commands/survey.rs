//! Luna survey fetch, submit, and answer-field helpers.

use super::*;

/// Fetch and parse a Luna survey detail page
#[tauri::command]
pub async fn luna_fetch_survey_detail(
    state: State<'_, LunaState>,
    db: crate::db::AccountDb,
    path: String,
) -> Result<luna_parser::LunaSurveyDetail, String> {
    if path.starts_with("http") || !path.starts_with('/') {
        return Err("許可されていないパスです".into());
    }
    let cache_key = format!("luna_survey:{}", path);
    match luna_http(&state).await {
        Ok(http) => match luna_get(&http, &path).await {
            Ok(html) => {
                #[cfg(debug_assertions)]
                {
                    if crate::should_dump_debug_html() {
                        let filename = path.replace(['/', '?', '&'], "_");
                        let dump_path =
                            std::env::temp_dir().join(format!("luna_survey{}.html", filename));
                        let _ = std::fs::write(&dump_path, &html);
                        log::info!(
                            "Luna survey detail dumped to {} ({} bytes)",
                            dump_path.display(),
                            html.len()
                        );
                    }
                }
                let data = luna_parser::parse_luna_survey_detail(&html);
                if let Ok(json) = serde_json::to_string(&data) {
                    let _ = db.save_data_cache(&cache_key, &json);
                }
                Ok(data)
            }
            Err(e) => {
                if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                    if let Ok(cached) = serde_json::from_str(&json) {
                        log::info!("luna_survey: cache fallback ({})", e);
                        return Ok(cached);
                    }
                }
                Err(e)
            }
        },
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("luna_survey: cache fallback ({})", e);
                    return Ok(cached);
                }
            }
            Err(e)
        }
    }
}

/// Submit survey answers to Luna
#[tauri::command]
pub async fn luna_submit_survey(
    state: State<'_, LunaState>,
    form_fields: Vec<(String, String)>,
    answers: std::collections::HashMap<String, serde_json::Value>,
    submit_path: Option<String>,
    referer_path: Option<String>,
) -> Result<(), String> {
    // Build the full POST params: hidden fields + user answers
    let mut params: Vec<(String, String)> = Vec::new();

    // Add all hidden form fields (includes _cid, _csrf, idnumber, surveyId, takeFlag,
    // answer[N].surveyNo, answer[N].surveyNoSub, answerDetail[N].*, enableSurveyItems[N])
    for (k, v) in &form_fields {
        params.push((k.clone(), v.clone()));
    }

    // Merge user answers: answers map is {questionIndex: selectedValue | selectedValues[]}
    for (idx_str, value) in &answers {
        let idx: usize = idx_str.parse().map_err(|_| "無効な質問インデックスです")?;
        let (answer_name, answer_value) = survey_answer_payload(idx, value);
        let values = survey_answer_values(answer_value, !answer_name.is_empty());
        for (item_idx, answer_value) in values.iter().enumerate() {
            let field_name = survey_answer_field_name(&answer_name, idx, item_idx);
            // Replace existing empty field or add new one
            let mut found = false;
            for p in &mut params {
                if p.0 == field_name {
                    p.1 = answer_value.clone();
                    found = true;
                    break;
                }
            }
            if !found {
                params.push((field_name, answer_value.clone()));
            }
        }
    }

    let http = luna_http(&state).await?;
    let submit_path = normalize_luna_relative_path(
        submit_path
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("/lms/course/surveys/take"),
    )?;
    let referer_path = normalize_luna_relative_path(
        referer_path
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&submit_path),
    )?;
    log::debug!(
        "luna_submit_survey: posting to {} with referer {} ({} fields)",
        client::safe_truncate(&submit_path, 120),
        client::safe_truncate(&referer_path, 120),
        params.len()
    );
    let response = luna_post_with_referer(&http, &submit_path, &referer_path, &params).await?;

    if let Some(error) = detect_survey_submit_error(&response) {
        return Err(error);
    }

    Ok(())
}

pub(super) fn normalize_luna_relative_path(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let path = if let Some(rest) = trimmed.strip_prefix(config::LUNA_BASE) {
        rest
    } else {
        trimmed
    };
    if path.contains("://") || path.contains('\n') || path.contains('\r') || !path.starts_with('/')
    {
        return Err("許可されていないパスです".into());
    }
    Ok(path.to_string())
}

pub(super) fn detect_survey_submit_error(response: &str) -> Option<String> {
    if response.contains("回答期間を過ぎている") {
        return Some("回答期間を過ぎています".to_string());
    }
    if response.contains("answer-type-") && response.contains("-error") {
        let text = html_to_compact_text(response);
        for marker in ["入力してください", "選択してください", "文字以内", "エラー"]
        {
            if text.contains(marker) {
                return Some(format!("回答を送信できませんでした: {}", marker));
            }
        }
        if response.contains("回答する") && response.contains("survey_question_subblock") {
            return Some("回答を送信できませんでした。入力内容を確認してください".to_string());
        }
    }
    if response.contains("survey_question_subblock") && response.contains("answer-btn") {
        return Some("回答が受け付けられませんでした。入力内容を確認してください".to_string());
    }
    None
}

fn html_to_compact_text(html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn survey_answer_payload(
    idx: usize,
    value: &serde_json::Value,
) -> (String, &serde_json::Value) {
    if let serde_json::Value::Object(obj) = value {
        let name = obj
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("answer[{}].answerItem[0].answer", idx));
        let answer_value = obj.get("value").unwrap_or(value);
        return (name, answer_value);
    }
    (format!("answer[{}].answerItem[0].answer", idx), value)
}

pub(super) fn survey_answer_values(value: &serde_json::Value, keep_empty: bool) -> Vec<String> {
    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(|s| s.to_string()))
            .filter(|s| keep_empty || !s.is_empty())
            .collect(),
        serde_json::Value::String(s) if keep_empty || !s.is_empty() => vec![s.clone()],
        serde_json::Value::Number(n) => vec![n.to_string()],
        serde_json::Value::Bool(b) => vec![b.to_string()],
        _ => Vec::new(),
    }
}

pub(super) fn survey_answer_field_name(base_name: &str, idx: usize, item_idx: usize) -> String {
    if item_idx == 0 {
        return base_name.to_string();
    }
    let marker = ".answerItem[";
    if let Some(start) = base_name.find(marker) {
        let after_start = start + marker.len();
        if let Some(end_rel) = base_name[after_start..].find(']') {
            let end = after_start + end_rel;
            return format!(
                "{}{}{}",
                &base_name[..after_start],
                item_idx,
                &base_name[end..]
            );
        }
    }
    format!("answer[{}].answerItem[{}].answer", idx, item_idx)
}
