use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;
use tauri::Emitter;
use tauri::Manager;

/// Shared HTTP client — reuses connection pool across all AI calls.
static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("failed to build HTTP client")
});

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    pub ai_enabled: bool,
    pub provider: String,    // "local" | "openai" | "gemini"
    pub local_model: String, // model id from catalog, e.g. "qwen3.5-2b"
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub reply_language: String,
    /// Auto-refresh interval for AI analysis in minutes (60..1440, 0 = disabled)
    pub ai_refresh_interval: u32,
    #[serde(default = "default_live_summary_interval_minutes")]
    pub live_summary_interval_minutes: u32,
}

fn default_live_summary_interval_minutes() -> u32 {
    5
}

fn normalize_ai_config(config: &mut AiConfig) {
    config.live_summary_interval_minutes = config.live_summary_interval_minutes.clamp(5, 30);
}

pub fn reply_language_hint<'a>(
    reply_language: &str,
    zh_hint: &'a str,
    en_hint: &'a str,
    ko_hint: &'a str,
) -> &'a str {
    match reply_language {
        "zh" => zh_hint,
        "en" => en_hint,
        "ko" => ko_hint,
        _ => "",
    }
}

// Custom Debug — mask API key in log output
impl std::fmt::Debug for AiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiConfig")
            .field("ai_enabled", &self.ai_enabled)
            .field("provider", &self.provider)
            .field("local_model", &self.local_model)
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "(empty)"
                } else {
                    "(set)"
                },
            )
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("max_tokens", &self.max_tokens)
            .field("temperature", &self.temperature)
            .field("reply_language", &self.reply_language)
            .field("ai_refresh_interval", &self.ai_refresh_interval)
            .field(
                "live_summary_interval_minutes",
                &self.live_summary_interval_minutes,
            )
            .finish()
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            ai_enabled: false,
            provider: "local".into(),
            local_model: "qwen3.5-2b".into(),
            api_key: String::new(),
            model: "gpt-5.4-nano".into(),
            base_url: "https://api.openai.com/v1".into(),
            max_tokens: 0,
            temperature: 0.7,
            reply_language: "ja".into(),
            ai_refresh_interval: 360,
            live_summary_interval_minutes: default_live_summary_interval_minutes(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImagePart>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImagePart {
    pub mime: String,
    pub data_base64: String,
}

// ============ OpenAI API types ============

#[derive(Debug, Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    temperature: f32,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    choices: Option<Vec<OpenAiChoice>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessageResponse,
}

#[derive(Debug, Deserialize)]
struct OpenAiMessageResponse {
    content: Option<String>,
}

// ============ Gemini API types ============

#[derive(Debug, Serialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(rename = "generationConfig")]
    generation_config: GeminiGenerationConfig,
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
}

#[derive(Debug, Serialize)]
struct GeminiContent {
    role: String,
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize)]
struct GeminiPart {
    text: String,
}

#[derive(Debug, Serialize)]
struct GeminiGenerationConfig {
    #[serde(rename = "maxOutputTokens")]
    max_output_tokens: u32,
    temperature: f32,
}

#[derive(Debug, Deserialize)]
struct GeminiResponse {
    candidates: Option<Vec<GeminiCandidate>>,
}

#[derive(Debug, Deserialize)]
struct GeminiCandidate {
    content: Option<GeminiContentResponse>,
}

#[derive(Debug, Deserialize)]
struct GeminiContentResponse {
    parts: Vec<GeminiPartResponse>,
}

#[derive(Debug, Deserialize)]
struct GeminiPartResponse {
    text: String,
}

// ============ Config persistence ============

fn config_path() -> PathBuf {
    crate::client::data_dir().join("ai_config.json")
}

/// Public accessor for other modules (e.g. timetable AI schedule).
pub fn load_ai_config() -> AiConfig {
    load_config()
}

