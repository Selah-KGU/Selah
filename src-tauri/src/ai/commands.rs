use tauri::{Emitter, Manager};

use super::completion::chat_completion;
use super::config::{load_config, normalize_ai_config, save_config, AiConfig, ChatMessage};

// ============ Tauri Commands ============

#[tauri::command]
pub fn get_ai_config() -> AiConfig {
    load_config()
}

#[tauri::command]
pub fn get_local_ai_support() -> crate::local_ai_support::LocalAiSupport {
    crate::local_ai_support::current()
}

#[tauri::command]
pub fn save_ai_config(app: tauri::AppHandle, mut config: AiConfig) -> Result<(), String> {
    config.temperature = config.temperature.clamp(0.0, 2.0);
    config.api_key = config.api_key.trim().to_string();
    config.base_url = config.base_url.trim().to_string();
    config.model = config.model.trim().to_string();
    config.local_model = config.local_model.trim().to_string();
    normalize_ai_config(&mut config);

    // Validate based on provider
    match config.provider.as_str() {
        "local" => {
            crate::local_ai_support::ensure_supported()?;
            config.local_model = crate::local_ai_support::APPLE_INTELLIGENCE_MODEL_ID.into();
        }
        "openai" | "openrouter" | "deepseek" | "gemini" => {
            config.max_tokens = config.max_tokens.clamp(8192, 32768);
            if config.model.is_empty() {
                return Err("モデル名を入力してください".into());
            }
            if !config.base_url.is_empty()
                && !config.base_url.starts_with("https://")
                && !config.base_url.starts_with("http://localhost")
                && !config.base_url.starts_with("http://127.0.0.1")
            {
                return Err("Base URLは https:// で始まる必要があります".into());
            }
        }
        _ => return Err("不明なプロバイダーです".into()),
    }

    // If switching away from local, unload the model to free memory
    #[cfg(target_os = "macos")]
    if config.provider != "local" {
        crate::local_ai::unload_model();
    }

    let result = save_config(&config);
    if result.is_ok() {
        // Notify all windows that AI config changed
        let _ = app.emit("ai-config-changed", ());
        if let Some(state) = app.try_state::<crate::live::LiveState>() {
            state.notify_flush_driver();
        }
    }
    result
}

#[tauri::command]
pub async fn ai_chat(messages: Vec<ChatMessage>) -> Result<String, String> {
    let config = load_config();
    chat_completion(&config, messages).await
}

#[tauri::command]
pub async fn ai_test_connection() -> Result<String, String> {
    let config = load_config();
    let test_messages = vec![ChatMessage {
        role: "user".into(),
        content: "Reply OK in one word.".into(),
        images: Vec::new(),
    }];
    chat_completion(&config, test_messages).await
}

// ============ Local model management commands ============

#[tauri::command]
pub fn list_local_models() -> Vec<serde_json::Value> {
    let support = crate::local_ai_support::current();
    if !support.supported {
        return Vec::new();
    }
    let name = if support.model.is_empty() {
        "Apple Intelligence".to_string()
    } else {
        support.model
    };
    vec![serde_json::json!({
        "id": crate::local_ai_support::APPLE_INTELLIGENCE_MODEL_ID,
        "name": name,
        "size_label": "システム内蔵",
        "param_size": "on-device",
        "file_size_mb": 0,
        "downloaded": true,
    })]
}

/// Send a native notification.
pub fn send_native_notification(
    app: &tauri::AppHandle,
    title: &str,
    body: &str,
) -> Result<String, String> {
    crate::native_notification::send_native_notification(app, title, body)
}

/// Debug-only test notification that bypasses notify-rust.
/// Uses osascript on macOS for reliable delivery even in dev mode.
#[tauri::command]
pub async fn debug_test_notification(title: String, body: String) -> Result<String, String> {
    log::info!("debug_test_notification: title={}, body={}", title, body);
    #[cfg(target_os = "macos")]
    {
        std::thread::spawn(move || {
            let script = format!(
                "display notification \"{}\" with title \"{}\"",
                body.replace('\\', "\\\\").replace('"', "\\\""),
                title.replace('\\', "\\\\").replace('"', "\\\""),
            );
            match std::process::Command::new("osascript")
                .arg("-e")
                .arg(&script)
                .output()
            {
                Ok(out) if !out.status.success() => {
                    log::warn!(
                        "osascript notification failed: {}",
                        String::from_utf8_lossy(&out.stderr)
                    );
                }
                Err(e) => log::warn!("osascript spawn failed: {}", e),
                _ => {}
            }
        });
        Ok("Notification sent".to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("debug_test_notification: use test_notification on non-macOS".to_string())
    }
}
