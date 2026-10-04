use super::*;

// ─────────────────────── Stream Events ───────────────────────

#[derive(Debug, Serialize)]
pub struct StreamPlanStep<'a> {
    pub(super) name: &'a str,
    pub(super) detail: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent<'a> {
    Phase {
        stage: &'a str,
    },
    Plan {
        steps: Vec<StreamPlanStep<'a>>,
    },
    ToolCall {
        name: &'a str,
    },
    ToolResult {
        name: &'a str,
        preview: &'a str,
        ok: bool,
    },
    Think {
        text: &'a str,
    },
    Token {
        text: &'a str,
    },
    Done,
    Error {
        message: &'a str,
    },
}

pub fn emit(app: &AppHandle, conv_id: &str, ev: &StreamEvent) {
    let topic = format!("agent_stream:{}", conv_id);
    let _ = app.emit(&topic, ev);
}

pub fn plan_step_detail(call: &ToolCall) -> Option<String> {
    let key = match call.name.as_str() {
        "browser_click" => "text",
        "browser_fill" | "browser_select_option" => "label",
        "browser_wait_for" => "text",
        "open_browser_url" | "download_url" => "url",
        "open_copilot_page" => {
            return call
                .args
                .get("context")
                .or_else(|| call.args.get("page"))
                .and_then(|v| v.as_str())
                .map(|value| trim_to(value.trim(), 80));
        }
        "read_downloaded_file"
        | "open_downloaded_file"
        | "delete_downloaded_file"
        | "write_downloaded_text_file" => "path",
        "get_course_context" | "search_courses" => "query",
        "list_downloaded_files"
        | "search_notifications"
        | "search_mail"
        | "list_luna_announcements" => "keyword",
        "get_luna_activity_detail"
        | "open_luna_attachment"
        | "download_luna_attachment"
        | "create_google_calendar_event" => "title",
        "download_course_material" => "filename",
        "update_google_calendar_event" | "delete_google_calendar_event" => "event_id",
        _ => return None,
    };
    let raw = call.args.get(key).and_then(|v| v.as_str())?.trim();
    if raw.is_empty() {
        return None;
    }
    let safe = if key == "path" {
        raw.rsplit(['/', '\\']).next().unwrap_or(raw)
    } else if key == "url" {
        raw.split(['?', '#']).next().unwrap_or(raw)
    } else {
        raw
    };
    Some(trim_to(safe, 80))
}