fn load_config() -> AiConfig {
    let path = config_path();
    let mut cfg: AiConfig = if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|d| serde_json::from_str(&d).ok())
            .unwrap_or_default()
    } else {
        AiConfig::default()
    };

    // Migration: move api_key from JSON file to OS keychain
    if !cfg.api_key.is_empty() {
        if crate::keychain::set_secret("ai_api_key", &cfg.api_key).is_ok() {
            let key = std::mem::take(&mut cfg.api_key);
            let _ = save_config_to_disk(&cfg);
            cfg.api_key = key; // keep in memory for this session
        }
    } else if let Some(key) = crate::keychain::get_secret("ai_api_key") {
        cfg.api_key = key;
    }

    normalize_ai_config(&mut cfg);
    cfg
}

fn save_config(config: &AiConfig) -> Result<(), String> {
    // Store api_key in OS keychain, never on disk
    if !config.api_key.is_empty() {
        crate::keychain::set_secret("ai_api_key", &config.api_key)?;
    } else {
        crate::keychain::delete_secret("ai_api_key");
    }

    let mut disk_cfg = config.clone();
    disk_cfg.api_key = String::new(); // strip secret from JSON
    save_config_to_disk(&disk_cfg)
}

fn save_config_to_disk(config: &AiConfig) -> Result<(), String> {
    let path = config_path();
    let data = serde_json::to_string_pretty(config)
        .map_err(|e| format!("JSON serialization error: {}", e))?;
    std::fs::write(&path, &data).map_err(|e| format!("Failed to write config: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&path, perms).ok();
    }

    Ok(())
}

// ============ API call logic ============

/// Public accessor for other modules (e.g. timetable AI schedule).
pub async fn chat_completion_public(
    config: &AiConfig,
    messages: Vec<ChatMessage>,
) -> Result<String, String> {
    chat_completion(config, messages).await
}

async fn chat_completion(config: &AiConfig, messages: Vec<ChatMessage>) -> Result<String, String> {
    if !config.ai_enabled {
        return Err("AI機能が無効になっています。設定画面で有効にしてください。".into());
    }

    match config.provider.as_str() {
        "local" => {
            // Run local inference in a blocking thread
            let model_id = config.local_model.clone();
            let catalog = crate::local_ai::model_catalog();
            let info = catalog
                .iter()
                .find(|m| m.id == model_id)
                .ok_or_else(|| format!("不明なモデル: {}", model_id))?;
            let file_name = info.file_name.clone();
            let msgs = messages;
            tokio::task::spawn_blocking(move || {
                crate::local_ai::run_inference(crate::local_ai::InferenceRequest {
                    model_id,
                    file_name,
                    messages: msgs,
                    sampler: crate::local_ai::SamplerConfig::default(),
                    max_tokens: 0,
                    prefill: String::new(),
                    gen_id: String::new(),
                    think_budget_pct: 40,
                })
            })
            .await
            .map_err(|e| format!("タスク実行エラー: {}", e))?
        }
        "gemini" => call_gemini(config, messages).await,
        _ => call_openai(config, messages).await,
    }
}

async fn call_openai(config: &AiConfig, messages: Vec<ChatMessage>) -> Result<String, String> {
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));

    let body = OpenAiRequest {
        model: config.model.clone(),
        messages,
        max_tokens: config.max_tokens,
        temperature: config.temperature,
    };

    let resp = HTTP_CLIENT
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("リクエスト失敗: {}", e))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("レスポンス読み取り失敗: {}", e))?;

    if !status.is_success() {
        return Err(format!("API error ({}): {}", status, truncate_error(&text)));
    }

    let parsed: OpenAiResponse =
        serde_json::from_str(&text).map_err(|e| format!("レスポンス解析失敗: {}", e))?;

    parsed
        .choices
        .as_ref()
        .and_then(|c| c.first())
        .and_then(|c| c.message.content.clone())
        .ok_or_else(|| "AIからの応答がありません".into())
}

