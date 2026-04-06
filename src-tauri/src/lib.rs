mod ai;
mod auth;
mod client;
pub(crate) mod config;
mod commands;
mod kwic_client;
mod kwic_commands;
mod luna_client;
mod luna_commands;
mod luna_parser;
mod mail;
mod mail_commands;
mod parser;
mod syllabus;
mod tray;
mod webview_toolbar;

use tokio::sync::Mutex;
use tauri::Manager;

pub struct AppState {
    pub client: Mutex<client::KgcClient>,
    pub luna: Mutex<luna_client::LunaClient>,
    pub kwic: Mutex<kwic_client::KwicClient>,
    pub mail: Mutex<mail::MailClient>,
}

/// Shared theme state so child webviews can read the current theme.
pub struct ThemeState(pub std::sync::Mutex<String>);

#[tauri::command]
fn get_app_theme(state: tauri::State<'_, ThemeState>) -> String {
    state.0.lock().unwrap().clone()
}

#[tauri::command]
fn set_app_theme(state: tauri::State<'_, ThemeState>, theme: String) {
    *state.0.lock().unwrap() = theme;
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            app.handle().plugin(tauri_plugin_notification::init())?;
            app.handle().plugin(
                tauri_plugin_log::Builder::default()
                    .level(log::LevelFilter::Debug)
                    .targets([
                        tauri_plugin_log::Target::new(
                            tauri_plugin_log::TargetKind::Stderr,
                        ),
                        tauri_plugin_log::Target::new(
                            tauri_plugin_log::TargetKind::LogDir { file_name: Some("kwic".into()) },
                        ),
                    ])
                    .build(),
            )?;
            let mut luna = luna_client::LunaClient::new();
            luna.try_restore_session();
            let mut kwic = kwic_client::KwicClient::new();
            kwic.try_restore_session();
            let mut mail_client = mail::MailClient::new();
            mail_client.try_restore_token();
            app.manage(AppState {
                client: Mutex::new(client::KgcClient::new()),
                luna: Mutex::new(luna),
                kwic: Mutex::new(kwic),
                mail: Mutex::new(mail_client),
            });
            app.manage(commands::SyllabusDetailData(std::sync::Mutex::new(std::collections::HashMap::new())));
            app.manage(ThemeState(std::sync::Mutex::new("system".to_string())));
            let tray_status = std::sync::Arc::new(tray::TrayStatusState::new());
            app.manage(tray_status.clone());
            tray::setup_tray(app.handle())?;
            tray::start_tray_cycle(app.handle(), tray_status);

            // Hide main window on close instead of quitting (keep in tray)
            if let Some(win) = app.get_webview_window("main") {
                let app_handle = app.handle().clone();
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        if let Some(w) = app_handle.get_webview_window("main") {
                            let _ = w.hide();
                        }
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::open_login_window,
            commands::logout,
            commands::check_session,
            commands::validate_session,
            commands::fetch_timetable,
            commands::fetch_timetable_week,
            commands::fetch_grades,
            commands::fetch_cancellations,
            commands::fetch_makeup_classes,
            commands::fetch_room_changes,
            commands::fetch_registration,
            commands::fetch_exam_timetable,
            commands::fetch_notifications,
            commands::fetch_weather,
            commands::fetch_page,
            commands::fetch_course_detail,
            commands::open_detail_window,
            commands::open_external_url,
            commands::open_profile_edit_window,
            commands::open_facility_reservation,
            commands::open_registration_window,
            commands::fetch_student_profile,
            commands::debug_info,
            commands::debug_ping,
            commands::search_syllabus,
            commands::fetch_syllabus_favorites,
            commands::toggle_syllabus_bookmark,
            commands::open_syllabus_detail,
            commands::get_syllabus_detail,
            commands::sync_calendar,
            commands::get_calendar_info,
            commands::clear_calendar,
            commands::sync_session,
            commands::get_session_states,
            luna_commands::luna_open_detail_window,
            luna_commands::luna_open_login,
            luna_commands::luna_fetch_page,
            luna_commands::luna_check_session,
            luna_commands::luna_fetch_dashboard,
            luna_commands::luna_fetch_courses,
            luna_commands::luna_fetch_notifications,
            luna_commands::luna_fetch_timetable,
            luna_commands::luna_fetch_todo,
            luna_commands::luna_fetch_updates,
            luna_commands::luna_fetch_course_content,
            luna_commands::luna_fetch_detail,
            luna_commands::luna_fetch_announcement_detail,
            luna_commands::luna_fetch_course_detail,
            luna_commands::luna_download_file,
            luna_commands::luna_download_material,
            luna_commands::luna_launch_lti,
            luna_commands::luna_reveal_file,
            luna_commands::luna_submit_report,
            luna_commands::luna_fetch_discussion_detail,
            luna_commands::luna_post_discussion,
            luna_commands::luna_reply_discussion,
            luna_commands::luna_fetch_thread_posts,
            kwic_commands::kwic_check_session,
            kwic_commands::kwic_fetch_page,
            kwic_commands::kwic_fetch_home,
            kwic_commands::kwic_fetch_notifications,
            kwic_commands::kwic_fetch_detail,
            kwic_commands::kwic_fetch_subportal,
            kwic_commands::kwic_open_detail_window,
            kwic_commands::kwic_open_link,
            kwic_commands::kwic_open_login,
            mail_commands::mail_check_session,
            mail_commands::mail_open_login,
            mail_commands::mail_logout,
            mail_commands::mail_fetch_profile,
            mail_commands::mail_fetch_inbox,
            mail_commands::mail_fetch_message,
            mail_commands::mail_get_config,
            mail_commands::mail_save_config,
            ai::get_ai_config,
            ai::save_ai_config,
            ai::ai_chat,
            ai::ai_test_connection,
            ai::open_settings_window,
            ai::open_ai_result_window,
            ai::request_ai_refresh,
            ai::toggle_debug_panel,
            ai::test_notification,
            tray::update_tray,
            tray::set_tray_status_items,
            get_app_theme,
            set_app_theme,
            webview_toolbar::browser_go_back,
            webview_toolbar::browser_go_forward,
            webview_toolbar::browser_reload,
            webview_toolbar::browser_get_url,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
