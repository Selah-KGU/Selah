use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

macro_rules! sel {
    ($name:ident, $s:expr) => {
        static $name: LazyLock<Selector> = LazyLock::new(|| Selector::parse($s).unwrap());
    };
}

// ── Timetable selectors ──
sel!(SEL_DATA_ROW,      ".div-table-data-row");
sel!(SEL_PERIOD_COL,    ".div-table-colomn-period");
sel!(SEL_TABLE_CELL,    ".div-table-cell");
sel!(SEL_COURSE_BTN,    ".timetable-course-top-btn");
sel!(SEL_CELL_DETAIL,   ".div-table-cell-detail span");
sel!(SEL_COMMUNITY_BTN, ".timetable-community-course .timetable-course-top-btn");

// ── Todo selectors ──
sel!(SEL_TODO_LIST,     ".todo-list");
sel!(SEL_TODO_COURSE,   ".todolist-course");
sel!(SEL_TODO_TYPE,     ".todolist-contents-type span");
sel!(SEL_TODO_NAME,     ".todolist-contents-name a");
sel!(SEL_TODO_DEADLINE, ".todolist-mobile-width-deadline");
sel!(SEL_TODO_STATUS,   ".todolist-contents-status span");
sel!(SEL_TODO_FEEDBACK, ".todolist-feedback .todolist-mobile-feedback");

// ── Notification selectors ──
sel!(SEL_NOTIF_LIST,    ".update-info-list");
sel!(SEL_NOTIF_DATE,    ".update-info-updateDate label");
sel!(SEL_NOTIF_COURSE,  ".update-info-courseInfo span");
sel!(SEL_NOTIF_MODULE,  ".update-info-module span");
sel!(SEL_NOTIF_CONTENT, ".update-info-contents .break-word");
sel!(SEL_NOTIF_URL,     ".updateInfoUrl");
sel!(SEL_INPUT_IDNUMBER,"input[id='idnumber']");
sel!(SEL_INPUT_IDNAME,  "input[name='idnumber']");

// ── Common shared selectors ──
sel!(SEL_DETAIL_VERT,   ".contents-detail.contents-vertical");
sel!(SEL_BLOCK_DETAIL,  ".block > .contents-list > .contents-detail.contents-vertical");
sel!(SEL_HEADER_BOLD,   ".contents-header-txt .bold-txt");
sel!(SEL_HEADER_COMBO,  ".contents-header-txt .bold-txt, .contents-header-txt");
sel!(SEL_INPUT_AREA,    ".contents-input-area");
sel!(SEL_DOWNLOAD_FILE, ".downloadFile");
sel!(SEL_OBJECT_NAME,   ".objectName");
sel!(SEL_HIDDEN_INPUT,  "input[type='hidden']");

// ── Discussion selectors ──
sel!(SEL_THEME_TOP,     "#themeTopList .result-list.sp-contents-hidden");
sel!(SEL_THREAD_TITLE,  ".theme-top-thread-title.link-txt");
sel!(SEL_THREAD_AUTHOR, ".theme-top-thread-author");
sel!(SEL_THREAD_DATE,   ".theme-top-thread-createdate");
sel!(SEL_THREAD_STATUS, ".theme-top-thread-postzyoukyou");

// ── Detail page selectors ──
sel!(SEL_DL_CMT,        ".downloadFile, .cmtInfoFileName");
sel!(SEL_REPORT_FORM,   "#reportDownloadForm");
sel!(SEL_FORUMS_FORM,   "#forumsPostFile");
sel!(SEL_TEMPFILE_LINK, "a[href*='tempfile']");
sel!(SEL_DOWNLOAD_LINK, "a[href*='download']");
sel!(SEL_VIDEO_LINK,    ".block-list-video a[href], .examination-movie a[href]");
sel!(SEL_BODY_LINK,     ".contents-input-area a[href], .ql-editor a[href]");

// ── Course top selectors ──
sel!(SEL_INFO_RESULT,   ".course-result-list.sp-contents-hidden");
sel!(SEL_INFO_NAME_A,   ".class-view-information-name a");
sel!(SEL_INFO_PRIORITY, ".portal-information-priority");
sel!(SEL_INFO_START,    ".class-view-information-start");
sel!(SEL_INFO_END,      ".class-view-information-end");
sel!(SEL_ONLINE_LINK,   "#online .online-link a[href]");
sel!(SEL_READMORE_DIV,  ".contents-detail-readmore-txt div");
sel!(SEL_READMORE_SPAN, ".contents-detail-readmore-txt span");
sel!(SEL_SYLLABUS_LINK, ".class-header-syllabus");
sel!(SEL_GRADE_LINK,    "a[href*='external_grade']");
sel!(SEL_SIDE_MENU,     "#sidemenuListMessage a[onclick], #sidemenuListEdit a[onclick]");
sel!(SEL_MATERIAL_LIST, "#courseContent #materialList");
sel!(SEL_MAT_TITLE,     ".course-material-title-txt");
sel!(SEL_INPUT_SPAN,    ".contents-input-area span");
sel!(SEL_MAT_FILE_NAME, ".material-file-name");
sel!(SEL_MAT_CSS,       ".course-result-list.materialCss");
sel!(SEL_QL_EDITOR,     ".ql-editor");
sel!(SEL_SCRIPT,        "script");
sel!(SEL_FILENAME,      ".fileName");
sel!(SEL_RESOURCE_ID,   ".resource_Id");
sel!(SEL_FILETYPE,      ".fileType");
sel!(SEL_DL_MAT_ID,     "#dlMaterialId");
sel!(SEL_OPEN_END_DATE, ".openEndDate");
sel!(SEL_SCAN_STATUS,   ".scanStatus");

// ── Report/Exam/Discussion list selectors ──
sel!(SEL_REPORT_LIST,   "#report .contents-result-list");
sel!(SEL_RPT_NAME,      ".course-view-report-name.link-txt");
sel!(SEL_RPT_START,     ".course-view-report-time-start");
sel!(SEL_RPT_END,       ".course-view-report-time-end");
sel!(SEL_RPT_STATUS,    ".course-view-report-status");
sel!(SEL_EXAM_LIST,     "#examination .contents-result-list");
sel!(SEL_EXAM_NAME,     ".course-view-examination-name.link-txt");
sel!(SEL_EXAM_NAME_FB,  ".course-view-examination-name");
sel!(SEL_LINK_TXT,      "a.link-txt");
sel!(SEL_EXAM_PERIOD,   ".course-view-examination-period.sp-contents-hidden");
sel!(SEL_EXAM_STATUS,   ".course-view-examination-answer-status");
sel!(SEL_DISC_LIST,     "#discussion .contents-result-list");
sel!(SEL_DISC_NAME,     ".course-view-forum-title.link-txt");
sel!(SEL_DISC_NAME_FB,  ".course-view-forum-title");
sel!(SEL_DISC_PERIOD,   ".course-view-forum-period.sp-contents-hidden");
sel!(SEL_DISC_STATUS,   ".course-view-forum-postzyoukyou");

// ── Survey/questionnaire list selectors ──
sel!(SEL_SURVEY_LIST,   "#questionnaire .course-result-list, #courseViewSurveyList .course-result-list");
sel!(SEL_SURV_NAME,     ".course-view-questionnaire-name.link-txt");
sel!(SEL_SURV_NAME_FB,  ".course-view-questionnaire-name");
sel!(SEL_SURV_PERIOD,   ".course-view-questionnaire-period.sp-contents-hidden");
sel!(SEL_SURV_STATUS,   ".course-view-questionnaire-answer-status");

// ── Attendance selectors ──
sel!(SEL_ATT_LIST,      "#attendance .course-result-list.contents-display-flex");
sel!(SEL_ATT_TITLE,     ".course-view-attendance-title");
sel!(SEL_ATT_DATE,      ".course-view-attendance-date");
sel!(SEL_ATT_STATUS,    ".course-view-attendance-status");
sel!(SEL_ATT_ACTION_A,  ".course-view-attendance-status a");

// ── Utility selectors ──
sel!(SEL_A_HREF,        "a[href]");
sel!(SEL_SPAN,          "span");
sel!(SEL_OPT_SELECTED,  "option[selected]");
sel!(SEL_OPTION,        "option");

