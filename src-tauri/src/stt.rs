use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use bzip2::read::BzDecoder;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::{Deserialize, Serialize};
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig, SileroVadModelConfig,
    VadModelConfig, VoiceActivityDetector,
};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::Emitter;

const TARGET_SAMPLE_RATE: i32 = 16_000;
const VAD_MODEL_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx";
const VAD_MODEL_FILE: &str = "silero_vad.onnx";
const STT_BACKEND_CPU: &str = "cpu";
const STT_BACKEND_COREML: &str = "coreml";
const STT_BACKEND_DIRECTML: &str = "directml";
const STT_DECODE_HELPER_ARG: &str = "--selah-stt-decode";
const STT_DECODE_SERVER_ARG: &str = "--selah-stt-decode-server";
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
    /// When true, this model is only listed on platforms where DirectML is
    /// available. It uses fp32 weights required by DirectML but is unnecessary
    /// overhead (4× larger) on CPU-only platforms such as macOS.
    pub requires_directml: bool,
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
        requires_directml: false,
    },
    SttModelInfo {
        id: "sensevoice-ja-en-fp32".into(),
        name: "SenseVoice 高精度".into(),
        size_label: "894 MB（DirectML 専用）".into(),
        archive_name: "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17.tar.bz2".into(),
        folder_name: "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17".into(),
        download_url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17.tar.bz2".into(),
        file_size_mb: 894,
        model_file: "model.onnx".into(),
        tokens_file: "tokens.txt".into(),
        requires_directml: true,
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

fn stt_directml_provider_config_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "selah-stt-directml-provider-{}.config",
        std::process::id()
    ))
}

fn stt_model_dir(model: &SttModelInfo) -> PathBuf {
    stt_models_dir().join(&model.folder_name)
}

fn stt_model_supports_backend(model: &SttModelInfo, execution_backend: &str) -> bool {
    execution_backend != STT_BACKEND_DIRECTML || model.model_file != "model.int8.onnx"
}

