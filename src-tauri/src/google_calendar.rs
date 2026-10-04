use reqwest::Client;

#[path = "google_calendar/agent.rs"]
mod agent;
#[path = "google_calendar/config.rs"]
mod config;
#[path = "google_calendar/oauth.rs"]
mod oauth;
#[path = "google_calendar/sync.rs"]
mod sync;
#[path = "google_calendar/types.rs"]
mod types;

#[cfg(test)]
pub(crate) use config::{default_client_id_for_test, default_client_secret_for_test};
pub use config::{load_config, resolve_with_defaults, save_config};
pub use types::{
    AgentEventMeta, CalendarSyncEntry, GoogleCalConfig, GoogleCalStatus, OAuthLoginAttempt,
    SyncState, TokenData,
};

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GCAL_API_BASE: &str = "https://www.googleapis.com/calendar/v3";
// Reads and writes both stay confined to the app's own "Selah 時間割"
// calendar — the agent never touches the user's other (primary) calendars.
const SCOPES: &str = "https://www.googleapis.com/auth/calendar.app.created";
const TOKEN_FILE: &str = "google_calendar_token.json";
const SYNC_STATE_FILE: &str = "google_calendar_sync.json";
const CONFIG_FILE: &str = "google_calendar_config.json";
const CALENDAR_SUMMARY: &str = "Selah 時間割";

use config::{load_sync_state, sync_state_path, token_path};

pub struct GoogleCalendarClient {
    http: Client,
    pub token: Option<TokenData>,
    pub config: GoogleCalConfig,
    pub sync_state: SyncState,
}

impl GoogleCalendarClient {
    pub fn new() -> Self {
        let http = Client::builder()
            .user_agent(crate::client::USER_AGENT)
            .build()
            .expect("failed to build Google Calendar HTTP client");
        Self {
            http,
            token: None,
            config: load_config(),
            sync_state: load_sync_state(),
        }
    }

    pub fn try_restore_token(&mut self) {
        // Prefer keychain
        if let Some(json) = crate::keychain::get_secret("gcal_token") {
            if let Ok(token) = serde_json::from_str::<TokenData>(&json) {
                log::info!("Restored Google Calendar token from keychain");
                self.token = Some(token);
                return;
            }
        }
        // Legacy file migration
        let path = token_path();
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(token) = serde_json::from_str::<TokenData>(&data) {
                log::info!("Migrating Google Calendar token from file to keychain");
                self.token = Some(token);
                self.save_token();
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    pub fn save_token(&self) {
        if let Some(ref token) = self.token {
            if let Ok(json) = serde_json::to_string(token) {
                if let Err(e) = crate::keychain::set_secret("gcal_token", &json) {
                    log::warn!("Failed to save Google Calendar token to keychain: {}", e);
                }
            }
        }
    }

    pub fn clear_token(&mut self) {
        self.token = None;
        crate::keychain::delete_secret("gcal_token");
        let _ = std::fs::remove_file(token_path()); // clean up legacy file
    }

    pub fn is_authenticated(&self) -> bool {
        self.token.is_some()
    }

    pub fn status(&self) -> GoogleCalStatus {
        GoogleCalStatus {
            authenticated: self.is_authenticated(),
            calendar_exists: !self.sync_state.calendar_id.is_empty(),
            synced_events: self.sync_state.event_map.len(),
        }
    }

    pub fn disconnect(&mut self) {
        self.clear_token();
        self.sync_state = SyncState::default();
        let _ = std::fs::remove_file(sync_state_path());
    }
}
