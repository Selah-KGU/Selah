use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use bzip2::read::BzDecoder;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::{Deserialize, Serialize};
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig, SileroVadModelConfig,
    VadModelConfig, VoiceActivityDetector,
};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::Emitter;

const TARGET_SAMPLE_RATE: i32 = 16_000;
const VAD_MODEL_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx";
const VAD_MODEL_FILE: &str = "silero_vad.onnx";
const STT_BACKEND_CPU: &str = "cpu";
const STT_BACKEND_COREML: &str = "coreml";
const STT_DECODE_HELPER_ARG: &str = "--selah-stt-decode";
const STT_PARTIAL_MODE_BALANCED: &str = "balanced";
const STT_PARTIAL_MODE_POWER_SAVER: &str = "power_saver";
const STT_PARTIAL_MODE_FINAL_ONLY: &str = "final_only";
const STT_SENSITIVITY_LOW: &str = "low";
const STT_SENSITIVITY_NORMAL: &str = "normal";
const STT_SENSITIVITY_HIGH: &str = "high";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttModelInfo {
    pub id: String,
    pub name: String,
    pub size_label: String,
    pub archive_name: String,
    pub folder_name: String,
    pub download_url: String,
    pub file_size_mb: u64,
    pub model_file: String,
    pub tokens_file: String,
}

static STT_MODEL_CATALOG: LazyLock<Vec<SttModelInfo>> = LazyLock::new(|| {
    vec![SttModelInfo {
        id: "sensevoice-ja-en".into(),
        name: "SenseVoice 標準".into(),
        size_label: "228 MB".into(),
        archive_name: "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17.tar.bz2".into(),
        folder_name: "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17".into(),
        download_url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17.tar.bz2".into(),
        file_size_mb: 228,
        model_file: "model.int8.onnx".into(),
        tokens_file: "tokens.txt".into(),
    }]
});

pub fn stt_model_catalog() -> &'static [SttModelInfo] {
    &STT_MODEL_CATALOG
}

fn stt_models_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = crate::client::data_dir().join("models").join("stt");
        let _ = std::fs::create_dir_all(&dir);
        dir
    })
}

fn stt_config_path() -> PathBuf {
    crate::client::data_dir().join("stt_config.json")
}

fn stt_model_dir(model: &SttModelInfo) -> PathBuf {
    stt_models_dir().join(&model.folder_name)
}

fn stt_archive_path(model: &SttModelInfo) -> PathBuf {
    stt_models_dir().join(&model.archive_name)
}

fn vad_model_path() -> PathBuf {
    stt_models_dir().join(VAD_MODEL_FILE)
}

fn file_exists(path: &Path) -> bool {
    path.exists() && path.metadata().map(|m| m.len() > 0).unwrap_or(false)
}

fn file_exists_with_min_size(path: &Path, min_bytes: u64) -> bool {
    path.exists()
        && path
            .metadata()
            .map(|metadata| metadata.len() >= min_bytes)
            .unwrap_or(false)
}

pub fn is_stt_model_downloaded(model: &SttModelInfo) -> bool {
    let dir = stt_model_dir(model);
    let min_model_bytes = model.file_size_mb.saturating_mul(1024 * 1024);
    file_exists_with_min_size(&dir.join(&model.model_file), min_model_bytes)
        && file_exists(&dir.join(&model.tokens_file))
        && file_exists(&vad_model_path())
}

fn stt_model_missing_message(model: &SttModelInfo) -> String {
    let model_path = stt_model_dir(model).join(&model.model_file);
    let min_model_bytes = model.file_size_mb.saturating_mul(1024 * 1024);
    if file_exists(&model_path) && !file_exists_with_min_size(&model_path, min_model_bytes) {
        return format!(
            "{} のダウンロードが不完全です。削除して再ダウンロードしてください。",
            model.name
        );
    }
    format!(
        "{} がダウンロードされていません。設定画面からダウンロードしてください。",
        model.name
    )
}

fn ensure_stt_model_downloaded(model: &SttModelInfo) -> Result<(), String> {
    if is_stt_model_downloaded(model) {
        Ok(())
    } else {
        Err(stt_model_missing_message(model))
    }
}

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
struct SttPartialThrottleProfile {
    enabled: bool,
    min_interval_ms: u64,
    stable_interval_ms: u64,
    very_stable_interval_ms: u64,
}

