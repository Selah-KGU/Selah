mod ai;
mod auth;
mod client;
pub(crate) mod config;
mod commands;
mod cookie_bridge;
mod db;
pub(crate) mod keychain;
mod kwic_client;
mod kwic_commands;
mod luna_client;
mod luna_commands;
mod luna_parser;
mod mail;
mod mail_commands;
mod read_state;
mod google_calendar;
mod google_commands;
mod parser;
mod syllabus;
mod timetable;
mod tray;
mod webview_toolbar;

use tokio::sync::Mutex;
use tauri::Manager;

// ── Decoupled per-service states (independent locking, zero cross-service contention) ──

/// KG-Course (KGC) service state.
pub struct KgcState {
    pub client: Mutex<client::KgcClient>,
    /// Serializes KGC HTTP requests to prevent Struts token races.
    ///
    /// Struts 1 stores ONE token per HTTP session (server-side). Any KGC page
    /// load that renders a form calls `saveToken()`, overwriting the previous
    /// token. When multiple KGC requests execute concurrently (e.g. background
    /// polling + syllabus enrichment), the token extracted from page A is
    /// invalidated by page B's load, causing all subsequent form POSTs to fail.
    pub gate: Mutex<()>,
}

/// Luna LMS service state.
pub struct LunaState {
    pub client: Mutex<luna_client::LunaClient>,
}

/// KWIC Portal service state.
pub struct KwicState {
    pub client: Mutex<kwic_client::KwicClient>,
}

/// Microsoft 365 Mail service state.
pub struct MailState {
    pub client: Mutex<mail::MailClient>,
}

/// Google Calendar service state.
pub struct GCalState {
    pub client: Mutex<google_calendar::GoogleCalendarClient>,
}

/// Shared theme state so child webviews can read the current theme.
pub struct ThemeState(pub std::sync::Mutex<String>);

