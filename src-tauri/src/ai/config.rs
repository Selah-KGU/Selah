use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    pub ai_enabled: bool,
    pub provider: String,    // "local" | "openai" | "openrouter" | "deepseek" | "gemini"
    pub local_model: String, // "apple-intelligence"; kept so saved configs stay valid
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

const OPENAI_DEFAULT_MODEL: &str = "gpt-6-luna";
const OPENROUTER_DEFAULT_MODEL: &str = "openai/gpt-6-luna";
const GEMINI_DEFAULT_MODEL: &str = "gemini-3.8-flash";

fn retired_preset_replacement(provider: &str, model: &str) -> Option<&'static str> {
    let model = model.trim();
    match provider {
        "openai" if matches!(model, "gpt-5.4" | "gpt-5.4-mini" | "gpt-5.4-nano") => {
            Some(OPENAI_DEFAULT_MODEL)
        }
        "gemini"
            if matches!(
                model,
                "gemini-3.5-flash" | "gemini-3.1-pro-preview" | "gemini-3-flash-preview"
            ) =>
        {
            Some(GEMINI_DEFAULT_MODEL)
        }
        "openrouter"
            if matches!(
                model,
                "moonshotai/kimi-k2.6" | "anthropic/claude-opus-4.8" | "minimax/minimax-m3"
            ) =>
        {
            Some(OPENROUTER_DEFAULT_MODEL)
        }
        _ => None,
    }
}

pub(in crate::ai) fn normalize_ai_config(config: &mut AiConfig) {
    config.live_summary_interval_minutes = config.live_summary_interval_minutes.max(5);
    config.local_model = crate::local_ai_support::APPLE_INTELLIGENCE_MODEL_ID.into();
    if let Some(replacement) = retired_preset_replacement(&config.provider, &config.model) {
        config.model = replacement.into();
    }
}

fn demote_unsupported_local_provider(config: &mut AiConfig) {
    if config.provider == "local" && crate::local_ai_support::should_demote_local_provider() {
        config.provider = "openai".into();
    }
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
            local_model: crate::local_ai_support::APPLE_INTELLIGENCE_MODEL_ID.into(),
            api_key: String::new(),
            model: OPENAI_DEFAULT_MODEL.into(),
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
// ============ Config persistence ============

fn config_path() -> PathBuf {
    crate::client::data_dir().join("ai_config.json")
}

/// Public accessor for other modules (e.g. timetable AI schedule).
pub fn load_ai_config() -> AiConfig {
    load_config()
}

pub(in crate::ai) fn load_config() -> AiConfig {
    let path = config_path();
    let mut cfg: AiConfig = if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|d| serde_json::from_str(&d).ok())
            .unwrap_or_default()
    } else {
        AiConfig::default()
    };

    let retired_replacement = retired_preset_replacement(&cfg.provider, &cfg.model);
    normalize_ai_config(&mut cfg);

    // Migration: move api_key from JSON file to OS keychain
    let mut persisted = false;
    if !cfg.api_key.is_empty() {
        if crate::keychain::set_secret("ai_api_key", &cfg.api_key).is_ok() {
            let key = std::mem::take(&mut cfg.api_key);
            let _ = save_config_to_disk(&cfg);
            cfg.api_key = key; // keep in memory for this session
            persisted = true;
        }
    } else if let Some(key) = crate::keychain::get_secret("ai_api_key") {
        // Backed by the in-memory secret bundle (one keychain read per process),
        // so calling this on every AI op no longer re-hits the keychain.
        cfg.api_key = key;
    }

    demote_unsupported_local_provider(&mut cfg);
    if let Some(replacement) = retired_replacement {
        if !persisted && path.exists() {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if let Some(obj) = value.as_object_mut() {
                        obj.insert(
                            "model".into(),
                            serde_json::Value::String(replacement.into()),
                        );
                        if let Ok(data) = serde_json::to_string_pretty(&value) {
                            let _ = std::fs::write(&path, data);
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::PermissionsExt;
                                let perms = std::fs::Permissions::from_mode(0o600);
                                std::fs::set_permissions(&path, perms).ok();
                            }
                        }
                    }
                }
            }
        }
    }
    cfg
}

pub(in crate::ai) fn save_config(config: &AiConfig) -> Result<(), String> {
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
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_presets_become_current_defaults() {
        for old in ["gpt-5.4", "gpt-5.4-mini", " gpt-5.4-nano "] {
            let mut cfg = AiConfig::default();
            cfg.provider = "openai".into();
            cfg.model = old.into();
            normalize_ai_config(&mut cfg);
            assert_eq!(cfg.model, OPENAI_DEFAULT_MODEL);
        }

        let mut gemini = AiConfig::default();
        gemini.provider = "gemini".into();
        gemini.model = "gemini-3.5-flash".into();
        normalize_ai_config(&mut gemini);
        assert_eq!(gemini.model, GEMINI_DEFAULT_MODEL);

        let mut openrouter = AiConfig::default();
        openrouter.provider = "openrouter".into();
        openrouter.model = "moonshotai/kimi-k2.6".into();
        normalize_ai_config(&mut openrouter);
        assert_eq!(openrouter.model, OPENROUTER_DEFAULT_MODEL);

        let mut custom = AiConfig::default();
        custom.provider = "openai".into();
        custom.model = "gpt-6.1-sol".into();
        custom.local_model = "qwen3.5-4b".into();
        normalize_ai_config(&mut custom);
        assert_eq!(custom.model, "gpt-6.1-sol");
        assert_eq!(
            custom.local_model,
            crate::local_ai_support::APPLE_INTELLIGENCE_MODEL_ID
        );
    }
}
