//! Preserve cache IPC admission order while moving DB waits and JSON off-thread.
use crate::{background_ipc, background_queue::Queue, db::Database, read_state};
use serde::{Deserialize, Deserializer};
use std::sync::{Arc, LazyLock};
use tauri::ipc::{CommandArg, CommandItem, Invoke, InvokeError, Response};
use tauri::{Manager, Runtime};

const FAILURE: &str = "キャッシュ処理失敗";
const ENCODING_FAILURE: &str = "キャッシュ応答の変換に失敗しました";
static QUEUE: LazyLock<Arc<Queue>> = LazyLock::new(|| Arc::new(Queue::new(FAILURE)));

// CommandItem's Value deserializer lends the input string. Keep String's
// existing type-error wording without cloning multi-megabyte save arguments.
struct CacheArgument<'a>(&'a str);
impl<'de> Deserialize<'de> for CacheArgument<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = CacheArgument<'de>;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a string")
            }
            fn visit_borrowed_str<E: serde::de::Error>(
                self,
                value: &'de str,
            ) -> Result<Self::Value, E> {
                Ok(CacheArgument(value))
            }
        }
        deserializer.deserialize_string(Visitor)
    }
}

pub(crate) fn seal_cache() {
    QUEUE.seal();
}
pub(crate) async fn drain_cache() {
    QUEUE.drained().await;
}
pub(crate) fn reopen_cache() {
    QUEUE.reopen();
}

#[derive(Clone, Copy)]
enum Kind {
    Read,
    Timestamp,
    Save,
    ReadIds,
    MarkRead,
    MarkBatchRead,
}
impl Kind {
    fn from_command(command: &str) -> Option<Self> {
        match command {
            "get_data_cache" => Some(Self::Read),
            "get_data_cache_updated_at" => Some(Self::Timestamp),
            "save_data_cache" => Some(Self::Save),
            "get_read_notifications" => Some(Self::ReadIds),
            "mark_notification_read" => Some(Self::MarkRead),
            "mark_batch_notification_read" => Some(Self::MarkBatchRead),
            _ => None,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Read => "get_data_cache",
            Self::Timestamp => "get_data_cache_updated_at",
            Self::Save => "save_data_cache",
            Self::ReadIds => "get_read_notifications",
            Self::MarkRead => "mark_notification_read",
            Self::MarkBatchRead => "mark_batch_notification_read",
        }
    }
}

fn apply<'a>(
    db: &Database,
    kind: Kind,
    mut argument: impl FnMut(&'static str) -> Result<&'a str, InvokeError>,
    ids: impl FnOnce() -> Result<Vec<CacheArgument<'a>>, InvokeError>,
) -> Result<Response, InvokeError> {
    let result = match kind {
        Kind::Read => {
            let key = argument("key")?;
            background_ipc::json_response(ENCODING_FAILURE, &db.cache_payload(key))
        }
        Kind::Timestamp => {
            let key = argument("key")?;
            background_ipc::json_response(ENCODING_FAILURE, &db.cache_updated_at(key))
        }
        Kind::Save => {
            let key = argument("key")?;
            // Decode both arguments before checking the reserved namespace,
            // just as the generated command wrapper did. Borrow large strings.
            let json = argument("json")?;
            if key.starts_with("seen_notifs_") {
                return Err("reserved cache key".into());
            }
            db.save_data_cache(key, json)?;
            Ok(Response::new("null".to_string()))
        }
        Kind::ReadIds => {
            background_ipc::json_response(ENCODING_FAILURE, &read_state::get_all_read_ids(db)?)
        }
        Kind::MarkRead => {
            let source = argument("source")?;
            let id = argument("id")?;
            read_state::mark_read(db, source, id)?;
            Ok(Response::new("null".to_string()))
        }
        Kind::MarkBatchRead => {
            let source = argument("source")?;
            read_state::mark_batch_read(db, source, ids()?.into_iter().map(|argument| argument.0))?;
            Ok(Response::new("null".to_string()))
        }
    };
    result.map_err(Into::into)
}

pub(crate) fn admission_handler<R: Runtime>(
    other: impl Fn(Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    handler(QUEUE.clone(), other)
}

fn handler<R: Runtime>(
    queue: Arc<Queue>,
    other: impl Fn(Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    move |invoke| {
        let Some(kind) = Kind::from_command(invoke.message.command()) else {
            return other(invoke);
        };
        let app = invoke.message.webview_ref().app_handle().clone();
        let message = invoke.message;
        let acl = invoke.acl;
        // Submit before polling the response. Later cache reads see prior
        // cache writes even if replies are dropped or polled in reverse order.
        let completion = queue.submit(move || {
            let item = |key| CommandItem {
                plugin: None,
                name: kind.name(),
                key,
                message: &message,
                acl: &acl,
            };
            Ok(apply(
                &app.state::<Database>(),
                kind,
                |key| CacheArgument::from_command(item(key)).map(|argument| argument.0),
                || Vec::<CacheArgument>::from_command(item("ids")),
            ))
        });
        invoke
            .resolver
            .respond_async(async move { completion.await.map_err(InvokeError::from)? });
        true
    }
}

#[cfg(test)]
#[path = "cache/tests.rs"]
mod tests;
