use super::*;
use crate::ai::{AiConfig, ChatMessage};
use serde_json::{json, Value};
use std::sync::Arc;

pub(super) fn request_body(model: &str, messages: &[ChatMessage], json_mode: bool) -> Value {
    let input: Vec<Value> = messages.iter().map(|message| {
        let role = match message.role.as_str() {
            "system" | "developer" => "developer",
            "assistant" => "assistant",
            _ => "user",
        };
        if message.images.is_empty() || role != "user" {
            json!({"role": role, "content": message.content})
        } else {
            let mut content = vec![json!({"type":"input_text", "text":message.content})];
            content.extend(message.images.iter().map(|image| json!({
                "type":"input_image", "image_url":format!("data:{};base64,{}", image.mime, image.data_base64)
            })));
            json!({"role":role, "content":content})
        }
    }).collect();
    let mut body = json!({"model":model, "input":input, "store":false, "stream":true});
    if json_mode {
        body["text"] = json!({"format":{"type":"json_object"}});
    }
    body
}

#[derive(Default)]
pub(super) struct Output {
    pub text: String,
    pub completed: bool,
    pub error: Option<String>,
}
impl Output {
    pub fn event(&mut self, data: &str, on_chunk: &mut impl FnMut(&str, bool)) -> bool {
        let event: Value = match serde_json::from_str(data) {
            Ok(value) => value,
            Err(_) => {
                self.error = Some("ChatGPT のストリーム応答が正しくありません".into());
                return false;
            }
        };
        match event.get("type").and_then(Value::as_str).unwrap_or("") {
            "response.output_text.delta" | "response.refusal.delta" => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    self.text.push_str(delta);
                    on_chunk(delta, false);
                }
            }
            "response.completed" => {
                self.completed =
                    event.pointer("/response/status").and_then(Value::as_str) == Some("completed");
                return false;
            }
            "response.failed" | "error" => {
                let body = event.get("response").unwrap_or(&event);
                self.error = Some(request_error(reqwest::StatusCode::BAD_REQUEST, body));
                return false;
            }
            "response.incomplete" => {
                self.error =
                    Some("ChatGPT の応答が途中で終了しました。もう一度お試しください。".into());
                return false;
            }
            _ => {}
        }
        true
    }
    pub fn finish(self) -> Result<String, String> {
        if let Some(error) = self.error {
            return Err(error);
        }
        if !self.completed {
            return Err("ChatGPT の接続が応答完了前に切断されました。再試行してください。".into());
        }
        if self.text.trim().is_empty() {
            return Err("ChatGPT からテキスト応答がありませんでした".into());
        }
        Ok(self.text)
    }
}

pub(crate) async fn complete(
    config: &AiConfig,
    messages: Arc<Vec<ChatMessage>>,
    generation: &str,
    json_mode: bool,
    mut on_chunk: impl FnMut(&str, bool) + Send,
) -> Result<String, String> {
    let mut session = auth::session();
    tokio::select! {
        biased;
        _ = session.changed() => Err("ChatGPT アカウントが変更されたため推論を中止しました".into()),
        result = async {
            let token = auth::access_token().await?;
            let model = config.model.clone();
            // Encoding large contexts and attachments must not block the runtime.
            let request = crate::background_ipc::run("ChatGPT リクエスト準備失敗", move || {
                Ok(HTTP.post(format!("{RESOURCE}/responses")).bearer_auth(token)
                    .json(&request_body(&model, &messages, json_mode)))
            }).await?;
            let response = request.send().await.map_err(|_| "ChatGPT に接続できませんでした。接続を確認してください。")?;
            if !response.status().is_success() {
                let status = response.status();
                let body = response.json::<Value>().await.unwrap_or(Value::Null);
                return Err(request_error(status, &body));
            }
            let mut output = Output::default();
            crate::agent_provider::sse::receive(response, generation, |data| output.event(data, &mut on_chunk)).await?;
            output.finish()
        } => result,
    }
}
