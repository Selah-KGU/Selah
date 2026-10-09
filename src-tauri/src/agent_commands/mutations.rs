use super::{lifecycle, uuid_v4, AppHandle, Database, Manager};
use crate::background_queue::Queue;
use serde::Deserialize;
use serde_json::Value;
use std::sync::{Arc, LazyLock};
use tauri::ipc::{Invoke, InvokeBody};
use tauri::Emitter;

static QUEUE: LazyLock<Arc<Queue>> =
    LazyLock::new(|| Arc::new(Queue::new("会話の処理に失敗しました")));

pub(crate) fn seal_mutations() {
    QUEUE.seal();
}
pub(crate) async fn drain_mutations() {
    QUEUE.drained().await;
}
pub(crate) fn reopen_mutations() {
    QUEUE.reopen();
}

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Create,
    Select,
    Rename,
    Delete,
}
impl Kind {
    pub(super) fn from_command(command: &str) -> Option<Self> {
        match command {
            "agent_create_conversation" => Some(Self::Create),
            "agent_set_active_conversation" => Some(Self::Select),
            "agent_rename_conversation" => Some(Self::Rename),
            "agent_delete_conversation" => Some(Self::Delete),
            _ => None,
        }
    }
}

enum Mutation {
    Create(Option<String>),
    Select(String),
    Rename(String, String),
    Delete(String),
}
#[derive(Deserialize)]
struct CreateFields {
    title: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConversationFields {
    conv_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenameFields {
    conv_id: String,
    title: String,
}

fn decode(kind: Kind, body: &InvokeBody) -> Result<Mutation, String> {
    let InvokeBody::Json(value) = body else {
        return Err("会話の命令は JSON で送信してください".into());
    };
    let decode_error = |error| format!("会話の命令形式が正しくありません: {error}");
    Ok(match kind {
        Kind::Create => Mutation::Create(
            CreateFields::deserialize(value)
                .map_err(decode_error)?
                .title,
        ),
        Kind::Select => Mutation::Select(
            ConversationFields::deserialize(value)
                .map_err(decode_error)?
                .conv_id,
        ),
        Kind::Delete => Mutation::Delete(
            ConversationFields::deserialize(value)
                .map_err(decode_error)?
                .conv_id,
        ),
        Kind::Rename => {
            let fields = RenameFields::deserialize(value).map_err(decode_error)?;
            Mutation::Rename(fields.conv_id, fields.title)
        }
    })
}

pub(super) fn admit(kind: Kind, invoke: Invoke) -> bool {
    let mutation = match decode(kind, invoke.message.payload()) {
        Ok(mutation) => mutation,
        Err(error) => {
            invoke.resolver.reject(error);
            return true;
        }
    };
    let app: AppHandle = invoke.message.webview_ref().app_handle().clone();
    // The job is queued synchronously before the completion future is polled.
    // Reads and model workers are independent of this small mutation lane.
    let completion = QUEUE.submit(move || {
        apply(&app.state::<Database>(), mutation, |name, id| {
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            if name == "agent-conversation-deleted" {
                crate::native_agent_events::conversation_deleted(&app, id);
            }
            let _ = app.emit(name, id);
        })
    });
    invoke
        .resolver
        .respond_async(async move { completion.await.map_err(Into::into) });
    true
}

fn apply(
    db: &Database,
    mutation: Mutation,
    mut event: impl FnMut(&str, &str),
) -> Result<Value, String> {
    match mutation {
        Mutation::Create(title) => {
            let id = uuid_v4();
            db.agent_create_conversation(&id, &title.unwrap_or_else(|| "新しい会話".into()))?;
            return Ok(Value::String(id));
        }
        Mutation::Select(id) => {
            if db.agent_set_active_conversation(&id)? {
                event("agent-active-conversation-changed", &id);
            }
        }
        Mutation::Rename(id, title) => {
            db.agent_rename_conversation(&id, &title)?;
            event("agent-conversations-changed", &id);
        }
        Mutation::Delete(id) => {
            lifecycle::delete_conversation(db, &id)?;
            event("agent-conversation-deleted", &id);
            event("agent-conversations-changed", &id);
        }
    }
    Ok(Value::Null)
}

#[cfg(test)]
#[path = "mutations_tests.rs"]
mod tests;