/// VAD tuning knobs driven by the user-facing sensitivity setting. Higher
/// sensitivity lowers every threshold so the recognizer triggers on quieter
/// or shorter fragments; lower sensitivity is stricter (less false-triggering
/// on keyboard / ambient noise) at the cost of missing whispered speech.
#[derive(Debug, Clone, Copy)]
struct SttSensitivityProfile {
    vad_threshold: f32,
    vad_min_speech: f32,
    vad_min_silence: f32,
    rms_gate: f32,
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

fn stt_execution_backend_catalog() -> Vec<SttExecutionBackendInfo> {
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

fn normalize_stt_language(language: &str) -> String {
    let language = language.trim();
    if language.is_empty() {
        "ja".into()
    } else {
        language.to_string()
    }
}

fn normalize_stt_model_id(model_id: &str) -> String {
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

fn validate_stt_execution_backend(requested: &str) -> Result<String, String> {
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

fn stt_execution_backend_label(backend: &str) -> &'static str {
    match backend {
        STT_BACKEND_COREML => "CoreML",
        _ => "CPU",
    }
}

fn normalize_stt_partial_mode(requested: &str) -> String {
    match requested.trim().to_ascii_lowercase().as_str() {
        STT_PARTIAL_MODE_POWER_SAVER => STT_PARTIAL_MODE_POWER_SAVER.into(),
        STT_PARTIAL_MODE_FINAL_ONLY => STT_PARTIAL_MODE_FINAL_ONLY.into(),
        _ => STT_PARTIAL_MODE_BALANCED.into(),
    }
}

fn stt_partial_mode_label(mode: &str) -> &'static str {
    match mode {
        STT_PARTIAL_MODE_POWER_SAVER => "省電",
        STT_PARTIAL_MODE_FINAL_ONLY => "最省電",
        _ => "標準",
    }
}

fn normalize_stt_sensitivity(requested: &str) -> String {
    match requested.trim().to_ascii_lowercase().as_str() {
        STT_SENSITIVITY_LOW => STT_SENSITIVITY_LOW.into(),
        STT_SENSITIVITY_HIGH => STT_SENSITIVITY_HIGH.into(),
        _ => STT_SENSITIVITY_NORMAL.into(),
    }
}

fn stt_sensitivity_label(mode: &str) -> &'static str {
    match mode {
        STT_SENSITIVITY_LOW => "控えめ",
        STT_SENSITIVITY_HIGH => "高感度",
        _ => "標準",
    }
}

fn stt_sensitivity_profile(mode: &str) -> SttSensitivityProfile {
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

fn stt_partial_throttle_profile(mode: &str) -> SttPartialThrottleProfile {
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

fn stt_fallback_message(requested_backend: &str) -> String {
    format!(
        "{} の初期化に失敗したため、CPU にフォールバックしました",
        stt_execution_backend_label(requested_backend)
    )
}

fn stt_runtime_preferences() -> (String, String) {
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

fn load_config() -> SttConfig {
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

fn save_config(config: &SttConfig) -> Result<(), String> {
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

static STT_DOWNLOAD_CANCEL: AtomicBool = AtomicBool::new(false);

pub fn cancel_stt_download() {
    STT_DOWNLOAD_CANCEL.store(true, Ordering::SeqCst);
}

fn emit_download_progress(app: &tauri::AppHandle, downloaded: u64, total: u64) {
    let _ = app.emit(
        "stt-model-download-progress",
        serde_json::json!({
            "downloaded": downloaded,
            "total": total,
            "percent": if total > 0 { (downloaded as f64 / total as f64 * 100.0) as u32 } else { 0 }
        }),
    );
}

fn download_file_blocking(
    app: &tauri::AppHandle,
    url: &str,
    dest: &Path,
    progress_scale: f64,
    progress_offset: f64,
) -> Result<(), String> {
    let partial = dest.with_extension("part");
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3600))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let mut resp = client
        .get(url)
        .send()
        .map_err(|e| format!("ダウンロード開始失敗: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("ダウンロードエラー ({})", resp.status()));
    }

    let total = resp.content_length().unwrap_or(0);
    let mut file =
        std::fs::File::create(&partial).map_err(|e| format!("ファイル作成失敗: {}", e))?;
    let mut downloaded = 0u64;
    let mut last_emit = Instant::now();
    let mut buf = vec![0u8; 256 * 1024];

    loop {
        if STT_DOWNLOAD_CANCEL.load(Ordering::SeqCst) {
            drop(file);
            let _ = std::fs::remove_file(&partial);
            return Err("cancelled".into());
        }
        let n = resp
            .read(&mut buf)
            .map_err(|e| format!("ダウンロード読み取りエラー: {}", e))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| format!("ファイル書き込みエラー: {}", e))?;
        downloaded += n as u64;
        if last_emit.elapsed() > Duration::from_millis(200) {
            let scaled_downloaded = (progress_offset + downloaded as f64 * progress_scale) as u64;
            let scaled_total = (progress_offset + total as f64 * progress_scale) as u64;
            emit_download_progress(app, scaled_downloaded, scaled_total);
            last_emit = Instant::now();
        }
    }

    file.flush()
        .map_err(|e| format!("ファイルフラッシュエラー: {}", e))?;
    std::fs::rename(&partial, dest).map_err(|e| format!("ファイルリネームエラー: {}", e))?;
    let scaled_downloaded = (progress_offset + total as f64 * progress_scale) as u64;
    let scaled_total = (progress_offset + total as f64 * progress_scale) as u64;
    emit_download_progress(app, scaled_downloaded, scaled_total);
    Ok(())
}

pub fn download_stt_model_blocking(
    app: &tauri::AppHandle,
    model: &SttModelInfo,
) -> Result<(), String> {
    STT_DOWNLOAD_CANCEL.store(false, Ordering::SeqCst);

    let archive_path = stt_archive_path(model);
    let model_dir = stt_model_dir(model);
    if model_dir.exists() {
        let _ = std::fs::remove_dir_all(&model_dir);
    }
    let _ = std::fs::create_dir_all(stt_models_dir());

    download_file_blocking(app, &model.download_url, &archive_path, 0.98, 0.0)?;

    let archive_file =
        File::open(&archive_path).map_err(|e| format!("圧縮ファイルを開けません: {}", e))?;
    let decoder = BzDecoder::new(archive_file);
    let mut archive = tar::Archive::new(decoder);
    archive
        .unpack(stt_models_dir())
        .map_err(|e| format!("モデル展開失敗: {}", e))?;

    let vad_path = vad_model_path();
    if !vad_path.exists() {
        download_file_blocking(app, VAD_MODEL_URL, &vad_path, 0.02, 98.0)?;
    }

    let _ = app.emit(
        "stt-model-download-progress",
        serde_json::json!({
            "downloaded": 100,
            "total": 100,
            "percent": 100,
            "done": true
        }),
    );

    Ok(())
}

fn build_sense_voice_config_for_backend(
    model: &SttModelInfo,
    language: &str,
    execution_backend: &str,
) -> Result<OfflineRecognizerConfig, String> {
    let dir = stt_model_dir(model);
    let model_path = dir.join(&model.model_file);
    let tokens_path = dir.join(&model.tokens_file);
    for path in [&model_path, &tokens_path] {
        if !path.exists() {
            return Err(format!(
                "モデルファイルが不足しています: {}",
                path.display()
            ));
        }
    }

    let mut config = OfflineRecognizerConfig::default();
    config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
        model: Some(model_path.to_string_lossy().into_owned()),
        language: Some(language.to_string()),
        use_itn: true,
    };
    config.model_config.tokens = Some(tokens_path.to_string_lossy().into_owned());
    config.model_config.provider = Some(execution_backend.to_string());
    config.model_config.num_threads = std::thread::available_parallelism()
        .map(|n| n.get().min(2) as i32)
        .unwrap_or(2);
    config.decoding_method = Some("greedy_search".into());
    Ok(config)
}

fn build_vad_config(profile: &SttSensitivityProfile) -> Result<VadModelConfig, String> {
    let path = vad_model_path();
    if !path.exists() {
        return Err("VAD モデルがまだダウンロードされていません".into());
    }
    Ok(VadModelConfig {
        sample_rate: TARGET_SAMPLE_RATE,
        num_threads: 1,
        provider: Some("cpu".into()),
        silero_vad: SileroVadModelConfig {
            model: Some(path.to_string_lossy().into_owned()),
            threshold: profile.vad_threshold,
            min_silence_duration: profile.vad_min_silence,
            min_speech_duration: profile.vad_min_speech,
            window_size: 512,
            // sherpa-onnx raises this model's threshold to 0.90 once the
            // buffer exceeds max_speech_duration, and ongoing speech then
            // looks like silence. Do not cut the utterance in userspace to
            // stay under a short limit: that reset drops the continuation
            // and queues a long final in front of the next live partial.
            max_speech_duration: 86_400.0,
        },
        ..Default::default()
    })
}

fn selected_model_from_config() -> Result<SttModelInfo, String> {
    let cfg = load_config();
    stt_model_catalog()
        .iter()
        .find(|m| m.id == cfg.selected_model)
        .cloned()
        .ok_or_else(|| format!("不明な STT モデル: {}", cfg.selected_model))
}

#[derive(Clone, Serialize)]
struct SttEventPayload {
    text: String,
    caller: String,
    /// Capture order. Live captions use it so a newer partial is not wiped
    /// when an older final finishes decoding later.
    seq: u64,
}

#[derive(Clone, Serialize)]
struct SttStatePayload {
    state: String,
    caller: String,
}

#[derive(Clone, Serialize)]
struct SttInfoPayload {
    message: String,
    caller: String,
}

#[derive(Debug, Clone, Default)]
struct SttRuntimeDebugState {
    execution_backend: Option<String>,
    fallback_from: Option<String>,
    state: String,
    active_caller: Option<String>,
    last_info: Option<String>,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SttRuntimeDebugInfo {
    pub configured_backend: String,
    pub configured_partial_mode: String,
    pub configured_sensitivity: String,
    pub runtime_backend: String,
    pub runtime_state: String,
    pub active_caller: String,
    pub runtime_note: String,
    pub runtime_error: String,
}

struct RecognizerInitResult {
    recognizer: OfflineRecognizer,
    execution_backend: String,
    fallback_from: Option<String>,
}

struct ActiveSttSession {
    id: u64,
    caller: String,
    stop_tx: mpsc::Sender<()>,
}

static STT_SESSION: Mutex<Option<ActiveSttSession>> = Mutex::new(None);
static STT_SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);
static STT_RUNTIME_DEBUG: LazyLock<Mutex<SttRuntimeDebugState>> = LazyLock::new(|| {
    Mutex::new(SttRuntimeDebugState {
        execution_backend: None,
        fallback_from: None,
        state: "idle".into(),
        active_caller: None,
        last_info: None,
        last_error: None,
    })
});
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

fn clear_session_if_matches(id: u64) {
    if let Ok(mut lock) = STT_SESSION.lock() {
        if lock.as_ref().map(|s| s.id) == Some(id) {
            *lock = None;
        }
    }
}

fn stt_runtime_state_label(state: &str) -> &'static str {
    match state {
        "initializing" => "初期化中",
        "listening" => "音声入力中",
        "test-ok" => "テスト成功",
        _ => "待機",
    }
}

fn update_runtime_debug_state(
    state: &str,
    caller: Option<&str>,
    execution_backend: Option<&str>,
    fallback_from: Option<&str>,
) {
    if let Ok(mut debug) = STT_RUNTIME_DEBUG.lock() {
        debug.state = state.to_string();
        debug.active_caller = caller.map(|value| value.to_string());
        if let Some(execution_backend) = execution_backend {
            debug.execution_backend = Some(execution_backend.to_string());
            debug.fallback_from = fallback_from.map(|value| value.to_string());
        }
        if state == "idle" {
            debug.active_caller = None;
        }
    }
}

fn update_runtime_debug_message(info: Option<String>, error: Option<String>) {
    if let Ok(mut debug) = STT_RUNTIME_DEBUG.lock() {
        if let Some(info) = info {
            debug.last_info = Some(info);
        }
        if let Some(error) = error {
            debug.last_error = Some(error);
        }
    }
}

fn emit_runtime_debug_changed(app: &tauri::AppHandle) {
    let _ = app.emit("stt-runtime-debug-changed", ());
}

fn stt_runtime_backend_debug_label(
    execution_backend: Option<&str>,
    fallback_from: Option<&str>,
) -> String {
    match execution_backend {
        Some(execution_backend) => {
            let active = stt_execution_backend_label(execution_backend);
            if let Some(fallback_from) = fallback_from {
                format!(
                    "{} ({} からフォールバック)",
                    active,
                    stt_execution_backend_label(fallback_from)
                )
            } else {
                active.to_string()
            }
        }
        None => "未初期化".into(),
    }
}

pub fn stt_runtime_debug_info() -> SttRuntimeDebugInfo {
    let config = load_config();
    let configured_backend = stt_execution_backend_label(&config.execution_backend).to_string();
    let configured_partial_mode = stt_partial_mode_label(&config.partial_mode).to_string();
    let configured_sensitivity = stt_sensitivity_label(&config.sensitivity).to_string();
    if let Ok(debug) = STT_RUNTIME_DEBUG.lock() {
        return SttRuntimeDebugInfo {
            configured_backend,
            configured_partial_mode,
            configured_sensitivity,
            runtime_backend: stt_runtime_backend_debug_label(
                debug.execution_backend.as_deref(),
                debug.fallback_from.as_deref(),
            ),
            runtime_state: stt_runtime_state_label(&debug.state).to_string(),
            active_caller: debug.active_caller.clone().unwrap_or_else(|| "-".into()),
            runtime_note: debug.last_info.clone().unwrap_or_default(),
            runtime_error: debug.last_error.clone().unwrap_or_default(),
        };
    }

    SttRuntimeDebugInfo {
        configured_backend,
        configured_partial_mode,
        configured_sensitivity,
        runtime_backend: "未取得".into(),
        runtime_state: "待機".into(),
        active_caller: "-".into(),
        runtime_note: String::new(),
        runtime_error: String::new(),
    }
}

fn emit_state(app: &tauri::AppHandle, state: &str, caller: &str) {
    update_runtime_debug_state(state, Some(caller), None, None);
    emit_runtime_debug_changed(app);
    let _ = app.emit(
        "stt-state",
        SttStatePayload {
            state: state.to_string(),
            caller: caller.to_string(),
        },
    );
}

fn emit_error(app: &tauri::AppHandle, message: impl Into<String>, caller: &str) {
    let message = message.into();
    update_runtime_debug_message(None, Some(message.clone()));
    emit_runtime_debug_changed(app);
    let _ = app.emit(
        "stt-error",
        serde_json::json!({ "message": message, "caller": caller }),
    );
}

fn emit_info(app: &tauri::AppHandle, message: impl Into<String>, caller: &str) {
    let message = message.into();
    update_runtime_debug_message(Some(message.clone()), None);
    emit_runtime_debug_changed(app);
    let _ = app.emit(
        "stt-info",
        SttInfoPayload {
            message,
            caller: caller.to_string(),
        },
    );
}

fn emit_partial(app: &tauri::AppHandle, text: String, caller: &str, seq: u64) {
    let _ = app.emit(
        "stt-partial",
        SttEventPayload {
            text,
            caller: caller.to_string(),
            seq,
        },
    );
}

fn emit_final(app: &tauri::AppHandle, text: String, caller: &str, seq: u64) {
    let _ = app.emit(
        "stt-final",
        SttEventPayload {
            text,
            caller: caller.to_string(),
            seq,
        },
    );
}

/// Emit a final transcript line, but suppress it when SenseVoice repeats
/// itself on adjacent VAD segments. This happens occasionally when the VAD
/// splits an utterance at an unlucky point and both pieces get decoded to
/// the same phrase. `last_final` is updated with whatever we end up keeping
/// (so any legitimate later repeat of the same phrase, spaced by other
/// content, still goes through).
fn emit_final_deduped(
    app: &tauri::AppHandle,
    text: String,
    caller: &str,
    seq: u64,
    last_final: &mut String,
) {
    if text.is_empty() {
        return;
    }
    if text == *last_final {
        return;
    }
    *last_final = text.clone();
    emit_final(app, text, caller, seq);
}

fn create_recognizer_with_fallback(model: &SttModelInfo) -> Result<RecognizerInitResult, String> {
    let (language, requested_backend) = stt_runtime_preferences();
    let requested_cfg = build_sense_voice_config_for_backend(model, &language, &requested_backend)?;

    if let Some(recognizer) = OfflineRecognizer::create(&requested_cfg) {
        return Ok(RecognizerInitResult {
            recognizer,
            execution_backend: requested_backend,
            fallback_from: None,
        });
    }

    if requested_backend == STT_BACKEND_CPU {
        return Err(format!(
            "SenseVoice 認識器の作成に失敗しました ({})",
            stt_execution_backend_label(STT_BACKEND_CPU)
        ));
    }

    // Non-CPU backend (CoreML) failed — fall back to CPU and report via
    // emit_info so the user knows when a requested backend falls back to CPU.
    // (it uses a helper process), so this fallback is CoreML-only in practice.
    log::warn!(
        "[stt] {} init failed; retrying with CPU",
        stt_execution_backend_label(&requested_backend)
    );

    let cpu_cfg = build_sense_voice_config_for_backend(model, &language, STT_BACKEND_CPU)?;
    let recognizer = OfflineRecognizer::create(&cpu_cfg).ok_or_else(|| {
        format!(
            "SenseVoice 認識器の作成に失敗しました ({} / CPU)",
            stt_execution_backend_label(&requested_backend)
        )
    })?;

    Ok(RecognizerInitResult {
        recognizer,
        execution_backend: STT_BACKEND_CPU.into(),
        fallback_from: Some(requested_backend),
    })
}

/// SenseVoice occasionally emits inline metadata tokens such as
/// `<|ja|><|NEUTRAL|><|Speech|><|withitn|>` at the start of decoded text
/// (and sometimes mid-output between utterances). Strip anything inside
/// `<|...|>` so users and the downstream AI summariser never see them.
fn strip_sense_voice_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' && i + 1 < bytes.len() && bytes[i + 1] == b'|' {
            // Find the matching "|>" closing marker.
            if let Some(rel) = text[i + 2..].find("|>") {
                i += 2 + rel + 2;
                continue;
            }
        }
        // UTF-8-safe copy by char boundary: advance one char.
        let ch_end = text[i..]
            .char_indices()
            .nth(1)
            .map(|(n, _)| i + n)
            .unwrap_or(bytes.len());
        out.push_str(&text[i..ch_end]);
        i = ch_end;
    }
    out.trim().to_string()
}

