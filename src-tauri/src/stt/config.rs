use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SttConfig {
    pub selected_model: String,
    pub language: String,
    pub execution_backend: String,
    pub partial_mode: String,
    pub sensitivity: String,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            selected_model: "sensevoice-ja-en".into(),
            language: "ja".into(),
            execution_backend: STT_BACKEND_CPU.into(),
            partial_mode: STT_PARTIAL_MODE_BALANCED.into(),
            sensitivity: STT_SENSITIVITY_NORMAL.into(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(in crate::stt) struct SttPartialThrottleProfile {
    pub(in crate::stt) enabled: bool,
    pub(in crate::stt) min_interval_ms: u64,
    pub(in crate::stt) stable_interval_ms: u64,
    pub(in crate::stt) very_stable_interval_ms: u64,
}

/// VAD tuning knobs driven by the user-facing sensitivity setting. Higher
/// sensitivity lowers every threshold so the recognizer triggers on quieter
/// or shorter fragments; lower sensitivity is stricter (less false-triggering
/// on keyboard / ambient noise) at the cost of missing whispered speech.
#[derive(Debug, Clone, Copy)]
pub(in crate::stt) struct SttSensitivityProfile {
    pub(in crate::stt) vad_threshold: f32,
    pub(in crate::stt) vad_min_speech: f32,
    pub(in crate::stt) vad_min_silence: f32,
    pub(in crate::stt) rms_gate: f32,
}

static STT_CONFIG_CACHE: LazyLock<Mutex<Option<SttConfig>>> = LazyLock::new(|| Mutex::new(None));

#[derive(Debug, Clone, Serialize)]
pub struct SttExecutionBackendInfo {
    pub id: String,
    pub label: String,
    pub description: String,
    pub experimental: bool,
    pub available: bool,
    pub availability_note: Option<String>,
}

fn coreml_build_enabled() -> bool {
    cfg!(target_os = "macos") && cfg!(feature = "stt-shared")
}

pub(in crate::stt) fn stt_execution_backend_catalog() -> Vec<SttExecutionBackendInfo> {
    let mut backends = vec![SttExecutionBackendInfo {
        id: STT_BACKEND_CPU.into(),
        label: "CPU（標準）".into(),
        description: "すべての環境で動作します。標準モデルを使用します。".into(),
        experimental: false,
        available: true,
        availability_note: None,
    }];

    if cfg!(target_os = "macos") {
        let available = coreml_build_enabled();
        backends.push(SttExecutionBackendInfo {
            id: STT_BACKEND_COREML.into(),
            label: "CoreML".into(),
            description: "Apple Neural Engine / GPU を使う実験的な高速化です。".into(),
            experimental: true,
            available,
            availability_note: if available {
                Some("認識器のみ CoreML に切り替わり、VAD は引き続き CPU を使います。".into())
            } else {
                Some("このビルドでは CoreML は利用できません。".into())
            },
        });
    }

    backends
}

pub(in crate::stt) fn normalize_stt_language(language: &str) -> String {
    let language = language.trim();
    if language.is_empty() {
        "ja".into()
    } else {
        language.to_string()
    }
}

pub(in crate::stt) fn normalize_stt_model_id(model_id: &str) -> String {
    let model_id = model_id.trim();
    if stt_model_catalog().iter().any(|model| model.id == model_id) {
        model_id.to_string()
    } else {
        SttConfig::default().selected_model
    }
}

fn normalize_stt_execution_backend(requested: &str) -> String {
    match requested.trim().to_ascii_lowercase().as_str() {
        STT_BACKEND_COREML if coreml_build_enabled() => STT_BACKEND_COREML.into(),
        _ => STT_BACKEND_CPU.into(),
    }
}

pub(in crate::stt) fn validate_stt_execution_backend(requested: &str) -> Result<String, String> {
    let requested = requested.trim().to_ascii_lowercase();
    match requested.as_str() {
        "" | STT_BACKEND_CPU => Ok(STT_BACKEND_CPU.into()),
        STT_BACKEND_COREML if coreml_build_enabled() => Ok(STT_BACKEND_COREML.into()),
        STT_BACKEND_COREML if cfg!(target_os = "macos") => {
            Err("CoreML は macOS の shared STT ビルドでのみ利用できます".into())
        }
        STT_BACKEND_COREML => Err("CoreML は macOS ビルドでのみ利用できます".into()),
        _ => Err("不明な音声認識モードです".into()),
    }
}

pub(in crate::stt) fn stt_execution_backend_label(backend: &str) -> &'static str {
    match backend {
        STT_BACKEND_COREML => "CoreML",
        _ => "CPU",
    }
}

pub(in crate::stt) fn normalize_stt_partial_mode(requested: &str) -> String {
    match requested.trim().to_ascii_lowercase().as_str() {
        STT_PARTIAL_MODE_POWER_SAVER => STT_PARTIAL_MODE_POWER_SAVER.into(),
        STT_PARTIAL_MODE_FINAL_ONLY => STT_PARTIAL_MODE_FINAL_ONLY.into(),
        _ => STT_PARTIAL_MODE_BALANCED.into(),
    }
}

pub(in crate::stt) fn stt_partial_mode_label(mode: &str) -> &'static str {
    match mode {
        STT_PARTIAL_MODE_POWER_SAVER => "省電",
        STT_PARTIAL_MODE_FINAL_ONLY => "最省電",
        _ => "標準",
    }
}