#[tauri::command]
fn get_app_theme(state: tauri::State<'_, ThemeState>) -> String {
    state.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[tauri::command]
fn set_app_theme(state: tauri::State<'_, ThemeState>, theme: String) {
    *state.0.lock().unwrap_or_else(|e| e.into_inner()) = theme;
}

#[tauri::command]
fn mark_notification_read(db: tauri::State<'_, db::Database>, source: String, id: String) {
    read_state::mark_read(&db, &source, &id);
}

#[tauri::command]
fn mark_batch_notification_read(db: tauri::State<'_, db::Database>, source: String, ids: Vec<String>) {
    read_state::mark_batch_read(&db, &source, ids);
}

#[tauri::command]
fn get_read_notifications(db: tauri::State<'_, db::Database>) -> read_state::ReadIdsResponse {
    read_state::get_all_read_ids(&db)
}

#[tauri::command]
fn get_seen_notif_ids(db: tauri::State<'_, db::Database>, source: String) -> Vec<String> {
    read_state::get_seen_notif_ids(&db, &source)
}

#[tauri::command]
fn save_seen_notif_ids(db: tauri::State<'_, db::Database>, source: String, ids: Vec<String>) {
    read_state::save_seen_notif_ids(&db, &source, ids);
}

#[tauri::command]
fn get_data_cache(db: tauri::State<'_, db::Database>, key: String) -> Option<String> {
    db.get_data_cache(&key).ok().flatten().map(|(json, _)| json)
}

#[tauri::command]
fn save_data_cache(db: tauri::State<'_, db::Database>, key: String, json: String) -> Result<(), String> {
    db.save_data_cache(&key, &json)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default();

    #[cfg(target_os = "windows")]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Another instance tried to launch — show & focus the existing main window
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.unminimize();
                let _ = win.set_focus();
            }
        }));
    }

    builder
        .setup(|app| {
            app.handle().plugin(tauri_plugin_notification::init())?;
            app.handle().plugin(tauri_plugin_opener::init())?;
            app.handle().plugin(
                tauri_plugin_log::Builder::default()
                    .level(if cfg!(debug_assertions) { log::LevelFilter::Debug } else { log::LevelFilter::Info })
                    .level_for("selectors", log::LevelFilter::Warn)
                    .level_for("html5ever", log::LevelFilter::Warn)
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
            let mut kgc = client::KgcClient::new();
            kgc.try_restore_session();
            let mut mail_client = mail::MailClient::new();
            mail_client.try_restore_token();
            let mut gcal_client = google_calendar::GoogleCalendarClient::new();
            gcal_client.try_restore_token();
            app.manage(KgcState { client: Mutex::new(kgc), gate: Mutex::new(()) });
            app.manage(LunaState { client: Mutex::new(luna) });
            app.manage(KwicState { client: Mutex::new(kwic) });
            app.manage(MailState { client: Mutex::new(mail_client) });
            app.manage(GCalState { client: Mutex::new(gcal_client) });
            app.manage(commands::SyllabusDetailData(std::sync::Mutex::new(std::collections::HashMap::new())));
            app.manage(ThemeState(std::sync::Mutex::new("system".to_string())));

            // Initialize SQLite database for timetable enrichment
            let data_dir = dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("com.kgu.selah");
            let database = db::Database::open(&data_dir)
                .map_err(|e| format!("Failed to open timetable database: {}", e))?;
            app.manage(database);

            let tray_status = std::sync::Arc::new(tray::TrayStatusState::new());
            app.manage(tray_status.clone());
            tray::setup_tray(app.handle())?;
            tray::start_tray_cycle(app.handle(), tray_status);

            // Hide main window on close instead of quitting (keep in tray)
            if let Some(win) = app.get_webview_window("main") {
                #[cfg(target_os = "windows")]
                {
                    let _ = win.set_decorations(false);
                }

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
            commands::fetch_grades,
            commands::fetch_cancellations,
            commands::fetch_makeup_classes,
            commands::fetch_room_changes,
            commands::fetch_registration,
            commands::fetch_exam_timetable,
            commands::fetch_notifications,
            commands::fetch_weather,
            commands::fetch_page,
            timetable::get_schedule_snapshot,
            timetable::sync_schedule_data,
            timetable::enrich_schedule,
            timetable::refresh_luna_counts,
            timetable::ai_generate_schedule,
            timetable::ai_analyze_todo,
            commands::fetch_course_detail,
            commands::open_detail_window,
            commands::open_external_url,
            commands::open_in_system_browser,
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
            commands::get_kgc_syllabus_fields,
            commands::sync_calendar,
            commands::get_calendar_info,
            commands::clear_calendar,
            commands::sync_session,
            commands::get_session_states,
            commands::get_session_expiry,
            luna_commands::luna_open_detail_window,
            luna_commands::luna_fetch_page,
            luna_commands::luna_check_session,
            luna_commands::luna_fetch_todo,
            luna_commands::luna_fetch_updates,
            luna_commands::luna_fetch_course_content,
            luna_commands::luna_fetch_detail,
            luna_commands::luna_fetch_announcement_detail,
            luna_commands::luna_fetch_survey_detail,
            luna_commands::luna_submit_survey,
            luna_commands::luna_submit_attendance,
            luna_commands::luna_fetch_course_detail,
            luna_commands::luna_download_file,
            luna_commands::luna_download_material,
            luna_commands::luna_resolve_material_link,
            luna_commands::luna_launch_lti,
            luna_commands::luna_reveal_file,
            luna_commands::luna_check_report_type,
            luna_commands::luna_submit_report,
            luna_commands::luna_submit_report_text,
            luna_commands::luna_fetch_discussion_detail,
            luna_commands::luna_post_discussion,
            luna_commands::luna_reply_discussion,
            luna_commands::luna_fetch_thread_posts,
            kwic_commands::kwic_check_session,
            kwic_commands::kwic_fetch_home,
            kwic_commands::kwic_fetch_detail,
            kwic_commands::kwic_fetch_subportal,
            kwic_commands::kwic_open_detail_window,
            kwic_commands::kwic_open_link,
            mail_commands::mail_check_session,
            mail_commands::mail_open_login,
            mail_commands::mail_logout,
            mail_commands::mail_fetch_profile,
            mail_commands::mail_fetch_inbox,
            mail_commands::mail_fetch_message,
            mail_commands::mail_get_config,
            mail_commands::mail_save_config,
            mail_commands::mail_fetch_attachments,
            mail_commands::mail_download_attachment,
            google_commands::gcal_check_session,
            google_commands::gcal_get_config,
            google_commands::gcal_save_config,
            google_commands::gcal_open_login,
            google_commands::gcal_disconnect,
            google_commands::gcal_sync_timetable,
            google_commands::gcal_clear_calendar,
            ai::get_ai_config,
            ai::save_ai_config,
            ai::ai_chat,
            ai::ai_test_connection,
            ai::open_settings_window,
            ai::open_ai_result_window,
            ai::request_ai_refresh,
            ai::toggle_debug_panel,
            ai::test_notification,
            commands::get_download_config,
            commands::save_download_config,
            commands::select_download_dir,
            commands::list_downloads,
            commands::scan_download_dir,
            commands::check_file_downloaded,
            commands::open_downloaded_file,
            commands::remove_download_record,
            commands::clear_download_history,
            commands::open_downloads_window,
            tray::update_tray,
            tray::set_tray_status_items,
            tray::get_tray_popup_data,
            tray::show_main_window,
            tray::quit_app,
            get_app_theme,
            set_app_theme,
            mark_notification_read,
            mark_batch_notification_read,
            get_read_notifications,
            get_seen_notif_ids,
            save_seen_notif_ids,
            get_data_cache,
            save_data_cache,
            webview_toolbar::browser_go_back,
            webview_toolbar::browser_go_forward,
            webview_toolbar::browser_reload,
            webview_toolbar::browser_get_url,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                // Persist all session cookies on exit so they survive restarts.
                // Use try_lock to avoid deadlock if another task holds the lock.
                let kgc = app.state::<KgcState>();
                match kgc.client.try_lock() {
                    Ok(c) => c.save_session(),
                    Err(_) => log::warn!("Exit: KGC mutex held, session not saved"),
                };
                let luna = app.state::<LunaState>();
                match luna.client.try_lock() {
                    Ok(l) => l.save_session(),
                    Err(_) => log::warn!("Exit: Luna mutex held, session not saved"),
                };
                let kwic = app.state::<KwicState>();
                match kwic.client.try_lock() {
                    Ok(k) => k.save_session(),
                    Err(_) => log::warn!("Exit: KWIC mutex held, session not saved"),
                };
                let mail = app.state::<MailState>();
                match mail.client.try_lock() {
                    Ok(m) => m.save_token(),
                    Err(_) => log::warn!("Exit: Mail mutex held, token not saved"),
                };
                let gcal = app.state::<GCalState>();
                match gcal.client.try_lock() {
                    Ok(g) => g.save_token(),
                    Err(_) => log::warn!("Exit: GCal mutex held, token not saved"),
                };
            }
        });
}
