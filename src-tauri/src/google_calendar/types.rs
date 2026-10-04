use serde::{Deserialize, Serialize};

pub(super) fn default_client_id() -> String {
    crate::embedded_keys::decode(&[
        0x44, 0x56, 0x54, 0x58, 0x5E, 0x1D, 0x5B, 0x40, 0x58, 0x57, 0x15, 0x1F, 0x46, 0x07, 0x0E,
        0x1B, 0x04, 0x1D, 0x14, 0x50, 0x1E, 0x53, 0x46, 0x59, 0x0A, 0x40, 0x46, 0x00, 0x5F, 0x5F,
        0x12, 0x17, 0x09, 0x0C, 0x58, 0x1F, 0x52, 0x4E, 0x0E, 0x57, 0x1E, 0x47, 0x5F, 0x56, 0x18,
        0x12, 0x15, 0x1C, 0x12, 0x46, 0x4A, 0x04, 0x18, 0x0E, 0x0F, 0x48, 0x47, 0x43, 0x57, 0x44,
        0x10, 0x0A, 0x02, 0x15, 0x0D, 0x43, 0x1F, 0x59, 0x0A, 0x0C, 0x40,
    ])
}

pub(super) fn default_client_secret() -> String {
    crate::embedded_keys::decode(&[
        0x34, 0x2A, 0x2F, 0x32, 0x38, 0x75, 0x46, 0x38, 0x0B, 0x2C, 0x1A, 0x59, 0x69, 0x7E, 0x5B,
        0x31, 0x0D, 0x5A, 0x0E, 0x12, 0x67, 0x28, 0x42, 0x0C, 0x56, 0x49, 0x73, 0x62, 0x5B, 0x51,
        0x43, 0x01, 0x03, 0x2E, 0x10,
    ])
}

/// Google Calendar OAuth settings.
/// Built-in credentials are used by default; users can override if needed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GoogleCalConfig {
    pub client_id: String,
    pub client_secret: String,
}

impl Default for GoogleCalConfig {
    fn default() -> Self {
        Self {
            client_id: default_client_id(),
            client_secret: default_client_secret(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenData {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}

/// Tracks which events we have synced.
/// event_map key: "YYYY-MM-DD-period" (e.g. "2026-04-07-3") — timetable sync only.
/// agent_event_map key: Google event ID — events created by the agent via
/// `create_google_calendar_event`. Stored separately so timetable sync never
/// touches them. Format of value: JSON-encoded `AgentEventMeta`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncState {
    pub calendar_id: String,
    pub event_map: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub agent_event_map: std::collections::HashMap<String, AgentEventMeta>,
}

/// Metadata stored locally for each agent-created calendar event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEventMeta {
    pub title: String,
    pub date: String,
    pub start_time: String,
    pub end_time: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarSyncEntry {
    pub day: String,
    pub period: i32,
    pub course_name: String,
    pub room: String,
    pub is_cancelled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleCalStatus {
    pub authenticated: bool,
    pub calendar_exists: bool,
    pub synced_events: usize,
}

pub struct OAuthLoginAttempt {
    pub url: String,
    pub verifier: String,
    pub redirect_uri: String,
    pub state: String,
}
