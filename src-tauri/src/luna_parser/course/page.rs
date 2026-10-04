use super::super::{
    try_selectors_text, Html, SEL_ATT_ACTION_A, SEL_ATT_DATE, SEL_ATT_LIST, SEL_ATT_STATUS,
    SEL_ATT_TITLE, SEL_GRADE_LINK, SEL_INFO_END, SEL_INFO_NAME_A, SEL_INFO_PRIORITY,
    SEL_INFO_RESULT, SEL_INFO_START, SEL_ONLINE_LINK, SEL_READMORE_DIV, SEL_READMORE_SPAN,
    SEL_SIDE_MENU, SEL_SPAN, SEL_SYLLABUS_LINK,
};
use super::model::*;

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
