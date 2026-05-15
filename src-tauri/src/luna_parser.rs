use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

#[path = "luna_parser/course.rs"]
mod course;
#[path = "luna_parser/detail.rs"]
mod detail;
#[path = "luna_parser/inquiry.rs"]
mod inquiry;
#[path = "luna_parser/overview.rs"]
mod overview;

pub use course::*;
#[cfg(test)]
use course::{extract_quill_delta_html, extract_quill_delta_text};
pub(crate) use detail::is_blacklisted_system_notice_text;
#[allow(unused_imports)]
use detail::{classify_link, extract_named_quill_text, extract_quill_rich_html};
#[allow(unused_imports)]
pub use detail::{
    parse_luna_announcement_detail, parse_luna_detail_page, LunaAttachment, LunaDetailPage,
    LunaDetailSection,
};
#[allow(unused_imports)]
pub use inquiry::LunaInquiryPost;
pub use inquiry::{parse_luna_inquiry_detail, LunaInquiryDetail};
pub use overview::*;

/// Five content lists returned from the contents page (materials, reports, examinations, discussions, surveys)
pub type ContentsPageResult = (
    Vec<LunaContentItem>,
    Vec<LunaContentItem>,
    Vec<LunaContentItem>,
    Vec<LunaContentItem>,
    Vec<LunaContentItem>,
);

macro_rules! sel {
    ($name:ident, $s:expr) => {
        static $name: LazyLock<Selector> = LazyLock::new(|| Selector::parse($s).unwrap());
    };
}

// ── Timetable selectors ──
sel!(SEL_DATA_ROW, ".div-table-data-row");
sel!(SEL_PERIOD_COL, ".div-table-colomn-period");
sel!(SEL_TABLE_CELL, ".div-table-cell");
sel!(SEL_COURSE_BTN, ".timetable-course-top-btn");
sel!(SEL_CELL_DETAIL, ".div-table-cell-detail span");
sel!(
    SEL_COMMUNITY_BTN,
    ".timetable-community-course .timetable-course-top-btn"
);

// ── Todo selectors ──
sel!(SEL_TODO_LIST, ".todo-list");
sel!(SEL_TODO_COURSE, ".todolist-course");
sel!(SEL_TODO_TYPE, ".todolist-contents-type span");
sel!(SEL_TODO_NAME, ".todolist-contents-name a");
sel!(SEL_TODO_DEADLINE, ".todolist-mobile-width-deadline");
sel!(SEL_TODO_STATUS, ".todolist-contents-status span");
sel!(
    SEL_TODO_FEEDBACK,
    ".todolist-feedback .todolist-mobile-feedback"
);

// ── Notification selectors ──
sel!(SEL_NOTIF_LIST, ".update-info-list");
sel!(SEL_NOTIF_DATE, ".update-info-updateDate label");
sel!(SEL_NOTIF_COURSE, ".update-info-courseInfo span");
sel!(SEL_NOTIF_MODULE, ".update-info-module span");
sel!(SEL_NOTIF_CONTENT, ".update-info-contents .break-word");
sel!(SEL_NOTIF_URL, ".updateInfoUrl");
sel!(SEL_INPUT_IDNUMBER, "input[id='idnumber']");
sel!(SEL_INPUT_IDNAME, "input[name='idnumber']");

// ── Common shared selectors ──
sel!(SEL_DETAIL_VERT, ".contents-detail.contents-vertical");
sel!(
    SEL_BLOCK_DETAIL,
    ".block > .contents-list > .contents-detail.contents-vertical"
);
sel!(SEL_HEADER_BOLD, ".contents-header-txt .bold-txt");
sel!(
    SEL_HEADER_COMBO,
    ".contents-header-txt .bold-txt, .contents-header-txt"
);
sel!(SEL_INPUT_AREA, ".contents-input-area");
sel!(SEL_DOWNLOAD_FILE, ".downloadFile");
sel!(SEL_OBJECT_NAME, ".objectName");
sel!(SEL_HIDDEN_INPUT, "input[type='hidden']");

