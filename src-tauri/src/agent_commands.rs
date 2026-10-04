//! Tauri commands for the Selah agent chat feature.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::agent;
use crate::ai::ImagePart;
use crate::db::{AgentConversationRow, AgentMessageRow, Database};

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
            tool_name: r.tool_name,
            tool_result,
            created_at: r.created_at,
        }
    }
}

#[tauri::command]
pub fn agent_list_conversations(
    db: State<'_, Database>,
) -> Result<Vec<AgentConversationSummary>, String> {
    Ok(db
        .agent_list_conversations()?
        .into_iter()
        .map(Into::into)
        .collect())
}

#[tauri::command]
pub fn agent_create_conversation(
    db: State<'_, Database>,
    title: Option<String>,
) -> Result<String, String> {
    let id = uuid_v4();
    let t = title.unwrap_or_else(|| "新しい会話".to_string());
    db.agent_create_conversation(&id, &t)?;
    Ok(id)
}

const ACTIVE_CONV_KEY: &str = "agent_active_conversation";

/// The globally-shared "current" agent conversation, used by both the sidebar
/// agent (across all tabs/pages) and the main-window agent so the chat stays
/// continuous across pages. Persisted so it survives restarts and is readable
/// from any webview as well as from backend (silent / scheduled) turns.
#[tauri::command]
pub fn agent_active_conversation(db: State<'_, Database>) -> Option<String> {
    db.get_data_cache(ACTIVE_CONV_KEY)
        .ok()
        .flatten()
        .map(|(value, _)| value)
        .filter(|v| !v.is_empty())
}

#[tauri::command]
pub fn agent_set_active_conversation(
    app: AppHandle,
    db: State<'_, Database>,
    conv_id: String,
) -> Result<(), String> {
    db.save_data_cache(ACTIVE_CONV_KEY, &conv_id)?;
    use tauri::Emitter;
    let _ = app.emit("agent-active-conversation-changed", conv_id);
    Ok(())
}

#[tauri::command]
pub fn agent_load_messages(
    db: State<'_, Database>,
    conv_id: String,
) -> Result<Vec<AgentMessageDto>, String> {
    Ok(db
        .agent_load_messages(&conv_id)?
        .into_iter()
        .map(Into::into)
        .collect())
}

#[tauri::command]
pub async fn agent_send(
    app: AppHandle,
    conv_id: String,
    content: String,
    images: Option<Vec<ImagePart>>,
) -> Result<(), String> {
    let content = content.trim().to_string();
    let imgs = images.unwrap_or_default();
    if content.is_empty() && imgs.is_empty() {
        return Err("メッセージが空です".into());
    }
    agent::agent_send(app, conv_id, content, imgs).await
}

#[tauri::command]
pub async fn agent_send_with_context(
    app: AppHandle,
    conv_id: String,
    content: String,
    images: Option<Vec<ImagePart>>,
    browser_target: Option<String>,
    page_title: Option<String>,
    page_kind: Option<String>,
) -> Result<(), String> {
    let content = content.trim().to_string();
    let imgs = images.unwrap_or_default();
    if content.is_empty() && imgs.is_empty() {
        return Err("メッセージが空です".into());
    }

    let browser_target = browser_target
        .as_deref()
        .map(str::trim)
        .filter(|target| !target.is_empty())
        .map(|target| crate::webview_toolbar::resolve_browser_target(&app, Some(target)))
        .transpose()?;

    agent::agent_send_with_context(
        app,
        conv_id,
        content,
        imgs,
        agent::AgentTurnContext {
            browser_target,
            browser_click_labels: Vec::new(),
            page_title: page_title
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            page_kind: page_kind
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            // Filled in agent_send_with_context from the live split-view panes.
            view_pane_targets: Vec::new(),
        },
    )
    .await
}

#[tauri::command]
pub fn agent_cancel(conv_id: String) {
    agent::cancel(&conv_id);
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

#[tauri::command]
pub fn agent_delete_conversation(db: State<'_, Database>, conv_id: String) -> Result<(), String> {
    db.agent_delete_conversation(&conv_id)
}

#[tauri::command]
pub fn agent_rename_conversation(
    app: AppHandle,
    db: State<'_, Database>,
    conv_id: String,
    title: String,
) -> Result<(), String> {
    db.agent_rename_conversation(&conv_id, &title)?;
    use tauri::Emitter;
    let _ = app.emit("agent-conversations-changed", conv_id);
    Ok(())
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