pub(in crate::stt) fn normalize_stt_sensitivity(requested: &str) -> String {
    match requested.trim().to_ascii_lowercase().as_str() {
        STT_SENSITIVITY_LOW => STT_SENSITIVITY_LOW.into(),
        STT_SENSITIVITY_HIGH => STT_SENSITIVITY_HIGH.into(),
        _ => STT_SENSITIVITY_NORMAL.into(),
    }
}

pub(in crate::stt) fn stt_sensitivity_label(mode: &str) -> &'static str {
    match mode {
        STT_SENSITIVITY_LOW => "控えめ",
        STT_SENSITIVITY_HIGH => "高感度",
        _ => "標準",
    }
}

pub(in crate::stt) fn stt_sensitivity_profile(mode: &str) -> SttSensitivityProfile {
    match normalize_stt_sensitivity(mode).as_str() {
        STT_SENSITIVITY_LOW => SttSensitivityProfile {
            vad_threshold: 0.65,
            vad_min_speech: 0.35,
            vad_min_silence: 0.60,
            rms_gate: 0.0035,
        },
        STT_SENSITIVITY_HIGH => SttSensitivityProfile {
            vad_threshold: 0.35,
            vad_min_speech: 0.15,
            vad_min_silence: 0.30,
            rms_gate: 0.0008,
        },
        _ => SttSensitivityProfile {
            vad_threshold: 0.5,
            vad_min_speech: 0.25,
            vad_min_silence: 0.45,
            rms_gate: RMS_GATE,
        },
    }
}

pub(in crate::stt) fn stt_partial_throttle_profile(mode: &str) -> SttPartialThrottleProfile {
    match normalize_stt_partial_mode(mode).as_str() {
        STT_PARTIAL_MODE_POWER_SAVER => SttPartialThrottleProfile {
            enabled: true,
            min_interval_ms: 1500,
            stable_interval_ms: 3000,
            very_stable_interval_ms: 5000,
        },
        STT_PARTIAL_MODE_FINAL_ONLY => SttPartialThrottleProfile {
            enabled: false,
            min_interval_ms: 0,
            stable_interval_ms: 0,
            very_stable_interval_ms: 0,
        },
        _ => SttPartialThrottleProfile {
            enabled: true,
            min_interval_ms: 600,
            stable_interval_ms: 1500,
            very_stable_interval_ms: 3000,
        },
    }
}

pub(in crate::stt) fn stt_fallback_message(requested_backend: &str) -> String {
    format!(
        "{} の初期化に失敗したため、CPU にフォールバックしました",
        stt_execution_backend_label(requested_backend)
    )
}

pub(in crate::stt) fn stt_runtime_preferences() -> (String, String) {
    let stt_cfg = load_config();
    (
        normalize_stt_language(&stt_cfg.language),
        normalize_stt_execution_backend(&stt_cfg.execution_backend),
    )
}

fn normalized_stt_config(mut config: SttConfig) -> SttConfig {
    config.selected_model = normalize_stt_model_id(&config.selected_model);
    config.language = normalize_stt_language(&config.language);
    config.execution_backend = normalize_stt_execution_backend(&config.execution_backend);
    config.partial_mode = normalize_stt_partial_mode(&config.partial_mode);
    config.sensitivity = normalize_stt_sensitivity(&config.sensitivity);
    config
}

pub(in crate::stt) fn load_config() -> SttConfig {
    if let Ok(cache) = STT_CONFIG_CACHE.lock() {
        if let Some(config) = cache.clone() {
            return config;
        }
    }

    let path = stt_config_path();
    let config = if !path.exists() {
        SttConfig::default()
    } else {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|v| serde_json::from_str(&v).ok())
            .map(normalized_stt_config)
            .unwrap_or_default()
    };

    if let Ok(mut cache) = STT_CONFIG_CACHE.lock() {
        *cache = Some(config.clone());
    }

    config
}

pub(in crate::stt) fn save_config(config: &SttConfig) -> Result<(), String> {
    let path = stt_config_path();
    let data = serde_json::to_string_pretty(config)
        .map_err(|e| format!("JSON serialization error: {}", e))?;
    std::fs::write(&path, data).map_err(|e| format!("Failed to write STT config: {}", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    if let Ok(mut cache) = STT_CONFIG_CACHE.lock() {
        *cache = Some(config.clone());
    }
    Ok(())
}