fn normalize_stt_model_backend_pair(config: &mut SttConfig) {
    if let Some(model) = stt_model_catalog()
        .iter()
        .find(|model| model.id == config.selected_model)
    {
        if !stt_model_supports_backend(model, &config.execution_backend) {
            if let Some(supported_model) = stt_model_catalog()
                .iter()
                .find(|model| stt_model_supports_backend(model, &config.execution_backend))
            {
                config.selected_model = supported_model.id.clone();
            }
        }
    }
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
    let cfg = load_config();
    let model_path = stt_model_dir(model).join(&model.model_file);
    let min_model_bytes = model.file_size_mb.saturating_mul(1024 * 1024);
    if file_exists(&model_path) && !file_exists_with_min_size(&model_path, min_model_bytes) {
        return format!(
            "{} のダウンロードが不完全です。削除して再ダウンロードしてください。",
            model.name
        );
    }
    if cfg.execution_backend == STT_BACKEND_DIRECTML {
        format!(
            "{} がダウンロードされていません。GPU 高精度モードを使用するには高精度モデルのダウンロードが必要です。",
            model.name
        )
    } else {
        format!("{} がダウンロードされていません。設定画面からダウンロードしてください。", model.name)
    }
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

fn directml_build_enabled() -> bool {
    cfg!(target_os = "windows") && option_env!("SELAH_STT_DIRECTML_ENABLED") == Some("1")
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
                Some(
                    "認識器のみ CoreML に切り替わり、VAD は引き続き CPU を使います。".into(),
                )
            } else {
                Some(
                    "このビルドでは CoreML は利用できません。".into(),
                )
            },
        });
    }

    if cfg!(target_os = "windows") {
        let available = directml_build_enabled();
        backends.push(SttExecutionBackendInfo {
            id: STT_BACKEND_DIRECTML.into(),
            label: "GPU 高精度（DirectML）".into(),
            description: "GPU を使ってより高精度な認識を行います。高精度モデルのダウンロードが必要です。".into(),
            experimental: false,
            available,
            availability_note: if available {
                None
            } else {
                Some("このビルドには DirectML ランタイムが含まれていません。".into())
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
        STT_BACKEND_DIRECTML if cfg!(target_os = "windows") && directml_build_enabled() => {
            STT_BACKEND_DIRECTML.into()
        }
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
        STT_BACKEND_DIRECTML if cfg!(target_os = "windows") && directml_build_enabled() => {
            Ok(STT_BACKEND_DIRECTML.into())
        }
        STT_BACKEND_DIRECTML if cfg!(target_os = "windows") => {
            Err("この Windows ビルドには DirectML ランタイムが含まれていません".into())
        }
        STT_BACKEND_DIRECTML => Err("DirectML は Windows ビルドでのみ利用できます".into()),
        _ => Err("不明な音声認識モードです".into()),
    }
}

fn stt_execution_backend_label(backend: &str) -> &'static str {
    match backend {
        STT_BACKEND_COREML => "CoreML",
        STT_BACKEND_DIRECTML => "DirectML",
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

fn stt_provider_value_for_backend(execution_backend: &str) -> Result<String, String> {
    if execution_backend != STT_BACKEND_DIRECTML {
        return Ok(execution_backend.to_string());
    }

    let path = stt_directml_provider_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to prepare DirectML provider config directory: {e}"))?;
    }
    let data = [
        "GraphOptimizationLevel=1",
        "EnableMemPattern=0",
        "EnableCpuMemArena=0",
        "LogSeverityLevel=2",
        "",
    ]
    .join("\n");
    std::fs::write(&path, data)
        .map_err(|e| format!("Failed to write DirectML provider config: {e}"))?;
    Ok(format!(
        "{}:{}",
        STT_BACKEND_DIRECTML,
        path.to_string_lossy()
    ))
}

fn normalized_stt_config(mut config: SttConfig) -> SttConfig {
    config.selected_model = normalize_stt_model_id(&config.selected_model);
    config.language = normalize_stt_language(&config.language);
    config.execution_backend = normalize_stt_execution_backend(&config.execution_backend);
    config.partial_mode = normalize_stt_partial_mode(&config.partial_mode);
    config.sensitivity = normalize_stt_sensitivity(&config.sensitivity);
    normalize_stt_model_backend_pair(&mut config);
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
    if !stt_model_supports_backend(model, execution_backend) {
        return Err(format!(
            "{} は {} モードに対応していません。GPU 高精度モードには高精度モデルが必要です。",
            model.name,
            stt_execution_backend_label(execution_backend)
        ));
    }

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
    config.model_config.provider = Some(stt_provider_value_for_backend(execution_backend)?);
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
    let mut config = VadModelConfig::default();
    config.sample_rate = TARGET_SAMPLE_RATE;
    config.num_threads = 1;
    config.provider = Some("cpu".into());
    config.silero_vad = SileroVadModelConfig {
        model: Some(path.to_string_lossy().into_owned()),
        threshold: profile.vad_threshold,
        min_silence_duration: profile.vad_min_silence,
        min_speech_duration: profile.vad_min_speech,
        window_size: 512,
        max_speech_duration: 8.0,
    };
    Ok(config)
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

fn emit_partial(app: &tauri::AppHandle, text: String, caller: &str) {
    let _ = app.emit(
        "stt-partial",
        SttEventPayload {
            text,
            caller: caller.to_string(),
        },
    );
}

fn emit_final(app: &tauri::AppHandle, text: String, caller: &str) {
    let _ = app.emit(
        "stt-final",
        SttEventPayload {
            text,
            caller: caller.to_string(),
        },
    );
}

/// Emit a final transcript line, but suppress it when SenseVoice repeats
/// itself on adjacent VAD segments. This happens occasionally when the VAD
/// splits an utterance at an unlucky point and both pieces get decoded to
/// the same phrase. `last_final` is updated with whatever we end up keeping
/// (so any legitimate later repeat of the same phrase, spaced by other
/// content, still goes through).
fn emit_final_deduped(app: &tauri::AppHandle, text: String, caller: &str, last_final: &mut String) {
    if text.is_empty() {
        return;
    }
    if text == *last_final {
        return;
    }
    *last_final = text.clone();
    emit_final(app, text, caller);
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
    // emit_info so the user knows. DirectML never reaches this function
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
    if bytes.len() % 4 != 0 {
        return Err("STT helper sample file is not aligned to f32".into());
    }
    f32_samples_from_bytes(&bytes)
}

fn f32_samples_to_base64(samples: &[f32]) -> String {
    let mut bytes = Vec::with_capacity(samples.len() * 4);
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    BASE64_STANDARD.encode(bytes)
}

fn f32_samples_from_bytes(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if bytes.len() % 4 != 0 {
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

struct DirectMlDecodeServer {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl DirectMlDecodeServer {
    fn start(
        model: &SttModelInfo,
        language: &str,
        execution_backend: &str,
    ) -> Result<Self, String> {
        let exe =
            std::env::current_exe().map_err(|e| format!("STT helper exe path failed: {e}"))?;
        let dir = stt_model_dir(model);
        let provider = stt_provider_value_for_backend(execution_backend)?;
        let mut child = Command::new(exe)
            .arg(STT_DECODE_SERVER_ARG)
            .arg("--model")
            .arg(dir.join(&model.model_file))
            .arg("--tokens")
            .arg(dir.join(&model.tokens_file))
            .arg("--language")
            .arg(language)
            .arg("--provider")
            .arg(provider)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("DirectML STT ヘルパーの起動に失敗しました: {e}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "DirectML STT ヘルパーの標準入力を取得できませんでした".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "DirectML STT ヘルパーの標準出力を取得できませんでした".to_string())?;
        let mut server = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        };
        let ready = server.read_response()?;
        if !ready.ready || !ready.ok {
            return Err(if ready.error.is_empty() {
                "DirectML STT ヘルパーの初期化に失敗しました".into()
            } else {
                ready.error
            });
        }
        // Pre-compile DirectML compute shaders with a silent decode so the
        // first real partial/final decode doesn't pay the shader compilation cost.
        let warmup = vec![0.0f32; TARGET_SAMPLE_RATE as usize * 2];
        server.decode(TARGET_SAMPLE_RATE, &warmup).map_err(|e| {
            format!("DirectML STT ヘルパーのウォームアップに失敗しました（クラッシュした可能性があります）: {e}")
        })?;
        Ok(server)
    }

    fn decode(&mut self, sample_rate: i32, samples: &[f32]) -> Result<String, String> {
        if samples.is_empty() {
            return Ok(String::new());
        }

        let request = SttHelperDecodeRequest {
            sample_rate,
            samples: String::new(),
            sample_data: f32_samples_to_base64(samples),
            shutdown: false,
        };
        serde_json::to_writer(&mut self.stdin, &request)
            .map_err(|e| format!("STT helper request serialization failed: {e}"))?;
        self.stdin
            .write_all(b"\n")
            .map_err(|e| format!("STT helper request write failed: {e}"))?;
        self.stdin
            .flush()
            .map_err(|e| format!("STT helper request flush failed: {e}"))?;

        let response = self.read_response()?;
        if response.ok {
            Ok(strip_sense_voice_tags(&response.text))
        } else if response.error.is_empty() {
            Err("DirectML STT ヘルパーのデコードに失敗しました".into())
        } else {
            Err(response.error)
        }
    }

    fn read_response(&mut self) -> Result<SttHelperDecodeResponse, String> {
        let mut line = String::new();
        let n = self
            .stdout
            .read_line(&mut line)
            .map_err(|e| format!("DirectML STT ヘルパーの応答読み取りに失敗しました: {e}"))?;
        if n == 0 {
            let status = self.child.try_wait().ok().flatten();
            return Err(match status {
                Some(status) => format!("DirectML STT ヘルパーが予期せず終了しました（{status}）"),
                None => "DirectML STT ヘルパーが出力を閉じました".into(),
            });
        }
        serde_json::from_str(line.trim())
            .map_err(|e| format!("DirectML STT ヘルパーの応答解析に失敗しました: {e}"))
    }
}

impl Drop for DirectMlDecodeServer {
    fn drop(&mut self) {
        let request = SttHelperDecodeRequest {
            sample_rate: TARGET_SAMPLE_RATE,
            samples: String::new(),
            sample_data: String::new(),
            shutdown: true,
        };
        let _ = serde_json::to_writer(&mut self.stdin, &request);
        let _ = self.stdin.write_all(b"\n");
        let _ = self.stdin.flush();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
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
    let provider = if provider_arg == STT_BACKEND_DIRECTML {
        stt_provider_value_for_backend(&provider_arg)?
    } else {
        provider_arg
    };

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
    if args.iter().any(|arg| arg == STT_DECODE_SERVER_ARG) {
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
/// Shorter window for DirectML partial decodes: attention is O(n²) in
/// sequence length, so 2 s costs roughly 1/6 the compute of 5 s.
const PARTIAL_WINDOW_SECS_DIRECTML: usize = 2;
/// Tail window we probe to decide whether the utterance has momentarily
/// fallen silent. Roughly one natural syllable gap.
const PARTIAL_TAIL_WINDOW_SAMPLES: usize = TARGET_SAMPLE_RATE as usize / 2; // 500 ms
/// RMS threshold below which the tail is treated as silent. Kept slightly
/// above the pre-VAD RMS gate so brief lulls count as silence even if the
/// gate would still let them through.
const PARTIAL_TAIL_SILENCE_RMS: f32 = 0.003;

/// Returns an owned copy of the audio window to decode for a partial result,
/// advancing throttle state when the tail is silent. Returns None if the
/// tick should be skipped entirely.
///
/// `window_secs`: how many seconds of tail audio to decode.
/// `fast`: when true (DirectML), all throttle intervals are halved so the
///   IPC + shorter-window inference budget still drives frequent updates.
fn partial_decode_slice(
    current_samples: &[f32],
    last_partial_at: &mut Instant,
    stable_streak: &mut u32,
    window_secs: usize,
    fast: bool,
) -> Option<Vec<f32>> {
    if current_samples.len() < (TARGET_SAMPLE_RATE as usize / 2) {
        return None;
    }
    let partial_profile = stt_partial_throttle_profile(&load_config().partial_mode);
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
    let interval = if fast { base_interval / 2 } else { base_interval };
    if last_partial_at.elapsed() < Duration::from_millis(interval) {
        return None;
    }
    // If the tail of the utterance is currently silent, nothing the decoder
    // produces can differ from last time — VAD hasn't cut the segment yet,
    // but the speaker is between phrases. Skip the encode entirely.
    let tail_start = current_samples
        .len()
        .saturating_sub(PARTIAL_TAIL_WINDOW_SAMPLES);
    if rms(&current_samples[tail_start..]) < PARTIAL_TAIL_SILENCE_RMS {
        *stable_streak = stable_streak.saturating_add(1);
        *last_partial_at = Instant::now();
        return None;
    }
    // Only decode the tail window — SenseVoice is non-streaming, so re-encoding
    // the full utterance every partial tick is the dominant CPU cost.
    let window = window_secs * TARGET_SAMPLE_RATE as usize;
    let slice = if current_samples.len() > window {
        &current_samples[current_samples.len() - window..]
    } else {
        current_samples
    };
    Some(slice.to_vec())
}

fn handle_partial_result(
    app: &tauri::AppHandle,
    text: String,
    last_partial: &mut String,
    last_partial_at: &mut Instant,
    stable_streak: &mut u32,
    caller: &str,
) {
    *last_partial_at = Instant::now();
    if text.is_empty() {
        return;
    }
    if text == *last_partial {
        *stable_streak = stable_streak.saturating_add(1);
        return;
    }
    *stable_streak = 0;
    *last_partial = text.clone();
    emit_partial(app, text, caller);
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
        let (language, execution_backend) = stt_runtime_preferences();
        let use_directml_helper = execution_backend == STT_BACKEND_DIRECTML;
        let recognizer_init = if use_directml_helper {
            None
        } else {
            Some(create_recognizer_with_fallback(&model)?)
        };
        update_runtime_debug_state(
            "initializing",
            Some(caller),
            Some(
                recognizer_init
                    .as_ref()
                    .map(|init| init.execution_backend.as_str())
                    .unwrap_or(&execution_backend),
            ),
            recognizer_init
                .as_ref()
                .and_then(|init| init.fallback_from.as_deref()),
        );
        if let Some(fallback_from) = recognizer_init
            .as_ref()
            .and_then(|init| init.fallback_from.as_ref())
        {
            emit_info(&app, stt_fallback_message(fallback_from), caller);
        }
        let recognizer = recognizer_init.map(|init| init.recognizer);
        let mut directml_server = if use_directml_helper {
            emit_info(
                &app,
                "DirectML STT をヘルパープロセスで起動しています...",
                caller,
            );
            Some(DirectMlDecodeServer::start(
                &model,
                &language,
                &execution_backend,
            )?)
        } else {
            None
        };
        if use_directml_helper {
            emit_info(
                &app,
                "DirectML STT の準備ができました。セッション中はヘルパーが常駐します。",
                caller,
            );
        }
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
        let mut last_partial = String::new();
        let mut last_partial_at = Instant::now();
        let mut stable_streak: u32 = 0;
        let mut last_final = String::new();
        let mut resampler = Resampler::new(sample_rate, channels);
        let mut agc = Agc::new();

        loop {
            if stop_rx.try_recv().is_ok() {
                break;
            }
            match audio_rx.recv_timeout(Duration::from_millis(120)) {
                Ok(chunk) => {
                    let mut resampled = resampler.process(&chunk);
                    if resampled.is_empty() {
                        continue;
                    }
                    // Noise-floor gate on the *raw* (pre-AGC) signal: when
                    // nothing is currently being captured and the chunk is
                    // below speech level, skip VAD entirely. Checked before
                    // AGC so amplified ambient noise can't trip the gate.
                    let in_utterance = !current_utterance.is_empty() || vad.detected();
                    if !in_utterance && rms(&resampled) < sensitivity_profile.rms_gate {
                        continue;
                    }
                    // Soft AGC: pull quiet mics up toward a conventional
                    // speech level without over-amplifying. Capped at 2×.
                    agc.apply(&mut resampled, in_utterance);
                    vad.accept_waveform(&resampled);
                    if vad.detected() {
                        current_utterance.extend_from_slice(&resampled);
                        if let Some(slice) = partial_decode_slice(
                            &current_utterance,
                            &mut last_partial_at,
                            &mut stable_streak,
                            if use_directml_helper { PARTIAL_WINDOW_SECS_DIRECTML } else { PARTIAL_WINDOW_SECS },
                            use_directml_helper,
                        ) {
                            let text = if let Some(recognizer) = recognizer.as_ref() {
                                decode_samples(recognizer, TARGET_SAMPLE_RATE, &slice)
                            } else if let Some(server) = directml_server.as_mut() {
                                server.decode(TARGET_SAMPLE_RATE, &slice).unwrap_or_default()
                            } else {
                                String::new()
                            };
                            handle_partial_result(
                                &app,
                                text,
                                &mut last_partial,
                                &mut last_partial_at,
                                &mut stable_streak,
                                caller,
                            );
                        }
                    }
                    while !vad.is_empty() {
                        if let Some(segment) = vad.front() {
                            let samples = segment.samples().to_vec();
                            let text = if use_directml_helper {
                                directml_server
                                    .as_mut()
                                    .expect("DirectML helper exists")
                                    .decode(TARGET_SAMPLE_RATE, &samples)?
                            } else {
                                decode_samples(
                                    recognizer.as_ref().expect("recognizer exists"),
                                    TARGET_SAMPLE_RATE,
                                    &samples,
                                )
                            };
                            if !text.is_empty() {
                                last_partial = text.clone();
                                emit_final_deduped(&app, text, caller, &mut last_final);
                            }
                        }
                        vad.pop();
                        current_utterance.clear();
                        stable_streak = 0;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        vad.flush();
        while !vad.is_empty() {
            if let Some(segment) = vad.front() {
                let samples = segment.samples().to_vec();
                let text = if use_directml_helper {
                    directml_server
                        .as_mut()
                        .expect("DirectML helper exists")
                        .decode(TARGET_SAMPLE_RATE, &samples)?
                } else {
                    decode_samples(
                        recognizer.as_ref().expect("recognizer exists"),
                        TARGET_SAMPLE_RATE,
                        &samples,
                    )
                };
                if !text.is_empty() {
                    emit_final_deduped(&app, text, caller, &mut last_final);
                }
            }
            vad.pop();
        }
        if !current_utterance.is_empty() {
            let text = if use_directml_helper {
                directml_server
                    .as_mut()
                    .expect("DirectML helper exists")
                    .decode(TARGET_SAMPLE_RATE, &current_utterance)?
            } else {
                decode_samples(
                    recognizer.as_ref().expect("recognizer exists"),
                    TARGET_SAMPLE_RATE,
                    &current_utterance,
                )
            };
            if !text.is_empty() {
                emit_final_deduped(&app, text, caller, &mut last_final);
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
    normalize_stt_model_backend_pair(&mut config);
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
        .filter(|m| !m.requires_directml || directml_build_enabled())
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
    let (language, execution_backend) = stt_runtime_preferences();
    if execution_backend == STT_BACKEND_DIRECTML {
        let silence = vec![0.0f32; TARGET_SAMPLE_RATE as usize / 2];
        let mut helper = DirectMlDecodeServer::start(&model, &language, &execution_backend)?;
        let _ = helper.decode(TARGET_SAMPLE_RATE, &silence)?;
        update_runtime_debug_state("test-ok", None, Some(&execution_backend), None);
        let message = format!(
            "OK: {} ({})",
            model.name,
            stt_execution_backend_label(&execution_backend)
        );
        update_runtime_debug_message(Some(message.clone()), None);
        emit_runtime_debug_changed(&app);
        return Ok(message);
    }
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