fn decode_samples(recognizer: &OfflineRecognizer, sample_rate: i32, samples: &[f32]) -> String {
    if samples.is_empty() {
        return String::new();
    }
    let stream = recognizer.create_stream();
    stream.accept_waveform(sample_rate, samples);
    recognizer.decode(&stream);
    stream
        .get_result()
        .map(|r| strip_sense_voice_tags(r.text.trim()))
        .unwrap_or_default()
}

fn read_f32_samples(path: &Path) -> Result<Vec<f32>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("STT helper sample read failed: {e}"))?;
    if !bytes.len().is_multiple_of(4) {
        return Err("STT helper sample file is not aligned to f32".into());
    }
    f32_samples_from_bytes(&bytes)
}

fn f32_samples_from_bytes(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if !bytes.len().is_multiple_of(4) {
        return Err("STT helper sample payload is not aligned to f32".into());
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

fn f32_samples_from_request(request: &SttHelperDecodeRequest) -> Result<Vec<f32>, String> {
    if !request.sample_data.is_empty() {
        let bytes = BASE64_STANDARD
            .decode(&request.sample_data)
            .map_err(|e| format!("STT helper sample payload decode failed: {e}"))?;
        f32_samples_from_bytes(&bytes)
    } else {
        read_f32_samples(Path::new(&request.samples))
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SttHelperDecodeRequest {
    sample_rate: i32,
    #[serde(default)]
    samples: String,
    #[serde(default)]
    sample_data: String,
    #[serde(default)]
    shutdown: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct SttHelperDecodeResponse {
    ready: bool,
    ok: bool,
    text: String,
    error: String,
}

fn write_helper_response(response: &SttHelperDecodeResponse) -> Result<(), String> {
    let mut stdout = std::io::stdout();
    serde_json::to_writer(&mut stdout, response)
        .map_err(|e| format!("STT helper response serialization failed: {e}"))?;
    stdout
        .write_all(b"\n")
        .map_err(|e| format!("STT helper response write failed: {e}"))?;
    stdout
        .flush()
        .map_err(|e| format!("STT helper response flush failed: {e}"))
}

fn stt_helper_ready_response() -> SttHelperDecodeResponse {
    SttHelperDecodeResponse {
        ready: true,
        ok: true,
        text: String::new(),
        error: String::new(),
    }
}

fn stt_helper_error_response(error: impl Into<String>) -> SttHelperDecodeResponse {
    SttHelperDecodeResponse {
        ready: false,
        ok: false,
        text: String::new(),
        error: error.into(),
    }
}

fn stt_helper_decode_response(text: String) -> SttHelperDecodeResponse {
    SttHelperDecodeResponse {
        ready: false,
        ok: true,
        text,
        error: String::new(),
    }
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
}

fn helper_recognizer_config_from_args(args: &[String]) -> Result<OfflineRecognizerConfig, String> {
    let model_path = PathBuf::from(arg_value(args, "--model").ok_or("missing --model")?);
    let tokens_path = PathBuf::from(arg_value(args, "--tokens").ok_or("missing --tokens")?);
    let language = arg_value(args, "--language").unwrap_or_else(|| "ja".into());
    let provider_arg = arg_value(args, "--provider").unwrap_or_else(|| STT_BACKEND_CPU.into());
    let provider = provider_arg;

    let mut config = OfflineRecognizerConfig::default();
    config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
        model: Some(model_path.to_string_lossy().into_owned()),
        language: Some(language),
        use_itn: true,
    };
    config.model_config.tokens = Some(tokens_path.to_string_lossy().into_owned());
    config.model_config.provider = Some(provider);
    config.model_config.num_threads = 4;
    config.decoding_method = Some("greedy_search".into());
    Ok(config)
}

fn run_decode_server_from_args(args: &[String]) -> i32 {
    let result: Result<(), String> = (|| {
        let config = helper_recognizer_config_from_args(args)?;
        let recognizer = match OfflineRecognizer::create(&config) {
            Some(recognizer) => recognizer,
            None => {
                let _ = write_helper_response(&stt_helper_error_response(
                    "failed to create STT helper recognizer",
                ));
                return Ok(());
            }
        };
        write_helper_response(&stt_helper_ready_response())?;

        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let line = line.map_err(|e| format!("STT helper request read failed: {e}"))?;
            if line.trim().is_empty() {
                continue;
            }
            let request: SttHelperDecodeRequest = serde_json::from_str(&line)
                .map_err(|e| format!("STT helper request parse failed: {e}"))?;
            if request.shutdown {
                break;
            }
            let response = match f32_samples_from_request(&request) {
                Ok(samples) => {
                    let text = decode_samples(&recognizer, request.sample_rate, &samples);
                    stt_helper_decode_response(text)
                }
                Err(err) => stt_helper_error_response(err),
            };
            write_helper_response(&response)?;
        }
        Ok(())
    })();

    match result {
        Ok(()) => 0,
        Err(err) => {
            let _ = write_helper_response(&stt_helper_error_response(err.clone()));
            eprintln!("{err}");
            1
        }
    }
}

pub fn run_decode_helper_from_args() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == STT_DECODE_HELPER_ARG) {
        return Some(run_decode_server_from_args(&args));
    }
    if !args.iter().any(|arg| arg == STT_DECODE_HELPER_ARG) {
        return None;
    }

    let result: Result<(), String> = (|| {
        let sample_rate = arg_value(&args, "--sample-rate")
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or(TARGET_SAMPLE_RATE);
        let samples_path = PathBuf::from(arg_value(&args, "--samples").ok_or("missing --samples")?);
        let samples = read_f32_samples(&samples_path)?;

        let config = helper_recognizer_config_from_args(&args)?;
        let recognizer =
            OfflineRecognizer::create(&config).ok_or("failed to create STT helper recognizer")?;
        let text = decode_samples(&recognizer, sample_rate, &samples);
        println!("{text}");
        Ok(())
    })();

    match result {
        Ok(()) => Some(0),
        Err(err) => {
            eprintln!("{err}");
            Some(1)
        }
    }
}

/// Design a Hamming-windowed sinc low-pass FIR.
/// `fs` is the input sample rate the filter runs at, `fc` the cutoff in Hz.
fn design_lowpass_fir(fs: f32, fc: f32, m: usize) -> Vec<f32> {
    let mid = (m as f32 - 1.0) / 2.0;
    let fc_norm = fc / fs; // 0..0.5
    let two_pi = 2.0 * std::f32::consts::PI;
    let mut taps: Vec<f32> = (0..m)
        .map(|n| {
            let x = n as f32 - mid;
            let sinc = if x.abs() < 1e-6 {
                2.0 * fc_norm
            } else {
                (two_pi * fc_norm * x).sin() / (std::f32::consts::PI * x)
            };
            let window = 0.54 - 0.46 * (two_pi * n as f32 / (m as f32 - 1.0)).cos();
            sinc * window
        })
        .collect();
    let sum: f32 = taps.iter().sum();
    if sum.abs() > 1e-6 {
        for v in taps.iter_mut() {
            *v /= sum;
        }
    }
    taps
}

/// Stateful resampler: stereo/mono-interleaved input → 16 kHz mono.
///
/// For source rates above the target we apply a windowed-sinc low-pass
/// before decimation to avoid aliasing (the previous pure-linear path
/// folded the 8-24 kHz band into speech, hurting sibilants). State is
/// carried across chunks so the filter has no boundary transients.
struct Resampler {
    src_rate: i32,
    channels: usize,
    taps: Vec<f32>,
    history: Vec<f32>,
    /// Scratch buffers reused across calls to avoid per-frame allocations
    /// in the audio hot path. Capacity grows once during warmup.
    scratch_mono: Vec<f32>,
    scratch_buf: Vec<f32>,
    scratch_filtered: Vec<f32>,
}

impl Resampler {
    fn new(src_rate: i32, channels: usize) -> Self {
        // Only engage the FIR when we actually need to band-limit. At 16 kHz
        // input the cutoff would eat useful energy; the caller handles that
        // case via the early-return in `process`.
        let taps = if src_rate > TARGET_SAMPLE_RATE {
            // ~7.5 kHz cutoff gives ~500 Hz guard band below Nyquist.
            // 63-tap Hamming yields ~60 dB stopband attenuation, plenty for
            // STT purposes; cost is ~63 mul-adds per input sample.
            design_lowpass_fir(src_rate as f32, 7500.0, 63)
        } else {
            Vec::new()
        };
        let history_len = taps.len().saturating_sub(1);
        Self {
            src_rate,
            channels: channels.max(1),
            taps,
            history: vec![0.0; history_len],
            scratch_mono: Vec::new(),
            scratch_buf: Vec::new(),
            scratch_filtered: Vec::new(),
        }
    }

    fn process(&mut self, interleaved: &[f32]) -> Vec<f32> {
        if interleaved.is_empty() {
            return Vec::new();
        }
        // Downmix to mono into reused scratch buffer.
        self.scratch_mono.clear();
        if self.channels == 1 {
            self.scratch_mono.extend_from_slice(interleaved);
        } else {
            let inv = 1.0 / self.channels as f32;
            self.scratch_mono.reserve(interleaved.len() / self.channels);
            for frame in interleaved.chunks(self.channels) {
                let sum: f32 = frame.iter().copied().sum();
                self.scratch_mono.push(sum * inv);
            }
        }

        if self.taps.is_empty() {
            // src == target rate: no filtering or resampling needed.
            // Return a fresh Vec so the caller can mutate independently of
            // the next process() call.
            return self.scratch_mono.clone();
        }

        // Apply stateful FIR: prepend history, convolve, emit samples that
        // had full filter context, carry the tail forward.
        let m = self.taps.len();
        self.scratch_buf.clear();
        self.scratch_buf
            .reserve(self.history.len() + self.scratch_mono.len());
        self.scratch_buf.extend_from_slice(&self.history);
        self.scratch_buf.extend_from_slice(&self.scratch_mono);

        let n_out = self.scratch_buf.len().saturating_sub(m - 1);
        self.scratch_filtered.clear();
        self.scratch_filtered.reserve(n_out);
        for i in 0..n_out {
            let mut acc = 0.0f32;
            for k in 0..m {
                acc += self.taps[k] * self.scratch_buf[i + k];
            }
            self.scratch_filtered.push(acc);
        }
        self.history.clear();
        self.history
            .extend_from_slice(&self.scratch_buf[self.scratch_buf.len().saturating_sub(m - 1)..]);

        // Linear interpolation from src_rate → 16 kHz on the already
        // band-limited signal. For the common 48 kHz input the step is
        // exactly 3.0 so there is no phase jitter across chunks; for odd
        // rates (44.1 kHz) the sub-sample jitter at chunk edges is well
        // under 1 input sample — negligible for STT.
        let ratio = TARGET_SAMPLE_RATE as f64 / self.src_rate as f64;
        let out_len = ((self.scratch_filtered.len() as f64) * ratio).round() as usize;
        let mut out = Vec::with_capacity(out_len);
        for i in 0..out_len {
            let pos = i as f64 / ratio;
            let idx = pos.floor() as usize;
            let frac = (pos - idx as f64) as f32;
            let s0 = *self.scratch_filtered.get(idx).unwrap_or(&0.0);
            let s1 = *self.scratch_filtered.get(idx + 1).unwrap_or(&s0);
            out.push(s0 + (s1 - s0) * frac);
        }
        out
    }
}

/// Soft automatic-gain control.
///
/// Tracks a slow EMA of the peak sample observed while the VAD reports
/// speech, then scales chunks by a gain that brings that tracked peak
/// toward a conventional speech level. Gain is clamped to [1.0, 2.0] so
/// we never attenuate and can't boost a whisper into distortion. Updates
/// only happen during speech so room tone can't pull the reference down.
struct Agc {
    ema_peak: f32,
    initialized: bool,
}

impl Agc {
    const TARGET_PEAK: f32 = 0.5;
    const MAX_GAIN: f32 = 2.0;
    const ATTACK: f32 = 0.15; // fast when a louder sample arrives
    const RELEASE: f32 = 0.02; // slow when the running peak decays

    fn new() -> Self {
        Self {
            ema_peak: 0.0,
            initialized: false,
        }
    }

    fn apply(&mut self, samples: &mut [f32], in_speech: bool) {
        if samples.is_empty() {
            return;
        }
        if in_speech {
            let chunk_peak: f32 = samples.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
            if !self.initialized {
                self.ema_peak = chunk_peak;
                self.initialized = true;
            } else if chunk_peak > self.ema_peak {
                self.ema_peak = self.ema_peak + Self::ATTACK * (chunk_peak - self.ema_peak);
            } else {
                self.ema_peak = self.ema_peak + Self::RELEASE * (chunk_peak - self.ema_peak);
            }
        }
        if !self.initialized || self.ema_peak < 1e-4 {
            return;
        }
        let gain = (Self::TARGET_PEAK / self.ema_peak).clamp(1.0, Self::MAX_GAIN);
        if gain <= 1.001 {
            return;
        }
        for s in samples.iter_mut() {
            *s = (*s * gain).clamp(-1.0, 1.0);
        }
    }
}

/// Root-mean-square of a slice. Returns 0 for empty input.
fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

/// Noise-floor gate. While no speech is in progress, skip VAD inference on
/// chunks that are clearly below any plausible speech level. Silero VAD is
/// lightweight but still runs ONNX forward passes every 512 samples; gating
/// lets long silent stretches cost almost nothing.
/// Threshold corresponds to roughly -55 dBFS.
const RMS_GATE: f32 = 0.0018;

fn normalize_i16_input(data: &[i16]) -> Vec<f32> {
    data.iter().map(|&s| s as f32 / i16::MAX as f32).collect()
}

fn normalize_u16_input(data: &[u16]) -> Vec<f32> {
    data.iter()
        .map(|&s| (s as f32 / u16::MAX as f32) * 2.0 - 1.0)
        .collect()
}

const PARTIAL_WINDOW_SECS: usize = 5;
/// Tail window we probe to decide whether the utterance has momentarily
/// fallen silent. Roughly one natural syllable gap.
const PARTIAL_TAIL_WINDOW_SAMPLES: usize = TARGET_SAMPLE_RATE as usize / 2; // 500 ms
/// Historical partial-tail cutoff. Low sensitivity's noise gate is louder
/// than this; using that gate here freezes quiet live captions again.
const PARTIAL_TAIL_SILENCE_RMS_CAP: f32 = 0.003;
/// After an utterance ends, keep feeding VAD even if the next chunks dip near
/// the noise gate. A continuation from the same speaker is otherwise dropped
/// and recognition looks like it stopped while people are still talking.
const UTTERANCE_RESTART_GRACE: Duration = Duration::from_millis(1_500);
/// Hard stop for the restart window so room tone cannot keep VAD awake.
const UTTERANCE_RESTART_LIMIT: Duration = Duration::from_millis(4_000);

fn partial_tail_silence_rms(rms_gate: f32) -> f32 {
    rms_gate.min(PARTIAL_TAIL_SILENCE_RMS_CAP)
}

fn arm_utterance_restart(now: Instant) -> (Instant, Instant) {
    (now + UTTERANCE_RESTART_GRACE, now + UTTERANCE_RESTART_LIMIT)
}

/// Keep the restart window open while sound is still arriving, but never
/// past the deadline captured when the utterance ended.
fn extend_restart_grace(
    now: Instant,
    grace_until: Instant,
    grace_deadline: Instant,
    in_utterance: bool,
    chunk_rms: f32,
    rms_gate: f32,
) -> Instant {
    if in_utterance || now >= grace_until || now >= grace_deadline {
        return grace_until;
    }
    if chunk_rms < rms_gate * 0.5 {
        return grace_until;
    }
    grace_until.max((now + UTTERANCE_RESTART_GRACE).min(grace_deadline))
}

fn trim_live_utterance(samples: &mut Vec<f32>) {
    let max_keep = (PARTIAL_WINDOW_SECS + 1) * TARGET_SAMPLE_RATE as usize;
    if samples.len() <= max_keep {
        return;
    }
    let drop_n = samples.len() - max_keep;
    samples.copy_within(drop_n.., 0);
    samples.truncate(max_keep);
}

/// Returns an owned copy of the audio window to decode for a partial result,
/// advancing throttle state when the tail is silent. Returns None if the
/// tick should be skipped entirely.
///
/// window_secs: how many seconds of tail audio to decode.
/// tail_silence_rms should be the sensitivity noise gate, not a louder cutoff.
/// A louder cutoff treats real but quiet speech as a lull and freezes the live
/// subtitle until VAD eventually endpoints.
fn partial_decode_slice(
    current_samples: &[f32],
    last_partial_at: &mut Instant,
    stable_streak: &mut u32,
    window_secs: usize,
    tail_silence_rms: f32,
) -> Option<Vec<f32>> {
    let partial_profile = stt_partial_throttle_profile(&load_config().partial_mode);
    partial_decode_slice_with_profile(
        current_samples,
        last_partial_at,
        stable_streak,
        window_secs,
        &partial_profile,
        tail_silence_rms,
    )
}

fn partial_decode_slice_with_profile(
    current_samples: &[f32],
    last_partial_at: &mut Instant,
    stable_streak: &mut u32,
    window_secs: usize,
    partial_profile: &SttPartialThrottleProfile,
    tail_silence_rms: f32,
) -> Option<Vec<f32>> {
    if current_samples.len() < (TARGET_SAMPLE_RATE as usize / 2) {
        return None;
    }
    if !partial_profile.enabled {
        return None;
    }
    let base_interval = if *stable_streak >= 4 {
        partial_profile.very_stable_interval_ms
    } else if *stable_streak >= 2 {
        partial_profile.stable_interval_ms
    } else {
        partial_profile.min_interval_ms
    };
    if last_partial_at.elapsed() < Duration::from_millis(base_interval) {
        return None;
    }
    // If the tail of the utterance is currently silent, nothing the decoder
    // produces can differ from last time. VAD has not cut the segment yet,
    // but the speaker is between phrases. Skip the encode entirely.
    let tail_start = current_samples
        .len()
        .saturating_sub(PARTIAL_TAIL_WINDOW_SAMPLES);
    if rms(&current_samples[tail_start..]) < tail_silence_rms {
        *stable_streak = stable_streak.saturating_add(1);
        *last_partial_at = Instant::now();
        return None;
    }
    // Only decode the tail window. SenseVoice is non-streaming, so re-encoding
    // the full utterance every partial tick is the dominant CPU cost.
    let window = window_secs * TARGET_SAMPLE_RATE as usize;
    let slice = if current_samples.len() > window {
        &current_samples[current_samples.len() - window..]
    } else {
        current_samples
    };
    *stable_streak = 0;
    *last_partial_at = Instant::now();
    Some(slice.to_vec())
}

static STT_EVENT_SEQ: AtomicU64 = AtomicU64::new(1);

fn next_stt_event_seq() -> u64 {
    STT_EVENT_SEQ.fetch_add(1, Ordering::SeqCst)
}

#[derive(Debug)]
enum SttDecodeJob {
    Partial { seq: u64, samples: Vec<f32> },
    Final { seq: u64, samples: Vec<f32> },
    Shutdown,
}

fn enqueue_stt_decode_job(
    jobs: &mut VecDeque<SttDecodeJob>,
    job: SttDecodeJob,
    prioritize_partials: bool,
) {
    let is_partial = matches!(job, SttDecodeJob::Partial { .. });
    if is_partial {
        // A newer partial supersedes any partial still waiting.
        jobs.retain(|existing| !matches!(existing, SttDecodeJob::Partial { .. }));
        if prioritize_partials {
            // Shared-queue fallback only. Live does not use this path: it has
            // a second recognizer, so an in-flight final cannot block a partial.
            // Finals stay in their own order. The UI ignores an older final
            // that would otherwise wipe the newer partial.
            let insert_at = jobs
                .iter()
                .position(|existing| {
                    matches!(
                        existing,
                        SttDecodeJob::Final { .. } | SttDecodeJob::Shutdown
                    )
                })
                .unwrap_or(jobs.len());
            jobs.insert(insert_at, job);
            return;
        }
    }
    jobs.push_back(job);
}

struct SttDecodeInbox {
    jobs: Mutex<VecDeque<SttDecodeJob>>,
    cv: Condvar,
    failed: Arc<AtomicBool>,
    prioritize_partials: bool,
}

impl SttDecodeInbox {
    fn new(prioritize_partials: bool, failed: Arc<AtomicBool>) -> Self {
        Self {
            jobs: Mutex::new(VecDeque::new()),
            cv: Condvar::new(),
            failed,
            prioritize_partials,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<SttDecodeJob>> {
        self.jobs.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn push(&self, job: SttDecodeJob) {
        let mut jobs = self.lock();
        enqueue_stt_decode_job(&mut jobs, job, self.prioritize_partials);
        self.cv.notify_one();
    }

    fn discard_pending(&self) {
        let mut jobs = self.lock();
        jobs.retain(|job| matches!(job, SttDecodeJob::Shutdown));
        self.cv.notify_one();
    }

    fn pop(&self) -> SttDecodeJob {
        let mut jobs = self.lock();
        loop {
            if let Some(job) = jobs.pop_front() {
                return job;
            }
            jobs = self.cv.wait(jobs).unwrap_or_else(|err| err.into_inner());
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().len()
    }
}

#[derive(Clone, Copy)]
enum SttDecodeLane {
    /// Agent and other short inputs: one recognizer handles both job kinds.
    Combined,
    /// Live partials. A final already being decoded must not block these.
    Partial,
    /// Live finals, kept in arrival order on their own recognizer.
    Final,
}

struct DecodePanicGuard {
    failed: Arc<AtomicBool>,
}

impl Drop for DecodePanicGuard {
    fn drop(&mut self) {
        if std::thread::panicking() {
            log::error!("[stt] recognizer thread panicked; stopping recognition");
            self.failed.store(true, Ordering::SeqCst);
        }
    }
}

struct LastPartialText {
    text: String,
    seq: u64,
}

fn lock_last_partial(
    last_partial: &Mutex<LastPartialText>,
) -> std::sync::MutexGuard<'_, LastPartialText> {
    last_partial.lock().unwrap_or_else(|err| err.into_inner())
}

struct SttDecodeWorker {
    partial_inbox: Arc<SttDecodeInbox>,
    final_inbox: Arc<SttDecodeInbox>,
    joins: Vec<std::thread::JoinHandle<()>>,
    failed: Arc<AtomicBool>,
}

impl SttDecodeWorker {
    fn spawn(
        app: tauri::AppHandle,
        caller: String,
        recognizer: OfflineRecognizer,
        live_partial_recognizer: Option<OfflineRecognizer>,
    ) -> Result<Self, String> {
        let failed = Arc::new(AtomicBool::new(false));
        let last_partial = Arc::new(Mutex::new(LastPartialText {
            text: String::new(),
            seq: 0,
        }));
        if let Some(partial_recognizer) = live_partial_recognizer {
            return Self::spawn_split(
                app,
                caller,
                recognizer,
                partial_recognizer,
                failed,
                last_partial,
            );
        }

        let inbox = Arc::new(SttDecodeInbox::new(false, Arc::clone(&failed)));
        let inbox_thread = Arc::clone(&inbox);
        let join = std::thread::Builder::new()
            .name("stt-decode".into())
            .spawn(move || {
                decode_worker_loop(
                    app,
                    caller,
                    recognizer,
                    inbox_thread,
                    SttDecodeLane::Combined,
                    last_partial,
                )
            })
            .map_err(|err| format!("音声認識スレッドの起動に失敗しました: {}", err))?;
        Ok(Self {
            partial_inbox: Arc::clone(&inbox),
            final_inbox: inbox,
            joins: vec![join],
            failed,
        })
    }

    fn spawn_split(
        app: tauri::AppHandle,
        caller: String,
        final_recognizer: OfflineRecognizer,
        partial_recognizer: OfflineRecognizer,
        failed: Arc<AtomicBool>,
        last_partial: Arc<Mutex<LastPartialText>>,
    ) -> Result<Self, String> {
        let partial_inbox = Arc::new(SttDecodeInbox::new(false, Arc::clone(&failed)));
        let final_inbox = Arc::new(SttDecodeInbox::new(false, Arc::clone(&failed)));
        let partial_for_thread = Arc::clone(&partial_inbox);
        let final_for_thread = Arc::clone(&final_inbox);
        let partial_app = app.clone();
        let final_caller = caller.clone();
        let partial_last = Arc::clone(&last_partial);
        let partial_join = std::thread::Builder::new()
            .name("stt-decode-partial".into())
            .spawn(move || {
                decode_worker_loop(
                    partial_app,
                    caller,
                    partial_recognizer,
                    partial_for_thread,
                    SttDecodeLane::Partial,
                    partial_last,
                )
            })
            .map_err(|err| format!("音声認識スレッドの起動に失敗しました: {}", err))?;
        let final_join = match std::thread::Builder::new()
            .name("stt-decode-final".into())
            .spawn(move || {
                decode_worker_loop(
                    app,
                    final_caller,
                    final_recognizer,
                    final_for_thread,
                    SttDecodeLane::Final,
                    last_partial,
                )
            }) {
            Ok(join) => join,
            Err(err) => {
                partial_inbox.push(SttDecodeJob::Shutdown);
                let _ = partial_join.join();
                return Err(format!("音声認識スレッドの起動に失敗しました: {}", err));
            }
        };
        Ok(Self {
            partial_inbox,
            final_inbox,
            joins: vec![partial_join, final_join],
            failed,
        })
    }

    fn failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }

    fn lanes_are_split(&self) -> bool {
        !Arc::ptr_eq(&self.partial_inbox, &self.final_inbox)
    }

    fn push_partial(&self, samples: Vec<f32>) {
        if samples.is_empty() || self.failed() {
            return;
        }
        self.partial_inbox.push(SttDecodeJob::Partial {
            seq: next_stt_event_seq(),
            samples,
        });
    }

    fn push_final(&self, samples: Vec<f32>) {
        if samples.is_empty() || self.failed() {
            return;
        }
        self.final_inbox.push(SttDecodeJob::Final {
            seq: next_stt_event_seq(),
            samples,
        });
    }
}

impl Drop for SttDecodeWorker {
    fn drop(&mut self) {
        let discard = STT_SHUTDOWN_REQUESTED.load(Ordering::SeqCst) || self.failed();
        if discard {
            self.partial_inbox.discard_pending();
            if self.lanes_are_split() {
                self.final_inbox.discard_pending();
            }
        }
        self.partial_inbox.push(SttDecodeJob::Shutdown);
        if self.lanes_are_split() {
            self.final_inbox.push(SttDecodeJob::Shutdown);
        }
        for join in self.joins.drain(..) {
            let _ = join.join();
        }
    }
}

fn decode_worker_loop(
    app: tauri::AppHandle,
    caller: String,
    recognizer: OfflineRecognizer,
    inbox: Arc<SttDecodeInbox>,
    lane: SttDecodeLane,
    last_partial: Arc<Mutex<LastPartialText>>,
) {
    let _panic_guard = DecodePanicGuard {
        failed: Arc::clone(&inbox.failed),
    };
    let mut last_final = String::new();
    loop {
        if inbox.failed.load(Ordering::SeqCst) {
            break;
        }
        match inbox.pop() {
            SttDecodeJob::Shutdown => break,
            SttDecodeJob::Partial { seq, samples } => {
                if matches!(lane, SttDecodeLane::Final) {
                    log::warn!("[stt] final decoder received a partial; dropping");
                    continue;
                }
                if STT_SHUTDOWN_REQUESTED.load(Ordering::SeqCst) {
                    continue;
                }
                let text = match decode_samples_safely(&recognizer, &samples) {
                    Ok(text) => text,
                    Err(()) => {
                        inbox.failed.store(true, Ordering::SeqCst);
                        break;
                    }
                };
                let mut seen = lock_last_partial(&last_partial);
                // An older partial can finish after a newer final. Do not let
                // it overwrite the text the next partial is compared against.
                if text.is_empty() || seq < seen.seq || text == seen.text {
                    continue;
                }
                seen.text = text.clone();
                seen.seq = seq;
                drop(seen);
                emit_partial(&app, text, &caller, seq);
            }
            SttDecodeJob::Final { seq, samples } => {
                if matches!(lane, SttDecodeLane::Partial) {
                    log::warn!("[stt] partial decoder received a final; dropping");
                    continue;
                }
                if STT_SHUTDOWN_REQUESTED.load(Ordering::SeqCst) {
                    continue;
                }
                let text = match decode_samples_safely(&recognizer, &samples) {
                    Ok(text) => text,
                    Err(()) => {
                        inbox.failed.store(true, Ordering::SeqCst);
                        break;
                    }
                };
                if text.is_empty() {
                    continue;
                }
                {
                    let mut seen = lock_last_partial(&last_partial);
                    if seq >= seen.seq {
                        seen.text = text.clone();
                        seen.seq = seq;
                    }
                }
                emit_final_deduped(&app, text, &caller, seq, &mut last_final);
            }
        }
    }
}

fn decode_samples_safely(recognizer: &OfflineRecognizer, samples: &[f32]) -> Result<String, ()> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        decode_samples(recognizer, TARGET_SAMPLE_RATE, samples)
    })) {
        Ok(text) => Ok(text),
        Err(_) => {
            log::error!("[stt] recognizer panicked; stopping recognition");
            Err(())
        }
    }
}

