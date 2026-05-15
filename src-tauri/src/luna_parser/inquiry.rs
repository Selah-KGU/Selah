use super::*;

// ──────────────────────────────────────────────
// Inquiry (お問い合わせ / メッセージ) detail page
//
// URL pattern: /lms/course/inquiry/post?idnumber=X&inquiryId=Y
// Layout:
//   form#inquirySetForm           — reply form (posts to action)
//     input[name=_csrf]
//     input[name=idnumber]
//     input[name=inquiryId]
//     input[name=inquiryPosts.inquiry.title]
//     #threadPostParts
//       .discussion-thread
//         .discussion-message-block.discussion-other|self  (repeated)
//           script with _QuillUtil.contents_N.setJsonData(...)
//           .discussion-message-main
//             .ql-container .ql-editor   ← rendered HTML body
//             .discuss_mess_file (optional)
//               .downloadFile / .fileName / .objectName / .postId / .scanStatus
//           .message-margin-top
//             span(date), "氏名:", span(author)
//             .contents-hidden.postId / fileName / objectName / contents.break
//   form#inquiryPostFile           — file download form (action / idnumber)
//   form#inquiryFileForm           — file upload form (action / idnumber / _csrf)
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaInquiryPost {
    pub post_id: String,
    pub author: String,
    pub date: String,
    /// Rendered HTML extracted from .ql-editor (preserves color/bold/links).
    pub content_html: String,
    /// Plain-text fallback from `.contents-hidden.contents`.
    pub content_text: String,
    pub is_self: bool,
    pub is_teacher: bool,
    pub attachments: Vec<LunaAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaInquiryDetail {
    pub title: String,
    pub course_name: String,
    pub posts: Vec<LunaInquiryPost>,
    /// Path to POST the reply to (e.g. `/lms/course/inquiry/postSet`).
    pub post_action: String,
    /// Hidden form fields for the reply — _csrf, idnumber, inquiryId, title, etc.
    /// Names ending in `Html`/`Text`/`inquiryComment` are caller-filled.
    pub post_form_fields: Vec<(String, String)>,
    /// Path to POST attachment uploads to (e.g. `/lms/course/inquiry/inquiry_upfile`).
    pub upload_action: String,
    pub upload_form_fields: Vec<(String, String)>,
    pub idnumber: String,
    pub inquiry_id: String,
}

fn collect_form_hidden_fields(
    form: scraper::ElementRef<'_>,
    skip: &[&str],
) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    for input in form.select(&SEL_HIDDEN_INPUT) {
        let name = input.value().attr("name").unwrap_or_default();
        let value = input.value().attr("value").unwrap_or_default();
        if name.is_empty() || skip.contains(&name) {
            continue;
        }
        fields.push((name.to_string(), value.to_string()));
    }
    fields
}

