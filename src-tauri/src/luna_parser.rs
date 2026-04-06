use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

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
    let row_sel = Selector::parse(".div-table-data-row").expect("valid selector");
    let period_sel = Selector::parse(".div-table-colomn-period").expect("valid selector");
    let cell_sel = Selector::parse(".div-table-cell").expect("valid selector");
    let course_btn_sel = Selector::parse(".timetable-course-top-btn").expect("valid selector");
    let detail_sel = Selector::parse(".div-table-cell-detail span").expect("valid selector");

    for row in doc.select(&row_sel) {
        // Extract period number from text like "１時限"
        let period_text = row.select(&period_sel).next()
            .map(|e| e.text().collect::<String>())
            .unwrap_or_default();
        let period = parse_japanese_number(&period_text);
        if period == 0 { continue; }

        // Each cell corresponds to a day (1=月 through 6=土)
        for (i, cell) in row.select(&cell_sel).enumerate() {
            let day = (i + 1) as u32;
            if let Some(btn) = cell.select(&course_btn_sel).next() {
                let idnumber = btn.value().attr("id").unwrap_or_default().to_string();
                let name = btn.text().collect::<String>().trim().to_string();
                let teacher = cell.select(&detail_sel)
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
    let comm_sel = Selector::parse(".timetable-community-course .timetable-course-top-btn").expect("valid selector");
    for el in doc.select(&comm_sel) {
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

    let list_sel = Selector::parse(".todo-list").expect("valid selector");
    let course_sel = Selector::parse(".todolist-course").expect("valid selector");
    let type_sel = Selector::parse(".todolist-contents-type span").expect("valid selector");
    let name_sel = Selector::parse(".todolist-contents-name a").expect("valid selector");
    let deadline_sel = Selector::parse(".todolist-mobile-width-deadline").expect("valid selector");
    let status_sel = Selector::parse(".todolist-contents-status span").expect("valid selector");
    let feedback_sel = Selector::parse(".todolist-feedback .todolist-mobile-feedback").expect("valid selector");

    for item in doc.select(&list_sel) {
        let course_name = item.select(&course_sel).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let content_type = item.select(&type_sel).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let (content_name, url) = item.select(&name_sel).next()
            .map(|e| (
                e.text().collect::<String>().trim().to_string(),
                e.value().attr("href").unwrap_or_default().to_string(),
            ))
            .unwrap_or_default();

        let deadline = item.select(&deadline_sel).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let status = item.select(&status_sel).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let feedback = item.select(&feedback_sel).next()
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

    let list_sel = Selector::parse(".update-info-list").expect("valid selector");
    let date_sel = Selector::parse(".update-info-updateDate label").expect("valid selector");
    let course_sel = Selector::parse(".update-info-courseInfo span").expect("valid selector");
    let module_sel = Selector::parse(".update-info-module span").expect("valid selector");
    let content_sel = Selector::parse(".update-info-contents .break-word").expect("valid selector");
    let url_sel = Selector::parse(".updateInfoUrl").expect("valid selector");
    let id_sel = Selector::parse("input[id='idnumber']").expect("valid selector");

    for item in doc.select(&list_sel) {
        let date = item.select(&date_sel).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let course_info = item.select(&course_sel).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let module = item.select(&module_sel).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let content = item.select(&content_sel).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let url = item.select(&url_sel).next()
            .and_then(|e| e.value().attr("value"))
            .unwrap_or_default()
            .to_string();

        let idnumber = item.select(&id_sel).next()
            .and_then(|e| e.value().attr("value"))
            .unwrap_or_default()
            .to_string();

        if !date.is_empty() {
            items.push(LunaNotification {
                date,
                course_info,
                module,
                content,
                url,
                idnumber,
            });
        }
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
    if let (Ok(row_sel), Ok(label_sel), Ok(value_sel)) = (
        Selector::parse(".block > .contents-list > .contents-detail.contents-vertical"),
        Selector::parse(".contents-header-txt .bold-txt, .contents-header-txt"),
        Selector::parse(".contents-input-area"),
    ) {
        for row in doc.select(&row_sel) {
            let label = row.select(&label_sel).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            if label == "内容" || label == "添付ファイル" { continue; }
            let value = row.select(&value_sel).next()
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
    if let Ok(row_sel) = Selector::parse("#themeTopList .result-list.sp-contents-hidden") {
        let title_sel = Selector::parse(".theme-top-thread-title.link-txt").ok();
        let author_sel = Selector::parse(".theme-top-thread-author").ok();
        let date_sel = Selector::parse(".theme-top-thread-createdate").ok();
        let status_sel = Selector::parse(".theme-top-thread-postzyoukyou").ok();

        for (idx, row) in doc.select(&row_sel).enumerate() {
            let thread_title = title_sel.as_ref()
                .and_then(|s| row.select(s).next())
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            // Extract threadId from onclick="viewthread(50406);"
            let thread_id = title_sel.as_ref()
                .and_then(|s| row.select(s).next())
                .and_then(|e| e.value().attr("onclick"))
                .and_then(|onclick| {
                    let start = onclick.find('(')? + 1;
                    let end = onclick.find(')')? ;
                    Some(onclick[start..end].trim().to_string())
                })
                .unwrap_or_default();
            let author = author_sel.as_ref()
                .and_then(|s| row.select(s).next())
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let date = date_sel.as_ref()
                .and_then(|s| row.select(s).next())
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let status = status_sel.as_ref()
                .and_then(|s| row.select(s).next())
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
    if let (Ok(row_sel), Ok(label_sel), Ok(value_sel)) = (
        Selector::parse(".block > .contents-list > .contents-detail.contents-vertical"),
        Selector::parse(".contents-header-txt .bold-txt"),
        Selector::parse(".contents-input-area"),
    ) {
        for row in doc.select(&row_sel) {
            let label = row.select(&label_sel).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            if label == "説明" || label == "投稿内容" || label == "添付ファイル"
                || label == "返信先投稿内容" { continue; }
            let value = row.select(&value_sel).next()
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
    extract_quill_plain_text(json_str)
}

/// Parse any Luna detail page (report/submission, examination, forum, etc.)
///
/// Luna detail pages use a consistent pattern:
///   .course-title-txt          → course name
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
    if let (Ok(row_sel), Ok(label_sel), Ok(value_sel)) = (
        Selector::parse(".contents-detail.contents-vertical"),
        Selector::parse(".contents-header-txt .bold-txt, .contents-header-txt"),
        Selector::parse(".contents-input-area"),
    ) {
        let dl_check = Selector::parse(".downloadFile").ok();
        for row in doc.select(&row_sel) {
            let label = row.select(&label_sel).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            // Skip attachment rows — they'll be handled in the attachments section
            if let Some(ref dls) = dl_check {
                if row.select(dls).next().is_some() {
                    continue;
                }
            }

            let value = row.select(&value_sel).next()
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
    let download_form_info: Option<(String, Vec<(String, String)>)> = {
        let form_selectors = [
            "#reportDownloadForm",
            "#forumsPostFile",
        ];
        let mut info = None;
        for sel_str in &form_selectors {
            if let Ok(sel) = Selector::parse(sel_str) {
                if let Some(form) = doc.select(&sel).next() {
                    let action = form.value().attr("action").unwrap_or_default().to_string();
                    if !action.is_empty() {
                        // Collect fixed hidden input params
                        let mut params = Vec::new();
                        if let Ok(input_sel) = Selector::parse("input[type='hidden']") {
                            for input in form.select(&input_sel) {
                                let iname = input.value().attr("name").unwrap_or_default();
                                let ival = input.value().attr("value").unwrap_or_default();
                                // Skip dynamic fields that are filled by JS (empty value)
                                // and CSRF/session tokens
                                if !ival.is_empty()
                                    && iname != "_cid" && iname != "_csrf"
                                    && iname != "downloadFileName" && iname != "downloadMode"
                                {
                                    params.push((iname.to_string(), ival.to_string()));
                                }
                            }
                        }
                        log::debug!("[attachments] download form: action='{}', params={:?}", action, params);
                        info = Some((action, params));
                        break;
                    }
                }
            }
        }
        info
    };

    // Pattern 1: .downloadFile elements (siblings of .objectName in same parent div)
    if let (Ok(dl_sel), Ok(obj_sel)) = (
        Selector::parse(".downloadFile"),
        Selector::parse(".objectName"),
    ) {
        for el in doc.select(&dl_sel) {
            let name = el.text().collect::<String>().trim().to_string();
            if !name.is_empty() {
                let object_name = el.parent()
                    .and_then(scraper::ElementRef::wrap)
                    .and_then(|parent_el| {
                        parent_el.select(&obj_sel).next()
                            .map(|e| e.text().collect::<String>().trim().to_string())
                    })
                    .unwrap_or_default();

                // Build URL using the discovered download form
                let url = if let Some((ref action, ref fixed_params)) = download_form_info {
                    let mut params: Vec<String> = fixed_params.iter()
                        .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
                        .collect();
                    if !object_name.is_empty() {
                        params.push(format!("objectName={}", urlencoding::encode(&object_name)));
                    }
                    // Append filename to the action path (Luna convention)
                    let encoded_name = urlencoding::encode(&name);
                    format!("{}/{}?{}", action, encoded_name, params.join("&"))
                } else {
                    // Fallback: generic tempfile endpoint
                    format!("/lms/course/make/tempfile?objectName={}", object_name)
                };

                log::debug!("[attachments] name='{}', url='{}'", name, url);
                attachments.push(LunaAttachment { name, url });
            }
        }
    }

    // Pattern 2: links with download/tempfile in href
    if attachments.is_empty() {
        let file_selectors = [
            "a[href*='tempfile']",
            "a[href*='download']",
        ];
        for sel_str in &file_selectors {
            if let Ok(file_sel) = Selector::parse(sel_str) {
                for a in doc.select(&file_sel) {
                    let name = a.text().collect::<String>().trim().to_string();
                    let url = a.value().attr("href").unwrap_or_default().to_string();
                    if !name.is_empty() && !url.is_empty() && !url.contains("javascript:") {
                        attachments.push(LunaAttachment { name, url });
                    }
                }
            }
        }
    }

    // Pattern 3: external links (e.g. SharePoint video links)
    if let Ok(video_sel) = Selector::parse(".block-list-video a[href], .examination-movie a[href]") {
        for a in doc.select(&video_sel) {
            let url = a.value().attr("href").unwrap_or_default().to_string();
            if !url.is_empty() && url.starts_with("http") {
                // Show a friendly name for external video links
                let display_name = if url.contains("sharepoint.com") {
                    "動画 (SharePoint)".to_string()
                } else if url.contains("youtube") || url.contains("youtu.be") {
                    "動画 (YouTube)".to_string()
                } else {
                    "外部リンク".to_string()
                };
                attachments.push(LunaAttachment { name: display_name, url });
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

    // Also extract any links from the page body that might be useful (external links, etc.)
    if let Ok(body_link_sel) = Selector::parse(".contents-input-area a[href], .ql-editor a[href]") {
        for a in doc.select(&body_link_sel) {
            let url = a.value().attr("href").unwrap_or_default().to_string();
            let name = a.text().collect::<String>().trim().to_string();
            if !url.is_empty() && url.starts_with("http")
                && !attachments.iter().any(|att| att.url == url)
            {
                let display = if name.is_empty() { url.clone() } else { name };
                attachments.push(LunaAttachment { name: display, url });
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
    if let (Ok(row_sel), Ok(label_sel), Ok(value_sel)) = (
        Selector::parse(".contents-detail.contents-vertical"),
        Selector::parse(".contents-header-txt .bold-txt"),
        Selector::parse(".contents-input-area"),
    ) {
        let dl_check = Selector::parse(".downloadFile, .cmtInfoFileName").ok();
        for row in doc.select(&row_sel) {
            let label = row.select(&label_sel).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            // Skip "内容" — it's the Quill content we already extracted
            if label == "内容" { continue; }

            // Check for attachment rows
            if let Some(ref dls) = dl_check {
                if row.select(dls).next().is_some() {
                    // TODO: extract attachment file info if needed
                    continue;
                }
            }

            // Collect text from the value area, filtering out script content
            let value = row.select(&value_sel).next()
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
    let teachers = Selector::parse(".contents-detail-readmore-txt span").ok()
        .map(|sel| {
            let spans: Vec<String> = doc.select(&sel)
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
        })
        .unwrap_or_default();

    // TA/LA info from the readmore section
    let ta_info = extract_staff_info(&doc, "担当TA");
    let la_info = extract_staff_info(&doc, "担当LA");

    // Syllabus link
    let syllabus_url = Selector::parse(".class-header-syllabus").ok()
        .and_then(|sel| doc.select(&sel).next())
        .and_then(|el| el.value().attr("href").map(|s| s.to_string()))
        .unwrap_or_default();

    // Grade link
    let grade_url = Selector::parse("a[href*='external_grade']").ok()
        .and_then(|sel| doc.select(&sel).next())
        .and_then(|el| el.value().attr("href").map(|s| s.to_string()))
        .unwrap_or_default();

    // Parse sidebar menu items (navigation categories only)
    let mut menus = Vec::new();
    if let Ok(menu_sel) = Selector::parse("#sidemenuListMessage a[onclick], #sidemenuListEdit a[onclick]") {
        for a in doc.select(&menu_sel) {
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
    }

    // Parse announcements
    let mut announcements = Vec::new();
    if let Ok(row_sel) = Selector::parse(".course-result-list.sp-contents-hidden") {
        let name_sel = Selector::parse(".class-view-information-name a").ok();
        let new_sel = Selector::parse(".portal-information-priority").ok();
        let start_sel = Selector::parse(".class-view-information-start").ok();
        let end_sel = Selector::parse(".class-view-information-end").ok();
        for row in doc.select(&row_sel) {
            let link = name_sel.as_ref()
                .and_then(|s| row.select(s).next());
            let title = link
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            // Extract info_id from onclick="InfoDetailCourseTop(event,414541);"
            let info_id = link
                .and_then(|e| e.value().attr("onclick"))
                .and_then(|onclick| {
                    let start = onclick.find(',')? + 1;
                    let end = onclick.find(')')? ;
                    Some(onclick[start..end].trim().to_string())
                })
                .unwrap_or_default();
            let is_new = new_sel.as_ref()
                .map(|s| row.select(s).next().is_some())
                .unwrap_or(false);
            let start_date = start_sel.as_ref()
                .and_then(|s| row.select(s).next())
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let end_date = end_sel.as_ref()
                .and_then(|s| row.select(s).next())
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            if !title.is_empty() {
                announcements.push(LunaCourseAnnouncement { title, info_id, start_date, end_date, is_new });
            }
        }
    }

    // Parse online tools (Zoom, Panopto, etc.)
    let mut online_tools = Vec::new();
    if let Ok(link_sel) = Selector::parse("#online .online-link a[href]") {
        for a in doc.select(&link_sel) {
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
    }

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
    }
}

/// Extract TA or LA info from the course page readmore section
fn extract_staff_info(doc: &Html, label: &str) -> String {
    if let Ok(sel) = Selector::parse(".contents-detail-readmore-txt div") {
        for div in doc.select(&sel) {
            let text = div.text().collect::<String>();
            if text.contains(label) {
                // Extract the value after the label span
                let span_sel = Selector::parse("span").expect("valid selector");
                let spans: Vec<String> = div.select(&span_sel)
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
pub fn parse_luna_contents_page(html: &str) -> (Vec<LunaContentItem>, Vec<LunaContentItem>, Vec<LunaContentItem>, Vec<LunaContentItem>) {
    let doc = Html::parse_document(html);
    let materials = parse_materials(&doc);
    let reports = parse_reports(&doc);
    let examinations = parse_examinations(&doc);
    let discussions = parse_discussions(&doc);
    (materials, reports, examinations, discussions)
}

fn parse_materials(doc: &Html) -> Vec<LunaContentItem> {
    let mut items = Vec::new();
    let folder_sel = match Selector::parse("#courseContent #materialList") {
        Ok(s) => s,
        Err(_) => return items,
    };
    let title_sel = Selector::parse(".course-material-title-txt").expect("valid selector");
    let period_sel = Selector::parse(".contents-input-area span").expect("valid selector");
    let file_link_sel = Selector::parse(".material-file-name").expect("valid selector");
    let result_sel = Selector::parse(".course-result-list.materialCss").expect("valid selector");
    let filename_sel = Selector::parse(".fileName").expect("valid selector");
    let objname_sel = Selector::parse(".objectName").expect("valid selector");
    let resid_sel = Selector::parse(".resource_Id").expect("valid selector");
    let filetype_sel = Selector::parse(".fileType").expect("valid selector");
    let dlmatid_sel = Selector::parse("#dlMaterialId").expect("valid selector");
    let enddate_sel = Selector::parse(".openEndDate").expect("valid selector");
    let scanstatus_sel = Selector::parse(".scanStatus").expect("valid selector");

    // Each materialList div is a folder with materials
    for folder in doc.select(&folder_sel) {
        let title = folder.select(&title_sel).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        if title.is_empty() { continue; }

        let period = folder.select(&period_sel)
            .map(|e| e.text().collect::<String>().trim().to_string()).find(|s| s.contains('～'))
            .unwrap_or_default();

        // Parse individual material files with download metadata
        let mut files = Vec::new();
        for row in folder.select(&result_sel) {
            let display_name = row.select(&file_link_sel).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            if display_name.is_empty() { continue; }

            let file_name = row.select(&filename_sel).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let object_name = row.select(&objname_sel).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let resource_id = row.select(&resid_sel).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let file_type = row.select(&filetype_sel).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let material_id = row.select(&dlmatid_sel).next()
                .and_then(|e| e.value().attr("value"))
                .unwrap_or_default()
                .to_string();
            let end_date = row.select(&enddate_sel).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let scan_status = row.select(&scanstatus_sel).next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            files.push(LunaMaterialFile {
                display_name,
                file_name,
                object_name,
                resource_id,
                material_id,
                file_type,
                end_date,
                scan_status,
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
            item_type: "material".to_string(),
            files,
        });
    }
    items
}

fn parse_reports(doc: &Html) -> Vec<LunaContentItem> {
    let mut items = Vec::new();
    let row_sel = match Selector::parse("#report .contents-result-list") {
        Ok(s) => s,
        Err(_) => return items,
    };
    let name_sel = Selector::parse(".course-view-report-name.link-txt").expect("valid selector");
    let start_sel = Selector::parse(".course-view-report-time-start").expect("valid selector");
    let end_sel = Selector::parse(".course-view-report-time-end").expect("valid selector");
    let status_sel = Selector::parse(".course-view-report-status").expect("valid selector");

    for row in doc.select(&row_sel) {
        let a = match row.select(&name_sel).next() {
            Some(a) => a,
            None => continue,
        };
        let title = a.text().collect::<String>().trim().to_string();
        let url = a.value().attr("href").unwrap_or_default().to_string();

        let start = row.select(&start_sel).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        let end = row.select(&end_sel).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        let period = if !start.is_empty() && !end.is_empty() {
            format!("{} ～ {}", start, end)
        } else {
            String::new()
        };

        let status = row.select(&status_sel).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        items.push(LunaContentItem {
            title,
            url,
            period,
            status,
            item_type: "report".to_string(),
            files: Vec::new(),
        });
    }
    items
}

fn parse_examinations(doc: &Html) -> Vec<LunaContentItem> {
    let mut items = Vec::new();
    let row_sel = match Selector::parse("#examination .contents-result-list") {
        Ok(s) => s,
        Err(_) => return items,
    };
    let name_sel = Selector::parse(".course-view-examination-name.link-txt").expect("valid selector");
    let name_fallback_sel = Selector::parse(".course-view-examination-name").expect("valid selector");
    let link_sel = Selector::parse("a.link-txt").expect("valid selector");
    let period_sel = Selector::parse(".course-view-examination-period.sp-contents-hidden").expect("valid selector");
    let status_sel = Selector::parse(".course-view-examination-answer-status").expect("valid selector");

    for row in doc.select(&row_sel) {
        // Try primary selector, then fallback
        let (title, mut url) = if let Some(a) = row.select(&name_sel).next() {
            let t = a.text().collect::<String>().trim().to_string();
            let u = a.value().attr("href").unwrap_or_default().to_string();
            (t, u)
        } else if let Some(a) = row.select(&link_sel).next() {
            let t = a.text().collect::<String>().trim().to_string();
            let u = a.value().attr("href").unwrap_or_default().to_string();
            (t, u)
        } else if let Some(el) = row.select(&name_fallback_sel).next() {
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

        let period = row.select(&period_sel).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let status = row.select(&status_sel).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        items.push(LunaContentItem {
            title,
            url,
            period,
            status,
            item_type: "examination".to_string(),
            files: Vec::new(),
        });
    }
    items
}

fn parse_discussions(doc: &Html) -> Vec<LunaContentItem> {
    let mut items = Vec::new();
    let row_sel = match Selector::parse("#discussion .contents-result-list") {
        Ok(s) => s,
        Err(_) => return items,
    };
    let name_sel = Selector::parse(".course-view-forum-title.link-txt").expect("valid selector");
    let name_fallback_sel = Selector::parse(".course-view-forum-title").expect("valid selector");
    let link_sel = Selector::parse("a.link-txt").expect("valid selector");
    let period_sel = Selector::parse(".course-view-forum-period.sp-contents-hidden").expect("valid selector");
    let status_sel = Selector::parse(".course-view-forum-postzyoukyou").expect("valid selector");

    for row in doc.select(&row_sel) {
        let (title, mut url) = if let Some(a) = row.select(&name_sel).next() {
            let t = a.text().collect::<String>().trim().to_string();
            let u = a.value().attr("href").unwrap_or_default().to_string();
            (t, u)
        } else if let Some(a) = row.select(&link_sel).next() {
            let t = a.text().collect::<String>().trim().to_string();
            let u = a.value().attr("href").unwrap_or_default().to_string();
            (t, u)
        } else if let Some(el) = row.select(&name_fallback_sel).next() {
            let t = el.text().collect::<String>().trim().to_string();
            (t, String::new())
        } else {
            continue;
        };

        if title.is_empty() { continue; }

        if url.is_empty() || url == "#" || url == "javascript:void(0)" {
            url = extract_url_from_row(&row);
        }

        let period = row.select(&period_sel).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let status = row.select(&status_sel).next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        items.push(LunaContentItem {
            title,
            url,
            period,
            status,
            item_type: "discussion".to_string(),
            files: Vec::new(),
        });
    }
    items
}

/// Extract a URL from onclick attributes or <a> tags within a row element
fn extract_url_from_row(row: &scraper::ElementRef) -> String {
    // Check all <a> tags for href
    if let Ok(a_sel) = Selector::parse("a[href]") {
        for a in row.select(&a_sel) {
            let href = a.value().attr("href").unwrap_or_default();
            if !href.is_empty() && href != "#" && !href.starts_with("javascript:") {
                return href.to_string();
            }
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
    let option_sel = Selector::parse("option[selected]").expect("valid selector");
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
    let option_sel = Selector::parse("option").expect("valid selector");
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
        let (materials, reports, examinations, discussions) = parse_luna_contents_page(&html);
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
}
