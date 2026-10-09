//! Register Agent sends synchronously at IPC receipt; process payload and input
//! persistence on the admitted blocking worker, before the async reply is polled.
use crate::agent::{self, AgentTurnContext};
use crate::agent_error::AgentError;
use crate::ai::ImagePart;
use serde::Deserialize;
use tauri::{
    ipc::{Invoke, InvokeBody},
    Manager,
};

#[derive(Deserialize)]
struct BorrowedImage<'a> {
    mime: &'a str,
    data_base64: &'a str,
}
#[derive(Deserialize)]
struct BorrowedDocument<'a> {
    name: &'a str,
    mime: &'a str,
    text: &'a str,
    size: usize,
    #[serde(default)]
    truncated: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendFields<'a> {
    conv_id: &'a str,
    content: &'a str,
    #[serde(default, borrow)]
    images: Option<Vec<BorrowedImage<'a>>>,
    #[serde(default, borrow)]
    documents: Option<Vec<BorrowedDocument<'a>>>,
    #[serde(default, borrow)]
    turn_id: Option<&'a str>,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextFields<'a> {
    #[serde(default, borrow)]
    browser_target: Option<&'a str>,
    #[serde(default, borrow)]
    page_title: Option<&'a str>,
    #[serde(default, borrow)]
    page_kind: Option<&'a str>,
}

fn validate(
    payload: &InvokeBody,
    context: bool,
) -> Result<(SendFields<'_>, ContextFields<'_>), String> {
    let InvokeBody::Json(value) = payload else {
        return Err("メッセージは JSON で送信してください".into());
    };
    let fields = SendFields::deserialize(value)
        .map_err(|error| format!("メッセージ形式が正しくありません: {error}"))?;
    let context = if context {
        ContextFields::deserialize(value)
            .map_err(|error| format!("ページ情報の形式が正しくありません: {error}"))?
    } else {
        ContextFields::default()
    };
    let count =
        fields.images.as_ref().map_or(0, Vec::len) + fields.documents.as_ref().map_or(0, Vec::len);
    if count > 4 {
        return Err("添付は最大4件までです".into());
    }
    if fields.documents.as_ref().is_some_and(|docs| {
        docs.iter().any(|d| {
            d.name.is_empty()
                || d.name.len() > 1020
                || d.size > crate::agent_attachments::MAX_BYTES
                || d.text.trim().is_empty()
                || d.text.len() > crate::agent_attachments::MAX_CHARS * 4
        })
    }) {
        return Err("添付ファイルの内容またはサイズが正しくありません".into());
    }
    if fields.content.trim().is_empty() && count == 0 {
        return Err("メッセージが空です".into());
    }
    Ok((fields, context))
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn decode(
    payload: &InvokeBody,
    context: bool,
    browser_target: Option<String>,
) -> Result<(String, Vec<ImagePart>, AgentTurnContext), AgentError> {
    let (fields, context) = validate(payload, context).map_err(AgentError::config)?;
    let text = fields.content.trim().to_owned();
    let documents: Vec<_> = fields
        .documents
        .unwrap_or_default()
        .into_iter()
        .map(|document| crate::agent_attachments::DocumentPart {
            name: document.name.into(),
            mime: document.mime.into(),
            size: document.size,
            text: document.text.into(),
            truncated: document.truncated,
        })
        .collect();
    crate::agent_attachments::validate_documents(&documents).map_err(AgentError::config)?;
    let images = fields
        .images
        .unwrap_or_default()
        .into_iter()
        .map(|image| ImagePart {
            mime: image.mime.to_owned(),
            data_base64: image.data_base64.to_owned(),
        })
        .collect();
    Ok((
        text,
        images,
        AgentTurnContext {
            documents,
            browser_target,
            page_title: nonempty(context.page_title).map(str::to_owned),
            page_kind: nonempty(context.page_kind).map(str::to_owned),
            ..Default::default()
        },
    ))
}

struct Control {
    conversation: String,
    request: Option<String>,
    browser_target: Option<String>,
    save: crate::pending_persistence::SavePermit,
}

fn prepare_control(
    payload: &InvokeBody,
    context: bool,
    resolve_target: impl FnOnce(&str) -> Result<String, String>,
    saves: &std::sync::Arc<crate::pending_persistence::PendingPersistence>,
) -> Result<Control, String> {
    // Borrow the content and every image; only small control strings become
    // owned on the IPC thread. Validate before reserving or replacing a turn.
    let (fields, page) = validate(payload, context)?;
    let browser_target = nonempty(page.browser_target)
        .map(resolve_target)
        .transpose()?;
    let save = saves.reserve()?;
    Ok(Control {
        conversation: fields.conv_id.to_owned(),
        request: fields.turn_id.map(str::to_owned),
        browser_target,
        save,
    })
}

/// Sends and ordered conversation mutations are admitted synchronously. All
/// other commands use the ordinary generated handler and keep their IPC names.
/// Tauri's public resolver schedules only completion; admission happens first.
pub(crate) fn admission_handler(
    other: impl Fn(Invoke) -> bool + Send + Sync + 'static,
) -> impl Fn(Invoke) -> bool + Send + Sync + 'static {
    move |invoke| {
        if let Some(kind) = super::mutations::Kind::from_command(invoke.message.command()) {
            return super::mutations::admit(kind, invoke);
        }
        let context = match invoke.message.command() {
            "agent_send" => false,
            "agent_send_with_context" => true,
            _ => return other(invoke),
        };
        let app = invoke.message.webview_ref().app_handle().clone();
        let control = prepare_control(
            invoke.message.payload(),
            context,
            |target| crate::webview_toolbar::resolve_browser_target(&app, Some(target)),
            &crate::pending_persistence::INPUT_SAVES,
        );
        let Control {
            conversation: conv_id,
            request: request_id,
            browser_target,
            save,
        } = match control {
            Ok(control) => control,
            Err(error) => {
                invoke.resolver.reject(error);
                return true;
            }
        };
        let message = invoke.message;
        let completion = agent::submit_rpc_turn(app, conv_id, request_id, save, move || {
            // The original IPC body is owned only by this decoder. The model
            // pipeline receives the decoded input, not the original message.
            decode(message.payload(), context, browser_target)
        });
        invoke
            .resolver
            .respond_async(async move { completion.await.map_err(Into::into) });
        true
    }
}

#[cfg(test)]
#[path = "submission/tests.rs"]
mod tests;