fn extract_inquiry_attachment(
    file_el: scraper::ElementRef<'_>,
    download_action: &str,
    download_fixed: &[(String, String)],
    fallback_post_id: &str,
) -> Option<LunaAttachment> {
    let file_name = file_el
        .select(&SEL_INQUIRY_FILENAME_INPUT)
        .next()
        .and_then(|e| e.value().attr("value"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            file_el
                .select(&SEL_DOWNLOAD_FILE)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .filter(|s| !s.is_empty())
        })?;

    let object_name = file_el
        .select(&SEL_INQUIRY_OBJECTNAME_INPUT)
        .next()
        .and_then(|e| e.value().attr("value"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if object_name.is_empty() {
        return None;
    }

    let post_id = file_el
        .select(&SEL_INQUIRY_POSTID_INPUT)
        .next()
        .and_then(|e| e.value().attr("value"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback_post_id.to_string());

    let scan_status = file_el
        .select(&SEL_INQUIRY_SCANSTATUS_INPUT)
        .next()
        .and_then(|e| e.value().attr("value"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let mut params = download_fixed.to_vec();
    params.push(("fileId".to_string(), object_name.clone()));
    params.push(("fileName".to_string(), file_name.clone()));
    if !post_id.is_empty() {
        params.push(("postId".to_string(), post_id));
    }
    if !scan_status.is_empty() {
        params.push(("scanStatus".to_string(), scan_status));
    }

    Some(LunaAttachment {
        name: file_name,
        url: String::new(),
        link_type: "file".to_string(),
        object_name,
        download_action: download_action.to_string(),
        download_params: params,
    })
}

pub fn parse_luna_inquiry_detail(html: &str) -> LunaInquiryDetail {
    let doc = Html::parse_document(html);

    let course_name = try_selectors_text(&doc, &[".course-title-txt"]);

    // Thread title from the bordered block header. Fall back to the page title.
    let title = doc
        .select(&SEL_INQUIRY_BLOCK_TITLE)
        .next()
        .map(|e| e.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| try_selectors_text(&doc, &[".contents-title-txt"]));

    // Download form: /lms/course/inquiry/inquiry_postfile, fixed = idnumber, csrf
    let (download_action, download_fixed) = doc
        .select(&SEL_INQUIRY_POSTFILE_FORM)
        .next()
        .map(|form| {
            let action = form.value().attr("action").unwrap_or_default().to_string();
            // fileId / fileName are per-attachment, _csrf is harmless to send but unnecessary.
            let fields = collect_form_hidden_fields(form, &["fileId", "fileName"]);
            (action, fields)
        })
        .unwrap_or_default();

    // Upload form: /lms/course/inquiry/inquiry_upfile, captures csrf + idnumber.
    let (upload_action, upload_form_fields) = doc
        .select(&SEL_INQUIRY_UPFILE_FORM)
        .next()
        .map(|form| {
            let action = form.value().attr("action").unwrap_or_default().to_string();
            let fields = collect_form_hidden_fields(form, &[]);
            (action, fields)
        })
        .unwrap_or_default();

    // Reply form: /lms/course/inquiry/postSet, drop fields the renderer will fill.
    let (post_action, post_form_fields, idnumber, inquiry_id) = doc
        .select(&SEL_INQUIRY_FORM)
        .next()
        .map(|form| {
            let action = form.value().attr("action").unwrap_or_default().to_string();
            // The renderer assigns inquiryComment* + clickedButton at submit time.
            let fields = collect_form_hidden_fields(
                form,
                &[
                    "inquiryComment",
                    "inquiryCommentText",
                    "inquiryCommentHtml",
                    "clickedButton",
                ],
            );
            let idnumber = fields
                .iter()
                .find(|(k, _)| k == "idnumber")
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            let inquiry_id = fields
                .iter()
                .find(|(k, _)| k == "inquiryId")
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            (action, fields, idnumber, inquiry_id)
        })
        .unwrap_or_default();

    // Each .discussion-message-block is one message in the thread.
    let mut posts = Vec::new();
    for block in doc.select(&SEL_INQUIRY_MSG_BLOCK) {
        let classes: Vec<&str> = block.value().classes().collect();
        let is_self = classes.contains(&"discussion-self");
        let is_teacher = block
            .select(&SEL_INQUIRY_MSG_MAIN)
            .next()
            .map(|e| {
                e.value()
                    .classes()
                    .any(|c| c == "discussion-teacher-color" || c == "discussion-teacher-comment")
            })
            .unwrap_or(false);

        // Footer carries author/date/postId in known positions.
        let (date, author, post_id, content_text) = block
            .select(&SEL_INQUIRY_MSG_FOOTER)
            .next()
            .map(|footer| {
                let spans: Vec<String> = footer
                    .select(&SEL_SPAN)
                    .map(|s| s.text().collect::<String>().trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                // Pattern: [date, "氏名:", author, optional 学生番号 label, optional id]
                let date = spans.first().cloned().unwrap_or_default();
                let author = spans
                    .iter()
                    .skip_while(|s| !s.starts_with("氏名"))
                    .nth(1)
                    .cloned()
                    .unwrap_or_default();
                let post_id = footer
                    .select(&SEL_INQUIRY_HIDDEN_POSTID)
                    .next()
                    .map(|e| e.text().collect::<String>().trim().to_string())
                    .unwrap_or_default();
                let content_text = footer
                    .select(&SEL_INQUIRY_HIDDEN_CONTENTS)
                    .next()
                    .map(|e| e.text().collect::<String>().trim().to_string())
                    .unwrap_or_default();
                (date, author, post_id, content_text)
            })
            .unwrap_or_default();

        // Rendered Quill HTML in .ql-editor; fall back to plain text below if empty.
        let content_html = block
            .select(&SEL_INQUIRY_QL_EDITOR)
            .next()
            .map(|e| e.inner_html().trim().to_string())
            .unwrap_or_default();

        // Attachments — same shape as forum_postfile but with the inquiry endpoint.
        let mut attachments = Vec::new();
        if !download_action.is_empty() {
            for file_el in block.select(&SEL_INQUIRY_MSG_FILE) {
                if let Some(att) =
                    extract_inquiry_attachment(file_el, &download_action, &download_fixed, &post_id)
                {
                    attachments.push(att);
                }
            }
        }

        if content_html.is_empty()
            && content_text.is_empty()
            && attachments.is_empty()
            && author.is_empty()
        {
            continue;
        }

        posts.push(LunaInquiryPost {
            post_id,
            author,
            date,
            content_html,
            content_text,
            is_self,
            is_teacher,
            attachments,
        });
    }

    LunaInquiryDetail {
        title,
        course_name,
        posts,
        post_action,
        post_form_fields,
        upload_action,
        upload_form_fields,
        idnumber,
        inquiry_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_HTML: &str = r#"
<div class="page-main-set">
  <div class="course-header">
    <div class="course-title-txt">日本語I 4</div>
    <div class="contents-title"><div class="contents-title-txt">メッセージ</div></div>
  </div>
  <form action="/lms/course/inquiry/postSet" method="post" id="inquirySetForm" enctype="multipart/form-data">
    <input type="hidden" name="_csrf" value="csrf-abc">
    <input type="hidden" id="inquiryComment_Text" name="inquiryCommentText" value="">
    <input type="hidden" id="inquiryComment_Html" name="inquiryCommentHtml" value="">
    <input type="hidden" id="inquiryComment" name="inquiryComment" value="">
    <div class="block clearfix">
      <div class="block-title block-wide inquiry-color">
        <div class="block-title-txt break-word">グループ発表 スライド コメント</div>
      </div>
      <div id="threadPostParts">
        <div class="discussion-main-board">
          <div class="discussion-thread">
            <div class="discussion-message-block discussion-other">
              <div class="discussion-message-main discussion-other-comment discussion-teacher-color discussion-teacher-comment">
                <div id="contentsEditor_0" class="ql-container">
                  <div class="ql-editor"><p>スライドの作成お疲れ様です。</p></div>
                </div>
                <div class="discuss_mess_file">
                  <span class="link-txt downloadFile">comment.pdf</span>
                  <input type="hidden" class="fileName" value="comment.pdf">
                  <input type="hidden" class="objectName" value="OBJ-1">
                  <input type="hidden" class="postId" value="452754">
                  <input type="hidden" class="scanStatus" value="1">
                </div>
              </div>
              <div class="message-margin-top">
                <span>2026/05/05 22:12</span>
                <span>氏名:</span>
                <span>掛橋 智佳子</span>
                <div class="contents-hidden postId">452754</div>
                <div class="contents-hidden contents break">スライドの作成お疲れ様です。</div>
              </div>
            </div>
            <div class="discussion-message-block discussion-self">
              <div class="discussion-message-main discussion-self-color discussion-self-comment">
                <div class="ql-container"><div class="ql-editor ql-blank"><p><br></p></div></div>
                <div class="discuss_mess_file">
                  <span class="link-txt downloadFile">slides.pptx</span>
                  <input type="hidden" class="fileName" value="slides.pptx">
                  <input type="hidden" class="objectName" value="OBJ-2">
                  <input type="hidden" class="postId" value="455839">
                </div>
              </div>
              <div class="message-margin-top">
                <span>2026/05/12 00:08</span>
                <span>氏名:</span>
                <span>オウ メイオン</span>
                <div class="contents-hidden postId">455839</div>
                <div class="contents-hidden contents break"></div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
    <input type="hidden" name="idnumber" class="idnumber" value="2026510010040201">
    <input type="hidden" name="inquiryId" class="inquiryId" value="320411">
    <input type="hidden" name="inquiryPosts.inquiry.title" class="title" value="グループ発表 スライド コメント">
    <input type="hidden" name="clickedButton" class="clickedButton" value="">
  </form>
  <form action="/lms/course/inquiry/inquiry_postfile" method="get" id="inquiryPostFile">
    <input type="hidden" class="idnumber" name="idnumber" value="2026510010040201">
    <input type="hidden" name="fileId" id="fileId">
    <input type="hidden" name="fileName" id="fileName">
  </form>
  <form action="/lms/course/inquiry/inquiry_upfile" method="post" id="inquiryFileForm" enctype="multipart/form-data">
    <input type="hidden" name="_csrf" value="csrf-abc">
    <input type="hidden" class="idnumber" name="idnumber" value="2026510010040201">
  </form>
</div>
"#;

    #[test]
    fn parses_header_and_form_fields() {
        let r = parse_luna_inquiry_detail(SAMPLE_HTML);
        assert_eq!(r.course_name, "日本語I 4");
        assert_eq!(r.title, "グループ発表 スライド コメント");
        assert_eq!(r.idnumber, "2026510010040201");
        assert_eq!(r.inquiry_id, "320411");
        assert_eq!(r.post_action, "/lms/course/inquiry/postSet");
        // _csrf must survive, the inquiryComment* placeholders must be dropped.
        assert!(r.post_form_fields.iter().any(|(k, _)| k == "_csrf"));
        assert!(!r
            .post_form_fields
            .iter()
            .any(|(k, _)| k == "inquiryCommentText"));
        assert_eq!(r.upload_action, "/lms/course/inquiry/inquiry_upfile");
    }

    #[test]
    fn parses_two_posts_with_attachments_and_role() {
        let r = parse_luna_inquiry_detail(SAMPLE_HTML);
        assert_eq!(r.posts.len(), 2);

        let teacher = &r.posts[0];
        assert!(teacher.is_teacher);
        assert!(!teacher.is_self);
        assert_eq!(teacher.author, "掛橋 智佳子");
        assert_eq!(teacher.date, "2026/05/05 22:12");
        assert_eq!(teacher.post_id, "452754");
        assert!(teacher.content_html.contains("スライドの作成"));
        assert_eq!(teacher.attachments.len(), 1);
        assert_eq!(teacher.attachments[0].name, "comment.pdf");
        assert_eq!(teacher.attachments[0].object_name, "OBJ-1");
        assert_eq!(
            teacher.attachments[0].download_action,
            "/lms/course/inquiry/inquiry_postfile"
        );
        assert!(teacher.attachments[0]
            .download_params
            .iter()
            .any(|(k, v)| k == "idnumber" && v == "2026510010040201"));
        assert!(teacher.attachments[0]
            .download_params
            .iter()
            .any(|(k, v)| k == "fileId" && v == "OBJ-1"));

        let me = &r.posts[1];
        assert!(me.is_self);
        assert!(!me.is_teacher);
        assert_eq!(me.author, "オウ メイオン");
        assert_eq!(me.attachments.len(), 1);
        assert_eq!(me.attachments[0].name, "slides.pptx");
    }
}
