//! Tauri commands for the Selah agent chat feature.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

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
        Self { id: r.id, title: r.title, created_at: r.created_at, updated_at: r.updated_at }
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
        let images = r.images_json.as_deref()
            .and_then(|s| serde_json::from_str::<Vec<ImagePart>>(s).ok());
        let tool_result = r.tool_result_json.as_deref()
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
    Ok(db.agent_list_conversations()?.into_iter().map(Into::into).collect())
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

#[tauri::command]
pub fn agent_load_messages(
    db: State<'_, Database>,
    conv_id: String,
) -> Result<Vec<AgentMessageDto>, String> {
    Ok(db.agent_load_messages(&conv_id)?.into_iter().map(Into::into).collect())
}

#[tauri::command]
pub async fn agent_send(
    app: AppHandle,
    conv_id: String,
    content: String,
    images: Option<Vec<ImagePart>>,
) -> Result<(), String> {
    let imgs = images.unwrap_or_default();
    agent::agent_send(app, conv_id, content, imgs).await
}

#[tauri::command]
pub fn agent_cancel(conv_id: String) {
    agent::cancel(&conv_id);
}

#[tauri::command]
pub fn agent_delete_conversation(
    db: State<'_, Database>,
    conv_id: String,
) -> Result<(), String> {
    db.agent_delete_conversation(&conv_id)
}

#[tauri::command]
pub fn agent_rename_conversation(
    db: State<'_, Database>,
    conv_id: String,
    title: String,
) -> Result<(), String> {
    db.agent_rename_conversation(&conv_id, &title)
}

/// Minimal UUIDv4 generator (no new dependency).  Uses `rand` (already a
/// transitive dep via llama-cpp-2 features).
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
