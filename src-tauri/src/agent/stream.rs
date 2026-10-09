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

#[derive(Serialize)]
struct OwnedEvent<'a, 'e> {
    turn_id: &'a str,
    #[serde(flatten)]
    event: &'a StreamEvent<'e>,
}

pub fn emit(app: &AppHandle, conv_id: &str, ev: &StreamEvent) {
    let owner = crate::agent_turn_scope::current(conv_id);
    emit_owned(app, conv_id, ev, owner.as_deref());
}

pub(super) fn emit_owned(
    app: &AppHandle,
    conv_id: &str,
    ev: &StreamEvent,
    owner: Option<&crate::agent_turn_scope::Turn>,
) {
    let topic = format!("agent_stream:{}", conv_id);
    if let Some(owner) = owner {
        if !owner.accepts_event(matches!(ev, StreamEvent::Done | StreamEvent::Error { .. })) {
            return;
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            if let Some(native) = native_event(ev) {
                crate::native_agent_events::publish_stream(app, conv_id, owner.request(), native);
            }
        }
        let _ = app.emit(
            &topic,
            &OwnedEvent {
                turn_id: owner.request(),
                event: ev,
            },
        );
    } else {
        let _ = app.emit(&topic, ev);
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn native_event<'a>(
    event: &StreamEvent<'a>,
) -> Option<crate::native_agent_events::StreamEvent<'a>> {
    use crate::native_agent_events::StreamEvent as NativeEvent;
    use std::borrow::Cow;
    match event {
        StreamEvent::Token { text } => Some(NativeEvent::Token(Cow::Borrowed(text))),
        StreamEvent::Done => Some(NativeEvent::Done),
        StreamEvent::Error { message } => Some(NativeEvent::Error(Cow::Borrowed(message))),
        _ => None,
    }
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

#[cfg(test)]
mod request_tests {
    use super::*;

    #[test]
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn native_delivery_borrows_full_text_and_skips_non_answer_events() {
        use crate::native_agent_events::StreamEvent as NativeEvent;
        use std::borrow::Cow;
        let text = "日本語\n\"quotes\"\t👩🏽‍💻".to_owned();
        let Some(NativeEvent::Token(Cow::Borrowed(actual))) =
            native_event(&StreamEvent::Token { text: &text })
        else {
            panic!("text was copied or omitted")
        };
        assert_eq!(actual, text);
        assert_eq!(actual.as_ptr(), text.as_ptr());
        let Some(NativeEvent::Error(Cow::Borrowed(actual))) =
            native_event(&StreamEvent::Error { message: &text })
        else {
            panic!("error was copied or omitted")
        };
        assert_eq!(actual.as_ptr(), text.as_ptr());
        assert!(matches!(
            native_event(&StreamEvent::Done),
            Some(NativeEvent::Done)
        ));
        for event in [
            StreamEvent::Phase { stage: "planning" },
            StreamEvent::Plan { steps: vec![] },
            StreamEvent::ToolCall { name: "tool" },
            StreamEvent::ToolResult {
                name: "tool",
                preview: "preview",
                ok: true,
            },
            StreamEvent::Think { text: &text },
        ] {
            assert!(native_event(&event).is_none());
        }
    }

    #[test]
    fn request_envelope_preserves_all_existing_stream_fields() {
        for event in [
            StreamEvent::Phase { stage: "planning" },
            StreamEvent::Plan {
                steps: vec![StreamPlanStep {
                    name: "search",
                    detail: Some("授業".into()),
                }],
            },
            StreamEvent::ToolCall { name: "search" },
            StreamEvent::ToolResult {
                name: "search",
                preview: "日本語",
                ok: true,
            },
            StreamEvent::Think { text: "thinking" },
            StreamEvent::Token {
                text: "日本語\n回答",
            },
            StreamEvent::Done,
            StreamEvent::Error { message: "failed" },
        ] {
            let mut expected = serde_json::to_value(&event).unwrap();
            expected
                .as_object_mut()
                .unwrap()
                .insert("turn_id".into(), serde_json::json!("request-a"));
            let encoded = serde_json::to_value(OwnedEvent {
                turn_id: "request-a",
                event: &event,
            })
            .unwrap();
            assert_eq!(encoded, expected);
        }
    }
}