fn drain_vad_segments(vad: &VoiceActivityDetector, worker: &SttDecodeWorker) -> bool {
    let mut drained = false;
    while !vad.is_empty() {
        if let Some(segment) = vad.front() {
            let samples = segment.samples().to_vec();
            drop(segment);
            worker.push_final(samples);
        }
        vad.pop();
        drained = true;
    }
    drained
}

fn choose_input_config(
    device: &cpal::Device,
) -> Result<(cpal::SupportedStreamConfig, usize), String> {
    if let Ok(configs) = device.supported_input_configs() {
        for range in configs {
            if range.channels() == 1
                && range.min_sample_rate().0 <= TARGET_SAMPLE_RATE as u32
                && range.max_sample_rate().0 >= TARGET_SAMPLE_RATE as u32
            {
                return Ok((
                    range.with_sample_rate(cpal::SampleRate(TARGET_SAMPLE_RATE as u32)),
                    1,
                ));
            }
        }
    }
    let cfg = device
        .default_input_config()
        .map_err(|e| format!("マイク設定取得失敗: {}", e))?;
    let channels = cfg.channels() as usize;
    Ok((cfg, channels))
}

fn run_stt_session(
    app: tauri::AppHandle,
    session_id: u64,
    stop_rx: mpsc::Receiver<()>,
    caller: &str,
) {
    let result: Result<(), String> = (|| {
        let model = selected_model_from_config()?;
        ensure_stt_model_downloaded(&model)?;
        if !is_stt_model_downloaded(&model) {
            return Err("STT モデルがまだダウンロードされていません".into());
        }

        emit_state(&app, "initializing", caller);
        let recognizer_init = create_recognizer_with_fallback(&model)?;
        update_runtime_debug_state(
            "initializing",
            Some(caller),
            Some(recognizer_init.execution_backend.as_str()),
            recognizer_init.fallback_from.as_deref(),
        );
        if let Some(fallback_from) = recognizer_init.fallback_from.as_ref() {
            emit_info(&app, stt_fallback_message(fallback_from), caller);
        }
        // Decode off this thread. SenseVoice is offline, so one partial or
        // final can take longer than the audio it covers. Decoding here
        // starves the VAD, and live speech stops being detected.
        // Live loads a second recognizer so a final already being decoded
        // cannot block the next partial. Agent input keeps one recognizer.
        let live_partial_recognizer = if caller == "live" {
            log::info!(
                "[stt] live captions load a second recognizer so partials are not blocked by an in-flight final"
            );
            let partial_init = create_recognizer_with_fallback(&model).map_err(|err| {
                format!(
                    "リアルタイム字幕用の追加認識器を読み込めませんでした: {}",
                    err
                )
            })?;
            if recognizer_init.fallback_from.is_none() {
                if let Some(fallback_from) = partial_init.fallback_from.as_ref() {
                    log::warn!(
                        "[stt] live partial recognizer fell back from {}",
                        fallback_from
                    );
                    emit_info(&app, stt_fallback_message(fallback_from), caller);
                }
            }
            Some(partial_init.recognizer)
        } else {
            None
        };
        let recognizer = recognizer_init.recognizer;
        let decode_worker = SttDecodeWorker::spawn(
            app.clone(),
            caller.to_string(),
            recognizer,
            live_partial_recognizer,
        )?;
        let sensitivity_profile = stt_sensitivity_profile(&load_config().sensitivity);
        let vad = VoiceActivityDetector::create(&build_vad_config(&sensitivity_profile)?, 30.0)
            .ok_or_else(|| "VAD の初期化に失敗しました".to_string())?;

        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "利用可能なマイク入力デバイスが見つかりません".to_string())?;
        let (supported_cfg, channels) = choose_input_config(&device)?;
        let sample_rate = supported_cfg.sample_rate().0 as i32;
        let mut stream_config = supported_cfg.config();
        // Request ~80ms callback period: at 48kHz mono this is ~3840 frames,
        // roughly 10× fewer wake-ups than cpal's 10ms default. Fewer callbacks
        // ⇒ fewer allocations, fewer channel sends, fewer thread context
        // switches, while still well under the VAD's ~12s speech window.
        let desired_frames = (sample_rate as u32 * channels as u32 * 80) / 1000;
        if let cpal::SupportedBufferSize::Range { min, max } = supported_cfg.buffer_size() {
            let clamped = desired_frames.clamp(*min, *max);
            stream_config.buffer_size = cpal::BufferSize::Fixed(clamped);
        }

        let (audio_tx, audio_rx) = mpsc::channel::<Vec<f32>>();
        let err_app = app.clone();
        let err_caller = caller.to_string();
        let err_fn = move |err| {
            log::error!("[stt] microphone stream error: {}", err);
            emit_error(&err_app, format!("マイク入力エラー: {}", err), &err_caller);
        };

        let input_stream = match supported_cfg.sample_format() {
            cpal::SampleFormat::F32 => device
                .build_input_stream(
                    &stream_config,
                    move |data: &[f32], _| {
                        let _ = audio_tx.send(data.to_vec());
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| format!("マイクストリーム開始失敗: {}", e))?,
            cpal::SampleFormat::I16 => {
                let audio_tx = audio_tx.clone();
                device
                    .build_input_stream(
                        &stream_config,
                        move |data: &[i16], _| {
                            let _ = audio_tx.send(normalize_i16_input(data));
                        },
                        err_fn,
                        None,
                    )
                    .map_err(|e| format!("マイクストリーム開始失敗: {}", e))?
            }
            cpal::SampleFormat::U16 => {
                let audio_tx = audio_tx.clone();
                device
                    .build_input_stream(
                        &stream_config,
                        move |data: &[u16], _| {
                            let _ = audio_tx.send(normalize_u16_input(data));
                        },
                        err_fn,
                        None,
                    )
                    .map_err(|e| format!("マイクストリーム開始失敗: {}", e))?
            }
            other => return Err(format!("未対応の音声フォーマットです: {:?}", other)),
        };

        input_stream
            .play()
            .map_err(|e| format!("マイク入力開始失敗: {}", e))?;
        emit_state(&app, "listening", caller);
        if caller == "live" {
            if let Err(err) =
                crate::power::prevent_sleep_start(Some("KWIC live transcription".into()))
            {
                log::warn!("[stt] failed to prevent system sleep during Live: {}", err);
            }
        }

        let mut current_utterance = Vec::<f32>::new();
        let mut last_partial_at = Instant::now();
        let mut stable_streak: u32 = 0;
        let mut resampler = Resampler::new(sample_rate, channels);
        let mut agc = Agc::new();
        let mut vad_feed_grace_until = Instant::now();
        let mut vad_feed_grace_deadline = Instant::now();

        let should_flush_pending_audio = loop {
            if decode_worker.failed() {
                return Err(
                    "音声認識が中断されました。マイクを一度止めて、もう一度開始してください".into(),
                );
            }
            if stop_rx.try_recv().is_ok() {
                break !STT_SHUTDOWN_REQUESTED.load(Ordering::SeqCst);
            }
            match audio_rx.recv_timeout(Duration::from_millis(120)) {
                Ok(chunk) => {
                    let mut resampled = resampler.process(&chunk);
                    if resampled.is_empty() {
                        continue;
                    }
                    // Noise-floor gate on the raw (pre-AGC) signal. A short
                    // grace after each cut keeps a quiet continuation visible
                    // to VAD; otherwise the next utterance never starts.
                    let chunk_rms = rms(&resampled);
                    let in_utterance = !current_utterance.is_empty() || vad.detected();
                    let now = Instant::now();
                    vad_feed_grace_until = extend_restart_grace(
                        now,
                        vad_feed_grace_until,
                        vad_feed_grace_deadline,
                        in_utterance,
                        chunk_rms,
                        sensitivity_profile.rms_gate,
                    );
                    let in_restart_grace = now < vad_feed_grace_until;
                    if !in_utterance
                        && !in_restart_grace
                        && chunk_rms < sensitivity_profile.rms_gate
                    {
                        continue;
                    }
                    agc.apply(&mut resampled, in_utterance);
                    vad.accept_waveform(&resampled);
                    if vad.detected() {
                        current_utterance.extend_from_slice(&resampled);
                        trim_live_utterance(&mut current_utterance);
                        if let Some(slice) = partial_decode_slice(
                            &current_utterance,
                            &mut last_partial_at,
                            &mut stable_streak,
                            PARTIAL_WINDOW_SECS,
                            partial_tail_silence_rms(sensitivity_profile.rms_gate),
                        ) {
                            decode_worker.push_partial(slice);
                        }
                    }
                    if drain_vad_segments(&vad, &decode_worker) {
                        current_utterance.clear();
                        stable_streak = 0;
                        let (until, deadline) = arm_utterance_restart(Instant::now());
                        vad_feed_grace_until = until;
                        vad_feed_grace_deadline = deadline;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break !STT_SHUTDOWN_REQUESTED.load(Ordering::SeqCst);
                }
            }
        };

        if should_flush_pending_audio && !decode_worker.failed() {
            vad.flush();
            // The flushed segment already contains this utterance. Pushing
            // current_utterance as well emits the same speech twice.
            if !drain_vad_segments(&vad, &decode_worker) && !current_utterance.is_empty() {
                decode_worker.push_final(std::mem::take(&mut current_utterance));
            }
        }

        Ok(())
    })();

    if let Err(err) = result {
        emit_error(&app, err, caller);
    }
    if caller == "live" {
        if let Err(err) = crate::power::prevent_sleep_stop() {
            log::warn!("[stt] failed to release Live sleep prevention: {}", err);
        }
    }
    emit_state(&app, "idle", caller);
    clear_session_if_matches(session_id);
    STT_SHUTDOWN_REQUESTED.store(false, Ordering::SeqCst);
}

#[tauri::command]
pub fn get_stt_config() -> SttConfig {
    load_config()
}

#[tauri::command]
pub fn save_stt_config(app: tauri::AppHandle, mut config: SttConfig) -> Result<(), String> {
    config.selected_model = normalize_stt_model_id(&config.selected_model);
    config.language = normalize_stt_language(&config.language);
    config.execution_backend = validate_stt_execution_backend(&config.execution_backend)?;
    config.partial_mode = normalize_stt_partial_mode(&config.partial_mode);
    config.sensitivity = normalize_stt_sensitivity(&config.sensitivity);
    if stt_model_catalog()
        .iter()
        .all(|m| m.id != config.selected_model)
    {
        return Err("不明な STT モデルです".into());
    }
    save_config(&config)?;
    let _ = app.emit("stt-config-changed", ());
    Ok(())
}

#[tauri::command]
pub fn list_stt_execution_backends() -> Vec<SttExecutionBackendInfo> {
    stt_execution_backend_catalog()
}

#[tauri::command]
pub fn list_stt_models() -> Vec<serde_json::Value> {
    stt_model_catalog()
        .iter()
        .map(|m| {
            serde_json::json!({
                "id": m.id,
                "name": m.name,
                "size_label": m.size_label,
                "file_size_mb": m.file_size_mb,
                "downloaded": is_stt_model_downloaded(m),
            })
        })
        .collect()
}

#[tauri::command]
pub async fn download_stt_model(app: tauri::AppHandle, model_id: String) -> Result<(), String> {
    let model = stt_model_catalog()
        .iter()
        .find(|m| m.id == model_id)
        .cloned()
        .ok_or_else(|| format!("不明な STT モデル: {}", model_id))?;
    let app_clone = app.clone();
    tokio::task::spawn_blocking(move || download_stt_model_blocking(&app_clone, &model))
        .await
        .map_err(|e| format!("タスク実行エラー: {}", e))??;
    let _ = app.emit("stt-config-changed", ());
    Ok(())
}

#[tauri::command]
pub fn delete_stt_model(app: tauri::AppHandle, model_id: String) -> Result<(), String> {
    let model = stt_model_catalog()
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("不明な STT モデル: {}", model_id))?;

    let model_dir = stt_model_dir(model);
    if model_dir.exists() {
        std::fs::remove_dir_all(&model_dir).map_err(|e| format!("削除失敗: {}", e))?;
    }
    let archive = stt_archive_path(model);
    if archive.exists() {
        let _ = std::fs::remove_file(&archive);
    }
    let _ = app.emit("stt-config-changed", ());
    Ok(())
}

