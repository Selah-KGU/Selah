use super::super::*;
use super::super::{extract_quill_delta_html, extract_quill_delta_text};

#[test]
fn test_parse_announcement_detail_blacklists_system_notice_body() {
    let html = r#"
        <div id="osiraseTitle">授業内お知らせ</div>
        <div class="contents-detail contents-vertical">
          <div class="contents-header-txt"><span class="bold-txt">内容</span></div>
          <div class="contents-input-area">
            <script>
              _QuillUtil.infoBody.setJsonData("{\"ops\":[{\"insert\":\"時間割\\n■「ゲストアクセス」と「履修登録」は違います。単位を取得するためには、履修登録期間中に kwic にて履修登録を必ず行ってください。\\n■LUNAの定期メンテナンスについて\\n\"}]}", 'reference');
            </script>
          </div>
        </div>
        <div class="contents-detail contents-vertical">
          <div class="contents-header-txt"><span class="bold-txt">発信者</span></div>
          <div class="contents-input-area">LUNAサポート</div>
        </div>
    "#;

    let result = parse_luna_announcement_detail(html);
    assert_eq!(result.title, "授業内お知らせ");
    assert!(
        result.sections.is_empty(),
        "blacklisted notice body should be dropped"
    );
    assert!(result
        .meta
        .iter()
        .any(|(k, v)| k == "発信者" && v == "LUNAサポート"));
}

#[test]
fn test_blacklist_does_not_drop_normal_course_body_with_single_keyword() {
    let html = r#"
        <div id="osiraseTitle">動画視聴について</div>
        <div class="contents-detail contents-vertical">
          <div class="contents-header-txt"><span class="bold-txt">内容</span></div>
          <div class="contents-input-area">
            <script>
              _QuillUtil.infoBody.setJsonData("{\"ops\":[{\"insert\":\"講義動画はPanoptoボタンから確認してください。レポート提出方法は次回説明します。\\n\"}]}", 'reference');
            </script>
          </div>
        </div>
    "#;

    let result = parse_luna_announcement_detail(html);
    assert_eq!(result.sections.len(), 1);
    assert!(result.sections[0]
        .body
        .contains("講義動画はPanoptoボタンから確認してください。"));
}

#[test]
fn test_blacklist_keeps_real_body_when_notice_lines_are_mixed_in() {
    let html = r#"
        <html>
          <head><title>課題1 提出</title></head>
          <body>
            <div class="course-title-txt">データサイエンス入門</div>
            <div class="contents-title-txt">課題1 提出</div>
            <div class="contents-detail contents-vertical">
              <div class="contents-header-txt"><span class="bold-txt">内容</span></div>
              <div class="contents-input-area">
                <script>
                  _QuillUtil.reportBody.setJsonData("{\"ops\":[{\"insert\":\"時間割\\n■「ゲストアクセス」と「履修登録」は違います。\\n履修データ連携に関する補足\\nレポート本文をPDFで提出してください。\\n提出時に表紙は不要です。\\n\"}]}", 'reference');
                </script>
              </div>
            </div>
          </body>
        </html>
    "#;

    let result = parse_luna_detail_page(html);
    assert_eq!(result.sections.len(), 1);
    assert!(result.sections[0]
        .body
        .contains("レポート本文をPDFで提出してください。"));
    assert!(result.sections[0].body.contains("提出時に表紙は不要です。"));
    assert!(!result.sections[0].body.contains("ゲストアクセス"));
    assert!(!result.sections[0].body.contains("履修データ連携"));
}

