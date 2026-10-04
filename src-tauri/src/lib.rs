#[cfg(all(feature = "stt-static", feature = "stt-shared"))]
compile_error!("features `stt-static` and `stt-shared` cannot be enabled together");

mod academic_period;
mod agent;
mod agent_commands;
mod agent_error;
mod agent_prompts;
mod agent_provider;
mod agent_pseudo_call;
mod agent_text;
mod agent_tools;
pub mod ai;
mod ai_refresh;
#[cfg(target_os = "macos")]
mod app_menu;
mod app_updates;
mod auth;
mod background_refresh;
mod client;
mod commands;
mod computer_control;
pub(crate) mod config;
mod cookie_bridge;
mod course_automation;
mod db;
mod detective;
mod document_tabs;
mod embedded_keys;
mod google_calendar;
mod google_commands;
pub(crate) mod keychain;
mod kwic_client;
mod kwic_commands;
mod live;
pub mod local_ai;
pub mod local_ai_support;
mod luna_client;
mod luna_commands;
mod luna_parser;
#[cfg(target_os = "macos")]
mod macos_fullscreen_exit;
#[cfg(target_os = "macos")]
mod macos_native_agent;
#[cfg(target_os = "macos")]
mod macos_subtitle_overlay;
mod mail;
mod mail_commands;
mod native_notification;
mod notifier;
mod paper_check;
mod parser;
mod power;
mod read_state;
mod stt;
mod syllabus;
mod timetable;
mod tray;
mod webview_toolbar;
mod widget_bridge;
#[cfg(target_os = "windows")]
mod windows_native_agent;
#[cfg(target_os = "windows")]
mod windows_subtitle_overlay;

use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

pub fn run_stt_decode_helper_from_args() -> Option<i32> {
    stt::run_decode_helper_from_args()
}

