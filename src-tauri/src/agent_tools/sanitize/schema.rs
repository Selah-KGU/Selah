use serde_json::{json, Value};

use super::super::catalog::ArgSchema;
use super::super::LIST_CAP;
use super::browser::{
    sanitize_browser_click_args, sanitize_browser_fill_args, sanitize_browser_mouse_click_args,
    sanitize_browser_mouse_drag_args, sanitize_browser_press_args, sanitize_browser_scroll_args,
    sanitize_browser_select_args, sanitize_browser_wait_args,
};
use super::calendar::{sanitize_calendar_event_args, sanitize_calendar_update_args};
use super::computer::{
    sanitize_computer_mouse_click_args, sanitize_computer_mouse_drag_args,
    sanitize_computer_screenshot_args, sanitize_computer_scroll_args,
};
use super::text::{
    sanitize_copilot_page_args, sanitize_course_code, sanitize_file_path_arg,
    sanitize_filename_arg, sanitize_text_arg, sanitize_text_blob_arg, sanitize_url_arg,
};

pub(in crate::agent_tools) fn sanitize_by_schema(schema: ArgSchema, args: &Value) -> Option<Value> {
    match schema {
        ArgSchema::Empty => Some(json!({})),
        ArgSchema::Int { key, max } => {
            let val = args
                .get(key)
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                .clamp(0, max);
            Some(json!({ key: val }))
        }
        ArgSchema::Text { key, max_len } => {
            let value = sanitize_text_arg(args, key, max_len).or_else(|| {
                if key == "query" {
                    sanitize_text_arg(args, "kgc_code", max_len)
                        .or_else(|| sanitize_text_arg(args, "course_code", max_len))
                        .or_else(|| sanitize_text_arg(args, "code", max_len))
                        .or_else(|| sanitize_text_arg(args, "idnumber", max_len))
                        .or_else(|| sanitize_text_arg(args, "luna_id", max_len))
                        .or_else(|| sanitize_text_arg(args, "course_name", max_len))
                        .or_else(|| sanitize_text_arg(args, "course", max_len))
                        .or_else(|| sanitize_text_arg(args, "keyword", max_len))
                } else {
                    None
                }
            });
            value.map(|v| json!({ key: v }))
        }
        ArgSchema::CourseCode { key } => sanitize_course_code(args, key)
            .or_else(|| sanitize_course_code(args, "course_code"))
            .or_else(|| sanitize_course_code(args, "code"))
            .or_else(|| sanitize_course_code(args, "query"))
            .map(|v| json!({ key: v })),
        ArgSchema::LimitKeyword => {
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(10)
                .min(LIST_CAP as u64);
            let keyword = sanitize_text_arg(args, "keyword", 80)
                .or_else(|| sanitize_text_arg(args, "course_name", 80))
                .or_else(|| sanitize_text_arg(args, "course", 80))
                .or_else(|| sanitize_text_arg(args, "query", 80));
            let mut out = json!({ "limit": limit });
            if let Some(keyword) = keyword {
                out["keyword"] = Value::String(keyword);
            }
            Some(out)
        }
        ArgSchema::MailMessageId => sanitize_text_arg(args, "message_id", 200)
            .or_else(|| sanitize_text_arg(args, "id", 200))
            .and_then(|message_id| {
                crate::mail::validate_message_id(&message_id).ok()?;
                Some(json!({ "message_id": message_id }))
            }),
        ArgSchema::FilePath => {
            if let Some(path) = sanitize_file_path_arg(args, "path") {
                Some(json!({ "path": path }))
            } else {
                let filename = sanitize_filename_arg(args, "filename", 240)
                    .or_else(|| sanitize_filename_arg(args, "file_name", 240))?;
                let mut out = serde_json::Map::new();
                out.insert("filename".to_string(), Value::String(filename));
                if let Some(course) = sanitize_text_arg(args, "course_name", 120)
                    .or_else(|| sanitize_text_arg(args, "course", 120))
                {
                    out.insert("course_name".to_string(), Value::String(course));
                }
                Some(Value::Object(out))
            }
        }
        ArgSchema::FileWrite => {
            let path = sanitize_file_path_arg(args, "path")?;
            let content = sanitize_text_blob_arg(args, "content", 100_000)?;
            Some(json!({ "path": path, "content": content }))
        }
        ArgSchema::TitleAttachment => {
            let title = sanitize_text_arg(args, "title", 120)
                .or_else(|| sanitize_text_arg(args, "activity_title", 120))
                .or_else(|| sanitize_text_arg(args, "activityTitle", 120))
                .or_else(|| sanitize_text_arg(args, "name", 120))?;
            let attachment_name = sanitize_text_arg(args, "attachment_name", 160)
                .or_else(|| sanitize_text_arg(args, "filename", 160))
                .or_else(|| sanitize_text_arg(args, "file_name", 160));
            let mut out = serde_json::Map::new();
            out.insert("title".to_string(), Value::String(title));
            if let Some(name) = attachment_name {
                out.insert("attachment_name".to_string(), Value::String(name));
            }
            Some(Value::Object(out))
        }
        ArgSchema::LunaActivityDetail => {
            let title = sanitize_text_arg(args, "title", 120)
                .or_else(|| sanitize_text_arg(args, "activity_title", 120))
                .or_else(|| sanitize_text_arg(args, "activityTitle", 120))
                .or_else(|| sanitize_text_arg(args, "name", 120));
            let activity_type = sanitize_text_arg(args, "activity_type", 80)
                .or_else(|| sanitize_text_arg(args, "type", 80));
            let luna_id = sanitize_text_arg(args, "luna_id", 80);
            // Require at least one meaningful field; reject fully-empty calls.
            if title.is_none() && activity_type.is_none() && luna_id.is_none() {
                return None;
            }
            let mut out = serde_json::Map::new();
            if let Some(t) = title {
                out.insert("title".to_string(), Value::String(t));
            }
            if let Some(atype) = activity_type {
                out.insert("activity_type".to_string(), Value::String(atype));
            }
            if let Some(id) = luna_id {
                out.insert("luna_id".to_string(), Value::String(id));
            }
            Some(Value::Object(out))
        }
        ArgSchema::DownloadLunaAttachment => {
            let title = sanitize_text_arg(args, "title", 120)
                .or_else(|| sanitize_text_arg(args, "activity_title", 120))
                .or_else(|| sanitize_text_arg(args, "activityTitle", 120))
                .or_else(|| sanitize_text_arg(args, "name", 120))?;
            let attachment_name = sanitize_text_arg(args, "attachment_name", 160)
                .or_else(|| sanitize_text_arg(args, "filename", 160))
                .or_else(|| sanitize_text_arg(args, "file_name", 160));
            let luna_id = sanitize_text_arg(args, "luna_id", 80);
            let mut out = serde_json::Map::new();
            out.insert("title".to_string(), Value::String(title));
            if let Some(name) = attachment_name {
                out.insert("attachment_name".to_string(), Value::String(name));
            }
            if let Some(id) = luna_id {
                out.insert("luna_id".to_string(), Value::String(id));
            }
            Some(Value::Object(out))
        }
        ArgSchema::DownloadCourseMaterial => {
            let filename = sanitize_text_arg(args, "filename", 160)
                .or_else(|| sanitize_text_arg(args, "file_name", 160))
                .or_else(|| sanitize_text_arg(args, "attachment_name", 160))
                .or_else(|| sanitize_text_arg(args, "name", 160))?;
            let title = sanitize_text_arg(args, "title", 120)
                .or_else(|| sanitize_text_arg(args, "activity_title", 120))
                .or_else(|| sanitize_text_arg(args, "activityTitle", 120));
            let luna_id = sanitize_text_arg(args, "luna_id", 80);
            let mut out = serde_json::Map::new();
            out.insert("filename".to_string(), Value::String(filename));
            if let Some(t) = title {
                out.insert("title".to_string(), Value::String(t));
            }
            if let Some(id) = luna_id {
                out.insert("luna_id".to_string(), Value::String(id));
            }
            Some(Value::Object(out))
        }
        ArgSchema::OptionalText { key, max_len } => {
            let val = sanitize_text_arg(args, key, max_len);
            let mut out = serde_json::Map::new();
            if let Some(val) = val {
                out.insert(key.to_string(), Value::String(val));
            }
            Some(Value::Object(out))
        }
        ArgSchema::Url => sanitize_url_arg(args, "url").map(|url| json!({ "url": url })),
        ArgSchema::CopilotPage => sanitize_copilot_page_args(args),
        ArgSchema::DownloadUrl => {
            let url = sanitize_url_arg(args, "url")?;
            let filename = sanitize_text_arg(args, "filename", 200);
            let mut out = serde_json::Map::new();
            out.insert("url".into(), Value::String(url));
            if let Some(name) = filename {
                out.insert("filename".into(), Value::String(name));
            }
            Some(Value::Object(out))
        }
        ArgSchema::BrowserClick => sanitize_browser_click_args(args),
        ArgSchema::BrowserMouseClick => sanitize_browser_mouse_click_args(args),
        ArgSchema::BrowserMouseDrag => sanitize_browser_mouse_drag_args(args),
        ArgSchema::BrowserFill => sanitize_browser_fill_args(args),
        ArgSchema::BrowserSelect => sanitize_browser_select_args(args),
        ArgSchema::BrowserPress => sanitize_browser_press_args(args),
        ArgSchema::BrowserScroll => sanitize_browser_scroll_args(args),
        ArgSchema::BrowserWait => sanitize_browser_wait_args(args),
        ArgSchema::ComputerScreenshot => sanitize_computer_screenshot_args(args),
        ArgSchema::ComputerMouseClick => sanitize_computer_mouse_click_args(args),
        ArgSchema::ComputerMouseDrag => sanitize_computer_mouse_drag_args(args),
        ArgSchema::ComputerScroll => sanitize_computer_scroll_args(args),
        ArgSchema::CalendarEvent => sanitize_calendar_event_args(args),
        ArgSchema::CalendarUpdate => sanitize_calendar_update_args(args),
        ArgSchema::CalendarEventId => {
            sanitize_text_arg(args, "event_id", 200).map(|id| json!({ "event_id": id }))
        }
    }
}
