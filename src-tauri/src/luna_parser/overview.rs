use super::*;

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
    pub period: u32, // 1-7
    pub day: u32,    // 1=月 ... 6=土
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
        let period_text = row
            .select(&SEL_PERIOD_COL)
            .next()
            .map(|e| e.text().collect::<String>())
            .unwrap_or_default();
        let period = parse_japanese_number(&period_text);
        if period == 0 {
            continue;
        }

        // Each cell corresponds to a day (1=月 through 6=土)
        for (i, cell) in row.select(&SEL_TABLE_CELL).enumerate() {
            let day = (i + 1) as u32;
            if let Some(btn) = cell.select(&SEL_COURSE_BTN).next() {
                let idnumber = btn.value().attr("id").unwrap_or_default().to_string();
                let name = btn.text().collect::<String>().trim().to_string();
                let teacher = cell
                    .select(&SEL_CELL_DETAIL)
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

    LunaTimetable {
        year,
        term,
        year_label,
        term_label,
        year_options,
        term_options,
        courses,
        communities,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LunaTimetableSwitch {
    pub action: String,
    pub method: String,
    pub fields: Vec<(String, String)>,
}

/// Build a form request that changes the timetable page to the target year/term.
/// Returns None when the page has no usable form or the target is not an option.
pub fn build_timetable_switch(html: &str, year: &str, term: &str) -> Option<LunaTimetableSwitch> {
    if year.is_empty() || term.is_empty() {
        return None;
    }
    let doc = Html::parse_document(html);
    let form_sel = scraper::Selector::parse("form").ok()?;
    let nendo_sel = scraper::Selector::parse("select#nendo, select[name='nendo']").ok()?;
    let term_sel = scraper::Selector::parse("select#kikanCd, select[name='kikanCd']").ok()?;

    if let Some(form) = doc.select(&form_sel).find(|form| {
        form.select(&nendo_sel).next().is_some() && form.select(&term_sel).next().is_some()
    }) {
        let nendo = form.select(&nendo_sel).next()?;
        let term_el = form.select(&term_sel).next()?;
        return switch_from_selects(form, nendo, term_el, year, term);
    }

    let nendo = doc.select(&nendo_sel).next()?;
    let term_el = doc.select(&term_sel).next()?;
    switch_from_selects(doc.root_element(), nendo, term_el, year, term)
}

fn switch_from_selects(
    scope: scraper::ElementRef<'_>,
    nendo: scraper::ElementRef<'_>,
    term_el: scraper::ElementRef<'_>,
    year: &str,
    term: &str,
) -> Option<LunaTimetableSwitch> {
    if !select_has_value(nendo, year) || !select_has_value(term_el, term) {
        return None;
    }
    let nendo_name = nendo.value().attr("name").unwrap_or("nendo").to_string();
    let term_name = term_el
        .value()
        .attr("name")
        .unwrap_or("kikanCd")
        .to_string();
    let mut fields = Vec::new();
    let input_sel = scraper::Selector::parse("input[name]").ok()?;
    for input in scope.select(&input_sel) {
        let name = input.value().attr("name").unwrap_or_default();
        if name.is_empty() || name == nendo_name || name == term_name {
            continue;
        }
        let typ = input
            .value()
            .attr("type")
            .unwrap_or("text")
            .to_ascii_lowercase();
        if matches!(typ.as_str(), "submit" | "button" | "image" | "file") {
            continue;
        }
        if matches!(typ.as_str(), "checkbox" | "radio") && input.value().attr("checked").is_none() {
            continue;
        }
        fields.push((
            name.to_string(),
            input.value().attr("value").unwrap_or_default().to_string(),
        ));
    }

    let select_sel = scraper::Selector::parse("select[name]").ok()?;
    for select in scope.select(&select_sel) {
        let name = select.value().attr("name").unwrap_or_default();
        if name.is_empty() || name == nendo_name || name == term_name {
            continue;
        }
        if let Some(value) = selected_option_value(select) {
            fields.push((name.to_string(), value));
        }
    }
    fields.push((nendo_name, year.to_string()));
    fields.push((term_name, term.to_string()));

    let action = scope
        .value()
        .attr("action")
        .unwrap_or("/lms/timetable")
        .trim()
        .to_string();
    let action = if action.is_empty() || action.starts_with("javascript:") || action == "#" {
        "/lms/timetable".to_string()
    } else {
        action
    };
    let method = scope
        .value()
        .attr("method")
        .unwrap_or("post")
        .trim()
        .to_ascii_lowercase();
    let method = if method.is_empty() {
        "post".to_string()
    } else {
        method
    };
    Some(LunaTimetableSwitch {
        action,
        method,
        fields,
    })
}

fn select_has_value(select: scraper::ElementRef<'_>, value: &str) -> bool {
    let Ok(option_sel) = scraper::Selector::parse("option") else {
        return false;
    };
    select
        .select(&option_sel)
        .any(|opt| opt.value().attr("value") == Some(value))
}

fn selected_option_value(select: scraper::ElementRef<'_>) -> Option<String> {
    let selected_sel = scraper::Selector::parse("option[selected]").ok()?;
    let option_sel = scraper::Selector::parse("option").ok()?;
    select
        .select(&selected_sel)
        .next()
        .or_else(|| select.select(&option_sel).next())
        .and_then(|opt| opt.value().attr("value"))
        .map(|value| value.to_string())
}

// ──────────────────────────────────────────────
// TODO list
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaTodoItem {
    pub course_name: String,
    pub content_type: String, // 課題, テスト, 掲示板
    pub content_name: String,
    pub url: String,
    pub deadline: String,
    pub status: String, // 未提出, 提出済み, etc.
    pub feedback: String,
}

pub fn parse_luna_todo(html: &str) -> Vec<LunaTodoItem> {
    let doc = Html::parse_document(html);
    let mut items = Vec::new();

    for item in doc.select(&SEL_TODO_LIST) {
        let course_name = item
            .select(&SEL_TODO_COURSE)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let content_type = item
            .select(&SEL_TODO_TYPE)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let (content_name, url) = item
            .select(&SEL_TODO_NAME)
            .next()
            .map(|e| {
                (
                    e.text().collect::<String>().trim().to_string(),
                    e.value().attr("href").unwrap_or_default().to_string(),
                )
            })
            .unwrap_or_default();

        let deadline = item
            .select(&SEL_TODO_DEADLINE)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let status = item
            .select(&SEL_TODO_STATUS)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let feedback = item
            .select(&SEL_TODO_FEEDBACK)
            .next()
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
    pub module: String, // 掲示板スレッド, お知らせ, 課題, etc.
    pub content: String,
    pub url: String,
    pub idnumber: String,
}

pub fn parse_luna_notifications(html: &str) -> Vec<LunaNotification> {
    let doc = Html::parse_document(html);
    let mut items = Vec::new();

    for item in doc.select(&SEL_NOTIF_LIST) {
        let date = item
            .select(&SEL_NOTIF_DATE)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let course_info = item
            .select(&SEL_NOTIF_COURSE)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let module = item
            .select(&SEL_NOTIF_MODULE)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let content = item
            .select(&SEL_NOTIF_CONTENT)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let url = item
            .select(&SEL_NOTIF_URL)
            .next()
            .and_then(|e| e.value().attr("value"))
            .unwrap_or_default()
            .to_string();

        let idnumber = item
            .select(&SEL_INPUT_IDNUMBER)
            .next()
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaDiscussionPost {
    pub title: String,
    pub author: String,
    pub date: String,
    pub content: String,
    pub status: String,
    pub thread_id: String,
    #[serde(default)]
    pub attachments: Vec<LunaAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaDiscussionThread {
    pub title: String,
    pub course_name: String,
    pub description: String,
    pub meta: Vec<(String, String)>,
    pub posts: Vec<LunaDiscussionPost>,
}

fn forum_download_form_info(doc: &Html) -> Option<(String, Vec<(String, String)>)> {
    let form = doc.select(&SEL_FORUMS_FORM).next()?;
    let action = form.value().attr("action").unwrap_or_default().to_string();
    if action.is_empty() {
        return None;
    }

    let mut params = Vec::new();
    for input in form.select(&SEL_HIDDEN_INPUT) {
        let name = input.value().attr("name").unwrap_or_default();
        let value = input.value().attr("value").unwrap_or_default();
        if !value.is_empty() && name != "fileId" && name != "fileName" {
            params.push((name.to_string(), value.to_string()));
        }
    }

    Some((action, params))
}

fn hidden_text(container: &scraper::ElementRef<'_>, selector: &Selector) -> String {
    container
        .select(selector)
        .next()
        .map(|e| e.text().collect::<String>().trim().to_string())
        .unwrap_or_default()
}

fn parse_discussion_file_attachments(
    block: &scraper::ElementRef<'_>,
    form_info: Option<&(String, Vec<(String, String)>)>,
    fallback_post_id: &str,
) -> Vec<LunaAttachment> {
    let Some((action, fixed_params)) = form_info else {
        return Vec::new();
    };

    let mut attachments = Vec::new();
    for file_el in block.select(&SEL_DISCUSS_MESS_FILE) {
        let name = hidden_text(&file_el, &SEL_DOWNLOAD_FILE);
        let file_name = if name.is_empty() {
            hidden_text(&file_el, &SEL_FILENAME)
        } else {
            name
        };
        let object_name = hidden_text(&file_el, &SEL_OBJECT_NAME);
        if file_name.is_empty() || object_name.is_empty() {
            continue;
        }

        let post_id = {
            let value = hidden_text(&file_el, &SEL_POST_ID);
            if value.is_empty() {
                fallback_post_id.to_string()
            } else {
                value
            }
        };
        let scan_status = hidden_text(&file_el, &SEL_SCAN_STATUS);

        let mut params = fixed_params.clone();
        params.push(("fileId".to_string(), object_name.clone()));
        params.push(("fileName".to_string(), file_name.clone()));
        if !post_id.is_empty() {
            params.push(("postId".to_string(), post_id));
        }
        if !scan_status.is_empty() {
            params.push(("scanStatus".to_string(), scan_status));
        }

        attachments.push(LunaAttachment {
            name: file_name,
            url: String::new(),
            link_type: "file".to_string(),
            object_name,
            download_action: action.clone(),
            download_params: params,
        });
    }

    attachments
}

/// Parse a Luna discussion themetop page (/lms/course/forums/themetop)
/// Structure:
///   - Theme info: title, description (Quill: themeContents), period
///   - Thread list: each thread has title, description (Quill: threadContentsN), author, date
pub fn parse_luna_discussion_thread(html: &str) -> LunaDiscussionThread {
    let doc = Html::parse_document(html);
    let thread_quill_payloads: Vec<String> = super::detail::extract_named_quill_payloads(html)
        .into_iter()
        .filter_map(|(name, payload)| {
            let suffix = name.strip_prefix("threadContents")?;
            if suffix.is_empty() || !suffix.chars().all(|ch| ch.is_ascii_digit()) {
                return None;
            }
            Some(payload)
        })
        .collect();

    let course_name = try_selectors_text(
        &doc,
        &[
            ".course-title-txt",
            ".class-title-txt.course-view-header-txt",
        ],
    );

    let title = try_selectors_text(&doc, &[".contents-title-txt", ".block-title-txt"]);

    // Extract theme description from themeContents Quill
    let description = extract_named_quill_text(html, "themeContents").unwrap_or_default();

    // Extract meta info from the top section (テーマタイトル, 投稿期間, etc.)
    let mut meta = Vec::new();
    {
        for row in doc.select(&SEL_BLOCK_DETAIL) {
            let label = row
                .select(&SEL_HEADER_COMBO)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            if label == "内容" || label == "添付ファイル" {
                continue;
            }
            let value = row
                .select(&SEL_INPUT_AREA)
                .next()
                .map(|e| {
                    e.text()
                        .map(|t| t.trim())
                        .filter(|t| {
                            !t.is_empty()
                                && !t.starts_with("/*")
                                && !t.starts_with("$(")
                                && !t.starts_with("var ")
                                && !t.contains("setJsonData")
                                && !t.contains("function")
                                && !t.contains("QuillUtil")
                        })
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
        // LUNA renders the thread list twice for responsive layouts (a desktop
        // copy and an .sp-* mobile copy), and the broadened row selector matches
        // both. Track which threads we've already emitted so each appears once.
        // `kept` indexes into the (un-duplicated) Quill payloads, so it must only
        // advance for rows we actually push — otherwise skipped duplicate rows
        // would shift every later thread's body by one.
        let mut seen_keys: Vec<String> = Vec::new();
        let mut kept = 0usize;
        for row in doc.select(&SEL_THEME_TOP) {
            let thread_title = row
                .select(&SEL_THREAD_TITLE)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            // Extract threadId from onclick="viewthread(50406);"
            let thread_id = row
                .select(&SEL_THREAD_TITLE)
                .next()
                .and_then(|e| e.value().attr("onclick"))
                .and_then(|onclick| {
                    let start = onclick.find('(')? + 1;
                    let end = onclick.find(')')?;
                    Some(onclick[start..end].trim().to_string())
                })
                .unwrap_or_default();
            let author = row
                .select(&SEL_THREAD_AUTHOR)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let date = row
                .select(&SEL_THREAD_DATE)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let status = row
                .select(&SEL_THREAD_STATUS)
                .next()
                .map(|e| {
                    e.text()
                        .map(|t| t.trim())
                        .filter(|t| !t.is_empty())
                        .collect::<Vec<_>>()
                        .join(" / ")
                })
                .unwrap_or_default();

            // Key on the unique threadId (viewthread id); fall back to the
            // visible fields when a row carries no onclick.
            let dedup_key = if !thread_id.is_empty() {
                thread_id.clone()
            } else {
                format!("{}|{}|{}", thread_title, author, date)
            };
            if !dedup_key.is_empty() && seen_keys.iter().any(|k| k == &dedup_key) {
                continue;
            }

            // Some themetop variants no longer keep stable threadContents{idx}
            // variable names. Prefer the row-local Quill payload when present,
            // then the positional payload list (document order, correct for both
            // 0-based and 1-based threadContents{N} naming), then the legacy
            // index-named lookups.
            let row_html = row.html();
            let quill_name = format!("threadContents{}", kept);
            let content = super::detail::extract_first_quill_rich_html(&row_html)
                .or_else(|| thread_quill_payloads.get(kept).cloned())
                .or_else(|| extract_named_quill_text(html, &quill_name))
                .or_else(|| extract_named_quill_text(html, &format!("threadContents{}", kept + 1)))
                .unwrap_or_default();

            if !thread_title.is_empty() || !content.is_empty() || !author.is_empty() {
                if !dedup_key.is_empty() {
                    seen_keys.push(dedup_key);
                }
                kept += 1;
                posts.push(LunaDiscussionPost {
                    title: thread_title,
                    author,
                    date,
                    content,
                    status,
                    thread_id,
                    attachments: Vec::new(),
                });
            }
        }
    }

    if posts.is_empty() {
        let titles: Vec<_> = doc.select(&SEL_THREAD_TITLE).collect();
        let authors: Vec<String> = doc
            .select(&SEL_THREAD_AUTHOR)
            .map(|e| e.text().collect::<String>().trim().to_string())
            .collect();
        let dates: Vec<String> = doc
            .select(&SEL_THREAD_DATE)
            .map(|e| e.text().collect::<String>().trim().to_string())
            .collect();
        let statuses: Vec<String> = doc
            .select(&SEL_THREAD_STATUS)
            .map(|e| {
                e.text()
                    .map(|t| t.trim())
                    .filter(|t| !t.is_empty())
                    .collect::<Vec<_>>()
                    .join(" / ")
            })
            .collect();

        for (idx, title_el) in titles.iter().enumerate() {
            let thread_title = title_el.text().collect::<String>().trim().to_string();
            let thread_id = title_el
                .value()
                .attr("onclick")
                .and_then(|onclick| {
                    let start = onclick.find('(')? + 1;
                    let end = onclick.find(')')?;
                    Some(onclick[start..end].trim().to_string())
                })
                .unwrap_or_default();
            let content = thread_quill_payloads
                .get(idx)
                .cloned()
                .or_else(|| extract_named_quill_text(html, &format!("threadContents{}", idx)))
                .or_else(|| extract_named_quill_text(html, &format!("threadContents{}", idx + 1)))
                .unwrap_or_default();
            let author = authors.get(idx).cloned().unwrap_or_default();
            let date = dates.get(idx).cloned().unwrap_or_default();
            let status = statuses.get(idx).cloned().unwrap_or_default();

            if !thread_title.is_empty() || !content.is_empty() || !author.is_empty() {
                posts.push(LunaDiscussionPost {
                    title: thread_title,
                    author,
                    date,
                    content,
                    status,
                    thread_id,
                    attachments: Vec::new(),
                });
            }
        }
    }

    // Thread page fallback: meta rows (テーマ, スレッド, 登録者, etc.)
    if posts.is_empty() {
        // This is a thread detail page (/lms/course/forums/thread)
        // All content is in meta rows, threadPostList is loaded via AJAX
        // Extract thread description from threadContents Quill if available
        if let Some(header_content) = extract_named_quill_text(html, "threadContents") {
            if !header_content.is_empty() {
                posts.push(LunaDiscussionPost {
                    title: String::new(),
                    author: String::new(),
                    date: String::new(),
                    content: header_content,
                    status: String::new(),
                    thread_id: String::new(),
                    attachments: Vec::new(),
                });
            }
        }
    }

    LunaDiscussionThread {
        title,
        course_name,
        description,
        meta,
        posts,
    }
}

/// Parse a Luna thread detail page (/lms/course/forums/thread)
/// Extracts: テーマ, スレッド title, 登録者, 説明 (threadContents Quill), 更新日時
pub fn parse_luna_thread_detail(html: &str) -> LunaDiscussionThread {
    let doc = Html::parse_document(html);

    let course_name = try_selectors_text(&doc, &[".course-title-txt"]);

    let title = try_selectors_text(&doc, &[".contents-title-txt"]);

    // Extract meta rows: テーマ, スレッド, 登録者, 学生番号, 更新日時
    let mut meta = Vec::new();
    let mut thread_title = String::new();
    {
        for row in doc.select(&SEL_BLOCK_DETAIL) {
            let label = row
                .select(&SEL_HEADER_BOLD)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            if label == "説明"
                || label == "投稿内容"
                || label == "添付ファイル"
                || label == "返信先投稿内容"
            {
                continue;
            }
            let value = row
                .select(&SEL_INPUT_AREA)
                .next()
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

    // Description from threadContents Quill
    let description = extract_named_quill_text(html, "threadContents").unwrap_or_default();

    // Parse posts from #threadPostList (embedded in the page)
    let mut posts = Vec::new();
    let download_form_info = forum_download_form_info(&doc);
    for block in doc.select(&SEL_THREAD_POST_BLOCK) {
        let content = block
            .select(&SEL_POST_CONTENTS_TEXT)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        let date = block
            .select(&SEL_POST_DATE)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        let raw_user = block
            .select(&SEL_POST_USER)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        // Format: "氏名:名前" → extract the name after ":"
        let author = raw_user
            .split(':')
            .nth(1)
            .unwrap_or(&raw_user)
            .trim()
            .to_string();
        let post_id = block
            .select(&SEL_POST_ID)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        let attachments =
            parse_discussion_file_attachments(&block, download_form_info.as_ref(), &post_id);
        // Check if teacher post (has board-discussion-teacher-color-left)
        let is_teacher = block
            .select(&SEL_MSG_BLOCK)
            .next()
            .map(|e| e.html().contains("board-discussion-teacher-color"))
            .unwrap_or(false);
        let is_self = block
            .select(&SEL_MSG_BLOCK)
            .next()
            .map(|e| e.value().classes().any(|c| c == "discussion-self"))
            .unwrap_or(false);
        let mut status = String::new();
        if is_teacher {
            status.push_str("teacher");
        }
        if is_self {
            if !status.is_empty() {
                status.push(',');
            }
            status.push_str("self");
        }

        if !content.is_empty() || !author.is_empty() || !attachments.is_empty() {
            posts.push(LunaDiscussionPost {
                title: String::new(),
                author,
                date,
                content,
                status,
                thread_id: post_id,
                attachments,
            });
        }
    }

    let display_title = if !thread_title.is_empty() {
        thread_title
    } else {
        title
    };

    LunaDiscussionThread {
        title: display_title,
        course_name,
        description,
        meta,
        posts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_selected_term_and_keeps_csrf() {
        let html = r#"
            <form id="timetableForm" action="/lms/timetable" method="post">
              <input type="hidden" name="_csrf" value="csrf-token">
              <select id="nendo" name="nendo">
                <option value="2025">2025</option>
                <option value="2026" selected>2026</option>
              </select>
              <select id="kikanCd" name="kikanCd">
                <option value="02" selected>春学期</option>
                <option value="03">秋学期</option>
              </select>
            </form>
        "#;
        let request = build_timetable_switch(html, "2026", "03").unwrap();
        assert_eq!(request.action, "/lms/timetable");
        assert_eq!(request.method, "post");
        assert!(request
            .fields
            .iter()
            .any(|(k, v)| k == "_csrf" && v == "csrf-token"));
        assert!(request
            .fields
            .iter()
            .any(|(k, v)| k == "nendo" && v == "2026"));
        assert!(request
            .fields
            .iter()
            .any(|(k, v)| k == "kikanCd" && v == "03"));
    }

    #[test]
    fn refuses_a_term_that_is_not_an_option() {
        let html = r#"
            <form action="/lms/timetable" method="get">
              <select id="nendo" name="nendo"><option value="2026" selected>2026</option></select>
              <select id="kikanCd" name="kikanCd"><option value="02" selected>春学期</option></select>
            </form>
        "#;
        assert!(build_timetable_switch(html, "2026", "03").is_none());
    }
}