async fn call_gemini(config: &AiConfig, messages: Vec<ChatMessage>) -> Result<String, String> {
    let model = urlencoding::encode(&config.model);
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
        model
    );

    // Extract system instruction from messages
    let system_instruction = messages
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| m.content.clone())
        .collect::<Vec<_>>();

    let system_instruction = if system_instruction.is_empty() {
        None
    } else {
        Some(GeminiContent {
            role: "user".into(), // Gemini systemInstruction uses "user" role
            parts: vec![GeminiPart {
                text: system_instruction.join("\n"),
            }],
        })
    };

    let contents: Vec<GeminiContent> = messages
        .into_iter()
        .filter(|m| m.role != "system")
        .map(|m| GeminiContent {
            role: if m.role == "assistant" {
                "model".into()
            } else {
                "user".into()
            },
            parts: vec![GeminiPart { text: m.content }],
        })
        .collect();

    let body = GeminiRequest {
        contents,
        generation_config: GeminiGenerationConfig {
            max_output_tokens: config.max_tokens,
            temperature: config.temperature,
        },
        system_instruction,
    };

    let resp = HTTP_CLIENT
        .post(&url)
        .header("Content-Type", "application/json")
        .header("x-goog-api-key", &config.api_key) // Header auth, not URL query
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("リクエスト失敗: {}", e))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("レスポンス読み取り失敗: {}", e))?;

    if !status.is_success() {
        return Err(format!("API error ({}): {}", status, truncate_error(&text)));
    }

    let parsed: GeminiResponse =
        serde_json::from_str(&text).map_err(|e| format!("レスポンス解析失敗: {}", e))?;

    parsed
        .candidates
        .as_ref()
        .and_then(|c| c.first())
        .and_then(|c| c.content.as_ref())
        .and_then(|c| c.parts.first())
        .map(|p| p.text.clone())
        .ok_or_else(|| {
            "AIからの応答がありません（安全フィルターによりブロックされた可能性があります）".into()
        })
}

/// Truncate error body to avoid leaking excessive API detail to the frontend.
fn truncate_error(body: &str) -> String {
    // Try to extract a human-friendly message from JSON error responses
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        // OpenAI / OpenRouter format: { "error": { "message": "..." } }
        if let Some(msg) = v
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            let msg = msg.trim();
            if !msg.is_empty() {
                return if msg.len() > 200 {
                    format!(
                        "{}...",
                        &msg[..msg
                            .char_indices()
                            .nth(200)
                            .map(|(i, _)| i)
                            .unwrap_or(msg.len())]
                    )
                } else {
                    msg.to_string()
                };
            }
        }
        // Gemini format: { "error": { "status": "...", "message": "..." } }
        if let Some(status) = v
            .get("error")
            .and_then(|e| e.get("status"))
            .and_then(|s| s.as_str())
        {
            let msg = v
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("");
            return format!(
                "{}: {}",
                status,
                if msg.len() > 150 { &msg[..150] } else { msg }
            );
        }
    }
    // Fallback: truncate raw body
    match body.char_indices().nth(200) {
        Some((i, _)) => format!("{}...", &body[..i]),
        None => body.to_string(),
    }
}

// ============ Tauri Commands ============

#[tauri::command]
pub fn get_ai_config() -> AiConfig {
    load_config()
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
            if config.local_model.is_empty() {
                return Err("ローカルモデルを選択してください".into());
            }
        }
        "openai" | "gemini" => {
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
    if config.provider != "local" {
        crate::local_ai::unload_model();
    }

    let result = save_config(&config);
    if result.is_ok() {
        // Notify all windows that AI config changed
        let _ = app.emit("ai-config-changed", ());
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
    let catalog = crate::local_ai::model_catalog();
    catalog
        .iter()
        .map(|m| {
            let downloaded = crate::local_ai::is_model_downloaded(&m.file_name);
            serde_json::json!({
                "id": m.id,
                "name": m.name,
                "size_label": m.size_label,
                "param_size": m.param_size,
                "file_size_mb": m.file_size_mb,
                "downloaded": downloaded,
            })
        })
        .collect()
}

#[tauri::command]
pub async fn download_local_model(app: tauri::AppHandle, model_id: String) -> Result<(), String> {
    let catalog = crate::local_ai::model_catalog();
    let info = catalog
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("不明なモデル: {}", model_id))?
        .clone();

    // Run download in blocking thread
    let app_clone = app.clone();
    tokio::task::spawn_blocking(move || crate::local_ai::download_model(&app_clone, &info))
        .await
        .map_err(|e| format!("タスク実行エラー: {}", e))??;

    // Model availability changed — notify frontend
    let _ = app.emit("ai-config-changed", ());
    Ok(())
}

