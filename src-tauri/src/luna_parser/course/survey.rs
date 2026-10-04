use super::super::{
    extract_named_quill_text, extract_quill_plain_text, extract_quill_text, Html, Selector,
    SEL_HIDDEN_INPUT, SEL_SPAN,
};
use super::model::*;

/// Parse a survey take/detail page (/lms/course/surveys/take?idnumber=...&surveyId=...)
pub fn parse_luna_survey_detail(html: &str) -> LunaSurveyDetail {
    let doc = Html::parse_document(html);

    // Extract survey download form: #surveysDownFileForm
    // Form: action=/lms/course/surveys/takefile, method=get
    // Static fields: _cid, idnumber, contentId
    // Dynamic per-file: fileId (=objectName), fileName (=raw filename)
    let survey_dl_form: Option<(String, Vec<(String, String)>)> = {
        let form_sel = Selector::parse("#surveysDownFileForm").unwrap();
        if let Some(form) = doc.select(&form_sel).next() {
            let action = form.value().attr("action").unwrap_or_default().to_string();
            if !action.is_empty() {
                let hidden_sel = Selector::parse("input[type=\"hidden\"]").unwrap();
                let params: Vec<(String, String)> = form
                    .select(&hidden_sel)
                    .filter_map(|input| {
                        let name = input.value().attr("name").unwrap_or_default();
                        let val = input.value().attr("value").unwrap_or_default();
                        if !val.is_empty() && name != "fileId" && name != "fileName" {
                            Some((name.to_string(), val.to_string()))
                        } else {
                            None
                        }
                    })
                    .collect();
                Some((action, params))
            } else {
                None
            }
        } else {
            None
        }
    };

    // Extract header info from .contents-detail.contents-vertical rows
    let mut title = String::new();
    let mut description = String::new();
    let mut period = String::new();
    let mut anonymity = String::new();
    let mut allow_edit = String::new();
    let mut answer_status = String::new();
    let mut respondent = String::new();
    let mut attachments = Vec::new();

    let header_sel =
        Selector::parse(".contents-list > .contents-detail.contents-vertical").unwrap();
    for row in doc.select(&header_sel) {
        let label_sel = Selector::parse(".contents-header .bold-txt").unwrap();
        let label = row
            .select(&label_sel)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let value_sel = Selector::parse(".contents-input-area").unwrap();
        let value_el = row.select(&value_sel).next();

        match label.as_str() {
            "タイトル" => {
                title = value_el
                    .map(|e| e.text().collect::<String>().trim().to_string())
                    .unwrap_or_default();
            }
            "内容" => {
                // Quill editor content — in static HTML, .ql-editor doesn't exist.
                // Try extracting from setJsonData in page-level scripts first.
                description = extract_named_quill_text(html, "bodyText").unwrap_or_default();
                if description.is_empty() {
                    // Fallback: try ql-editor if Quill somehow rendered
                    let ql_sel = Selector::parse(".ql-editor").unwrap();
                    description = value_el
                        .and_then(|v| v.select(&ql_sel).next())
                        .map(|e| e.text().collect::<String>().trim().to_string())
                        .unwrap_or_default();
                }
                if description.is_empty() {
                    let row_html = row.html();
                    if let Some(qt) = extract_quill_text(&row_html) {
                        description = extract_quill_plain_text(&qt).unwrap_or_default();
                    }
                }
            }
            "回答期間" => {
                period = value_el
                    .map(|e| {
                        e.select(&SEL_SPAN)
                            .map(|s| s.text().collect::<String>().trim().to_string())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
            }
            "記名・無記名" => {
                anonymity = value_el
                    .map(|e| e.text().collect::<String>().trim().to_string())
                    .unwrap_or_default();
            }
            "回答の修正" => {
                allow_edit = value_el
                    .map(|e| e.text().collect::<String>().trim().to_string())
                    .unwrap_or_default();
            }
            "回答状況" => {
                answer_status = value_el
                    .map(|e| e.text().collect::<String>().trim().to_string())
                    .unwrap_or_default();
            }
            "氏名" => {
                respondent = value_el
                    .map(|e| e.text().collect::<String>().trim().to_string())
                    .unwrap_or_default();
            }
            "添付ファイル" => {
                if let Some(area) = value_el {
                    let dl_sel = Selector::parse(".downloadFile").unwrap();
                    let obj_sel = Selector::parse(".objectName").unwrap();
                    let fname_sel = Selector::parse(".fileName").unwrap();
                    let file_name = area
                        .select(&fname_sel)
                        .next()
                        .or_else(|| area.select(&dl_sel).next())
                        .map(|e| e.text().collect::<String>().trim().to_string())
                        .unwrap_or_default();
                    let object_name = area
                        .select(&obj_sel)
                        .next()
                        .map(|e| e.text().collect::<String>().trim().to_string())
                        .unwrap_or_default();
                    if !file_name.is_empty() {
                        let (dl_action, mut dl_params) =
                            if let Some((ref act, ref params)) = survey_dl_form {
                                (act.clone(), params.clone())
                            } else {
                                (String::new(), Vec::new())
                            };
                        // Add per-file dynamic fields: fileId = objectName, fileName = raw filename
                        dl_params.push(("fileId".to_string(), object_name.clone()));
                        dl_params.push(("fileName".to_string(), file_name.clone()));
                        attachments.push(LunaSurveyAttachment {
                            file_name,
                            object_name,
                            url: String::new(),
                            download_action: dl_action,
                            download_params: dl_params,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    // Parse questions from #survey_question_subblock
    // Note: Quill content is NOT rendered into .ql-editor in static HTML.
    // Question bodies are in: _QuillUtil.surveyTakeItemText.setJsonData("{...}", 'reference');
    // Answer labels are in: _QuillUtil.answerListContents_X_Y.setJsonData("{...}", 'reference');
    // Answer types are in: <input type="hidden" class="branchType" value="list|radio|check|text|textArea|multRadio|multCheck">
    let mut questions = Vec::new();
    let q_block_sel = Selector::parse("#survey_question_subblock .question_itme").unwrap();
    let q_required_sel = Selector::parse(".contents-hissu").unwrap();
    let q_branch_type_sel = Selector::parse(".branchType").unwrap();
    let q_answer_control_sel =
        Selector::parse("textarea[name], select[name], input[name]").unwrap();

    for (q_idx, q_el) in doc.select(&q_block_sel).enumerate() {
        let required = q_el.select(&q_required_sel).next().is_some();
        let number = (q_idx + 1).to_string();

        // Extract question body from surveyTakeItemText.setJsonData in script
        let q_html = q_el.html();
        let body = extract_named_quill_text(&q_html, "surveyTakeItemText").unwrap_or_default();

        // Determine answer type from hidden branchType input
        let branch_type = q_el
            .select(&q_branch_type_sel)
            .next()
            .and_then(|e| e.value().attr("value"))
            .unwrap_or_default();
        let mut answer_type = normalize_survey_answer_type(branch_type);

        // Extract option labels from answerListContents_X_Y.setJsonData
        let mut options = Vec::new();
        if answer_type != "text" && answer_type != "textarea" {
            let mut opt_idx = 0;
            loop {
                let var_name = format!("answerListContents_{}_{}", q_idx, opt_idx);
                if let Some(label) = extract_named_quill_text(&q_html, &var_name) {
                    options.push(LunaSurveyOption {
                        value: (opt_idx + 1).to_string(),
                        label,
                    });
                    opt_idx += 1;
                } else {
                    break;
                }
            }
        }
        let answer_name = q_el
            .select(&q_answer_control_sel)
            .filter_map(|e| {
                let name = e.value().attr("name").unwrap_or_default();
                let input_type = e.value().attr("type").unwrap_or_default();
                if is_survey_answer_input_name(name, input_type) {
                    Some(name.to_string())
                } else {
                    None
                }
            })
            .next()
            .unwrap_or_default();
        if answer_type.is_empty() {
            answer_type = infer_survey_answer_type(&q_html, &answer_name);
        }

        if !body.is_empty() || !options.is_empty() {
            questions.push(LunaSurveyQuestion {
                number,
                body,
                required,
                answer_type,
                answer_name,
                options,
            });
        }
    }

    // Extract hidden form fields from #surveysTakeForm for submission
    let mut form_fields: Vec<(String, String)> = Vec::new();
    let mut form_action = String::new();
    let form_sel = Selector::parse("#surveysTakeForm").unwrap();
    if let Some(form) = doc.select(&form_sel).next() {
        form_action = form.value().attr("action").unwrap_or_default().to_string();
        for input in form.select(&SEL_HIDDEN_INPUT) {
            let name = input.value().attr("name").unwrap_or_default();
            let value = input.value().attr("value").unwrap_or_default();
            if !name.is_empty() {
                form_fields.push((name.to_string(), value.to_string()));
            }
        }
    }

    // Store download form info in attachments for the download command to use with fresh _cid
    {
        let form_selectors = [
            Selector::parse("#questionnaireDownloadForm").unwrap(),
            Selector::parse("form[action*='download']").unwrap(),
            Selector::parse("#reportDownloadForm").unwrap(),
            Selector::parse("#forumsPostFile").unwrap(),
        ];
        for sel in &form_selectors {
            if let Some(form) = doc.select(sel).next() {
                let action = form.value().attr("action").unwrap_or_default().to_string();
                if !action.is_empty() {
                    let mut params = Vec::new();
                    for input in form.select(&SEL_HIDDEN_INPUT) {
                        let iname = input.value().attr("name").unwrap_or_default();
                        let ival = input.value().attr("value").unwrap_or_default();
                        if !ival.is_empty()
                            && iname != "objectName"
                            && iname != "downloadFileName"
                            && iname != "downloadMode"
                        {
                            params.push((iname.to_string(), ival.to_string()));
                        }
                    }
                    for att in &mut attachments {
                        att.url = action.clone();
                    }
                    log::debug!(
                        "[survey attachments] action='{}', params={:?}",
                        action,
                        params
                    );
                    break;
                }
            }
        }
    }

    LunaSurveyDetail {
        title,
        description,
        period,
        anonymity,
        allow_edit,
        answer_status,
        respondent,
        attachments,
        questions,
        form_fields,
        form_action,
    }
}

fn normalize_survey_answer_type(branch_type: &str) -> String {
    match branch_type.trim().to_ascii_lowercase().as_str() {
        "list" => "list",
        "radio" | "multradio" => "radio",
        "check" | "multcheck" => "checkbox",
        "text" => "text",
        "textarea" => "textarea",
        _ => "",
    }
    .to_string()
}

fn infer_survey_answer_type(question_html: &str, answer_name: &str) -> String {
    let lower = question_html.to_ascii_lowercase();
    let answer_name = answer_name.to_ascii_lowercase();
    if answer_name.ends_with(".commenttext")
        || lower.contains("textarea")
        || (lower.contains("branchtype")
            && (lower.contains("value=\"textarea\"") || lower.contains("value='textarea'")))
    {
        return "textarea".to_string();
    }
    if lower.contains("type=\"text\"")
        || lower.contains("type='text'")
        || lower.contains("type=text")
        || (lower.contains("branchtype")
            && (lower.contains("value=\"text\"") || lower.contains("value='text'")))
    {
        return "text".to_string();
    }
    String::new()
}

fn is_survey_answer_input_name(name: &str, input_type: &str) -> bool {
    if !name.starts_with("answer[") {
        return false;
    }
    let input_type = input_type.trim().to_ascii_lowercase();
    if input_type == "hidden" || input_type == "file" || input_type == "button" {
        return false;
    }
    !(name.ends_with(".surveyNo") || name.ends_with(".surveyNoSub"))
}
