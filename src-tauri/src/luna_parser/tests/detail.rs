use super::super::*;

#[test]
#[ignore] // requires local HTML dump file
fn test_parse_announcement_detail() {
    let html = std::fs::read_to_string("/tmp/luna_announcement_2026510010040201_414248.html")
        .expect("Announcement HTML dump file not found");
    let result = parse_luna_announcement_detail(&html);
    println!("Title: {}", result.title);
    println!("Sections: {}", result.sections.len());
    for s in &result.sections {
        let preview: String = s.body.chars().take(100).collect();
        println!("  heading='{}' body='{}'", s.heading, preview);
    }
    println!("Meta: {:?}", result.meta);
    assert!(!result.title.is_empty(), "Title should not be empty");
    assert!(
        !result.sections.is_empty(),
        "Should have body content from Quill"
    );
    // The body should contain the teacher's message
    let body = &result.sections[0].body;
    assert!(body.contains("掛橋"), "Body should contain teacher name");
    assert!(
        body.contains("オンデマンド"),
        "Body should contain 'オンデマンド'"
    );
    // Meta should have 掲示期間 and 発信者
    assert!(
        result.meta.iter().any(|(k, _)| k == "掲示期間"),
        "Should have 掲示期間"
    );
    assert!(
        result.meta.iter().any(|(k, _)| k == "発信者"),
        "Should have 発信者"
    );
}

#[test]
fn test_parse_announcement_detail_ignores_unrelated_page_quill() {
    let html = r#"
        <div id="osiraseTitle">第1回授業のお知らせ</div>
        <script>
          _QuillUtil.portalNotice.setJsonData("{\"ops\":[{\"insert\":\"時間割\\nゲストアクセスと履修登録は違います。\\n\"}]}", 'reference');
        </script>
        <div class="contents-detail contents-vertical">
          <div class="contents-header-txt"><span class="bold-txt">内容</span></div>
          <div class="contents-input-area">
            <script>
              _QuillUtil.infoBody.setJsonData("{\"ops\":[{\"insert\":\"初回授業は対面で実施します。\\n\"}]}", 'reference');
            </script>
          </div>
        </div>
        <div class="contents-detail contents-vertical">
          <div class="contents-header-txt"><span class="bold-txt">発信者</span></div>
          <div class="contents-input-area">山田太郎</div>
        </div>
    "#;

    let result = parse_luna_announcement_detail(html);
    assert_eq!(result.title, "第1回授業のお知らせ");
    assert_eq!(result.sections.len(), 1);
    assert!(result.sections[0]
        .body
        .contains("初回授業は対面で実施します。"));
    assert!(!result.sections[0].body.contains("ゲストアクセス"));
}

#[test]
fn test_parse_announcement_detail_preserves_quill_image_embeds() {
    let html = r#"
        <div id="osiraseTitle">画像つきのお知らせ</div>
        <div class="contents-detail contents-vertical">
          <div class="contents-header-txt"><span class="bold-txt">内容</span></div>
          <div class="contents-input-area">
            <script>
              _QuillUtil.infoBody.setJsonData("{\"ops\":[{\"insert\":\"座席表です。\\n\"},{\"insert\":{\"image\":\"/lms/information/file/view/sample.png\"},\"attributes\":{\"alt\":\"座席表\"}},{\"insert\":\"\\n\"}]}", 'reference');
            </script>
          </div>
        </div>
    "#;

    let result = parse_luna_announcement_detail(html);
    assert_eq!(result.title, "画像つきのお知らせ");
    assert_eq!(result.sections.len(), 1);
    assert!(result.sections[0]
        .body
        .contains("<img src=\"/lms/information/file/view/sample.png\" alt=\"座席表\">"));
}

#[test]
fn test_parse_detail_page_ignores_unrelated_page_quill() {
    let html = r#"
        <html>
          <head><title>課題1 提出</title></head>
          <body>
            <div class="course-title-txt">データサイエンス入門</div>
            <div class="contents-title-txt">課題1 提出</div>
            <script>
              _QuillUtil.portalNotice.setJsonData("{\"ops\":[{\"insert\":\"時間割\\nLUNAサポートからのお知らせ\\n\"}]}", 'reference');
            </script>
            <div class="contents-detail contents-vertical">
              <div class="contents-header-txt"><span class="bold-txt">内容</span></div>
              <div class="contents-input-area">
                <script>
                  _QuillUtil.reportBody.setJsonData("{\"ops\":[{\"insert\":\"レポート本文をPDFで提出してください。\\n\"}]}", 'reference');
                </script>
              </div>
            </div>
            <div class="contents-detail contents-vertical">
              <div class="contents-header-txt"><span class="bold-txt">提出期限</span></div>
              <div class="contents-input-area">2026/04/30 23:59</div>
            </div>
          </body>
        </html>
    "#;

    let result = parse_luna_detail_page(html);
    assert_eq!(result.title, "課題1 提出");
    assert_eq!(result.course_name, "データサイエンス入門");
    assert_eq!(result.sections.len(), 1);
    assert!(result.sections[0]
        .body
        .contains("レポート本文をPDFで提出してください。"));
    assert!(!result.sections[0].body.contains("LUNAサポート"));
    assert!(result
        .meta
        .iter()
        .any(|(k, v)| k == "提出期限" && v == "2026/04/30 23:59"));
}

#[test]
fn test_parse_detail_page_preserves_quill_image_embeds() {
    let html = r#"
        <html>
          <head><title>図解資料</title></head>
          <body>
            <div class="course-title-txt">情報処理</div>
            <div class="contents-title-txt">図解資料</div>
            <div class="contents-detail contents-vertical">
              <div class="contents-header-txt"><span class="bold-txt">内容</span></div>
              <div class="contents-input-area">
                <script>
                  _QuillUtil.materialBody.setJsonData("{\"ops\":[{\"insert\":\"下の図を確認してください。\\n\"},{\"insert\":{\"image\":\"/lms/course/file/image/sample.png\"},\"attributes\":{\"alt\":\"図1\"}},{\"insert\":\"\\n\"}]}", 'reference');
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
        .contains("<img src=\"/lms/course/file/image/sample.png\" alt=\"図1\">"));
}

#[test]
fn test_parse_detail_page_accepts_unlabeled_report_body_row() {
    let html = r#"
        <html>
          <head><title>第7回 レポート課題</title></head>
          <body>
            <div class="course-title-txt">アルゴリズムとデータ構造</div>
            <div class="contents-title-txt">第7回 レポート課題</div>
            <div class="contents-detail contents-vertical">
              <div class="contents-input-area">
                <script>
                  _QuillUtil.reportBody.setJsonData("{\"ops\":[{\"insert\":\"グラフ探索アルゴリズムの比較を800字程度でまとめてください。\\n\"}]}", 'reference');
                </script>
              </div>
            </div>
            <div class="contents-detail contents-vertical">
              <div class="contents-header-txt"><span class="bold-txt">提出期限</span></div>
              <div class="contents-input-area">2026/05/01 23:59</div>
            </div>
          </body>
        </html>
    "#;

    let result = parse_luna_detail_page(html);
    assert_eq!(result.sections.len(), 1);
    assert!(result.sections[0]
        .body
        .contains("グラフ探索アルゴリズムの比較を800字程度でまとめてください。"));
    assert!(result
        .meta
        .iter()
        .any(|(k, v)| k == "提出期限" && v == "2026/05/01 23:59"));
}