// ── Discussion selectors ──
sel!(
    SEL_THEME_TOP,
    "#themeTopList .result-list.sp-contents-hidden"
);
sel!(SEL_THREAD_TITLE, ".theme-top-thread-title.link-txt");
sel!(SEL_THREAD_AUTHOR, ".theme-top-thread-author");
sel!(SEL_THREAD_DATE, ".theme-top-thread-createdate");
sel!(SEL_THREAD_STATUS, ".theme-top-thread-postzyoukyou");

// ── Thread post selectors ──
sel!(SEL_THREAD_POST_BLOCK, "#threadPostListArea .clearfix");
sel!(SEL_POST_CONTENTS_TEXT, ".postContentsText");
sel!(SEL_POST_DATE, ".postDate");
sel!(SEL_POST_USER, ".postUser");
sel!(SEL_POST_ID, ".postId");
sel!(SEL_MSG_BLOCK, ".discussion-message-block");
sel!(SEL_DISCUSS_MESS_FILE, ".discuss_mess_file");

// ── Inquiry (お問い合わせ / メッセージ) selectors ──
sel!(SEL_INQUIRY_FORM, "#inquirySetForm");
sel!(SEL_INQUIRY_MSG_BLOCK, ".discussion-message-block");
sel!(SEL_INQUIRY_MSG_MAIN, ".discussion-message-main");
sel!(SEL_INQUIRY_MSG_FILE, ".discuss_mess_file");
sel!(SEL_INQUIRY_QL_EDITOR, ".ql-editor");
sel!(SEL_INQUIRY_MSG_FOOTER, ".message-margin-top");
sel!(SEL_INQUIRY_HIDDEN_POSTID, ".contents-hidden.postId");
sel!(SEL_INQUIRY_HIDDEN_CONTENTS, ".contents-hidden.contents");
sel!(SEL_INQUIRY_BLOCK_TITLE, ".block-title .block-title-txt");
sel!(SEL_INQUIRY_POSTFILE_FORM, "#inquiryPostFile");
sel!(SEL_INQUIRY_UPFILE_FORM, "#inquiryFileForm");
sel!(SEL_INQUIRY_FILENAME_INPUT, "input.fileName");
sel!(SEL_INQUIRY_OBJECTNAME_INPUT, "input.objectName");
sel!(SEL_INQUIRY_POSTID_INPUT, "input.postId");
sel!(SEL_INQUIRY_SCANSTATUS_INPUT, "input.scanStatus");

// ── Detail page selectors ──
sel!(SEL_REPORT_FORM, "#reportDownloadForm");
sel!(SEL_FORUMS_FORM, "#forumsPostFile");
// Luna announcement attachment hidden inputs
sel!(SEL_CMT_FILENAME, "input.cmtInfoFileName");
sel!(SEL_CMT_OBJECTNAME, "input.cmtInfoObjectName");
sel!(SEL_TEMPFILE_LINK, "a[href*='tempfile']");
sel!(SEL_DOWNLOAD_LINK, "a[href*='download']");
sel!(
    SEL_VIDEO_LINK,
    ".block-list-video a[href], .examination-movie a[href]"
);
sel!(
    SEL_BODY_LINK,
    ".contents-input-area a[href], .ql-editor a[href]"
);

// ── Forum post fallback selectors (detail page) ──
sel!(SEL_FORUM_POST_THREAD_AREA, ".thread-post-area");
sel!(SEL_FORUM_POST_LIST_BODY, ".post-list-area .post-body");
sel!(SEL_FORUM_POST_CONTENT, ".forum-post-content");
sel!(SEL_FORUMS_THREAD_CONTENT, ".forums-thread-content");

