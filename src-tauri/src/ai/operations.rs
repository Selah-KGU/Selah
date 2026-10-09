//! Order config preparation at IPC admission, independently of model inference.
use super::config::{load_config, AiConfig, ChatMessage};
use crate::background_queue::Queue;
use serde::Deserialize;
use std::future::Future;
use std::sync::{Arc, LazyLock};
use tauri::ipc::{Invoke, InvokeBody, Response};
use tauri::Manager;

const FAILURE: &str = "AI設定処理失敗";
static QUEUE: LazyLock<Arc<Queue>> = LazyLock::new(|| Arc::new(Queue::new(FAILURE)));

pub(crate) fn seal_operations() {
    QUEUE.seal();
}
pub(crate) async fn drain_operations() {
    QUEUE.drained().await;
}
pub(crate) fn reopen_operations() {
    QUEUE.reopen();
}

#[derive(Clone, Copy)]
enum Kind {
    Read,
    Save,
    Chat,
    Test,
}
impl Kind {
    fn from_command(command: &str) -> Option<Self> {
        match command {
            "get_ai_config" => Some(Self::Read),
            "save_ai_config" => Some(Self::Save),
            "ai_chat" => Some(Self::Chat),
            "ai_test_connection" => Some(Self::Test),
            _ => None,
        }
    }
}

enum Request {
    Read,
    Save(AiConfig),
    Infer(Vec<ChatMessage>),
}
enum Prepared {
    Reply(Response),
    Infer(AiConfig, Vec<ChatMessage>),
}

fn decode(kind: Kind, body: &InvokeBody) -> Result<Request, String> {
    // No-argument commands keep accepting unused payload fields.
    match kind {
        Kind::Read => return Ok(Request::Read),
        Kind::Test => {
            return Ok(Request::Infer(vec![ChatMessage {
                role: "user".into(),
                content: "Reply OK in one word.".into(),
                images: Vec::new(),
            }]))
        }
        _ => {}
    }
    let InvokeBody::Json(value) = body else {
        return Err("AIの命令は JSON で送信してください".into());
    };
    let malformed = |error| format!("AIの命令形式が正しくありません: {error}");
    match kind {
        Kind::Save => {
            #[derive(Deserialize)]
            struct Fields {
                config: AiConfig,
            }
            Ok(Request::Save(
                Fields::deserialize(value).map_err(malformed)?.config,
            ))
        }
        Kind::Chat => {
            #[derive(Deserialize)]
            struct Fields {
                messages: Vec<ChatMessage>,
            }
            Ok(Request::Infer(
                Fields::deserialize(value).map_err(malformed)?.messages,
            ))
        }
        _ => unreachable!(),
    }
}

fn submit(
    queue: &Arc<Queue>,
    decode: impl FnOnce() -> Result<Request, String> + Send + 'static,
    load: impl FnOnce() -> AiConfig + Send + 'static,
    save: impl FnOnce(AiConfig) -> Result<(), String> + Send + 'static,
) -> impl Future<Output = Result<Prepared, String>> + Send + 'static {
    // Queue submission is synchronous; IO and ownership conversion are not.
    queue.submit(move || match decode()? {
        Request::Read => {
            crate::background_ipc::json_response("AI設定の変換に失敗しました", &load())
                .map(Prepared::Reply)
        }
        Request::Save(config) => {
            let config = super::commands::validate_config(config)?;
            save(config)?;
            Ok(Prepared::Reply(Response::new("null".to_string())))
        }
        Request::Infer(messages) => Ok(Prepared::Infer(load(), messages)),
    })
}

async fn finish<F: Future<Output = Result<String, String>>>(
    prepared: Prepared,
    infer: impl FnOnce(AiConfig, Vec<ChatMessage>) -> F,
) -> Result<Response, String> {
    match prepared {
        Prepared::Reply(response) => Ok(response),
        Prepared::Infer(config, messages) => {
            let text = infer(config, messages).await?;
            crate::background_ipc::respond(
                "AI応答処理失敗",
                "AI応答の変換に失敗しました",
                move || Ok(text),
            )
            .await
        }
    }
}

pub(crate) fn admission_handler(
    other: impl Fn(Invoke) -> bool + Send + Sync + 'static,
) -> impl Fn(Invoke) -> bool + Send + Sync + 'static {
    move |invoke| {
        let Some(kind) = Kind::from_command(invoke.message.command()) else {
            return other(invoke);
        };
        let app = invoke.message.webview_ref().app_handle().clone();
        let message = invoke.message;
        let prepared = submit(
            &QUEUE,
            move || decode(kind, message.payload()),
            load_config,
            move |config| super::commands::persist_config(&app, config),
        );
        invoke.resolver.respond_async(async move {
            let prepared = prepared.await.map_err(tauri::ipc::InvokeError::from)?;
            finish(prepared, |config, messages| async move {
                super::completion::chat_completion(&config, messages).await
            })
            .await
            .map_err(Into::into)
        });
        true
    }
}

#[cfg(test)]
#[path = "operations/tests.rs"]
mod tests;
