use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;
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
    pub provider: String,    // "openai" | "gemini" | "custom"
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub reply_language: String,
}

// Custom Debug — mask API key in log output
impl std::fmt::Debug for AiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiConfig")
            .field("provider", &self.provider)
            .field("api_key", &if self.api_key.is_empty() { "(empty)" } else { "(set)" })
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("max_tokens", &self.max_tokens)
            .field("temperature", &self.temperature)
            .field("reply_language", &self.reply_language)
            .finish()
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: "openai".into(),
            api_key: String::new(),
            model: "gpt-5.4-nano".into(),
            base_url: "https://api.openai.com/v1".into(),
            max_tokens: 4096,
            temperature: 0.7,
            reply_language: "ja".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
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
    let dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.kgu.selah");
    std::fs::create_dir_all(&dir).ok();
    // Migrate from old config_dir location
    let old = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.haru.kwic")
        .join("ai_config.json");
    let new = dir.join("ai_config.json");
    if old.exists() && !new.exists() {
        let _ = std::fs::rename(&old, &new);
    }
    new
}

pub fn load_config() -> AiConfig {
    let path = config_path();
    if path.exists() {
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str(&data) {
                return cfg;
            }
        }
    }
    AiConfig::default()
}

pub fn save_config(config: &AiConfig) -> Result<(), String> {
    let path = config_path();
    let data = serde_json::to_string_pretty(config)
        .map_err(|e| format!("JSON serialization error: {}", e))?;
    std::fs::write(&path, &data)
        .map_err(|e| format!("Failed to write config: {}", e))?;

    // Restrict file permissions to owner-only (0600) — API key inside
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&path, perms).ok();
    }

    Ok(())
}

// ============ API call logic ============

pub async fn chat_completion(
    config: &AiConfig,
    messages: Vec<ChatMessage>,
) -> Result<String, String> {
    if config.api_key.is_empty() {
        return Err("APIキーが設定されていません。設定画面でAPIキーを入力してください。".into());
    }

    match config.provider.as_str() {
        "gemini" => call_gemini(config, messages).await,
        _ => call_openai(config, messages).await, // "openai" and "custom" both use OpenAI format
    }
}

async fn call_openai(
    config: &AiConfig,
    messages: Vec<ChatMessage>,
) -> Result<String, String> {
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
    let text = resp.text().await.map_err(|e| format!("レスポンス読み取り失敗: {}", e))?;

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

async fn call_gemini(
    config: &AiConfig,
    messages: Vec<ChatMessage>,
) -> Result<String, String> {
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
            parts: vec![GeminiPart { text: system_instruction.join("\n") }],
        })
    };

    let contents: Vec<GeminiContent> = messages
        .into_iter()
        .filter(|m| m.role != "system")
        .map(|m| GeminiContent {
            role: if m.role == "assistant" { "model".into() } else { "user".into() },
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
    let text = resp.text().await.map_err(|e| format!("レスポンス読み取り失敗: {}", e))?;

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
        .ok_or_else(|| "AIからの応答がありません（安全フィルターによりブロックされた可能性があります）".into())
}

/// Truncate error body to avoid leaking excessive API detail to the frontend.
fn truncate_error(body: &str) -> String {
    if body.chars().count() <= 200 {
        body.to_string()
    } else {
        let truncated: String = body.chars().take(200).collect();
        format!("{}…", truncated)
    }
}

// ============ Tauri Commands ============

#[tauri::command]
pub fn get_ai_config() -> AiConfig {
    load_config()
}

#[tauri::command]
pub fn save_ai_config(mut config: AiConfig) -> Result<(), String> {
    // Clamp values to valid ranges
    config.temperature = config.temperature.clamp(0.0, 2.0);
    config.max_tokens = config.max_tokens.clamp(1, 65536);
    config.api_key = config.api_key.trim().to_string();
    config.base_url = config.base_url.trim().to_string();
    config.model = config.model.trim().to_string();

    if config.model.is_empty() {
        return Err("モデル名を入力してください".into());
    }

    save_config(&config)
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
    }];
    chat_completion(&config, test_messages).await
}

#[tauri::command]
pub async fn open_settings_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.set_focus();
        return Ok(());
    }

    tauri::WebviewWindowBuilder::new(
        &app,
        "settings",
        tauri::WebviewUrl::App("settings.html".into()),
    )
    .title("設定")
    .inner_size(720.0, 460.0)
    .min_inner_size(600.0, 400.0)
    .resizable(true)
    .build()
    .map_err(|e| format!("Failed to open settings window: {}", e))?;

    Ok(())
}

#[tauri::command]
pub async fn request_ai_refresh(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Emitter;
    println!("[ai] request_ai_refresh called, emitting to all windows");
    app.emit("ai-refresh-request", ())
        .map_err(|e| format!("emit failed: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn test_notification(app: tauri::AppHandle, title: String, body: String) -> Result<String, String> {
    log::info!("test_notification called: title={}, body={}", title, body);
    use tauri_plugin_notification::NotificationExt;
    app.notification()
        .builder()
        .title(&title)
        .body(&body)
        .show()
        .map_err(|e| format!("notification show failed: {}", e))?;
    Ok("Notification sent via plugin".to_string())
}

#[tauri::command]
pub async fn toggle_debug_panel(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Emitter;
    if let Some(win) = app.get_webview_window("main") {
        win.emit("toggle-debug", ())
            .map_err(|e| format!("emit failed: {}", e))?;
    }
    // Close the settings window
    if let Some(settings_win) = app.get_webview_window("settings") {
        let _ = settings_win.close();
    }
    Ok(())
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
