#[cfg(all(feature = "stt-static", feature = "stt-shared"))]
compile_error!("features `stt-static` and `stt-shared` cannot be enabled together");

mod academic_period;
mod agent;
mod agent_attachments;
mod agent_commands;
mod agent_error;
mod agent_prompts;
mod agent_provider;
mod agent_pseudo_call;
mod agent_text;
mod agent_tools;
mod agent_turn_scope;
pub mod ai;
mod ai_refresh;
#[cfg(target_os = "macos")]
mod app_menu;
mod app_shutdown;
mod app_updates;
mod atomic_file;
mod auth;
mod background_ipc;
mod background_queue;
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
mod frontend_cache;
mod frontend_health;
mod google_calendar;
mod google_commands;
pub(crate) mod keychain;
mod kwic_client;
mod kwic_commands;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod latest_ui_mailbox;
mod live;
pub mod local_ai;
pub mod local_ai_support;
mod luna_client;
mod luna_commands;
mod luna_parser;
#[cfg(target_os = "macos")]
mod macos_fullscreen_exit;
#[cfg(target_os = "macos")]
mod macos_layer_transaction;
#[cfg(target_os = "macos")]
mod macos_native_agent;
#[cfg(target_os = "macos")]
mod macos_subtitle_overlay;
#[cfg(target_os = "macos")]
mod macos_widget_open;
mod mail;
mod mail_commands;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod main_thread_animation;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod native_agent_events;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod native_agent_state;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod native_agent_submission;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod native_capture;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod native_shortcut;
mod native_notification;
mod notifier;
#[cfg(any(target_os = "windows", all(target_os = "macos", test)))]
mod owned_ui_jobs;
mod paper_check;
mod parser;
mod pending_persistence;
mod power;
mod read_state;
mod stt;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod subtitle_events;
mod syllabus;
mod timetable;
mod tray;
mod webview_toolbar;
mod widget_bridge;
#[cfg(target_os = "windows")]
mod windows_native_agent;
#[cfg(target_os = "windows")]
mod windows_subtitle_overlay;

#[path = "app_lifecycle.rs"]
mod app_lifecycle;
#[path = "app_run.rs"]
mod app_run;
#[path = "app_state.rs"]
mod app_state;

pub use app_run::run;
pub use app_state::*;

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

#[cfg(test)]
#[path = "frontend_cache/legacy_commands.rs"]
mod cache_reply_legacy;