// ──────────────────────────────────────────────
// Timetable
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaTimetable {
    pub year: String,
    pub term: String,
    pub year_label: String,
    pub term_label: String,
    pub year_options: Vec<SelectOption>,
    pub term_options: Vec<SelectOption>,
    pub courses: Vec<LunaCourse>,
    pub communities: Vec<LunaCommunity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaCourse {
    pub idnumber: String,
    pub name: String,
    pub teacher: String,
    pub period: u32,   // 1-7
    pub day: u32,      // 1=月 ... 6=土
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaCommunity {
    pub idnumber: String,
    pub name: String,
}

pub fn parse_luna_timetable(html: &str) -> LunaTimetable {
    let doc = Html::parse_document(html);

    let (year, year_label) = extract_selected_value(&doc, "#nendo");
    let (term, term_label) = extract_selected_value(&doc, "#kikanCd");
    let year_options = extract_select_options(&doc, "#nendo");
    let term_options = extract_select_options(&doc, "#kikanCd");

    let mut courses = Vec::new();

    for row in doc.select(&SEL_DATA_ROW) {
        // Extract period number from text like "１時限"
        let period_text = row.select(&SEL_PERIOD_COL).next()
            .map(|e| e.text().collect::<String>())
            .unwrap_or_default();
        let period = parse_japanese_number(&period_text);
        if period == 0 { continue; }

        // Each cell corresponds to a day (1=月 through 6=土)
        for (i, cell) in row.select(&SEL_TABLE_CELL).enumerate() {
            let day = (i + 1) as u32;
            if let Some(btn) = cell.select(&SEL_COURSE_BTN).next() {
                let idnumber = btn.value().attr("id").unwrap_or_default().to_string();
                let name = btn.text().collect::<String>().trim().to_string();
                let teacher = cell.select(&SEL_CELL_DETAIL)
                    .map(|s| s.text().collect::<String>().trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ");

                courses.push(LunaCourse {
                    idnumber,
                    name,
                    teacher,
                    period,
                    day,
                });
            }
        }
    }

    // Parse communities
    let mut communities = Vec::new();
    for el in doc.select(&SEL_COMMUNITY_BTN) {
        let idnumber = el.value().attr("id").unwrap_or_default().to_string();
        let name = el.text().collect::<String>().trim().to_string();
        communities.push(LunaCommunity { idnumber, name });
    }

    LunaTimetable { year, term, year_label, term_label, year_options, term_options, courses, communities }
}

// ──────────────────────────────────────────────
// TODO list
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaTodoItem {
    pub course_name: String,
    pub content_type: String,  // 課題, テスト, 掲示板
    pub content_name: String,
    pub url: String,
    pub deadline: String,
    pub status: String,        // 未提出, 提出済み, etc.
    pub feedback: String,
}

pub fn parse_luna_todo(html: &str) -> Vec<LunaTodoItem> {
    let doc = Html::parse_document(html);
    let mut items = Vec::new();

    for item in doc.select(&SEL_TODO_LIST) {
        let course_name = item.select(&SEL_TODO_COURSE).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let content_type = item.select(&SEL_TODO_TYPE).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let (content_name, url) = item.select(&SEL_TODO_NAME).next()
            .map(|e| (
                e.text().collect::<String>().trim().to_string(),
                e.value().attr("href").unwrap_or_default().to_string(),
            ))
            .unwrap_or_default();

        let deadline = item.select(&SEL_TODO_DEADLINE).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let status = item.select(&SEL_TODO_STATUS).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let feedback = item.select(&SEL_TODO_FEEDBACK).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        items.push(LunaTodoItem {
            course_name,
            content_type,
            content_name,
            url,
            deadline,
            status,
            feedback,
        });
    }

    items
}

// ──────────────────────────────────────────────
// Update notifications
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaNotification {
    pub date: String,
    pub course_info: String,
    pub module: String,       // 掲示板スレッド, お知らせ, 課題, etc.
    pub content: String,
    pub url: String,
    pub idnumber: String,
}

pub fn parse_luna_notifications(html: &str) -> Vec<LunaNotification> {
    let doc = Html::parse_document(html);
    let mut items = Vec::new();

    for item in doc.select(&SEL_NOTIF_LIST) {
        let date = item.select(&SEL_NOTIF_DATE).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let course_info = item.select(&SEL_NOTIF_COURSE).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let module = item.select(&SEL_NOTIF_MODULE).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let content = item.select(&SEL_NOTIF_CONTENT).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let url = item.select(&SEL_NOTIF_URL).next()
            .and_then(|e| e.value().attr("value"))
            .unwrap_or_default()
            .to_string();

        let idnumber = item.select(&SEL_INPUT_IDNUMBER).next()
            .or_else(|| item.select(&SEL_INPUT_IDNAME).next())
            .and_then(|e| e.value().attr("value"))
            .unwrap_or_default()
            .to_string();

        if date.is_empty() {
            continue;
        }

        // Skip LUNA system-wide announcements (e.g. 時間割 section notices about
        // guest access, maintenance schedules, etc.) — these are not course-specific
        // and cause errors when detail-fetched.
        if course_info == "時間割" {
            continue;
        }

        items.push(LunaNotification {
            date,
            course_info,
            module,
            content,
            url,
            idnumber,
        });
    }

    items
}

// ──────────────────────────────────────────────
// Course top page
// ──────────────────────────────────────────────

// ──────────────────────────────────────────────
// Generic detail page
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaDetailSection {
    pub heading: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaDetailPage {
    pub title: String,
    pub course_name: String,
    pub sections: Vec<LunaDetailSection>,
    pub attachments: Vec<LunaAttachment>,
    pub meta: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaAttachment {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub link_type: String,  // "file", "external", "video", "zoom", "panopto", "web"
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub object_name: String,
    /// Form action path for download (e.g. /lms/course/report/submission_download)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub download_action: String,
    /// Fixed form params (reportId, idnumber, etc.) serialized as key=value pairs
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub download_params: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaDiscussionPost {
    pub title: String,
    pub author: String,
    pub date: String,
    pub content: String,
    pub status: String,
    pub thread_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaDiscussionThread {
    pub title: String,
    pub course_name: String,
    pub description: String,
    pub meta: Vec<(String, String)>,
    pub posts: Vec<LunaDiscussionPost>,
}

/// Parse a Luna discussion themetop page (/lms/course/forums/themetop)
/// Structure:
///   - Theme info: title, description (Quill: themeContents), period
///   - Thread list: each thread has title, description (Quill: threadContentsN), author, date
pub fn parse_luna_discussion_thread(html: &str) -> LunaDiscussionThread {
    let doc = Html::parse_document(html);

    let course_name = try_selectors_text(&doc, &[
        ".course-title-txt",
        ".class-title-txt.course-view-header-txt",
    ]);

    let title = try_selectors_text(&doc, &[
        ".contents-title-txt",
        ".block-title-txt",
    ]);

    // Extract theme description from themeContents Quill
    let description = extract_named_quill_text(html, "themeContents")
        .unwrap_or_default();

    // Extract meta info from the top section (テーマタイトル, 投稿期間, etc.)
    let mut meta = Vec::new();
    {
        for row in doc.select(&SEL_BLOCK_DETAIL) {
            let label = row.select(&SEL_HEADER_COMBO).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            if label == "内容" || label == "添付ファイル" { continue; }
            let value = row.select(&SEL_INPUT_AREA).next()
                .map(|e| {
                    e.text()
                        .map(|t| t.trim())
                        .filter(|t| !t.is_empty() && !t.starts_with("/*") && !t.starts_with("$(")
                            && !t.starts_with("var ") && !t.contains("setJsonData")
                            && !t.contains("function") && !t.contains("QuillUtil"))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            if !label.is_empty() && !value.is_empty() {
                meta.push((label, value));
            }
        }
    }

    // Extract thread list from #themeTopList or .result-list
    let mut posts = Vec::new();

    // Themetop page: threads are in result-list divs with .theme-top-thread-* classes
    {
        for (idx, row) in doc.select(&SEL_THEME_TOP).enumerate() {
            let thread_title = row.select(&SEL_THREAD_TITLE).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            // Extract threadId from onclick="viewthread(50406);"
            let thread_id = row.select(&SEL_THREAD_TITLE).next()
                .and_then(|e| e.value().attr("onclick"))
                .and_then(|onclick| {
                    let start = onclick.find('(')? + 1;
                    let end = onclick.find(')')? ;
                    Some(onclick[start..end].trim().to_string())
                })
                .unwrap_or_default();
            let author = row.select(&SEL_THREAD_AUTHOR).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let date = row.select(&SEL_THREAD_DATE).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let status = row.select(&SEL_THREAD_STATUS).next()
                .map(|e| {
                    e.text()
                        .map(|t| t.trim())
                        .filter(|t| !t.is_empty())
                        .collect::<Vec<_>>()
                        .join(" / ")
                })
                .unwrap_or_default();

            // Extract thread content from threadContentsN Quill
            let quill_name = format!("threadContents{}", idx);
            let content = extract_named_quill_text(html, &quill_name)
                .unwrap_or_default();

            if !thread_title.is_empty() || !content.is_empty() || !author.is_empty() {
                posts.push(LunaDiscussionPost { title: thread_title, author, date, content, status, thread_id });
            }
        }
    }

    // Thread page fallback: meta rows (テーマ, スレッド, 登録者, etc.)
    if posts.is_empty() {
        // This is a thread detail page (/lms/course/forums/thread)
        // All content is in meta rows, threadPostList is loaded via AJAX
        // Extract thread description from headerContents Quill if available
        if let Some(header_content) = extract_named_quill_text(html, "headerContents") {
            if !header_content.is_empty() {
                posts.push(LunaDiscussionPost {
                    title: String::new(),
                    author: String::new(),
                    date: String::new(),
                    content: header_content,
                    status: String::new(),
                    thread_id: String::new(),
                });
            }
        }
    }

    LunaDiscussionThread { title, course_name, description, meta, posts }
}

/// Parse a Luna thread detail page (/lms/course/forums/thread)
/// Extracts: テーマ, スレッド title, 登録者, 説明 (headerContents Quill), 更新日時
pub fn parse_luna_thread_detail(html: &str) -> LunaDiscussionThread {
    let doc = Html::parse_document(html);

    let course_name = try_selectors_text(&doc, &[
        ".course-title-txt",
    ]);

    let title = try_selectors_text(&doc, &[
        ".contents-title-txt",
    ]);

    // Extract meta rows: テーマ, スレッド, 登録者, 学生番号, 更新日時
    let mut meta = Vec::new();
    let mut thread_title = String::new();
    {
        for row in doc.select(&SEL_BLOCK_DETAIL) {
            let label = row.select(&SEL_HEADER_BOLD).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            if label == "説明" || label == "投稿内容" || label == "添付ファイル"
                || label == "返信先投稿内容" { continue; }
            let value = row.select(&SEL_INPUT_AREA).next()
                .map(|e| {
                    e.text()
                        .map(|t| t.trim())
                        .filter(|t| !t.is_empty())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            if label == "スレッド" {
                thread_title = value.clone();
            }
            if !label.is_empty() && !value.is_empty() {
                meta.push((label, value));
            }
        }
    }

    // Description from headerContents Quill
    let description = extract_named_quill_text(html, "headerContents")
        .unwrap_or_default();

    // Posts are loaded via AJAX, so initial page is empty
    // We return the thread info for now
    let posts = Vec::new();

    let display_title = if !thread_title.is_empty() {
        thread_title
    } else {
        title
    };

    LunaDiscussionThread { title: display_title, course_name, description, meta, posts }
}

/// Extract Quill text from a specific named variable (e.g. "themeContents", "threadContents0")
fn extract_named_quill_text(html: &str, var_name: &str) -> Option<String> {
    // Pattern: _QuillUtil.varName.setJsonData("...", ...)
    // or: _QuillUtil.varName = ... .setJsonData("...", ...)
    let pattern = format!("{}.setJsonData(\"", var_name);
    let pos = html.find(&pattern)?;
    let after = &html[pos + pattern.len()..];
    // Find the closing ", 'reference') or similar
    // The JSON string ends at the last " before the next , or )
    // The JSON is double-escaped: \" inside the JS string
    let mut end = 0;
    let chars: Vec<char> = after.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            i += 2; // skip escaped char
            continue;
        }
        if chars[i] == '"' {
            end = i;
            break;
        }
        i += 1;
    }
    if end == 0 { return None; }
    let json_str = &after[..end];
    extract_quill_rich_html(json_str)
}

fn escape_html_fragment(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn wrap_quill_inline_attrs(text: &str, attrs: Option<&serde_json::Map<String, serde_json::Value>>) -> String {
    let mut out = escape_html_fragment(text);
    let Some(attrs) = attrs else { return out; };

    if attrs.get("bold").and_then(|v| v.as_bool()) == Some(true) {
        out = format!("<strong>{}</strong>", out);
    }
    if attrs.get("italic").and_then(|v| v.as_bool()) == Some(true) {
        out = format!("<em>{}</em>", out);
    }
    if attrs.get("underline").and_then(|v| v.as_bool()) == Some(true) {
        out = format!("<u>{}</u>", out);
    }
    if attrs.get("code").and_then(|v| v.as_bool()) == Some(true) {
        out = format!("<code>{}</code>", out);
    }

    let mut style_parts: Vec<String> = Vec::new();
    if let Some(color) = attrs.get("color").and_then(|v| v.as_str()) {
        style_parts.push(format!("color:{}", escape_html_fragment(color)));
    }
    if let Some(bg) = attrs.get("background").and_then(|v| v.as_str()) {
        style_parts.push(format!("background-color:{}", escape_html_fragment(bg)));
    }
    if !style_parts.is_empty() {
        out = format!("<span style=\"{}\">{}</span>", style_parts.join(";"), out);
    }

    if let Some(link) = attrs.get("link").and_then(|v| v.as_str()) {
        let lower = link.to_lowercase();
        if lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.starts_with("mailto:")
            || lower.starts_with("tel:")
        {
            out = format!(
                "<a href=\"{}\" target=\"_blank\" rel=\"noopener\">{}</a>",
                escape_html_fragment(link),
                out
            );
        }
    }

    out
}

fn extract_quill_rich_html(json_str: &str) -> Option<String> {
    let unescaped = unescape_js_string(json_str);
    let val: serde_json::Value = serde_json::from_str(&unescaped).ok()?;
    let ops = val.get("ops")?.as_array()?;

    let mut html = String::new();
    for op in ops {
        let Some(insert) = op.get("insert").and_then(|v| v.as_str()) else { continue; };
        let attrs = op.get("attributes").and_then(|a| a.as_object());

        let mut segment = String::new();
        for ch in insert.chars() {
            if ch == '\n' {
                if !segment.is_empty() {
                    html.push_str(&wrap_quill_inline_attrs(&segment, attrs));
                    segment.clear();
                }
                html.push_str("<br>");
            } else {
                segment.push(ch);
            }
        }
        if !segment.is_empty() {
            html.push_str(&wrap_quill_inline_attrs(&segment, attrs));
        }
    }

    let compact = html
        .replace("<br>", "")
        .replace("<br/>", "")
        .replace("<br />", "")
        .trim()
        .to_string();
    if compact.is_empty() { None } else { Some(html) }
}

/// Parse any Luna detail page (report/submission, examination, forum, etc.)
///
/// Luna detail pages use a consistent pattern:
///   .course-title-txt          → course name
/// Classify a URL into a link type for display purposes.
fn classify_link(url: &str, name: &str) -> String {
    let u = url.to_lowercase();
    let n = name.to_lowercase();

    // Internal Luna download paths → file
    if !u.starts_with("http") {
        return "file".into();
    }

    // Zoom
    if u.contains("zoom.us") || u.contains("zoom.") || u.contains("/lti/zoom") {
        return "zoom".into();
    }
    // Panopto
    if u.contains("panopto") || u.contains("/lti/panopto") || u.contains("/Panopto/") {
        return "panopto".into();
    }
    // Video platforms
    if u.contains("youtube.com") || u.contains("youtu.be") || u.contains("vimeo.com") {
        return "video".into();
    }
    // SharePoint / OneDrive (often used for video/file sharing)
    if u.contains("sharepoint.com") || u.contains("onedrive.live.com") || u.contains("1drv.ms") {
        return "cloud".into();
    }
    // Google (Drive, Docs, Slides, Sheets, Forms)
    if u.contains("drive.google.com") || u.contains("docs.google.com")
        || u.contains("forms.gle") || u.contains("forms.google.com")
    {
        return "google".into();
    }
    // Microsoft Teams
    if u.contains("teams.microsoft.com") || u.contains("teams.live.com") {
        return "teams".into();
    }
    // Known file extensions in URL or name → treat as downloadable external file
    let file_exts = [".pdf", ".doc", ".docx", ".ppt", ".pptx", ".xls", ".xlsx",
                     ".zip", ".rar", ".7z", ".mp4", ".mp3", ".wav", ".png", ".jpg", ".jpeg"];
    for ext in &file_exts {
        if u.ends_with(ext) || n.ends_with(ext) {
            return "file".into();
        }
    }

    "web".into()
}

///   .contents-title-txt        → page title (e.g. "テスト 受験トップ")
///   .contents-detail.contents-vertical → each field row:
///     .contents-header-txt .bold-txt → label
///     .contents-input-area           → value
///   .downloadFile              → attachment file names
///   setJsonData("...", ...)    → Quill rich text content (in <script> tags)
pub fn parse_luna_detail_page(html: &str) -> LunaDetailPage {
    let doc = Html::parse_document(html);

    // Course name: .course-title-txt
    let course_name = try_selectors_text(&doc, &[
        ".course-title-txt",
        ".class-title-txt.course-view-header-txt",
    ]);

    // Page title: .contents-title-txt
    let title = try_selectors_text(&doc, &[
        ".contents-title-txt",
        ".contents-title .contents-title-txt",
        "title",
    ]);

    let mut meta = Vec::new();
    let mut attachments = Vec::new();

    // === Primary pattern: .contents-detail.contents-vertical rows ===
    {
        for row in doc.select(&SEL_DETAIL_VERT) {
            let label = row.select(&SEL_HEADER_COMBO).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            // Skip attachment rows — they'll be handled in the attachments section
            if row.select(&SEL_DOWNLOAD_FILE).next().is_some() {
                continue;
            }

            let value = row.select(&SEL_INPUT_AREA).next()
                .map(|e| {
                    // Collect text but skip script content and hidden elements
                    let mut text_parts = Vec::new();
                    for child in e.text() {
                        let trimmed = child.trim();
                        if !trimmed.is_empty()
                            && !trimmed.starts_with("/*")
                            && !trimmed.starts_with("$(")
                            && !trimmed.starts_with("_Quill")
                            && !trimmed.starts_with("var ")
                            && !trimmed.starts_with("*/")
                            && !trimmed.contains("setJsonData")
                            && !trimmed.contains("function")
                            && !trimmed.contains("QuillUtil")
                        {
                            text_parts.push(trimmed.to_string());
                        }
                    }
                    text_parts.join(" ")
                })
                .unwrap_or_default();

            if !label.is_empty() && !value.is_empty() {
                meta.push((label, value));
            } else if !label.is_empty() {
                // Value might be in Quill rich text (empty div + script)
                let row_html = row.html();
                if let Some(quill_text) = extract_quill_text(&row_html) {
                    meta.push((label, quill_text));
                }
            }
        }
    }

    // === Extract Quill rich text content from scripts (page-level) ===
    // If we didn't find content in meta, try to extract main Quill text
    let quill_texts = extract_all_quill_texts(html);

    // Build sections from top-level Quill content that wasn't captured in meta
    let mut sections: Vec<LunaDetailSection> = if !quill_texts.is_empty() {
        // Check which quill texts are already in meta values
        let meta_values: Vec<&str> = meta.iter().map(|(_, v)| v.as_str()).collect();
        quill_texts.into_iter()
            .filter(|t| !meta_values.iter().any(|mv| mv.contains(t.as_str())))
            .filter(|t| t.len() > 5)
            .map(|t| LunaDetailSection { heading: String::new(), body: t })
            .collect()
    } else {
        Vec::new()
    };

    // === Extract attachments ===
    // First, find the download form to determine the correct download endpoint
    // Report pages: #reportDownloadForm -> /lms/course/report/submission_download
    // Forum pages: #forumsPostFile -> /lms/course/forums/thread_postfile
    let download_form_info: Option<(String, Vec<(String, String)>, bool)> = {
        let form_selectors: &[&Selector] = &[&SEL_REPORT_FORM, &SEL_FORUMS_FORM];
        let mut info = None;
        for sel in form_selectors {
            if let Some(form) = doc.select(sel).next() {
                let action = form.value().attr("action").unwrap_or_default().to_string();
                if !action.is_empty() {
                    let is_forum = form.value().id() == Some("forumsPostFile");
                    // Collect static hidden input params. Skip the per-file dynamic fields:
                    // Report: objectName, downloadFileName, downloadMode
                    // Forum: fileId, fileName
                    let mut params = Vec::new();
                    for input in form.select(&SEL_HIDDEN_INPUT) {
                        let iname = input.value().attr("name").unwrap_or_default();
                        let ival = input.value().attr("value").unwrap_or_default();
                        if !ival.is_empty()
                            && iname != "objectName"
                            && iname != "downloadFileName"
                            && iname != "downloadMode"
                            && iname != "fileId"
                            && iname != "fileName"
                        {
                            params.push((iname.to_string(), ival.to_string()));
                        }
                    }
                    log::debug!("[attachments] download form: action='{}', is_forum={}, params={:?}", action, is_forum, params);
                    info = Some((action, params, is_forum));
                    break;
                }
            }
        }
        info
    };

    // Pattern 1: .downloadFile elements (siblings of .objectName in same parent div)
    // Store objectName + form info so the download command can re-fetch _cid tokens
    {
        for el in doc.select(&SEL_DOWNLOAD_FILE) {
            let name = el.text().collect::<String>().trim().to_string();
            if !name.is_empty() {
                let obj_name = el.parent()
                    .and_then(scraper::ElementRef::wrap)
                    .and_then(|parent_el| {
                        parent_el.select(&SEL_OBJECT_NAME).next()
                            .map(|e| e.text().collect::<String>().trim().to_string())
                    })
                    .unwrap_or_default();

                if let Some((ref action, ref fixed_params, is_forum)) = download_form_info {
                    let link_type = classify_link("", &name);
                    // Merge static form params with per-file dynamic fields
                    let mut all_params = fixed_params.clone();
                    if is_forum {
                        // Forum: fileId = objectName, fileName = raw filename
                        all_params.push(("fileId".to_string(), obj_name.clone()));
                        all_params.push(("fileName".to_string(), name.clone()));
                    } else {
                        // Report: downloadFileName = raw filename, objectName, downloadMode = ""
                        all_params.push(("downloadFileName".to_string(), name.clone()));
                        all_params.push(("objectName".to_string(), obj_name.clone()));
                        all_params.push(("downloadMode".to_string(), String::new()));
                    }
                    log::debug!("[attachments] name='{}', objectName='{}', action='{}', type='{}'",
                        name, obj_name, action, link_type);
                    attachments.push(LunaAttachment {
                        name,
                        url: String::new(),
                        link_type,
                        object_name: obj_name,
                        download_action: action.clone(),
                        download_params: all_params,
                    });
                } else {
                    log::warn!("[attachments] no download form for '{}', objectName='{}'", name, obj_name);
                }
            }
        }
    }

    // Pattern 2: links with download/tempfile in href
    if attachments.is_empty() {
        let file_selectors: &[&Selector] = &[&SEL_TEMPFILE_LINK, &SEL_DOWNLOAD_LINK];
        for file_sel in file_selectors {
            for a in doc.select(file_sel) {
                let name = a.text().collect::<String>().trim().to_string();
                let url = a.value().attr("href").unwrap_or_default().to_string();
                if !name.is_empty() && !url.is_empty() && !url.contains("javascript:") {
                    let link_type = classify_link(&url, &name);
                    attachments.push(LunaAttachment { name, url, link_type, object_name: String::new(), download_action: String::new(), download_params: Vec::new() });
                }
            }
        }
    }

    // Pattern 3: external links (e.g. SharePoint video links)
    {
        for a in doc.select(&SEL_VIDEO_LINK) {
            let url = a.value().attr("href").unwrap_or_default().to_string();
            if !url.is_empty() && url.starts_with("http") {
                // Show a friendly name for external video links
                let link_type = classify_link(&url, "");
                let display_name = match link_type.as_str() {
                    "cloud" => format!("動画 ({})", if url.contains("sharepoint") { "SharePoint" } else { "OneDrive" }),
                    "video" => format!("動画 ({})", if url.contains("youtube") || url.contains("youtu.be") { "YouTube" } else { "Vimeo" }),
                    "zoom" => "Zoom ミーティング".to_string(),
                    "panopto" => "Panopto 録画".to_string(),
                    _ => "外部リンク".to_string(),
                };
                attachments.push(LunaAttachment { name: display_name, url, link_type, object_name: String::new(), download_action: String::new(), download_params: Vec::new() });
            }
        }
    }

    // === Extract forum posts (掲示板 thread pages) ===
    // Luna forum threads show posts in .post-body or .thread-post-body etc.
    let forum_post_selectors = [
        ".thread-post-area",
        ".post-list-area .post-body",
        ".forum-post-content",
        ".forums-thread-content",
    ];
    for sel_str in &forum_post_selectors {
        if let Ok(post_sel) = Selector::parse(sel_str) {
            for post_el in doc.select(&post_sel) {
                let post_text = post_el.text().collect::<String>();
                let trimmed = post_text.trim().to_string();
                if !trimmed.is_empty() && trimmed.len() > 3
                    && !sections.iter().any(|s| s.body.contains(&trimmed))
                {
                    sections.push(LunaDetailSection {
                        heading: String::new(),
                        body: trimmed,
                    });
                }
            }
        }
    }
    // Note: forum_post_selectors are rarely-matched CSS that stay dynamic

    // Also extract any links from the page body that might be useful (external links, etc.)
    {
        for a in doc.select(&SEL_BODY_LINK) {
            let url = a.value().attr("href").unwrap_or_default().to_string();
            let name = a.text().collect::<String>().trim().to_string();
            if !url.is_empty() && url.starts_with("http")
                && !attachments.iter().any(|att| att.url == url)
            {
                let display = if name.is_empty() { url.clone() } else { name };
                let link_type = classify_link(&url, &display);
                attachments.push(LunaAttachment { name: display, url, link_type, object_name: String::new(), download_action: String::new(), download_params: Vec::new() });
            }
        }
    }

    LunaDetailPage { title, course_name, sections, attachments, meta }
}
/// This returns an HTML fragment that typically contains:
///   - .information-detail-title or similar title
///   - Quill Delta JSON via setJsonData() for the main body
///   - .contents-detail.contents-vertical rows for meta (掲示期間, 発信者, etc.)
pub fn parse_luna_announcement_detail(html: &str) -> LunaDetailPage {
    let doc = Html::parse_fragment(html);
    let mut sections = Vec::new();
    let mut meta = Vec::new();
    let attachments = Vec::new();

    // Title from #osiraseTitle or .block-title-txt
    let title = try_selectors_text(&doc, &[
        "#osiraseTitle",
        ".block-title-txt",
        ".contents-title-txt",
    ]);

    // Extract Quill rich text content (main body) from setJsonData() calls
    let quill_texts = extract_all_quill_texts(html);
    for text in &quill_texts {
        if text.len() > 3 {
            sections.push(LunaDetailSection {
                heading: String::new(),
                body: text.clone(),
            });
        }
    }

    // Extract meta rows from .contents-detail.contents-vertical
    {
        for row in doc.select(&SEL_DETAIL_VERT) {
            let label = row.select(&SEL_HEADER_BOLD).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            // Skip "内容" — it's the Quill content we already extracted
            if label == "内容" { continue; }

            // Check for attachment rows
            if row.select(&SEL_DL_CMT).next().is_some() {
                // TODO: extract attachment file info if needed
                continue;
            }

            // Collect text from the value area, filtering out script content
            let value = row.select(&SEL_INPUT_AREA).next()
                .map(|e| {
                    let mut text_parts = Vec::new();
                    for child in e.text() {
                        let trimmed = child.trim();
                        if !trimmed.is_empty()
                            && !trimmed.starts_with("/*")
                            && !trimmed.starts_with("$(")
                            && !trimmed.starts_with("_Quill")
                            && !trimmed.starts_with("var ")
                            && !trimmed.starts_with("*/")
                            && !trimmed.contains("setJsonData")
                            && !trimmed.contains("function")
                            && !trimmed.contains("QuillUtil")
                        {
                            text_parts.push(trimmed.to_string());
                        }
                    }
                    text_parts.join("  ")
                })
                .unwrap_or_default();

            // Skip "添付ファイル" if empty
            if label == "添付ファイル" && value.is_empty() { continue; }

            if !label.is_empty() && !value.is_empty() {
                meta.push((label, value));
            }
        }
    }

    LunaDetailPage { title, course_name: String::new(), sections, attachments, meta }
}

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
            search = if end_pos + 2 < rest.len() { &rest[end_pos + 2..] } else { "" };
        } else {
            break;
        }
    }

    let trimmed = result.trim().to_string();
    if trimmed.is_empty() { None } else { Some(trimmed) }
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
            let link = op.get("attributes")
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
    if trimmed.is_empty() { None } else { Some(trimmed) }
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
                Some('n') => { chars.next(); result.push('\n'); }
                Some('t') => { chars.next(); result.push('\t'); }
                Some('r') => { chars.next(); result.push('\r'); }
                Some('\\') => { chars.next(); result.push('\\'); }
                Some('"') => { chars.next(); result.push('"'); }
                Some('/') => { chars.next(); result.push('/'); }
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
                _ => { result.push(c); }
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
    let end = rest.find("\", '")
        .or_else(|| rest.find("\");"))?;
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
        let end = rest.find("\", '")
            .or_else(|| rest.find("\");"));
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
// Course top page (/lms/course?idnumber=)
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaCourseContents {
    pub course_name: String,
    pub semester: String,
    pub teachers: String,
    pub ta_info: String,
    pub la_info: String,
    pub syllabus_url: String,
    pub grade_url: String,
    pub menus: Vec<LunaCourseMenu>,
    pub announcements: Vec<LunaCourseAnnouncement>,
    pub online_tools: Vec<LunaOnlineTool>,
    pub materials: Vec<LunaContentItem>,
    pub reports: Vec<LunaContentItem>,
    pub examinations: Vec<LunaContentItem>,
    pub discussions: Vec<LunaContentItem>,
    pub surveys: Vec<LunaContentItem>,
    #[serde(default)]
    pub attendances: Vec<LunaAttendanceItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaAttendanceItem {
    pub title: String,
    pub date: String,
    pub status: String,
    #[serde(default)]
    pub can_register: bool,
    #[serde(default)]
    pub idnumber: String,
    #[serde(default)]
    pub attendance_id: String,
    #[serde(default)]
    pub log_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaCourseMenu {
    pub name: String,
    pub module_type: String,
    pub icon: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaContentItem {
    pub title: String,
    pub url: String,
    pub period: String,
    pub status: String,
    pub item_type: String,   // material, report, examination, discussion
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub files: Vec<LunaMaterialFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaMaterialFile {
    pub display_name: String,   // link text (e.g. "2026日本語Ⅰ金_配布用シラバス")
    pub file_name: String,      // actual filename (e.g. "2026日本語Ⅰ金_配布用シラバス.pdf")
    pub object_name: String,    // storage path (e.g. "2026/ee/3c/1b/...")
    pub resource_id: String,    // resource ID
    pub material_id: String,    // dlMaterialId
    pub file_type: String,      // "0" = file, else HTML
    pub end_date: String,       // open end date (e.g. "2026-07-04 00:00:00.0")
    pub scan_status: String,    // virus scan status ("1" = clean)
    #[serde(default)]
    pub link_type: String,      // "file", "zoom", "panopto", "video", "cloud", "google", "teams", "web"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaCourseAnnouncement {
    pub title: String,
    pub info_id: String,
    pub start_date: String,
    pub end_date: String,
    pub is_new: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaOnlineTool {
    pub name: String,
    pub url: String,
    pub icon: String,
}

// ── Survey (questionnaire) detail types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaSurveyDetail {
    pub title: String,
    pub description: String,
    pub period: String,
    pub anonymity: String,
    pub allow_edit: String,
    pub answer_status: String,
    pub respondent: String,
    pub attachments: Vec<LunaSurveyAttachment>,
    pub questions: Vec<LunaSurveyQuestion>,
    /// Hidden form fields needed for submission (_cid, _csrf, idnumber, surveyId, takeFlag,
    /// and per-question answer[N].surveyNo / answer[N].surveyNoSub)
    #[serde(default)]
    pub form_fields: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaSurveyAttachment {
    pub file_name: String,
    pub object_name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub download_action: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub download_params: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaSurveyQuestion {
    pub number: String,
    pub body: String,
    pub required: bool,
    pub answer_type: String,
    pub options: Vec<LunaSurveyOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaSurveyOption {
    pub value: String,
    pub label: String,
}

pub fn parse_luna_course_contents(html: &str, idnumber: &str) -> LunaCourseContents {
    let doc = Html::parse_document(html);

    // Course name from header
    let course_name = try_selectors_text(&doc, &[
        ".class-title-txt.course-view-header-txt",
        ".course-title-txt",
        "title",
    ]);

    // Semester from subblock
    let semester = try_selectors_text(&doc, &[".subblock_form"]);

    // Teachers from .contents-detail-readmore-txt
    let teachers = {
        let spans: Vec<String> = doc.select(&SEL_READMORE_SPAN)
            .map(|el| el.text().collect::<String>().trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        // The pattern is: "担当教員" label, then teacher names, "担当TA" label, etc.
        // Extract teachers: text after "担当教員" before "担当TA"
        let mut result = Vec::new();
        let mut in_teacher = false;
        for s in &spans {
            if s.contains("担当教員") {
                in_teacher = true;
                continue;
            }
            if s.contains("担当TA") || s.contains("担当LA") {
                in_teacher = false;
                continue;
            }
            if in_teacher && !s.is_empty() {
                // Clean up: "榎本　可奈子" or ",  掛橋　智佳子"
                let cleaned = s.trim_start_matches(',').trim().to_string();
                if !cleaned.is_empty() {
                    result.push(cleaned);
                }
            }
        }
        result.join(", ")
    };

    // TA/LA info from the readmore section
    let ta_info = extract_staff_info(&doc, "担当TA");
    let la_info = extract_staff_info(&doc, "担当LA");

    // Syllabus link
    let syllabus_url = doc.select(&SEL_SYLLABUS_LINK).next()
        .and_then(|el| el.value().attr("href").map(|s| s.to_string()))
        .unwrap_or_default();

    // Grade link
    let grade_url = doc.select(&SEL_GRADE_LINK).next()
        .and_then(|el| el.value().attr("href").map(|s| s.to_string()))
        .unwrap_or_default();

    // Parse sidebar menu items (navigation categories only)
    let mut menus = Vec::new();
    for a in doc.select(&SEL_SIDE_MENU) {
        let name = a.text().collect::<String>().trim().to_string();
        let onclick = a.value().attr("onclick").unwrap_or_default();
        let module_type = extract_onclick_tag(onclick);

        if !name.is_empty() && !module_type.is_empty() {
            let icon = match module_type.as_str() {
                "bodyEditor" => "globe",
                "information" => "bell",
                "message" => "doc.text",
                "attendance" => "checkmark.circle",
                "courseContent" => "folder",
                "report" => "doc.text",
                "examination" => "list.clipboard",
                "questionnaire" => "list.clipboard",
                "discussion" => "megaphone",
                "wiki" => "book",
                _ => "folder",
            };

            menus.push(LunaCourseMenu {
                name,
                module_type,
                icon: icon.to_string(),
            });
        }
    }

    // Parse announcements
    let mut announcements = Vec::new();
    {
        for row in doc.select(&SEL_INFO_RESULT) {
            let link = row.select(&SEL_INFO_NAME_A).next();
            let title = link
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let info_id = link
                .and_then(|e| e.value().attr("onclick"))
                .and_then(|onclick| {
                    let start = onclick.find(',')? + 1;
                    let end = onclick.find(')')? ;
                    Some(onclick[start..end].trim().to_string())
                })
                .unwrap_or_default();
            let is_new = row.select(&SEL_INFO_PRIORITY).next().is_some();
            let start_date = row.select(&SEL_INFO_START).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let end_date = row.select(&SEL_INFO_END).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            if !title.is_empty() {
                announcements.push(LunaCourseAnnouncement { title, info_id, start_date, end_date, is_new });
            }
        }
    }

    // Parse online tools (Zoom, Panopto, etc.)
    let mut online_tools = Vec::new();
    for a in doc.select(&SEL_ONLINE_LINK) {
        let href = a.value().attr("href").unwrap_or_default().to_string();
        if href.is_empty() { continue; }
        let (name, icon) = if href.contains("zoom") {
            ("Zoom".to_string(), "video".to_string())
        } else if href.contains("panopto") {
            ("Panopto".to_string(), "play.rectangle".to_string())
        } else {
            ("オンラインツール".to_string(), "link".to_string())
        };
        online_tools.push(LunaOnlineTool { name, url: href, icon });
    }

    // Parse attendance rows from the course top page
    let attendances = parse_attendances(&doc, idnumber);

    // Fallback if page didn't load
    if menus.is_empty() && course_name.is_empty() {
        return LunaCourseContents {
            course_name: format!("Course {}", idnumber),
            semester: String::new(),
            teachers: String::new(),
            ta_info: String::new(),
            la_info: String::new(),
            syllabus_url: String::new(),
            grade_url: String::new(),
            menus: Vec::new(),
            announcements: Vec::new(),
            online_tools: Vec::new(),
            materials: Vec::new(),
            reports: Vec::new(),
            examinations: Vec::new(),
            discussions: Vec::new(),
            surveys: Vec::new(),
            attendances: Vec::new(),
        };
    }

    LunaCourseContents {
        course_name,
        semester,
        teachers,
        ta_info,
        la_info,
        syllabus_url,
        grade_url,
        menus,
        announcements,
        online_tools,
        materials: Vec::new(),
        reports: Vec::new(),
        examinations: Vec::new(),
        discussions: Vec::new(),
        surveys: Vec::new(),
        attendances,
    }
}

fn parse_attendances(doc: &Html, fallback_idnumber: &str) -> Vec<LunaAttendanceItem> {
    let mut items = Vec::new();
    for row in doc.select(&SEL_ATT_LIST) {
        let title = row.select(&SEL_ATT_TITLE).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        let date = row.select(&SEL_ATT_DATE).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let mut status = row.select(&SEL_ATT_STATUS).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        let mut can_register = false;
        let mut idnumber = fallback_idnumber.to_string();
        let mut attendance_id = String::new();
        let mut log_type = String::new();

        if let Some(a) = row.select(&SEL_ATT_ACTION_A).next() {
            let link_text = a.text().collect::<String>().trim().to_string();
            if !link_text.is_empty() {
                status = link_text;
            }
            let data1 = a.value().attr("data1").unwrap_or_default().to_string();
            let data2 = a.value().attr("data2").unwrap_or_default().to_string();
            let data3 = a.value().attr("data3").unwrap_or_default().to_string();

            if !data1.is_empty() { idnumber = data1; }
            attendance_id = data2;
            log_type = data3;
            can_register = !attendance_id.is_empty() && status.contains("受付");
        }

        if title.is_empty() && date.is_empty() && status.is_empty() {
            continue;
        }

        items.push(LunaAttendanceItem {
            title,
            date,
            status,
            can_register,
            idnumber,
            attendance_id,
            log_type,
        });
    }
    items
}

/// Extract TA or LA info from the course page readmore section
fn extract_staff_info(doc: &Html, label: &str) -> String {
    for div in doc.select(&SEL_READMORE_DIV) {
        let text = div.text().collect::<String>();
        if text.contains(label) {
            // Extract the value after the label span
            let spans: Vec<String> = div.select(&SEL_SPAN)
                .map(|s| s.text().collect::<String>().trim().to_string())
                .collect();
            // spans[0] = label, spans[1..] = values
            if spans.len() > 1 {
                let val = spans[1..].join(", ").trim().to_string();
                if !val.is_empty() && val != "担当者なし" {
                    return val;
                }
            }
            break;
        }
    }
    String::new()
}

/// Extract the tag name from onclick like: sidemenuLinkMaker(this.getAttribute('data1'), ..., 'courseContent')
fn extract_onclick_tag(onclick: &str) -> String {
    // Pattern: last argument in single quotes
    if let Some(last_quote) = onclick.rfind('\'') {
        let before = &onclick[..last_quote];
        if let Some(start_quote) = before.rfind('\'') {
            return before[start_quote + 1..].to_string();
        }
    }
    String::new()
}

/// Parse the contents top page (/lms/contents?idnumber=XXX)
/// Extracts materials, reports, examinations, discussions
pub fn parse_luna_contents_page(html: &str) -> (Vec<LunaContentItem>, Vec<LunaContentItem>, Vec<LunaContentItem>, Vec<LunaContentItem>, Vec<LunaContentItem>) {
    let doc = Html::parse_document(html);
    let materials = parse_materials(&doc);
    let reports = parse_reports(&doc);
    let examinations = parse_examinations(&doc);
    let discussions = parse_discussions(&doc);
    let surveys = parse_surveys(&doc);
    (materials, reports, examinations, discussions, surveys)
}

/// Extract plain text from a Quill Delta JSON embedded in a JS script.
/// The script contains: `_QuillUtil.xxx.setJsonData("{...}", ...)`
#[cfg(test)]
fn extract_quill_delta_text(script: &str) -> Option<String> {
    let marker = ".setJsonData(\"";
    let start = script.find(marker)? + marker.len();
    let rest = &script[start..];
    // Walk to find the unescaped closing quote
    let mut i = 0;
    let bytes = rest.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2; // skip escape sequence
        } else if bytes[i] == b'"' {
            break;
        } else {
            i += 1;
        }
    }
    if i >= bytes.len() { return None; }
    let escaped = &rest[..i];
    // Treat as a JSON string body to decode \uXXXX, \", \\n etc.
    let json_lit = format!("\"{}\"", escaped);
    let inner_json: String = serde_json::from_str(&json_lit).ok()?;
    let val: serde_json::Value = serde_json::from_str(&inner_json).ok()?;
    let ops = val.get("ops")?.as_array()?;
    let mut text = String::new();
    for op in ops {
        if let Some(s) = op.get("insert").and_then(|v| v.as_str()) {
            text.push_str(s);
        }
    }
    let trimmed = text.trim().to_string();
    if trimmed.is_empty() { None } else { Some(trimmed) }
}

fn extract_quill_delta_html(script: &str) -> Option<String> {
    let marker = ".setJsonData(\"";
    let start = script.find(marker)? + marker.len();
    let rest = &script[start..];
    let mut i = 0;
    let bytes = rest.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
        } else if bytes[i] == b'"' {
            break;
        } else {
            i += 1;
        }
    }
    if i >= bytes.len() { return None; }
    let escaped = &rest[..i];
    extract_quill_rich_html(escaped)
}

fn parse_materials(doc: &Html) -> Vec<LunaContentItem> {
    let mut items = Vec::new();

    // Each materialList div is a folder with materials
    for folder in doc.select(&SEL_MATERIAL_LIST) {
        let title = folder.select(&SEL_MAT_TITLE).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        if title.is_empty() { continue; }

        let period = folder.select(&SEL_INPUT_SPAN)
            .map(|e| e.text().collect::<String>().trim().to_string()).find(|s| s.contains('～'))
            .unwrap_or_default();

        // Prefer rendered Quill HTML to preserve rich text formatting.
        // Fallback: parse Quill Delta JSON from <script> tags as rich HTML.
        let description = {
            let mut text = String::new();
            if let Some(editor) = folder.select(&SEL_QL_EDITOR).next() {
                text = editor.inner_html().trim().to_string();
            }
            if text.is_empty() {
                for el in folder.select(&SEL_SCRIPT) {
                    let src = el.inner_html();
                    if let Some(t) = extract_quill_delta_html(&src) {
                        text = t;
                        break;
                    }
                }
            }
            text
        };

        // Parse individual material files with download metadata
        let mut files = Vec::new();
        for row in folder.select(&SEL_MAT_CSS) {
            let display_name = row.select(&SEL_MAT_FILE_NAME).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            if display_name.is_empty() { continue; }

            let file_name = row.select(&SEL_FILENAME).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let object_name = row.select(&SEL_OBJECT_NAME).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let resource_id = row.select(&SEL_RESOURCE_ID).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let file_type = row.select(&SEL_FILETYPE).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let material_id = row.select(&SEL_DL_MAT_ID).next()
                .and_then(|e| e.value().attr("value"))
                .unwrap_or_default()
                .to_string();
            let end_date = row.select(&SEL_OPEN_END_DATE).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let scan_status = row.select(&SEL_SCAN_STATUS).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            let link_type = if file_type == "0" {
                classify_link(&file_name, &display_name)
            } else {
                let cl = classify_link(&display_name, &file_name);
                if cl == "file" { "web".to_string() } else { cl }
            };

            files.push(LunaMaterialFile {
                display_name,
                file_name,
                object_name,
                resource_id,
                material_id,
                file_type,
                end_date,
                scan_status,
                link_type,
            });
        }

        let status = if files.is_empty() {
            String::new()
        } else {
            files.iter().map(|f| f.display_name.as_str()).collect::<Vec<_>>().join(", ")
        };

        items.push(LunaContentItem {
            title,
            url: String::new(),
            period,
            status,
            description,
            item_type: "material".to_string(),
            files,
        });
    }
    items
}

fn parse_reports(doc: &Html) -> Vec<LunaContentItem> {
    let mut items = Vec::new();

    for row in doc.select(&SEL_REPORT_LIST) {
        let a = match row.select(&SEL_RPT_NAME).next() {
            Some(a) => a,
            None => continue,
        };
        let title = a.text().collect::<String>().trim().to_string();
        let url = a.value().attr("href").unwrap_or_default().to_string();

        let start = row.select(&SEL_RPT_START).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        let end = row.select(&SEL_RPT_END).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        let period = if !start.is_empty() && !end.is_empty() {
            format!("{} ～ {}", start, end)
        } else {
            String::new()
        };

        let status = row.select(&SEL_RPT_STATUS).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        items.push(LunaContentItem {
            title,
            url,
            period,
            status,
            description: String::new(),
            item_type: "report".to_string(),
            files: Vec::new(),
        });
    }
    items
}

fn parse_examinations(doc: &Html) -> Vec<LunaContentItem> {
    let mut items = Vec::new();

    for row in doc.select(&SEL_EXAM_LIST) {
        // Try primary selector, then fallback
        let (title, mut url) = if let Some(a) = row.select(&SEL_EXAM_NAME).next() {
            let t = a.text().collect::<String>().trim().to_string();
            let u = a.value().attr("href").unwrap_or_default().to_string();
            (t, u)
        } else if let Some(a) = row.select(&SEL_LINK_TXT).next() {
            let t = a.text().collect::<String>().trim().to_string();
            let u = a.value().attr("href").unwrap_or_default().to_string();
            (t, u)
        } else if let Some(el) = row.select(&SEL_EXAM_NAME_FB).next() {
            let t = el.text().collect::<String>().trim().to_string();
            (t, String::new())
        } else {
            continue;
        };

        if title.is_empty() { continue; }

        // If href is empty or "#", try extracting URL from onclick
        if url.is_empty() || url == "#" || url == "javascript:void(0)" {
            url = extract_url_from_row(&row);
        }

        let period = row.select(&SEL_EXAM_PERIOD).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let status = row.select(&SEL_EXAM_STATUS).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        items.push(LunaContentItem {
            title,
            url,
            period,
            status,
            description: String::new(),
            item_type: "examination".to_string(),
            files: Vec::new(),
        });
    }
    items
}

fn parse_discussions(doc: &Html) -> Vec<LunaContentItem> {
    let mut items = Vec::new();

    for row in doc.select(&SEL_DISC_LIST) {
        let (title, mut url) = if let Some(a) = row.select(&SEL_DISC_NAME).next() {
            let t = a.text().collect::<String>().trim().to_string();
            let u = a.value().attr("href").unwrap_or_default().to_string();
            (t, u)
        } else if let Some(a) = row.select(&SEL_LINK_TXT).next() {
            let t = a.text().collect::<String>().trim().to_string();
            let u = a.value().attr("href").unwrap_or_default().to_string();
            (t, u)
        } else if let Some(el) = row.select(&SEL_DISC_NAME_FB).next() {
            let t = el.text().collect::<String>().trim().to_string();
            (t, String::new())
        } else {
            continue;
        };

        if title.is_empty() { continue; }

        if url.is_empty() || url == "#" || url == "javascript:void(0)" {
            url = extract_url_from_row(&row);
        }

        let period = row.select(&SEL_DISC_PERIOD).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let status = row.select(&SEL_DISC_STATUS).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        items.push(LunaContentItem {
            title,
            url,
            period,
            status,
            description: String::new(),
            item_type: "discussion".to_string(),
            files: Vec::new(),
        });
    }
    items
}

fn parse_surveys(doc: &Html) -> Vec<LunaContentItem> {
    let mut items = Vec::new();

    for row in doc.select(&SEL_SURVEY_LIST) {
        let (title, mut url) = if let Some(a) = row.select(&SEL_SURV_NAME).next() {
            let t = a.text().collect::<String>().trim().to_string();
            let u = a.value().attr("href").unwrap_or_default().to_string();
            (t, u)
        } else if let Some(a) = row.select(&SEL_LINK_TXT).next() {
            let t = a.text().collect::<String>().trim().to_string();
            let u = a.value().attr("href").unwrap_or_default().to_string();
            (t, u)
        } else if let Some(el) = row.select(&SEL_SURV_NAME_FB).next() {
            let t = el.text().collect::<String>().trim().to_string();
            (t, String::new())
        } else {
            continue;
        };

        if title.is_empty() { continue; }

        if url.is_empty() || url == "#" || url == "javascript:void(0)" {
            url = extract_url_from_row(&row);
        }

        let period = row.select(&SEL_SURV_PERIOD).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let status = row.select(&SEL_SURV_STATUS).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        items.push(LunaContentItem {
            title,
            url,
            period,
            status,
            description: String::new(),
            item_type: "survey".to_string(),
            files: Vec::new(),
        });
    }
    items
}

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
                let params: Vec<(String, String)> = form.select(&hidden_sel)
                    .filter_map(|input| {
                        let name = input.value().attr("name").unwrap_or_default();
                        let val = input.value().attr("value").unwrap_or_default();
                        if !val.is_empty() && name != "fileId" && name != "fileName" {
                            Some((name.to_string(), val.to_string()))
                        } else { None }
                    })
                    .collect();
                Some((action, params))
            } else { None }
        } else { None }
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

    let header_sel = Selector::parse(".contents-list > .contents-detail.contents-vertical").unwrap();
    for row in doc.select(&header_sel) {
        let label_sel = Selector::parse(".contents-header .bold-txt").unwrap();
        let label = row.select(&label_sel).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let value_sel = Selector::parse(".contents-input-area").unwrap();
        let value_el = row.select(&value_sel).next();

        match label.as_str() {
            "タイトル" => {
                title = value_el.map(|e| e.text().collect::<String>().trim().to_string()).unwrap_or_default();
            }
            "内容" => {
                // Quill editor content — in static HTML, .ql-editor doesn't exist.
                // Try extracting from setJsonData in page-level scripts first.
                description = extract_named_quill_text(html, "bodyText")
                    .unwrap_or_default();
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
                    .map(|e| e.select(&SEL_SPAN).map(|s| s.text().collect::<String>().trim().to_string()).collect::<Vec<_>>().join(" "))
                    .unwrap_or_default();
            }
            "記名・無記名" => {
                anonymity = value_el.map(|e| e.text().collect::<String>().trim().to_string()).unwrap_or_default();
            }
            "回答の修正" => {
                allow_edit = value_el.map(|e| e.text().collect::<String>().trim().to_string()).unwrap_or_default();
            }
            "回答状況" => {
                answer_status = value_el.map(|e| e.text().collect::<String>().trim().to_string()).unwrap_or_default();
            }
            "氏名" => {
                respondent = value_el.map(|e| e.text().collect::<String>().trim().to_string()).unwrap_or_default();
            }
            "添付ファイル" => {
                if let Some(area) = value_el {
                    let dl_sel = Selector::parse(".downloadFile").unwrap();
                    let obj_sel = Selector::parse(".objectName").unwrap();
                    let fname_sel = Selector::parse(".fileName").unwrap();
                    let file_name = area.select(&fname_sel).next()
                        .or_else(|| area.select(&dl_sel).next())
                        .map(|e| e.text().collect::<String>().trim().to_string())
                        .unwrap_or_default();
                    let object_name = area.select(&obj_sel).next()
                        .map(|e| e.text().collect::<String>().trim().to_string())
                        .unwrap_or_default();
                    if !file_name.is_empty() {
                        let (dl_action, mut dl_params) = if let Some((ref act, ref params)) = survey_dl_form {
                            (act.clone(), params.clone())
                        } else {
                            (String::new(), Vec::new())
                        };
                        // Add per-file dynamic fields: fileId = objectName, fileName = raw filename
                        dl_params.push(("fileId".to_string(), object_name.clone()));
                        dl_params.push(("fileName".to_string(), file_name.clone()));
                        attachments.push(LunaSurveyAttachment {
                            file_name, object_name, url: String::new(),
                            download_action: dl_action, download_params: dl_params,
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

    for (q_idx, q_el) in doc.select(&q_block_sel).enumerate() {
        let required = q_el.select(&q_required_sel).next().is_some();
        let number = (q_idx + 1).to_string();

        // Extract question body from surveyTakeItemText.setJsonData in script
        let q_html = q_el.html();
        let body = extract_named_quill_text(&q_html, "surveyTakeItemText")
            .unwrap_or_default();

        // Determine answer type from hidden branchType input
        let branch_type = q_el.select(&q_branch_type_sel).next()
            .and_then(|e| e.value().attr("value"))
            .unwrap_or_default();
        let answer_type = match branch_type {
            "list" => "list",
            "radio" | "multRadio" => "radio",
            "check" | "multCheck" => "checkbox",
            "text" | "textArea" => "text",
            _ => "",
        }.to_string();

        // Extract option labels from answerListContents_X_Y.setJsonData
        let mut options = Vec::new();
        if answer_type != "text" {
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

        if !body.is_empty() || !options.is_empty() {
            questions.push(LunaSurveyQuestion {
                number,
                body,
                required,
                answer_type,
                options,
            });
        }
    }

    // Extract hidden form fields from #surveysTakeForm for submission
    let mut form_fields: Vec<(String, String)> = Vec::new();
    let form_sel = Selector::parse("#surveysTakeForm").unwrap();
    if let Some(form) = doc.select(&form_sel).next() {
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
                    log::debug!("[survey attachments] action='{}', params={:?}", action, params);
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
    }
}

/// Extract a URL from onclick attributes or <a> tags within a row element
fn extract_url_from_row(row: &scraper::ElementRef) -> String {
    // Check all <a> tags for href
    for a in row.select(&SEL_A_HREF) {
        let href = a.value().attr("href").unwrap_or_default();
        if !href.is_empty() && href != "#" && !href.starts_with("javascript:") {
            return href.to_string();
        }
    }
    // Check onclick attributes for URL patterns
    let row_html = row.html();
    // Pattern: location.href='...' or window.open('...')
    for pattern in &["location.href='", "location.href=\"", "window.open('", "window.open(\""] {
        if let Some(start) = row_html.find(pattern) {
            let after = &row_html[start + pattern.len()..];
            let quote = if pattern.ends_with('\'') { '\'' } else { '"' };
            if let Some(end) = after.find(quote) {
                let url = &after[..end];
                if url.starts_with('/') {
                    return url.to_string();
                }
            }
        }
    }
    String::new()
}

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
    match select_el.select(&option_sel).next() {
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
    select_el.select(&option_sel).map(|opt| {
        SelectOption {
            value: opt.value().attr("value").unwrap_or_default().to_string(),
            label: opt.text().collect::<String>().trim().to_string(),
            selected: opt.value().attr("selected").is_some(),
        }
    }).collect()
}

fn parse_japanese_number(s: &str) -> u32 {
    if s.contains('１') { return 1; }
    if s.contains('２') { return 2; }
    if s.contains('３') { return 3; }
    if s.contains('４') { return 4; }
    if s.contains('５') { return 5; }
    if s.contains('６') { return 6; }
    if s.contains('７') { return 7; }
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
        assert!(!result.attachments.is_empty(), "Should have at least one attachment");
        let att = &result.attachments[0];
        assert!(!att.name.is_empty(), "Attachment name should not be empty");
        assert!(!att.url.is_empty(), "Attachment URL should not be empty");
        println!("PASS: name='{}', url='{}'", att.name, att.url);
    }

    #[test]
    #[ignore] // requires local HTML dump file
    fn test_parse_course_page() {
        let html = std::fs::read_to_string("/tmp/luna_detail_lms_course_idnumber=2026510010040201#information.html")
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
            println!("  {} [{}~{}] new={}", a.title, a.start_date, a.end_date, a.is_new);
        }
        println!("Online tools: {}", result.online_tools.len());
        for t in &result.online_tools {
            println!("  {} -> {}", t.name, t.url);
        }
        assert!(!result.course_name.is_empty(), "Course name should not be empty");
        assert!(!result.menus.is_empty(), "Should have menus");
    }

    #[test]
    #[ignore] // requires local HTML dump file
    fn test_parse_contents_page() {
        let html = std::fs::read_to_string("/tmp/luna_contents_2026510010040201.html")
            .expect("Contents HTML dump file not found");
        let (materials, reports, examinations, discussions, _surveys) = parse_luna_contents_page(&html);
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
        assert!(!materials.is_empty() || !reports.is_empty() || !examinations.is_empty() || !discussions.is_empty(),
            "Should have at least some content items");
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
        assert!(!result.sections.is_empty(), "Should have body content from Quill");
        // The body should contain the teacher's message
        let body = &result.sections[0].body;
        assert!(body.contains("掛橋"), "Body should contain teacher name");
        assert!(body.contains("オンデマンド"), "Body should contain 'オンデマンド'");
        // Meta should have 掲示期間 and 発信者
        assert!(result.meta.iter().any(|(k, _)| k == "掲示期間"), "Should have 掲示期間");
        assert!(result.meta.iter().any(|(k, _)| k == "発信者"), "Should have 発信者");
    }

    #[test]
    fn test_extract_quill_delta_text() {
        let script = r#"
            _QuillUtil.materialContents_0.setJsonData("{\"ops\":[{\"insert\":\"\u51FA\u5E2D\u78BA\u8A8D\u306F\u6388\u696D\u5192\u982D\u306B\u884C\u3044\u307E\u3059\u3002\\n\"},{\"attributes\":{\"bold\":true},\"insert\":\"\u5EA7\u5E2D\u8868\u304C\u3042\u308A\u307E\u3059\u3002\"},{\"insert\":\"\\n\"}]}", 'reference');
        "#;
        let result = extract_quill_delta_text(script);
        assert!(result.is_some(), "Should extract text from Quill Delta");
        let text = result.unwrap();
        assert!(text.contains("出席確認は授業冒頭に行います。"), "Should contain decoded Japanese text");
        assert!(text.contains("座席表があります。"), "Should contain bold text too");
    }

    #[test]
    fn test_extract_quill_delta_html() {
        let script = r#"
            _QuillUtil.materialContents_0.setJsonData("{\"ops\":[{\"attributes\":{\"bold\":true},\"insert\":\"\u592A\u5B57\"},{\"insert\":\" \"},{\"attributes\":{\"italic\":true,\"link\":\"https://example.com\"},\"insert\":\"\u30EA\u30F3\u30AF\"},{\"insert\":\"\\n\"}]}", 'reference');
        "#;
        let result = extract_quill_delta_html(script);
        assert!(result.is_some(), "Should extract rich HTML from Quill Delta");
        let html = result.unwrap();
        assert!(html.contains("<strong>太字</strong>"), "Should preserve bold style");
        assert!(html.contains("<em>リンク</em>"), "Should preserve italic style");
        assert!(html.contains("href=\"https://example.com\""), "Should preserve link href");
    }
}
