//! Tauri commands for the Selah agent chat feature.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::agent;
use crate::ai::ImagePart;
use crate::db::{AgentConversationRow, AgentMessageRow, Database};

#[path = "agent_commands/history.rs"]
mod history;
#[path = "agent_commands/lifecycle.rs"]
mod lifecycle;
#[path = "agent_commands/mutations.rs"]
mod mutations;
#[path = "agent_commands/submission.rs"]
mod submission;
#[path = "agent_commands/worker.rs"]
mod worker;
pub(crate) use mutations::{drain_mutations, reopen_mutations, seal_mutations};
pub(crate) use submission::admission_handler;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConversationSummary {
    pub id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<AgentConversationRow> for AgentConversationSummary {
    fn from(r: AgentConversationRow) -> Self {
        Self {
            id: r.id,
            title: r.title,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessageDto {
    pub id: i64,
    pub conv_id: String,
    pub role: String,
    pub content: String,
    pub images: Option<Vec<ImagePart>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub documents: Vec<crate::agent_attachments::DocumentPart>,
    pub tool_name: Option<String>,
    pub tool_result: Option<serde_json::Value>,
    pub created_at: i64,
}

impl From<AgentMessageRow> for AgentMessageDto {
    fn from(r: AgentMessageRow) -> Self {
        let images = r
            .images_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<Vec<ImagePart>>(s).ok());
        let tool_result = r
            .tool_result_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
        Self {
            id: r.id,
            conv_id: r.conv_id,
            role: r.role,
            content: r.content,
            images,
            documents: Vec::new(),
            tool_name: r.tool_name,
            tool_result,
            created_at: r.created_at,
        }
    }
}

#[tauri::command]
pub async fn agent_list_conversations(app: AppHandle) -> Result<tauri::ipc::Response, String> {
    worker::with_database(app, |db| {
        let rows = db
            .agent_list_conversations()?
            .into_iter()
            .map(Into::into)
            .collect::<Vec<AgentConversationSummary>>();
        worker::json_response(&rows)
    })
    .await
}

/// The globally-shared "current" agent conversation, used by both the sidebar
/// agent (across all tabs/pages) and the main-window agent so the chat stays
/// continuous across pages. Persisted so it survives restarts and is readable
/// from any webview as well as from backend (silent / scheduled) turns.
#[tauri::command]
pub async fn agent_active_conversation(app: AppHandle) -> Result<tauri::ipc::Response, String> {
    worker::with_database(app, |db| {
        worker::json_response(&db.agent_active_conversation()?)
    })
    .await
}

#[tauri::command]
pub async fn agent_load_messages(
    app: AppHandle,
    conv_id: String,
) -> Result<tauri::ipc::Response, String> {
    worker::with_database(app, move |db| history::load_full_response(db, &conv_id)).await
}

/// Displayed messages only; SQLite, attachment decoding and JSON encoding run
/// off the IPC thread. The existing full-history command remains available to older callers.
#[tauri::command]
pub async fn agent_load_display_messages(
    app: AppHandle,
    conv_id: String,
) -> Result<tauri::ipc::Response, String> {
    worker::with_database(app, move |db| history::load_display_response(db, &conv_id)).await
}

#[tauri::command]
pub fn agent_cancel(conv_id: String, turn_id: Option<String>) {
    agent::cancel_request(&conv_id, turn_id.as_deref());
}

// Async so it runs off the event-loop thread: open_agent_workspace creates child
// webviews, which deadlocks in a sync command on Windows (see
// document_tabs::ensure_window).
#[tauri::command]
pub async fn open_agent_popup(
    app: AppHandle,
    owner_label: Option<String>,
    target: Option<String>,
    title: Option<String>,
    kind: Option<String>,
) -> Result<(), String> {
    if owner_label.as_deref().is_some() || target.as_deref().is_some() {
        let _ = (title, kind);
        return crate::document_tabs::open_agent_workspace(&app);
    }

    const LABEL: &str = "agent-popup";
    if let Some(win) = app.get_webview_window(LABEL) {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(());
    }

    let builder = tauri::WebviewWindowBuilder::new(
        &app,
        LABEL,
        tauri::WebviewUrl::App(
            "index.html#surface=agent-panel&owner=agent-popup&target=agent-popup&title=%E3%82%A8%E3%83%BC%E3%82%B8%E3%82%A7%E3%83%B3%E3%83%88&kind=agent"
                .into(),
        ),
    )
    .title("Agent")
    .inner_size(420.0, 620.0)
    .min_inner_size(340.0, 420.0)
    .resizable(true);

    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);

    builder
        .build()
        .map(|_| ())
        .map_err(|e| format!("Agent popup を開けませんでした: {}", e))
}

/// Minimal UUIDv4 generator (no new dependency). Uses `rand`, already a
/// direct dependency.
fn uuid_v4() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant RFC4122
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    )
}
