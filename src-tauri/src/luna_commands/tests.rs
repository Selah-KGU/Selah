use super::*;

#[test]
fn parses_luna_report_period_with_24_hour_deadline() {
    let (_start, end, raw_start, raw_end) =
        parse_luna_report_period("2026/04/22 17:00 ～ 2026/04/29 24:00").unwrap();

    assert_eq!(raw_start, "2026/04/22 17:00");
    assert_eq!(raw_end, "2026/04/29 24:00");
    assert_eq!(end.format("%Y/%m/%d %H:%M").to_string(), "2026/04/30 00:00");
}

#[test]
fn extracts_luna_report_period_from_html_text() {
    let html = r#"
            <div class="contents-detail contents-vertical">
              <div class="contents-header contents-header-txt"><span>提出期間</span></div>
              <div class="contents-input-area">
                <span>2999/04/22 17:00</span><span>～</span><span>2999/04/29 24:00</span>
              </div>
            </div>
        "#;

    assert_eq!(
        extract_report_period_from_html(html).as_deref(),
        Some("2999/04/22 17:00 ～ 2999/04/29 24:00")
    );
}

#[test]
fn reports_future_luna_period_as_before_start() {
    let message =
        report_period_unavailable_message(Some("2999/04/22 17:00 ～ 2999/04/29 24:00")).unwrap();

    assert!(message.contains("提出開始前です"));
    assert!(message.contains("2999/04/22 17:00 ～ 2999/04/29 24:00"));
}

#[test]
fn detects_survey_submit_returned_answer_form_as_error() {
    let html = r#"
            <div id="survey_question_subblock"></div>
            <div class="highlight-txt answer-type-textarea-error">入力してください</div>
            <a class="under-btn btn-txt btn-color answer-btn">回答する</a>
        "#;

    let error = detect_survey_submit_error(html).unwrap();
    assert!(error.contains("入力してください"));
}

#[test]
fn keeps_blank_survey_comment_text_value() {
    let value = serde_json::json!({
        "name": "answer[0].commentText",
        "value": ""
    });

    let (name, answer_value) = survey_answer_payload(0, &value);
    assert_eq!(name, "answer[0].commentText");
    assert_eq!(
        survey_answer_values(answer_value, !name.is_empty()),
        vec![""]
    );
}

#[test]
fn expands_survey_checkbox_answer_item_names() {
    assert_eq!(
        survey_answer_field_name("answer[3].answerItem[0].answer", 3, 0),
        "answer[3].answerItem[0].answer"
    );
    assert_eq!(
        survey_answer_field_name("answer[3].answerItem[0].answer", 3, 1),
        "answer[3].answerItem[1].answer"
    );
}

#[test]
fn normalizes_luna_relative_submit_paths() {
    assert_eq!(
        normalize_luna_relative_path("https://luna.kwansei.ac.jp/lms/course/surveys/take?_cid=abc")
            .unwrap(),
        "/lms/course/surveys/take?_cid=abc"
    );
    assert!(normalize_luna_relative_path("https://example.com/lms/course/surveys/take").is_err());
}
