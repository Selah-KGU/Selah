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
mod frontend_cache;
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