// ── Course top selectors ──
sel!(SEL_INFO_RESULT, ".course-result-list.sp-contents-hidden");
sel!(SEL_INFO_NAME_A, ".class-view-information-name a");
sel!(SEL_INFO_PRIORITY, ".portal-information-priority");
sel!(SEL_INFO_START, ".class-view-information-start");
sel!(SEL_INFO_END, ".class-view-information-end");
sel!(SEL_ONLINE_LINK, "#online .online-link a[href]");
sel!(SEL_READMORE_DIV, ".contents-detail-readmore-txt div");
sel!(SEL_READMORE_SPAN, ".contents-detail-readmore-txt span");
sel!(SEL_SYLLABUS_LINK, ".class-header-syllabus");
sel!(SEL_GRADE_LINK, "a[href*='external_grade']");
sel!(
    SEL_SIDE_MENU,
    "#sidemenuListMessage a[onclick], #sidemenuListEdit a[onclick]"
);
sel!(SEL_MATERIAL_LIST, "#courseContent #materialList");
sel!(SEL_MAT_TITLE, ".course-material-title-txt");
sel!(SEL_INPUT_SPAN, ".contents-input-area span");
sel!(SEL_MAT_FILE_NAME, ".material-file-name");
sel!(SEL_MAT_CSS, ".course-result-list.materialCss");
sel!(SEL_QL_EDITOR, ".ql-editor");
sel!(SEL_SCRIPT, "script");
sel!(SEL_FILENAME, ".fileName");
sel!(SEL_RESOURCE_ID, ".resource_Id");
sel!(SEL_FILETYPE, ".fileType");
sel!(SEL_DL_MAT_ID, "#dlMaterialId");
sel!(SEL_OPEN_END_DATE, ".openEndDate");
sel!(SEL_SCAN_STATUS, ".scanStatus");

// ── Report/Exam/Discussion list selectors ──
sel!(SEL_REPORT_LIST, "#report .contents-result-list");
sel!(SEL_RPT_NAME, ".course-view-report-name.link-txt");
sel!(SEL_RPT_START, ".course-view-report-time-start");
sel!(SEL_RPT_END, ".course-view-report-time-end");
sel!(SEL_RPT_STATUS, ".course-view-report-status");
sel!(SEL_EXAM_LIST, "#examination .contents-result-list");
sel!(SEL_EXAM_NAME, ".course-view-examination-name.link-txt");
sel!(SEL_EXAM_NAME_FB, ".course-view-examination-name");
sel!(SEL_LINK_TXT, "a.link-txt");
sel!(
    SEL_EXAM_PERIOD,
    ".course-view-examination-period.sp-contents-hidden"
);
sel!(SEL_EXAM_STATUS, ".course-view-examination-answer-status");
sel!(SEL_DISC_LIST, "#discussion .contents-result-list");
sel!(SEL_DISC_NAME, ".course-view-forum-title.link-txt");
sel!(SEL_DISC_NAME_FB, ".course-view-forum-title");
sel!(
    SEL_DISC_PERIOD,
    ".course-view-forum-period.sp-contents-hidden"
);
sel!(SEL_DISC_STATUS, ".course-view-forum-postzyoukyou");

// ── Survey/questionnaire list selectors ──
sel!(
    SEL_SURVEY_LIST,
    "#questionnaire .course-result-list, #courseViewSurveyList .course-result-list"
);
sel!(SEL_SURV_NAME, ".course-view-questionnaire-name.link-txt");
sel!(SEL_SURV_NAME_FB, ".course-view-questionnaire-name");
sel!(
    SEL_SURV_PERIOD,
    ".course-view-questionnaire-period.sp-contents-hidden"
);
sel!(SEL_SURV_STATUS, ".course-view-questionnaire-answer-status");

// ── Attendance selectors ──
sel!(
    SEL_ATT_LIST,
    "#attendance .course-result-list.contents-display-flex"
);
sel!(SEL_ATT_TITLE, ".course-view-attendance-title");
sel!(SEL_ATT_DATE, ".course-view-attendance-date");
sel!(SEL_ATT_STATUS, ".course-view-attendance-status");
sel!(SEL_ATT_ACTION_A, ".course-view-attendance-status a");

// ── Utility selectors ──
sel!(SEL_A_HREF, "a[href]");
sel!(SEL_SPAN, "span");
sel!(SEL_OPT_SELECTED, "option[selected]");
sel!(SEL_OPTION, "option");

