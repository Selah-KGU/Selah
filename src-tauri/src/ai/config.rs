use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[path = "config/timing.rs"]
mod timing;
static LIVE_TIMING: timing::Timing = timing::Timing::new();

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    pub ai_enabled: bool,
    pub provider: String, // "local" | "openai" | "openrouter" | "deepseek" | "gemini"
    pub local_model: String, // "apple-intelligence"; kept so saved configs stay valid
    pub api_key: String,
    /// Runtime credential availability, never accepted from saved/user JSON.
    #[serde(skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub credential_error: Option<crate::keychain::StoreError>,
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
            credential_error: None,
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

/// Reading LIVE timing must not load credentials or perform config migrations.
/// It also must not acquire the config IO mutex or access files under LIVE locks.
pub(crate) fn live_summary_interval_minutes() -> i64 {
    LIVE_TIMING.minutes()
}

/// The LIVE start command runs this on a worker before taking either LIVE lock.
pub(crate) fn refresh_live_summary_interval() {
    LIVE_TIMING.refresh(|| read_live_summary_interval(&config_path()) as u32);
}

#[cfg(test)]
pub(crate) fn hold_live_timing_io<R>(work: impl FnOnce() -> R) -> R {
    LIVE_TIMING.hold_io(work)
}

fn read_live_summary_interval(path: &std::path::Path) -> i64 {
    std::fs::read_to_string(path)
        .ok()
        .map(|json| parse_live_summary_interval(&json))
        .unwrap_or_else(|| i64::from(default_live_summary_interval_minutes()))
}

fn parse_live_summary_interval(json: &str) -> i64 {
    #[derive(Deserialize)]
    struct Timing {
        #[serde(default = "default_live_summary_interval_minutes")]
        live_summary_interval_minutes: u32,
    }
    serde_json::from_str::<Timing>(json)
        .ok()
        .map(|timing| timing.live_summary_interval_minutes.max(5))
        .unwrap_or_else(default_live_summary_interval_minutes) as i64
}

pub(in crate::ai) fn load_config() -> AiConfig {
    let path = config_path();
    // Only file IO is gated. Credential lookup/migration and normalization run
    // after releasing the gate; snapshots never take this gate at all.
    let (observed, mut cfg) = LIVE_TIMING.read(|| {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|d| serde_json::from_str::<AiConfig>(&d).ok())
            .unwrap_or_default()
    });

    let retired_replacement = retired_preset_replacement(&cfg.provider, &cfg.model);
    normalize_ai_config(&mut cfg);
    LIVE_TIMING.observe(observed, cfg.live_summary_interval_minutes);

    // Migration: move api_key from JSON file to OS keychain
    let mut persisted = false;
    if !cfg.api_key.is_empty() {
        if crate::keychain::set_secret("ai_api_key", &cfg.api_key).is_ok() {
            let key = std::mem::take(&mut cfg.api_key);
            let _ = save_config_to_disk_if_current(&cfg, Some(observed));
            cfg.api_key = key; // keep in memory for this session
            persisted = true;
        }
    } else {
        match crate::keychain::get_secret("ai_api_key") {
            Ok(Some(key)) => cfg.api_key = key,
            Ok(None) => {}
            Err(error) => cfg.credential_error = Some(error),
        }
    }

    demote_unsupported_local_provider(&mut cfg);
    if let Some(replacement) = retired_replacement {
        if !persisted && path.exists() {
            let (_, reread) = LIVE_TIMING.read(|| std::fs::read_to_string(&path));
            if let Ok(raw) = reread {
                if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if let Some(obj) = value.as_object_mut() {
                        obj.insert(
                            "model".into(),
                            serde_json::Value::String(replacement.into()),
                        );
                        if let Ok(data) = serde_json::to_string_pretty(&value) {
                            // Reread JSON preserves unrelated fields, while the
                            // revision check prevents an older migration from
                            // rewriting a newer user save.
                            let minutes = parse_live_summary_interval(&data) as u32;
                            let _ = LIVE_TIMING.commit_if(Some(observed), minutes, || {
                                write_config_data(&path, &data)
                            });
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
        crate::keychain::delete_secrets(&["ai_api_key"])?;
    }

    let mut disk_cfg = config.clone();
    disk_cfg.credential_error = None;
    disk_cfg.api_key = String::new(); // strip secret from JSON
    save_config_to_disk(&disk_cfg)
}

fn save_config_to_disk(config: &AiConfig) -> Result<(), String> {
    save_config_to_disk_if_current(config, None).map(|_| ())
}

fn save_config_to_disk_if_current(
    config: &AiConfig,
    expected: Option<u64>,
) -> Result<bool, String> {
    let path = config_path();
    let data = serde_json::to_string_pretty(config)
        .map_err(|e| format!("JSON serialization error: {}", e))?;
    if let Some(expected) = expected {
        LIVE_TIMING.commit_if(Some(expected), config.live_summary_interval_minutes, || {
            write_config_data(&path, &data)
        })
    } else {
        LIVE_TIMING
            .commit(config.live_summary_interval_minutes, || {
                write_config_data(&path, &data)
            })
            .map(|_| true)
    }
}

fn write_config_data(path: &std::path::Path, data: &str) -> Result<(), String> {
    std::fs::write(path, data).map_err(|e| format!("Failed to write config: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, perms).ok();
    }

    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_timing_reads_only_non_secret_settings_without_rewriting_legacy_config() {
        let path =
            std::env::temp_dir().join(format!("selah-live-timing-{}.json", uuid::Uuid::new_v4()));
        assert_eq!(read_live_summary_interval(&path), 5);
        for (json, expected) in [
            (
                r#"{"live_summary_interval_minutes":15,"api_key":"legacy-placeholder"}"#,
                15,
            ),
            (r#"{"live_summary_interval_minutes":1}"#, 5),
            (r#"{"provider":"local"}"#, 5),
            (
                r#"{"live_summary_interval_minutes":4294967295}"#,
                4294967295,
            ),
            (r#"{"live_summary_interval_minutes":4294967296}"#, 5),
            (r#"{"live_summary_interval_minutes":-1}"#, 5),
            (r#"{"live_summary_interval_minutes":"15"}"#, 5),
            (r#"{"live_summary_interval_minutes":null}"#, 5),
            ("broken json", 5),
        ] {
            std::fs::write(&path, json).unwrap();
            assert_eq!(read_live_summary_interval(&path), expected);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), json);
        }
        std::fs::remove_file(path).unwrap();
    }

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

impl AiConfig {
    pub(crate) fn ensure_credentials_readable(&self) -> Result<(), String> {
        if self.provider != "local" {
            if let Some(error) = &self.credential_error {
                return Err(error.to_string());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod credential_tests {
    use super::*;
    #[test]
    fn unreadable_remote_credential_is_explicit_and_not_persisted_as_input() {
        let config = AiConfig {
            provider: "openai".into(),
            credential_error: Some(crate::keychain::StoreError::new(
                "locked",
                "unlock required",
            )),
            ..AiConfig::default()
        };
        assert_eq!(
            config.ensure_credentials_readable().unwrap_err(),
            "unlock required"
        );
        let json = serde_json::to_string(&config).unwrap();
        let decoded: AiConfig = serde_json::from_str(&json).unwrap();
        assert!(decoded.credential_error.is_none());
        let local = AiConfig {
            provider: "local".into(),
            ..config
        };
        assert!(local.ensure_credentials_readable().is_ok());
    }
}