#[tauri::command]
pub fn cancel_model_download() {
    crate::local_ai::cancel_download();
}

#[tauri::command]
pub fn delete_local_model(app: tauri::AppHandle, model_id: String) -> Result<(), String> {
    let catalog = crate::local_ai::model_catalog();
    let info = catalog
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("不明なモデル: {}", model_id))?;

    // Unload if currently loaded
    crate::local_ai::unload_model();

    let path = crate::local_ai::model_path(&info.file_name);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("削除失敗: {}", e))?;
    }

    // Also remove partial file
    let part = path.with_extension("gguf.part");
    if part.exists() {
        let _ = std::fs::remove_file(&part);
    }

    // Model availability changed — notify frontend
    let _ = app.emit("ai-config-changed", ());

    Ok(())
}

#[tauri::command]
pub async fn request_ai_refresh(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Emitter;
    log::info!("[ai] request_ai_refresh called, emitting to all windows");
    app.emit("ai-refresh-request", ())
        .map_err(|e| format!("emit failed: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn test_notification(
    app: tauri::AppHandle,
    title: String,
    body: String,
) -> Result<String, String> {
    log::info!("test_notification called: title={}, body={}", title, body);
    send_native_notification(&app, &title, &body)
}

/// Send a native notification.
/// macOS: uses notify-rust directly so the app's own bundle ID (and icon) is
/// always used, bypassing the tauri plugin's dev-mode fallback to Terminal.
/// Other platforms: uses the tauri plugin as before.
pub fn send_native_notification(
    app: &tauri::AppHandle,
    title: &str,
    body: &str,
) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        let bundle_id = &app.config().identifier;
        let _ = notify_rust::set_application(bundle_id);
        let mut n = notify_rust::Notification::new();
        n.summary(title);
        n.body(body);
        n.auto_icon();
        let n = n; // move into spawn
        std::thread::spawn(move || {
            if let Err(e) = n.show() {
                log::warn!("notify-rust show failed: {}", e);
            }
        });
        Ok("Notification sent".to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        use tauri_plugin_notification::NotificationExt;
        app.notification()
            .builder()
            .title(title)
            .body(body)
            .show()
            .map(|_| "Notification sent".to_string())
            .map_err(|e| format!("Notification unavailable: {}", e))
    }
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

#[tauri::command]
pub async fn open_ai_result_window(
    app: tauri::AppHandle,
    result: String,
    error: Option<String>,
) -> Result<(), String> {
    use tauri::Emitter;

    let payload = serde_json::json!({
        "result": result,
        "error": error,
    });

    // If window already exists, just send new data and focus
    if let Some(win) = app.get_webview_window("ai-result") {
        let _ = win.emit_to("ai-result", "ai-result", &payload);
        let _ = win.set_focus();
        return Ok(());
    }

    let win = tauri::WebviewWindowBuilder::new(
        &app,
        "ai-result",
        tauri::WebviewUrl::App("ai-result.html".into()),
    )
    .title("AI 選課分析")
    .inner_size(520.0, 620.0)
    .min_inner_size(400.0, 400.0)
    .resizable(true)
    .build()
    .map_err(|e| format!("ウィンドウ作成失敗: {}", e))?;

    // Wait for window to be ready, then emit data
    let payload_clone = payload.clone();
    let win_clone = win.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let _ = win_clone.emit_to("ai-result", "ai-result", &payload_clone);
    });

    Ok(())
}