/// Extract text from a Quill Delta JSON string, preserving link URLs.
/// The json_str is double-escaped: it was a JS string literal containing JSON.
/// E.g. in the HTML: setJsonData("{\"ops\":[{\"insert\":\"text\\n\u306F\"}]}", ...)
/// So json_str = {\"ops\":[{\"insert\":\"text\\n\u306F\"}]}
///
/// Links in Quill Delta look like:
///   {"insert":"click here","attributes":{"link":"https://example.com"}}
/// We output them as: click here ( https://example.com )
fn extract_quill_plain_text(json_str: &str) -> Option<String> {
    // First, try proper JSON parsing after unescaping
    if let Some(text) = extract_quill_via_json(json_str) {
        return Some(text);
    }

    // Fallback: string-level extraction (no link support)
    let mut result = String::new();
    let marker = "\\\"insert\\\":\\\"";
    let mut search = json_str;

    while let Some(pos) = search.find(marker) {
        let rest = &search[pos + marker.len()..];
        if let Some(end_pos) = find_closing_escaped_quote(rest) {
            let raw_value = &rest[..end_pos];
            let pass1 = unescape_js_string(raw_value);
            let pass2 = unescape_js_string(&pass1);
            result.push_str(&pass2);
            search = if end_pos + 2 < rest.len() {
                &rest[end_pos + 2..]
            } else {
                ""
            };
        } else {
            break;
        }
    }

    let trimmed = result.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Try to parse Quill Delta JSON properly via serde_json.
/// Handles link attributes by appending URLs after the link text.
fn extract_quill_via_json(json_str: &str) -> Option<String> {
    // Unescape the JS string to get valid JSON
    let unescaped = unescape_js_string(json_str);

    // Try parsing as JSON
    let val: serde_json::Value = serde_json::from_str(&unescaped).ok()?;
    let ops = val.get("ops")?.as_array()?;

    let mut result = String::new();
    for op in ops {
        if let Some(text) = op.get("insert").and_then(|v| v.as_str()) {
            let link = op
                .get("attributes")
                .and_then(|a| a.get("link"))
                .and_then(|l| l.as_str());

            result.push_str(text);
            if let Some(url) = link {
                // Append link URL after the text, avoiding duplication if text IS the URL
                if text.trim() != url.trim() {
                    result.push_str(" ( ");
                    result.push_str(url);
                    result.push_str(" )");
                }
            }
        }
    }

    let trimmed = result.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Find the position of the closing \" in a JS-escaped string value.
/// Skips past \\\\ (escaped backslash) so \\\\\" is read as \\\\ + \" (end).
fn find_closing_escaped_quote(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            if bytes[i + 1] == b'"' {
                return Some(i); // found \"
            }
            // skip any escape sequence (\\, \n, \u, etc.)
            i += 2;
        } else {
            i += 1;
        }
    }
    None
}

