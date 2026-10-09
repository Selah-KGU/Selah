use crate::oauth_http::Http;

#[path = "google_calendar/agent.rs"]
mod agent;
#[path = "google_calendar/binding.rs"]
mod binding;
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
    AgentEventMeta, AutoSyncBinding, CalendarSyncEntry, GoogleCalConfig, GoogleCalStatus,
    OAuthLoginAttempt, SyncState, TokenData,
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
    http: Http,
    pub(crate) lifecycle: crate::oauth_lifecycle::Lifecycle,
    new_login: bool,
    logout_requested: bool,
    pub token: Option<TokenData>,
    pub config: GoogleCalConfig,
    pub(crate) config_error: Option<crate::keychain::StoreError>,
    pub sync_state: SyncState,
}

impl GoogleCalendarClient {
    pub fn new() -> Self {
        let http = Http::new();
        let (config, config_error) = match load_config() {
            Ok(config) => (config, None),
            Err(error) => (
                GoogleCalConfig {
                    client_id: String::new(),
                    client_secret: String::new(),
                },
                Some(error),
            ),
        };
        Self {
            http,
            lifecycle: Default::default(),
            new_login: false,
            logout_requested: false,
            token: None,
            config,
            config_error,
            sync_state: load_sync_state(),
        }
    }

    pub(crate) fn ensure_config(&self) -> Result<(), crate::keychain::StoreError> {
        self.config_error
            .as_ref()
            .map_or(Ok(()), |error| Err(error.clone()))
    }
    pub(crate) fn reload_config(&mut self) -> Result<(), crate::keychain::StoreError> {
        match load_config() {
            Ok(config) => {
                if (self.config_error.is_none() || self.token.is_some())
                    && (config.client_id != self.config.client_id
                        || config.client_secret != self.config.client_secret)
                {
                    self.disconnect()
                        .map_err(|e| crate::keychain::StoreError::new("write_failed", e))?;
                }
                self.config = config;
                self.config_error = None;
                Ok(())
            }
            Err(error) => {
                self.config_error = Some(error.clone());
                Err(error)
            }
        }
    }

    pub fn try_restore_token(&mut self) -> Result<bool, crate::keychain::StoreError> {
        self.ensure_config()?;
        if self.logout_requested {
            crate::keychain::tokens::revoke("gcal_token", &token_path())?;
            return Ok(false);
        }
        if let Some(mut token) =
            crate::keychain::tokens::restore::<TokenData>("gcal_token", &token_path())?
        {
            if token.connection_id.is_empty() {
                token.connection_id = uuid::Uuid::new_v4().to_string();
            }
            self.token = Some(token);
            self.lifecycle.published();
            self.save_token()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn save_token(&self) -> Result<(), crate::keychain::StoreError> {
        if self.logout_requested {
            return crate::keychain::tokens::revoke("gcal_token", &token_path());
        }
        crate::keychain::tokens::save(
            "gcal_token",
            &token_path(),
            self.token.as_ref(),
            self.new_login,
        )
    }

    pub(crate) fn cancellation(&self) -> crate::oauth_http::Cancellation {
        self.http.cancellation()
    }

    pub(crate) fn cancel_requests(&mut self) {
        self.lifecycle.invalidate();
        self.http.cancellation().cancel();
        self.http.renew();
    }

    pub(crate) fn retire(&mut self) {
        self.cancel_requests();
        self.token = None;
        self.new_login = false;
        self.logout_requested = false;
    }

    pub fn clear_token(&mut self) -> Result<(), crate::keychain::StoreError> {
        self.retire();
        self.logout_requested = true;
        crate::keychain::tokens::revoke("gcal_token", &token_path())
    }

    pub fn is_authenticated(&self) -> bool {
        self.token.is_some()
    }

    pub fn status(&self) -> GoogleCalStatus {
        let account = crate::session_coordinator::SESSIONS.account_context();
        GoogleCalStatus {
            authenticated: self.is_authenticated(),
            calendar_exists: !self.sync_state.calendar_id.is_empty(),
            synced_events: self.sync_state.event_map.len(),
            calendar_id: self.sync_state.calendar_id.clone(),
            auto_sync_ready: self.auto_sync_allowed(&account),
        }
    }

    pub fn disconnect(&mut self) -> Result<(), String> {
        let result = self.clear_token();
        self.sync_state = SyncState::default();
        let _ = std::fs::remove_file(sync_state_path());
        result.map_err(Into::into)
    }
}