#[cfg(debug_assertions)]
pub(crate) fn should_dump_debug_html() -> bool {
    std::env::var("SELAH_DUMP_HTML")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

#[cfg(debug_assertions)]
fn should_run_browser_mouse_selftest() -> bool {
    std::env::var("SELAH_BROWSER_MOUSE_SELFTEST")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

#[cfg(unix)]
fn protect_log_storage(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;

    let dir = app.path().app_log_dir()?;
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;

    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.is_file() {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
    }

    let log_path = dir.join("kwic.log");
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    std::fs::set_permissions(log_path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn protect_log_storage(_app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

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
fn set_app_theme(app: tauri::AppHandle, state: tauri::State<'_, ThemeState>, theme: String) {
    *state.0.lock().unwrap_or_else(|e| e.into_inner()) = theme;
    let _ = app.emit("app-theme-changed", ());
}

#[tauri::command]
fn mark_notification_read(db: tauri::State<'_, db::Database>, source: String, id: String) {
    read_state::mark_read(&db, &source, &id);
}

#[tauri::command]
fn mark_batch_notification_read(
    db: tauri::State<'_, db::Database>,
    source: String,
    ids: Vec<String>,
) {
    read_state::mark_batch_read(&db, &source, ids);
}

#[tauri::command]
fn get_read_notifications(db: tauri::State<'_, db::Database>) -> read_state::ReadIdsResponse {
    read_state::get_all_read_ids(&db)
}

#[tauri::command]
fn get_data_cache(db: tauri::State<'_, db::Database>, key: String) -> Option<String> {
    db.get_data_cache(&key).ok().flatten().map(|(json, _)| json)
}

#[tauri::command]
fn get_data_cache_updated_at(db: tauri::State<'_, db::Database>, key: String) -> Option<i64> {
    db.get_data_cache(&key)
        .ok()
        .flatten()
        .map(|(_, updated_at)| updated_at)
}

#[tauri::command]
fn save_data_cache(
    db: tauri::State<'_, db::Database>,
    key: String,
    json: String,
) -> Result<(), String> {
    if key.starts_with("seen_notifs_") {
        return Err("reserved cache key".into());
    }
    db.save_data_cache(&key, &json)
}

#[tauri::command]
fn request_app_restart(app: tauri::AppHandle) {
    app.request_restart();
}

fn run_event_panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

fn persist_sessions_before_exit(app: &tauri::AppHandle) {
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

fn defer_fullscreen_quit(
    app: &tauri::AppHandle,
    api: &tauri::ExitRequestApi,
    code: Option<i32>,
) -> bool {
    #[cfg(target_os = "macos")]
    {
        if macos_fullscreen_exit::defer_programmed_exit(app, code) {
            api.prevent_exit();
            return true;
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, api, code);
    }
    false
}

fn handle_run_event(app: &tauri::AppHandle, event: tauri::RunEvent) {
    match event {
        tauri::RunEvent::ExitRequested { api, code, .. } => {
            if defer_fullscreen_quit(app, &api, code) {
                return;
            }
            stt::stt_shutdown_for_exit(std::time::Duration::from_millis(1500));
        }
        tauri::RunEvent::Exit => {
            stt::stt_shutdown_for_exit(std::time::Duration::from_millis(500));
            persist_sessions_before_exit(app);
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen {
            has_visible_windows,
            ..
        } => {
            reopen_windows_from_dock(app, has_visible_windows);
        }
        _ => {}
    }
}

fn attach_main_window_close_handler(window: &tauri::WebviewWindow, app_handle: tauri::AppHandle) {
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            if let Some(main) = app_handle.get_webview_window("main") {
                let _ = main.hide();
            }
        }
    });
}

/// Close hides the main window instead of destroying it. tao answers
/// applicationShouldHandleReopen with hasVisibleWindows, so AppKit will not
/// restore that window on a Dock click unless we show it ourselves.
#[cfg(target_os = "macos")]
fn reopen_windows_from_dock(app: &tauri::AppHandle, has_visible_windows: bool) {
    let any_window_on_screen = app.windows().values().any(window_is_on_screen);
    if !should_restore_on_dock_click(has_visible_windows, any_window_on_screen) {
        return;
    }

    // Cmd+H hides the process; unhide before ordering a window front.
    let _ = app.show();
    if app.get_webview_window("main").is_none() {
        if let Err(err) = recreate_main_window(app) {
            log::warn!("failed to recreate main window from Dock: {err}");
        }
    }
    if app.get_webview_window("main").is_some() {
        present_main_window(app);
        return;
    }
    if let Some(window) = app.get_window("document-tabs") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg(target_os = "macos")]
fn present_main_window(app: &tauri::AppHandle) {
    let _ = tray::show_main_window_with_tab(app, None);
    // AppKit can order the window back out after applicationShouldHandleReopen
    // returns. Show again on the next main-thread turn so the Dock click sticks.
    let deferred = app.clone();
    let _ = app.run_on_main_thread(move || {
        let _ = tray::show_main_window_with_tab(&deferred, None);
    });
}

#[cfg(target_os = "macos")]
fn recreate_main_window(app: &tauri::AppHandle) -> Result<(), String> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == "main")
        .cloned()
        .or_else(|| app.config().app.windows.first().cloned())
        .ok_or_else(|| "main window config missing".to_string())?;
    let window = tauri::WebviewWindowBuilder::from_config(app, &config)
        .map_err(|err| err.to_string())?
        .build()
        .map_err(|err| err.to_string())?;
    attach_main_window_close_handler(&window, app.clone());
    Ok(())
}

#[cfg(target_os = "macos")]
fn window_is_on_screen(window: &tauri::Window) -> bool {
    window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn should_restore_on_dock_click(has_visible_windows: bool, any_window_on_screen: bool) -> bool {
    !has_visible_windows || !any_window_on_screen
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

    #[cfg(target_os = "macos")]
    {
        builder = builder
            .menu(app_menu::build)
            .on_menu_event(app_menu::handle_event);
    }

    builder
        .setup(|app| {
            document_tabs::record_main_thread();
            #[cfg(debug_assertions)]
            let browser_mouse_selftest = should_run_browser_mouse_selftest();
            #[cfg(not(debug_assertions))]
            let browser_mouse_selftest = false;
            #[cfg(debug_assertions)]
            if browser_mouse_selftest {
                eprintln!("SELAH_BROWSER_MOUSE_SELFTEST_REQUESTED");
            }

            #[cfg(not(target_os = "macos"))]
            app.handle().plugin(tauri_plugin_notification::init())?;
            app.handle().plugin(tauri_plugin_opener::init())?;
            app.handle()
                .plugin(tauri_plugin_global_shortcut::Builder::new().build())?;
            app_updates::init(app.handle())?;
            protect_log_storage(app.handle())?;
            app.handle().plugin(
                tauri_plugin_log::Builder::default()
                    .level(if cfg!(debug_assertions) {
                        log::LevelFilter::Debug
                    } else {
                        log::LevelFilter::Info
                    })
                    .level_for("selectors", log::LevelFilter::Warn)
                    .level_for("html5ever", log::LevelFilter::Warn)
                    .targets([
                        tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stderr),
                        tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                            file_name: Some("kwic".into()),
                        }),
                    ])
                    .build(),
            )?;
            protect_log_storage(app.handle())?;
            // Unlock the secret bundle once now (Touch ID on macOS) so the
            // prompt fires at launch rather than from a background task later;
            // the session restores below then read from the in-memory copy.
            keychain::prewarm();
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
            app.manage(KgcState {
                client: Mutex::new(kgc),
                gate: Mutex::new(()),
            });
            app.manage(LunaState {
                client: Mutex::new(luna),
            });
            app.manage(KwicState {
                client: Mutex::new(kwic),
            });
            app.manage(MailState {
                client: Mutex::new(mail_client),
            });
            app.manage(GCalState {
                client: Mutex::new(gcal_client),
            });
            app.manage(commands::SyllabusDetailData(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )));
            app.manage(ThemeState(std::sync::Mutex::new("system".to_string())));
            app.manage(live::LiveState::new());
            app.manage(background_refresh::BackendRefreshState::new());
            app.manage(ai_refresh::AiRefreshState::new());
            app.manage(course_automation::CourseAutomationState::new());
            app.manage(notifier::NotificationPollState::new());

            // Initialize SQLite database for timetable enrichment
            let data_dir = dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("com.kgu.selah");
            let database = db::Database::open(&data_dir)
                .map_err(|e| format!("Failed to open timetable database: {}", e))?;
            widget_bridge::publish(&database);
            widget_bridge::ensure_host_registered();
            app.manage(database);

            let tray_status = std::sync::Arc::new(tray::TrayStatusState::new());
            app.manage(tray_status.clone());
            tray::setup_tray(app.handle())?;
            if !browser_mouse_selftest {
                // Restore backed-up SSO cookies into the webview store before any
                // headless re-auth runs, so a wiped webview store can still
                // present a valid SSO session / device token.
                let restore_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    cookie_bridge::restore_sso_cookies(&restore_handle).await;
                });
                tray::start_tray_cycle(app.handle(), tray_status);
                background_refresh::start_background_refresh_loop(app.handle());
                ai_refresh::start_ai_refresh_loop(app.handle());
                course_automation::start_course_automation_loop(app.handle());
                notifier::start_notification_loop(app.handle());
            }
            commands::migrate_uncategorized_to_other();
            commands::migrate_rename_course_folders();
            commands::migrate_normalize_course_names();
            commands::migrate_deduplicate_by_filename();
            #[cfg(target_os = "macos")]
            {
                macos_fullscreen_exit::install(app.handle());
                macos_native_agent::setup(app.handle());
                macos_subtitle_overlay::setup(app.handle());
                let native_agent_cfg = commands::load_native_agent_config();
                let _ = macos_native_agent::apply_config(app.handle(), &native_agent_cfg);
                if native_agent_cfg.subtitle_overlay_enabled {
                    let _ = macos_subtitle_overlay::open_overlay(app.handle());
                }
            }
            #[cfg(target_os = "windows")]
            {
                windows_native_agent::setup(app.handle());
                windows_subtitle_overlay::setup(app.handle());
                let native_agent_cfg = commands::load_native_agent_config();
                if let Err(err) =
                    windows_native_agent::apply_config(app.handle(), &native_agent_cfg)
                {
                    log::error!("failed to restore Windows agent shortcut: {err}");
                }
                if native_agent_cfg.subtitle_overlay_enabled {
                    if let Err(err) = windows_subtitle_overlay::open_overlay(app.handle()) {
                        log::error!("failed to restore Windows subtitle overlay: {err}");
                    }
                }
            }

            // Hide main window on close instead of quitting (keep in tray).
            // macOS Dock clicks show it again in reopen_windows_from_dock.
            if let Some(win) = app.get_webview_window("main") {
                #[cfg(target_os = "windows")]
                {
                    let _ = win.set_decorations(false);
                }
                attach_main_window_close_handler(&win, app.handle().clone());
            }

            #[cfg(debug_assertions)]
            if browser_mouse_selftest {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.hide();
                }
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    eprintln!("SELAH_BROWSER_MOUSE_SELFTEST_START");
                    tokio::time::sleep(std::time::Duration::from_millis(900)).await;
                    match webview_toolbar::debug_browser_mouse_click_selftest(app_handle.clone())
                        .await
                    {
                        Ok(result) => {
                            eprintln!("SELAH_BROWSER_MOUSE_SELFTEST_PASS {result}");
                            app_handle.exit(0);
                        }
                        Err(err) => {
                            eprintln!("SELAH_BROWSER_MOUSE_SELFTEST_FAIL {err}");
                            app_handle.exit(1);
                        }
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::open_login_window,
            commands::logout,
            commands::reset_university_login,
            commands::delete_all_local_data,
            commands::get_kgc_session_snapshot,
            commands::check_session,
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
            timetable::ai_extract_detail_todos,
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
            commands::debug_computer_mouse_click,
            commands::debug_browser_mouse_click_selftest,
            commands::search_syllabus,
            commands::fetch_syllabus_favorites,
            commands::toggle_syllabus_bookmark,
            commands::open_syllabus_detail,
            commands::get_syllabus_detail,
            commands::get_kgc_syllabus_fields,
            commands::sync_session,
            commands::get_session_states,
            commands::get_saved_cookie_summaries,
            luna_commands::university_open_detail_window,
            luna_commands::luna_open_detail_window,
            luna_commands::luna_fetch_page,
            luna_commands::luna_check_session,
            luna_commands::luna_fetch_todo,
            luna_commands::luna_fetch_updates,
            luna_commands::luna_fetch_detail,
            luna_commands::luna_fetch_announcement_detail,
            luna_commands::luna_fetch_survey_detail,
            luna_commands::luna_fetch_inquiry_detail,
            luna_commands::luna_reply_inquiry,
            luna_commands::luna_submit_survey,
            luna_commands::luna_prefetch_attendance_form,
            luna_commands::luna_submit_attendance,
            luna_commands::luna_fetch_course_detail,
            luna_commands::luna_download_file,
            luna_commands::luna_download_material,
            luna_commands::luna_resolve_material_link,
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
            kwic_commands::kwic_fetch_cabinet_reference,
            kwic_commands::kwic_open_detail_window,
            kwic_commands::kwic_open_cabinet_window,
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
            ai::get_local_ai_support,
            ai::save_ai_config,
            ai::ai_chat,
            ai::ai_test_connection,
            ai::list_local_models,
            ai::debug_test_notification,
            native_notification::native_notification_permission_granted,
            stt::get_stt_config,
            stt::save_stt_config,
            stt::list_stt_execution_backends,
            stt::list_stt_models,
            stt::download_stt_model,
            stt::delete_stt_model,
            stt::cancel_stt_model_download,
            stt::stt_test_model,
            stt::stt_start_stream,
            stt::stt_stop_stream,
            stt::stt_is_running,
            stt::stt_get_active_caller,
            power::prevent_sleep_start,
            power::prevent_sleep_stop,
            live::live_get_session,
            live::live_peek_day_cache,
            live::live_start_session,
            live::live_append_transcript,
            live::live_flush_summary,
            live::live_generate_overall_summary,
            live::live_cancel_session,
            live::live_clear_day_cache,
            live::live_finish_session,
            detective::detective_get_context,
            detective::detective_generate_campaign,
            detective::detective_finalize_finale,
            detective::detective_get_chapters,
            detective::detective_generate_chapter,
            detective::detective_save_doubts,
            detective::detective_save_case_result,
            detective::detective_save_included_courses,
            detective::detective_save_memory_outcome,
            paper_check::paper_check_extract_text,
            paper_check::paper_check_text_stats,
            paper_check::paper_check_analyze_ai,
            paper_check::paper_check_analyze_similarity,
            paper_check::paper_check_calibrate,
            paper_check::paper_check_get_calibration,
            paper_check::paper_check_get_search_config,
            paper_check::paper_check_save_search_config,
            paper_check::paper_check_save_report,
            document_tabs::document_tabs_list,
            document_tabs::document_tabs_set_controls,
            document_tabs::document_tabs_report_title,
            document_tabs::document_tabs_send_control,
            document_tabs::document_tabs_report_probe,
            document_tabs::document_tabs_activate,
            document_tabs::document_tabs_reveal,
            document_tabs::document_tabs_close,
            document_tabs::document_tabs_new_tab,
            document_tabs::document_tabs_reorder,
            document_tabs::document_tabs_close_split,
            document_tabs::document_tabs_close_pane,
            document_tabs::document_tabs_begin_split_drag,
            document_tabs::document_tabs_end_split_drag,
            document_tabs::document_tabs_drag_split,
            document_tabs::document_tabs_open_bookmark,
            document_tabs::document_tabs_open_agent,
            document_tabs::document_tabs_agent_is_open,
            document_tabs::document_tabs_resize_agent_panel,
            commands::get_download_config,
            commands::save_download_config,
            commands::select_download_dir,
            commands::get_security_config,
            commands::save_security_config,
            commands::get_notification_config,
            commands::save_notification_config,
            commands::get_native_agent_config,
            commands::save_native_agent_config,
            commands::get_calendar_config,
            commands::save_calendar_config,
            commands::save_image_file,
            commands::copy_image_to_clipboard,
            commands::share_image_native,
            notifier::notification_sync_now,
            background_refresh::backend_refresh_now,
            background_refresh::backend_sync_session_status_now,
            ai_refresh::backend_ai_refresh_now,
            ai_refresh::get_backend_ai_refresh_status,
            course_automation::course_automation_get,
            course_automation::course_automation_set_enabled,
            course_automation::course_automation_run_now,
            course_automation::course_automation_reanalyze_all,
            course_automation::course_automation_reanalyze_document,
            course_automation::course_automation_rebuild_memory,
            course_automation::course_automation_confirm_print,
            course_automation::course_automation_set_item_state,
            course_automation::course_automation_organize_now,
            course_automation::course_automation_undo_organize,
            commands::list_downloads,
            commands::scan_download_dir,
            commands::scan_duplicate_downloads,
            commands::cleanup_duplicate_downloads,
            commands::delete_downloaded_files,
            commands::check_files_downloaded,
            commands::open_downloaded_file,
            commands::open_downloaded_file_external,
            commands::share_downloaded_file_native,
            commands::share_downloaded_files_native,
            commands::get_download_preview,
            commands::open_markdown_file_window,
            commands::get_pending_markdown_payload,
            commands::write_markdown_file,
            commands::remove_download_record,
            commands::remove_download_records,
            commands::clear_download_history,
            commands::open_files_tab,
            commands::open_detective_tab,
            commands::open_paper_check_tab,
            tray::set_tray_status_items,
            tray::show_main_window,
            tray::show_main_agent_window,
            get_app_theme,
            set_app_theme,
            mark_notification_read,
            mark_batch_notification_read,
            get_read_notifications,
            get_data_cache,
            get_data_cache_updated_at,
            save_data_cache,
            request_app_restart,
            webview_toolbar::browser_go_back,
            webview_toolbar::browser_go_forward,
            webview_toolbar::browser_reload,
            webview_toolbar::browser_get_url,
            webview_toolbar::browser_navigate,
            webview_toolbar::browser_report_page_text,
            webview_toolbar::browser_report_action_result,
            webview_toolbar::debug_browser_mouse_selftest_report,
            agent_commands::agent_list_conversations,
            agent_commands::agent_create_conversation,
            agent_commands::agent_active_conversation,
            agent_commands::agent_set_active_conversation,
            agent_commands::agent_load_messages,
            agent_commands::agent_send,
            agent_commands::agent_send_with_context,
            agent_commands::agent_cancel,
            agent_commands::open_agent_popup,
            agent_commands::agent_delete_conversation,
            agent_commands::agent_rename_conversation,
            commands::open_subtitle_overlay,
            commands::close_subtitle_overlay,
            commands::subtitle_overlay_is_open,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                handle_run_event(app, event);
            })) {
                log::error!(
                    "run event cleanup panicked: {}",
                    run_event_panic_message(payload.as_ref())
                );
            }
        });
}

#[cfg(all(test, target_os = "macos"))]
mod dock_reopen_tests {
    use super::should_restore_on_dock_click;

    #[test]
    fn restores_when_no_window_is_on_screen() {
        assert!(should_restore_on_dock_click(false, false));
        assert!(should_restore_on_dock_click(true, false));
    }

    #[test]
    fn restores_when_appkit_reports_no_visible_windows() {
        assert!(should_restore_on_dock_click(false, true));
    }

    #[test]
    fn leaves_an_open_window_alone() {
        assert!(!should_restore_on_dock_click(true, true));
    }
}
