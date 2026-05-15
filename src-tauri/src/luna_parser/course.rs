use super::*;

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
    pub item_type: String, // material, report, examination, discussion
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub files: Vec<LunaMaterialFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaMaterialFile {
    pub display_name: String, // link text (e.g. "2026日本語Ⅰ金_配布用シラバス")
    pub file_name: String,    // actual filename (e.g. "2026日本語Ⅰ金_配布用シラバス.pdf")
    pub object_name: String,  // storage path (e.g. "2026/ee/3c/1b/...")
    pub resource_id: String,  // resource ID
    pub material_id: String,  // dlMaterialId
    pub file_type: String,    // "0" = file, else HTML
    pub end_date: String,     // open end date (e.g. "2026-07-04 00:00:00.0")
    pub scan_status: String,  // virus scan status ("1" = clean)
    #[serde(default)]
    pub link_type: String, // "file", "zoom", "panopto", "video", "cloud", "google", "teams", "web"
    /// Direct external URL for pure link-type materials (Zoom, YouTube, etc.).
    /// When set, frontend should open this directly instead of using the tempfile flow.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub external_url: String,
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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub form_action: String,
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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub answer_name: String,
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
    let course_name = try_selectors_text(
        &doc,
        &[
            ".class-title-txt.course-view-header-txt",
            ".course-title-txt",
            "title",
        ],
    );

    // Semester from subblock
    let semester = try_selectors_text(&doc, &[".subblock_form"]);

    // Teachers from .contents-detail-readmore-txt
    let teachers = {
        let spans: Vec<String> = doc
            .select(&SEL_READMORE_SPAN)
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
    let syllabus_url = doc
        .select(&SEL_SYLLABUS_LINK)
        .next()
        .and_then(|el| el.value().attr("href").map(|s| s.to_string()))
        .unwrap_or_default();

    // Grade link
    let grade_url = doc
        .select(&SEL_GRADE_LINK)
        .next()
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
                    let end = onclick.find(')')?;
                    Some(onclick[start..end].trim().to_string())
                })
                .unwrap_or_default();
            let is_new = row.select(&SEL_INFO_PRIORITY).next().is_some();
            let start_date = row
                .select(&SEL_INFO_START)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let end_date = row
                .select(&SEL_INFO_END)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            if !title.is_empty() {
                announcements.push(LunaCourseAnnouncement {
                    title,
                    info_id,
                    start_date,
                    end_date,
                    is_new,
                });
            }
        }
    }

    // Parse online tools (Zoom, Panopto, etc.)
    let mut online_tools = Vec::new();
    for a in doc.select(&SEL_ONLINE_LINK) {
        let href = a.value().attr("href").unwrap_or_default().to_string();
        if href.is_empty() {
            continue;
        }
        let (name, icon) = if href.contains("zoom") {
            ("Zoom".to_string(), "video".to_string())
        } else if href.contains("panopto") {
            ("Panopto".to_string(), "play.rectangle".to_string())
        } else {
            ("オンラインツール".to_string(), "link".to_string())
        };
        online_tools.push(LunaOnlineTool {
            name,
            url: href,
            icon,
        });
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
        let title = row
            .select(&SEL_ATT_TITLE)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        let date = row
            .select(&SEL_ATT_DATE)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let mut status = row
            .select(&SEL_ATT_STATUS)
            .next()
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

            if !data1.is_empty() {
                idnumber = data1;
            }
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
            let spans: Vec<String> = div
                .select(&SEL_SPAN)
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
pub fn parse_luna_contents_page(html: &str) -> ContentsPageResult {
    let doc = Html::parse_document(html);
    let materials = parse_materials(&doc);
    let reports = parse_reports(&doc);
    let examinations = parse_content_list(
        &doc,
        &SEL_EXAM_LIST,
        &SEL_EXAM_NAME,
        &SEL_EXAM_NAME_FB,
        &SEL_EXAM_PERIOD,
        &SEL_EXAM_STATUS,
        "examination",
    );
    let discussions = parse_content_list(
        &doc,
        &SEL_DISC_LIST,
        &SEL_DISC_NAME,
        &SEL_DISC_NAME_FB,
        &SEL_DISC_PERIOD,
        &SEL_DISC_STATUS,
        "discussion",
    );
    let surveys = parse_content_list(
        &doc,
        &SEL_SURVEY_LIST,
        &SEL_SURV_NAME,
        &SEL_SURV_NAME_FB,
        &SEL_SURV_PERIOD,
        &SEL_SURV_STATUS,
        "survey",
    );
    (materials, reports, examinations, discussions, surveys)
}

/// Extract plain text from a Quill Delta JSON embedded in a JS script.
/// The script contains: `_QuillUtil.xxx.setJsonData("{...}", ...)`
#[cfg(test)]
pub(super) fn extract_quill_delta_text(script: &str) -> Option<String> {
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
    if i >= bytes.len() {
        return None;
    }
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
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

pub(super) fn extract_quill_delta_html(script: &str) -> Option<String> {
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
    if i >= bytes.len() {
        return None;
    }
    let escaped = &rest[..i];
    extract_quill_rich_html(escaped)
}

fn parse_materials(doc: &Html) -> Vec<LunaContentItem> {
    let mut items = Vec::new();

    // Each materialList div is a folder with materials
    for folder in doc.select(&SEL_MATERIAL_LIST) {
        let title = folder
            .select(&SEL_MAT_TITLE)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        if title.is_empty() {
            continue;
        }

        let period = folder
            .select(&SEL_INPUT_SPAN)
            .map(|e| e.text().collect::<String>().trim().to_string())
            .find(|s| s.contains('～'))
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
            let display_name = row
                .select(&SEL_MAT_FILE_NAME)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            if display_name.is_empty() {
                continue;
            }

            let file_name = row
                .select(&SEL_FILENAME)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let object_name = row
                .select(&SEL_OBJECT_NAME)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let resource_id = row
                .select(&SEL_RESOURCE_ID)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let file_type = row
                .select(&SEL_FILETYPE)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let material_id = row
                .select(&SEL_DL_MAT_ID)
                .next()
                .and_then(|e| e.value().attr("value"))
                .unwrap_or_default()
                .to_string();
            let end_date = row
                .select(&SEL_OPEN_END_DATE)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let scan_status = row
                .select(&SEL_SCAN_STATUS)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            let mut link_type = if file_type == "0" {
                classify_link(&file_name, &display_name)
            } else {
                let cl = classify_link(&display_name, &file_name);
                if cl == "file" {
                    "web".to_string()
                } else {
                    cl
                }
            };

            // For pure external-link materials (file_type != "0" and no backing file),
            // Luna embeds the URL directly somewhere in the row — scan for it so the
            // frontend can open it without the tempfile flow.
            let mut external_url = String::new();
            if file_type != "0" && file_name.is_empty() {
                for a in row.select(&SEL_A_HREF) {
                    if let Some(href) = a.value().attr("href") {
                        let h = href.trim();
                        if h.starts_with("http") && !h.contains("luna.kwansei.ac.jp") {
                            external_url = h.to_string();
                            break;
                        }
                    }
                }
                if external_url.is_empty() {
                    let row_html = row.html();
                    if let Some(idx) = row_html.find("http") {
                        let tail = &row_html[idx..];
                        let end = tail
                            .find(|c: char| c == '"' || c == '\'' || c == '<' || c == ' ')
                            .unwrap_or(tail.len());
                        let candidate = &tail[..end];
                        if candidate.len() > 10 && !candidate.contains("luna.kwansei.ac.jp") {
                            external_url = candidate.to_string();
                        }
                    }
                }
                if !external_url.is_empty() {
                    let cl = classify_link(&external_url, &display_name);
                    if cl != "file" {
                        link_type = cl;
                    } else if link_type == "file" {
                        link_type = "web".to_string();
                    }
                    log::info!(
                        "[material] extracted external URL: resource_id='{}', url='{}'",
                        resource_id,
                        crate::client::safe_truncate(&external_url, 200)
                    );
                } else {
                    let row_html = row.html();
                    log::warn!(
                        "[material] link-type row has no fileName and no extractable URL: resource_id='{}', display='{}'. Raw HTML:\n{}",
                        resource_id,
                        display_name,
                        crate::client::safe_truncate(&row_html, 3000)
                    );
                }
            }

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
                external_url,
            });
        }

        let status = if files.is_empty() {
            String::new()
        } else {
            files
                .iter()
                .map(|f| f.display_name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
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

        let start = row
            .select(&SEL_RPT_START)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        let end = row
            .select(&SEL_RPT_END)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        let period = if !start.is_empty() && !end.is_empty() {
            format!("{} ～ {}", start, end)
        } else {
            String::new()
        };

        let status = row
            .select(&SEL_RPT_STATUS)
            .next()
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

fn parse_content_list(
    doc: &Html,
    list_sel: &Selector,
    name_sel: &Selector,
    name_fb_sel: &Selector,
    period_sel: &Selector,
    status_sel: &Selector,
    item_type: &str,
) -> Vec<LunaContentItem> {
    let mut items = Vec::new();

    for row in doc.select(list_sel) {
        let (title, mut url) = if let Some(a) = row.select(name_sel).next() {
            let t = a.text().collect::<String>().trim().to_string();
            let u = a.value().attr("href").unwrap_or_default().to_string();
            (t, u)
        } else if let Some(a) = row.select(&SEL_LINK_TXT).next() {
            let t = a.text().collect::<String>().trim().to_string();
            let u = a.value().attr("href").unwrap_or_default().to_string();
            (t, u)
        } else if let Some(el) = row.select(name_fb_sel).next() {
            let t = el.text().collect::<String>().trim().to_string();
            (t, String::new())
        } else {
            continue;
        };

        if title.is_empty() {
            continue;
        }

        if url.is_empty() || url == "#" || url == "javascript:void(0)" {
            url = extract_url_from_row(&row);
        }

        let period = row
            .select(period_sel)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let status = row
            .select(status_sel)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        items.push(LunaContentItem {
            title,
            url,
            period,
            status,
            description: String::new(),
            item_type: item_type.to_string(),
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
    for pattern in &[
        "location.href='",
        "location.href=\"",
        "window.open('",
        "window.open(\"",
    ] {
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