#[tauri::command]
pub fn cancel_stt_model_download() {
    cancel_stt_download();
}

#[tauri::command]
pub fn stt_test_model(app: tauri::AppHandle) -> Result<String, String> {
    let model = selected_model_from_config()?;
    ensure_stt_model_downloaded(&model)?;
    if !is_stt_model_downloaded(&model) {
        return Err("STT モデルを先にダウンロードしてください".into());
    }
    let recognizer_init = create_recognizer_with_fallback(&model)?;
    update_runtime_debug_state(
        "test-ok",
        None,
        Some(&recognizer_init.execution_backend),
        recognizer_init.fallback_from.as_deref(),
    );
    let _recognizer = recognizer_init.recognizer;

    if let Some(fallback_from) = recognizer_init.fallback_from {
        let message = format!(
            "OK: {} ({}) / {}",
            model.name,
            stt_execution_backend_label(&recognizer_init.execution_backend),
            stt_fallback_message(&fallback_from)
        );
        update_runtime_debug_message(Some(message.clone()), None);
        emit_runtime_debug_changed(&app);
        return Ok(message);
    }

    let message = format!(
        "OK: {} ({})",
        model.name,
        stt_execution_backend_label(&recognizer_init.execution_backend)
    );
    update_runtime_debug_message(Some(message.clone()), None);
    emit_runtime_debug_changed(&app);
    Ok(message)
}