#[test]
fn test_parse_survey_text_questions() {
    let html = r#"
        <form id="surveysTakeForm" action="/lms/course/surveys/take?_cid=abc">
          <input type="hidden" name="_csrf" value="token">
          <input type="hidden" name="answer[0].surveyNo" value="1">
          <input type="hidden" name="answer[1].answerItem[0].answer" value="">
          <input type="hidden" name="answer[2].answerItem[0].answer" value="">
        </form>
        <div id="survey_question_subblock">
          <div class="question_itme">
            <script>
              _QuillUtil.surveyTakeItemText.setJsonData("{\"ops\":[{\"insert\":\"本日の活動内容\\n\"}]}", 'reference');
            </script>
            <textarea id="answer_comment_0" class="answerComment branch_itme textarea" name="answer[0].commentText"></textarea>
          </div>
          <div class="question_itme">
            <input type="hidden" class="branchType" value="list">
            <script>
              _QuillUtil.surveyTakeItemText.setJsonData("{\"ops\":[{\"insert\":\"進捗度合い\\n\"}]}", 'reference');
              _QuillUtil.answerListContents_1_0.setJsonData("{\"ops\":[{\"insert\":\"0%\\n\"}]}", 'reference');
              _QuillUtil.answerListContents_1_1.setJsonData("{\"ops\":[{\"insert\":\"10%\\n\"}]}", 'reference');
            </script>
            <select id="answerSelector_1" class="answer-type-list" name="answer[1].answerItem[0].answer">
              <option value="1">0%</option>
              <option value="2">10%</option>
            </select>
          </div>
          <div class="question_itme">
            <input type="hidden" class="branchType" value="text">
            <script>
              _QuillUtil.surveyTakeItemText.setJsonData("{\"ops\":[{\"insert\":\"名前を入力してください\\n\"}]}", 'reference');
            </script>
          </div>
        </div>
    "#;

    let result = parse_luna_survey_detail(html);
    assert_eq!(result.form_action, "/lms/course/surveys/take?_cid=abc");
    assert_eq!(result.questions.len(), 3);
    assert_eq!(result.questions[0].answer_type, "textarea");
    assert_eq!(result.questions[0].answer_name, "answer[0].commentText");
    assert_eq!(result.questions[1].answer_type, "list");
    assert_eq!(
        result.questions[1].answer_name,
        "answer[1].answerItem[0].answer"
    );
    assert_eq!(result.questions[1].options.len(), 2);
    assert_eq!(result.questions[2].answer_type, "text");
}

#[test]
fn test_extract_quill_delta_text() {
    let script = r#"
        _QuillUtil.materialContents_0.setJsonData("{\"ops\":[{\"insert\":\"\u51FA\u5E2D\u78BA\u8A8D\u306F\u6388\u696D\u5192\u982D\u306B\u884C\u3044\u307E\u3059\u3002\\n\"},{\"attributes\":{\"bold\":true},\"insert\":\"\u5EA7\u5E2D\u8868\u304C\u3042\u308A\u307E\u3059\u3002\"},{\"insert\":\"\\n\"}]}", 'reference');
    "#;
    let result = extract_quill_delta_text(script);
    assert!(result.is_some(), "Should extract text from Quill Delta");
    let text = result.unwrap();
    assert!(
        text.contains("出席確認は授業冒頭に行います。"),
        "Should contain decoded Japanese text"
    );
    assert!(
        text.contains("座席表があります。"),
        "Should contain bold text too"
    );
}

#[test]
fn test_extract_quill_delta_html() {
    let script = r#"
        _QuillUtil.materialContents_0.setJsonData("{\"ops\":[{\"attributes\":{\"bold\":true},\"insert\":\"\u592A\u5B57\"},{\"insert\":\" \"},{\"attributes\":{\"italic\":true,\"link\":\"https://example.com\"},\"insert\":\"\u30EA\u30F3\u30AF\"},{\"insert\":\"\\n\"}]}", 'reference');
    "#;
    let result = extract_quill_delta_html(script);
    assert!(
        result.is_some(),
        "Should extract rich HTML from Quill Delta"
    );
    let html = result.unwrap();
    assert!(
        html.contains("<strong>太字</strong>"),
        "Should preserve bold style"
    );
    assert!(
        html.contains("<em>リンク</em>"),
        "Should preserve italic style"
    );
    assert!(
        html.contains("href=\"https://example.com\""),
        "Should preserve link href"
    );
}