/// Unescape one level of JS/JSON string escaping:
/// \\n → newline, \\t → tab, \\\\ → \\, \\/ → /, \\uXXXX → char
fn unescape_js_string(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek().copied() {
                Some('n') => {
                    chars.next();
                    result.push('\n');
                }
                Some('t') => {
                    chars.next();
                    result.push('\t');
                }
                Some('r') => {
                    chars.next();
                    result.push('\r');
                }
                Some('\\') => {
                    chars.next();
                    result.push('\\');
                }
                Some('"') => {
                    chars.next();
                    result.push('"');
                }
                Some('/') => {
                    chars.next();
                    result.push('/');
                }
                Some('u') => {
                    chars.next();
                    let hex: String = chars.by_ref().take(4).collect();
                    if hex.len() == 4 {
                        if let Ok(code) = u32::from_str_radix(&hex, 16) {
                            if let Some(ch) = char::from_u32(code) {
                                result.push(ch);
                                continue;
                            }
                        }
                    }
                    result.push_str("\\u");
                    result.push_str(&hex);
                }
                _ => {
                    result.push(c);
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Extract setJsonData content from an HTML fragment
fn extract_quill_text(html: &str) -> Option<String> {
    // Pattern: setJsonData("{...}", 'reference') or setJsonData("{...}");
    let marker = "setJsonData(\"";
    let pos = html.find(marker)?;
    let rest = &html[pos + marker.len()..];
    // Find the closing: try "\", '" first, then "\");"
    let end = rest.find("\", '").or_else(|| rest.find("\");"))?;
    let json_str = &rest[..end];
    extract_quill_plain_text(json_str)
}

/// Extract all Quill texts from the full page HTML
fn extract_all_quill_texts(html: &str) -> Vec<String> {
    let marker = "setJsonData(\"";
    let mut results = Vec::new();
    let mut search = html;
    while let Some(pos) = search.find(marker) {
        let rest = &search[pos + marker.len()..];
        // Try both closing patterns
        let end = rest.find("\", '").or_else(|| rest.find("\");"));
        if let Some(end) = end {
            let json_str = &rest[..end];
            if let Some(text) = extract_quill_plain_text(json_str) {
                results.push(text);
            }
            search = &rest[end..];
        } else {
            break;
        }
    }
    results
}

/// Try multiple CSS selectors and return the first non-empty text match.
fn try_selectors_text(doc: &Html, selectors: &[&str]) -> String {
    for sel_str in selectors {
        if let Ok(sel) = Selector::parse(sel_str) {
            if let Some(el) = doc.select(&sel).next() {
                let text = el.text().collect::<String>().trim().to_string();
                if !text.is_empty() {
                    return text;
                }
            }
        }
    }
    String::new()
}

// ──────────────────────────────────────────────
// ──────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────

fn extract_selected_value(doc: &Html, selector: &str) -> (String, String) {
    let sel = match Selector::parse(selector) {
        Ok(s) => s,
        Err(_) => return (String::new(), String::new()),
    };
    let select_el = match doc.select(&sel).next() {
        Some(e) => e,
        None => return (String::new(), String::new()),
    };
    let option_sel = &*SEL_OPT_SELECTED;
    match select_el.select(option_sel).next() {
        Some(opt) => {
            let value = opt.value().attr("value").unwrap_or_default().to_string();
            let label = opt.text().collect::<String>().trim().to_string();
            (value, label)
        }
        None => (String::new(), String::new()),
    }
}

fn extract_select_options(doc: &Html, selector: &str) -> Vec<SelectOption> {
    let sel = match Selector::parse(selector) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let select_el = match doc.select(&sel).next() {
        Some(e) => e,
        None => return Vec::new(),
    };
    let option_sel = &*SEL_OPTION;
    select_el
        .select(option_sel)
        .map(|opt| SelectOption {
            value: opt.value().attr("value").unwrap_or_default().to_string(),
            label: opt.text().collect::<String>().trim().to_string(),
            selected: opt.value().attr("selected").is_some(),
        })
        .collect()
}

fn parse_japanese_number(s: &str) -> u32 {
    if s.contains('１') {
        return 1;
    }
    if s.contains('２') {
        return 2;
    }
    if s.contains('３') {
        return 3;
    }
    if s.contains('４') {
        return 4;
    }
    if s.contains('５') {
        return 5;
    }
    if s.contains('６') {
        return 6;
    }
    if s.contains('７') {
        return 7;
    }
    // Also try ASCII digits
    for c in s.chars() {
        if c.is_ascii_digit() {
            return c.to_digit(10).unwrap_or(0);
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // requires local HTML dump file
    fn test_parse_detail_attachments() {
        let html = std::fs::read_to_string("/tmp/luna_detail_lms_course_report_submission_idnumber=2026510010040201_reportId=225148.html")
            .expect("HTML dump file not found");
        let result = parse_luna_detail_page(&html);
        println!("Title: {}", result.title);
        println!("Sections: {}", result.sections.len());
        println!("Meta: {:?}", result.meta);
        println!("Attachments: {:?}", result.attachments);
        assert!(
            !result.attachments.is_empty(),
            "Should have at least one attachment"
        );
        let att = &result.attachments[0];
        assert!(!att.name.is_empty(), "Attachment name should not be empty");
        assert!(!att.url.is_empty(), "Attachment URL should not be empty");
        println!("PASS: name='{}', url='{}'", att.name, att.url);
    }

    #[test]
    #[ignore] // requires local HTML dump file
    fn test_parse_course_page() {
        let html = std::fs::read_to_string(
            "/tmp/luna_detail_lms_course_idnumber=2026510010040201#information.html",
        )
        .expect("Course HTML dump file not found");
        let result = parse_luna_course_contents(&html, "2026510010040201");
        println!("Course name: {}", result.course_name);
        println!("Semester: {}", result.semester);
        println!("Teachers: {}", result.teachers);
        println!("TA: {}", result.ta_info);
        println!("LA: {}", result.la_info);
        println!("Syllabus: {}", result.syllabus_url);
        println!("Menus: {}", result.menus.len());
        for m in &result.menus {
            println!("  {} ({})", m.name, m.module_type);
        }
        println!("Announcements: {}", result.announcements.len());
        for a in &result.announcements {
            println!(
                "  {} [{}~{}] new={}",
                a.title, a.start_date, a.end_date, a.is_new
            );
        }
        println!("Online tools: {}", result.online_tools.len());
        for t in &result.online_tools {
            println!("  {} -> {}", t.name, t.url);
        }
        assert!(
            !result.course_name.is_empty(),
            "Course name should not be empty"
        );
        assert!(!result.menus.is_empty(), "Should have menus");
    }

    #[test]
    #[ignore] // requires local HTML dump file
    fn test_parse_contents_page() {
        let html = std::fs::read_to_string("/tmp/luna_contents_2026510010040201.html")
            .expect("Contents HTML dump file not found");
        let (materials, reports, examinations, discussions, _surveys) =
            parse_luna_contents_page(&html);
        println!("Materials: {}", materials.len());
        for m in &materials {
            println!("  {} | {} | {}", m.title, m.period, m.status);
        }
        println!("Reports: {}", reports.len());
        for r in &reports {
            println!("  {} | {} | {}", r.title, r.period, r.status);
        }
        println!("Examinations: {}", examinations.len());
        for e in &examinations {
            println!("  {} | {} | {}", e.title, e.period, e.status);
        }
        println!("Discussions: {}", discussions.len());
        for d in &discussions {
            println!("  {} | {} | {}", d.title, d.period, d.status);
        }
        assert!(
            !materials.is_empty()
                || !reports.is_empty()
                || !examinations.is_empty()
                || !discussions.is_empty(),
            "Should have at least some content items"
        );
    }

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

    #[test]
    fn test_parse_thread_detail_extracts_discussion_file_attachments() {
        let html = r#"
            <form id="forumsPostFile" action="/lms/course/forums/thread_postfile">
              <input type="hidden" name="idnumber" value="202647210001">
              <input type="hidden" name="forumId" value="4721">
              <input type="hidden" name="threadId" value="222700">
              <input type="hidden" name="fileId" value="">
              <input type="hidden" name="fileName" value="">
            </form>
            <div class="contents-title-txt">生成AIと政治情報</div>
            <div id="threadPostListArea">
              <div class="clearfix">
                <div class="discussion-message-block">
                  <div class="postUser">氏名:山田太郎</div>
                  <div class="postDate">2026/04/29 10:12</div>
                  <div class="postContentsText">資料を添付します。</div>
                  <div class="postId contents-hidden">222720</div>
                  <div class="discuss_mess_file">
                    <span class="link-txt downloadFile">政治情報課題における生成AIの影響に関する一考察.pdf</span>
                    <div class="contents-hidden fileName">政治情報課題における生成AIの影響に関する一考察.pdf</div>
                    <div class="contents-hidden objectName">2026/47/21/b3/4721b302-3519-4bff-acdc-e5736fe8b6c2</div>
                    <div class="contents-hidden postId">222720</div>
                    <div class="contents-hidden scanStatus">1</div>
                  </div>
                </div>
              </div>
            </div>
        "#;

        let result = parse_luna_thread_detail(html);
        assert_eq!(result.posts.len(), 1);
        let post = &result.posts[0];
        assert_eq!(post.thread_id, "222720");
        assert_eq!(post.attachments.len(), 1);
        let att = &post.attachments[0];
        assert_eq!(
            att.name,
            "政治情報課題における生成AIの影響に関する一考察.pdf"
        );
        assert_eq!(
            att.object_name,
            "2026/47/21/b3/4721b302-3519-4bff-acdc-e5736fe8b6c2"
        );
        assert_eq!(att.download_action, "/lms/course/forums/thread_postfile");
        assert!(att
            .download_params
            .iter()
            .any(|(k, v)| k == "fileId" && v == &att.object_name));
        assert!(att
            .download_params
            .iter()
            .any(|(k, v)| k == "fileName" && v == &att.name));
        assert!(att
            .download_params
            .iter()
            .any(|(k, v)| k == "postId" && v == "222720"));
        assert!(att
            .download_params
            .iter()
            .any(|(k, v)| k == "scanStatus" && v == "1"));
    }

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
}