#[tauri::command]
pub fn stt_is_running() -> bool {
    STT_SESSION.lock().map(|s| s.is_some()).unwrap_or(false)
}

#[tauri::command]
pub fn stt_get_active_caller() -> Option<String> {
    STT_SESSION
        .lock()
        .ok()
        .and_then(|s| s.as_ref().map(|sess| sess.caller.clone()))
}

#[tauri::command]
pub fn stt_start_stream(
    app: tauri::AppHandle,
    caller: String,
    preempt: Option<bool>,
) -> Result<Option<String>, String> {
    STT_SHUTDOWN_REQUESTED.store(false, Ordering::SeqCst);
    let caller = if caller.is_empty() {
        "unknown".to_string()
    } else {
        caller
    };
    let mut lock = STT_SESSION
        .lock()
        .map_err(|_| "STT state lock failed".to_string())?;

    let previous_caller = if let Some(session) = lock.as_ref() {
        if preempt.unwrap_or(false) {
            let prev = session.caller.clone();
            let _ = session.stop_tx.send(());
            *lock = None;
            // Give the previous session a moment to clean up
            Some(prev)
        } else {
            return Err(format!("音声入力は「{}」で使用中です", session.caller));
        }
    } else {
        None
    };

    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::SeqCst);
    *lock = Some(ActiveSttSession {
        id: session_id,
        caller: caller.clone(),
        stop_tx,
    });
    drop(lock);

    let app_clone = app.clone();
    std::thread::spawn(move || run_stt_session(app_clone, session_id, stop_rx, &caller));
    Ok(previous_caller)
}

