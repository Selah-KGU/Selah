use super::academic::*;
use super::calendar::*;
use super::catalog::canonical_tool_name;
use super::copilot_page::*;
use super::files_browser::*;
use super::insights::*;
use super::mail_lookup::*;
use super::records::*;
use serde_json::{json, Value};

/// Dispatch a single tool call.  Returns a JSON value even on failure so the
/// agent can still surface the error to the user.
pub async fn dispatch(app: &tauri::AppHandle, name: &str, args: &Value) -> Value {
    let Some(name) = canonical_tool_name(name) else {
        return json!({ "error": format!("unknown tool: {}", name) });
    };
    let result: Result<Value, String> = match name {
        "list_today_classes" => list_today_classes(app).await,
        "list_week_classes" => list_week_classes(app, args).await,
        "search_courses" => search_courses(app, args).await,
        "get_course_context" => get_course_context(app, args).await,
        "list_luna_todos" => list_luna_todos(app).await,
        "list_recent_notifications" => list_recent_notifications(app, args).await,
        "get_notification_detail" => get_notification_detail(app, args).await,
        "get_course_detail" => get_course_detail(app, args).await,
        "list_recent_mail" => list_recent_mail(app, args).await,
        "read_mail" => read_mail(app, args).await,
        "list_luna_announcements" => list_luna_announcements(app, args).await,
        "get_student_profile" => get_student_profile(app).await,
        "get_mail_profile" => get_mail_profile(app).await,
        "list_syllabus_favorites" => list_syllabus_favorites(app, args).await,
        "get_grades" => get_grades(app).await,
        "get_cancellations" => get_cancellations(app).await,
        "get_makeup_classes" => get_makeup_classes(app).await,
        "get_room_changes" => get_room_changes(app).await,
        "get_registration" => get_registration(app).await,
        "get_exam_timetable" => get_exam_timetable(app).await,
        "get_weather" => get_weather(app).await,
        "get_weekly_summary" => get_weekly_summary(app).await,
        "get_todo_guide" => get_todo_guide(app).await,
        "get_upcoming_deadlines" => get_upcoming_deadlines(app).await,
        "get_luna_activity_detail" => get_luna_activity_detail(app, args).await,
        "refresh_data" => refresh_data(app).await,
        "list_downloaded_files" => list_downloaded_files(args).await,
        "read_downloaded_file" => read_downloaded_file(app, args).await,
        "write_downloaded_text_file" => write_downloaded_text_file(args).await,
        "open_downloaded_file" => open_downloaded_file(app, args).await,
        "delete_downloaded_file" => delete_downloaded_file(args).await,
        "download_url" => download_url(args).await,
        "open_luna_attachment" => open_luna_attachment(app, args).await,
        "download_luna_attachment" => download_luna_attachment(app, args).await,
        "download_course_material" => download_course_material(app, args).await,
        "list_browser_windows" => list_browser_windows(app).await,
        "open_browser_url" => open_browser_url(app, args).await,
        "open_copilot_page" => open_copilot_page(app, args).await,
        "read_browser_page" => read_browser_page(app, args).await,
        "browser_back" => browser_back(app, args).await,
        "browser_forward" => browser_forward(app, args).await,
        "browser_reload_page" => browser_reload_page(app, args).await,
        "browser_click" => browser_click(app, args).await,
        "browser_mouse_click" => browser_mouse_click(app, args).await,
        "browser_mouse_drag" => browser_mouse_drag(app, args).await,
        "browser_fill" => browser_fill(app, args).await,
        "browser_select_option" => browser_select_option(app, args).await,
        "browser_press" => browser_press(app, args).await,
        "browser_scroll" => browser_scroll(app, args).await,
        "browser_wait_for" => browser_wait_for(app, args).await,
        "browser_close" => browser_close_tool(app, args).await,
        "computer_screenshot" => computer_screenshot(app, args).await,
        "computer_mouse_click" => computer_mouse_click(app, args).await,
        "computer_mouse_drag" => computer_mouse_drag(app, args).await,
        "computer_scroll" => computer_scroll(app, args).await,
        "get_today_brief" => get_today_brief(app).await,
        "create_google_calendar_event" => create_google_calendar_event(app, args).await,
        "list_google_calendar_events" => list_google_calendar_events(app).await,
        "delete_google_calendar_event" => delete_google_calendar_event(app, args).await,
        "update_google_calendar_event" => update_google_calendar_event(app, args).await,
        // Listed in TOOL_SPECS but not yet wired here. Treated as a soft
        // failure so a forgotten dispatch arm cannot panic in production.
        other => {
            log::error!(
                "[agent tools] tool {} is registered but has no dispatch arm",
                other
            );
            Err(format!("tool {} is not implemented yet", other))
        }
    };
    match result {
        Ok(v) => v,
        Err(e) => json!({ "error": e }),
    }
}