#[tauri::command]
pub fn stt_stop_stream() -> Result<(), String> {
    let mut lock = STT_SESSION
        .lock()
        .map_err(|_| "STT state lock failed".to_string())?;
    if let Some(session) = lock.take() {
        let _ = session.stop_tx.send(());
    }
    Ok(())
}

pub(crate) fn stt_shutdown_for_exit(timeout: Duration) {
    STT_SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
    let had_session = if let Ok(lock) = STT_SESSION.lock() {
        if let Some(session) = lock.as_ref() {
            let _ = session.stop_tx.send(());
            true
        } else {
            false
        }
    } else {
        log::warn!("[stt] shutdown: STT state lock failed");
        false
    };

    if !had_session {
        STT_SHUTDOWN_REQUESTED.store(false, Ordering::SeqCst);
        return;
    }

    let start = Instant::now();
    while start.elapsed() < timeout {
        if !stt_is_running() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    log::warn!("[stt] shutdown: timed out waiting for STT session to stop");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn balanced_profile() -> SttPartialThrottleProfile {
        SttPartialThrottleProfile {
            enabled: true,
            min_interval_ms: 600,
            stable_interval_ms: 1500,
            very_stable_interval_ms: 5000,
        }
    }

    fn audible_tail() -> Vec<f32> {
        let mut samples = vec![0.0; TARGET_SAMPLE_RATE as usize];
        let tail_start = samples.len() - PARTIAL_TAIL_WINDOW_SAMPLES;
        for sample in &mut samples[tail_start..] {
            *sample = 0.0028;
        }
        samples
    }

    #[test]
    fn audible_tail_is_not_treated_as_silence() {
        let samples = audible_tail();
        let profile = balanced_profile();

        let mut last_partial_at = Instant::now() - Duration::from_secs(10);
        let mut stable_streak = 0;
        let skipped = partial_decode_slice_with_profile(
            &samples,
            &mut last_partial_at,
            &mut stable_streak,
            PARTIAL_WINDOW_SECS,
            &profile,
            0.003,
        );
        assert!(skipped.is_none());
        assert_eq!(stable_streak, 1);

        let mut last_partial_at = Instant::now() - Duration::from_secs(10);
        let mut stable_streak = 4;
        let kept = partial_decode_slice_with_profile(
            &samples,
            &mut last_partial_at,
            &mut stable_streak,
            PARTIAL_WINDOW_SECS,
            &profile,
            0.0018,
        );
        assert!(kept.is_some());
        assert_eq!(stable_streak, 0);
    }

    #[test]
    fn silent_tail_skips_partial_decode() {
        let mut last_partial_at = Instant::now() - Duration::from_secs(10);
        let mut stable_streak = 0;
        let samples = vec![0.0001; TARGET_SAMPLE_RATE as usize];
        let slice = partial_decode_slice_with_profile(
            &samples,
            &mut last_partial_at,
            &mut stable_streak,
            PARTIAL_WINDOW_SECS,
            &balanced_profile(),
            0.0018,
        );
        assert!(slice.is_none());
        assert_eq!(stable_streak, 1);
    }

    #[test]
    fn tail_silence_cutoff_is_never_stricter_than_before() {
        assert_eq!(partial_tail_silence_rms(0.0008), 0.0008);
        assert_eq!(partial_tail_silence_rms(0.0018), 0.0018);
        assert_eq!(partial_tail_silence_rms(0.0035), 0.003);
    }

    #[test]
    fn restart_grace_extends_while_sound_continues_until_deadline() {
        let now = Instant::now();
        let deadline = now + UTTERANCE_RESTART_LIMIT;
        let until = now + Duration::from_millis(200);
        let extended = extend_restart_grace(now, until, deadline, false, 0.002, 0.0018);
        assert_eq!(extended, (now + UTTERANCE_RESTART_GRACE).min(deadline));

        let quiet = extend_restart_grace(now, until, deadline, false, 0.0001, 0.0018);
        assert_eq!(quiet, until);

        let speaking = extend_restart_grace(now, until, deadline, true, 0.02, 0.0018);
        assert_eq!(speaking, until);

        let expired = extend_restart_grace(now, now, deadline, false, 0.02, 0.0018);
        assert_eq!(expired, now);
    }

    #[test]
    fn live_utterance_keeps_only_the_partial_window() {
        let mut samples = vec![1.0; (PARTIAL_WINDOW_SECS + 3) * TARGET_SAMPLE_RATE as usize];
        samples[0] = 7.0;
        trim_live_utterance(&mut samples);
        assert_eq!(
            samples.len(),
            (PARTIAL_WINDOW_SECS + 1) * TARGET_SAMPLE_RATE as usize
        );
        assert_ne!(samples[0], 7.0);
    }

    #[test]
    fn newer_partial_replaces_queued_partial_without_passing_finals() {
        let mut jobs = VecDeque::new();
        enqueue_stt_decode_job(
            &mut jobs,
            SttDecodeJob::Final {
                seq: 1,
                samples: vec![1.0],
            },
            false,
        );
        enqueue_stt_decode_job(
            &mut jobs,
            SttDecodeJob::Partial {
                seq: 2,
                samples: vec![2.0],
            },
            false,
        );
        enqueue_stt_decode_job(
            &mut jobs,
            SttDecodeJob::Partial {
                seq: 3,
                samples: vec![3.0],
            },
            false,
        );
        assert_eq!(jobs.len(), 2);
        match &jobs[0] {
            SttDecodeJob::Final { samples, .. } => assert_eq!(samples, &vec![1.0]),
            other => panic!("expected final, got {:?}", other),
        }
        match &jobs[1] {
            SttDecodeJob::Partial { seq, samples } => {
                assert_eq!(*seq, 3);
                assert_eq!(samples, &vec![3.0]);
            }
            other => panic!("expected partial, got {:?}", other),
        }
    }

    #[test]
    fn live_partial_jumps_ahead_of_queued_finals() {
        let mut jobs = VecDeque::new();
        enqueue_stt_decode_job(
            &mut jobs,
            SttDecodeJob::Final {
                seq: 1,
                samples: vec![1.0],
            },
            true,
        );
        enqueue_stt_decode_job(
            &mut jobs,
            SttDecodeJob::Partial {
                seq: 2,
                samples: vec![2.0],
            },
            true,
        );
        enqueue_stt_decode_job(
            &mut jobs,
            SttDecodeJob::Final {
                seq: 3,
                samples: vec![3.0],
            },
            true,
        );
        enqueue_stt_decode_job(
            &mut jobs,
            SttDecodeJob::Partial {
                seq: 4,
                samples: vec![4.0],
            },
            true,
        );
        assert_eq!(jobs.len(), 3);
        match &jobs[0] {
            SttDecodeJob::Partial { seq, samples } => {
                assert_eq!(*seq, 4);
                assert_eq!(samples, &vec![4.0]);
            }
            other => panic!("expected partial, got {:?}", other),
        }
        match &jobs[1] {
            SttDecodeJob::Final { seq, .. } => assert_eq!(*seq, 1),
            other => panic!("expected first final, got {:?}", other),
        }
        match &jobs[2] {
            SttDecodeJob::Final { seq, .. } => assert_eq!(*seq, 3),
            other => panic!("expected second final, got {:?}", other),
        }
    }

    #[test]
    fn live_partial_lane_is_not_blocked_by_queued_finals() {
        let failed = Arc::new(AtomicBool::new(false));
        let partial_inbox = SttDecodeInbox::new(false, Arc::clone(&failed));
        let final_inbox = SttDecodeInbox::new(false, failed);
        final_inbox.push(SttDecodeJob::Final {
            seq: 1,
            samples: vec![1.0],
        });
        partial_inbox.push(SttDecodeJob::Partial {
            seq: 2,
            samples: vec![2.0],
        });
        final_inbox.push(SttDecodeJob::Final {
            seq: 3,
            samples: vec![3.0],
        });
        partial_inbox.push(SttDecodeJob::Partial {
            seq: 4,
            samples: vec![4.0],
        });
        assert_eq!(partial_inbox.len(), 1);
        assert_eq!(final_inbox.len(), 2);

        match partial_inbox.pop() {
            SttDecodeJob::Partial { seq, samples } => {
                assert_eq!(seq, 4);
                assert_eq!(samples, vec![4.0]);
            }
            other => panic!("expected partial, got {:?}", other),
        }
        match final_inbox.pop() {
            SttDecodeJob::Final { seq, .. } => assert_eq!(seq, 1),
            other => panic!("expected first final, got {:?}", other),
        }
        match final_inbox.pop() {
            SttDecodeJob::Final { seq, .. } => assert_eq!(seq, 3),
            other => panic!("expected second final, got {:?}", other),
        }
    }
}
