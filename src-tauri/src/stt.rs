use bzip2::read::BzDecoder;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::{Deserialize, Serialize};
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig, SileroVadModelConfig,
    VadModelConfig, VoiceActivityDetector,
};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::Emitter;

const TARGET_SAMPLE_RATE: i32 = 16_000;
const VAD_MODEL_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx";
const VAD_MODEL_FILE: &str = "silero_vad.onnx";

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
        name: "多言語リアルタイム転写".into(),
        size_label: "SenseVoice (Japanese / English / Chinese / Korean / Cantonese)".into(),
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

pub fn is_stt_model_downloaded(model: &SttModelInfo) -> bool {
    let dir = stt_model_dir(model);
    file_exists(&dir.join(&model.model_file))
        && file_exists(&dir.join(&model.tokens_file))
        && file_exists(&vad_model_path())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SttConfig {
    pub selected_model: String,
    pub language: String,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            selected_model: "sensevoice-ja-en".into(),
            language: "ja".into(),
        }
    }
}

fn load_config() -> SttConfig {
    let path = stt_config_path();
    if !path.exists() {
        return SttConfig::default();
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
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

fn build_sense_voice_config(model: &SttModelInfo) -> Result<OfflineRecognizerConfig, String> {
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

    let stt_cfg = load_config();
    let lang = if stt_cfg.language.is_empty() {
        "ja".to_string()
    } else {
        stt_cfg.language
    };
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
        model: Some(model_path.to_string_lossy().into_owned()),
        language: Some(lang),
        use_itn: false,
    };
    config.model_config.tokens = Some(tokens_path.to_string_lossy().into_owned());
    config.model_config.num_threads = std::thread::available_parallelism()
        .map(|n| n.get().min(4) as i32)
        .unwrap_or(2);
    config.decoding_method = Some("greedy_search".into());
    Ok(config)
}

fn build_vad_config() -> Result<VadModelConfig, String> {
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
        threshold: 0.5,
        min_silence_duration: 0.35,
        min_speech_duration: 0.25,
        window_size: 512,
        max_speech_duration: 12.0,
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

struct ActiveSttSession {
    id: u64,
    caller: String,
    stop_tx: mpsc::Sender<()>,
}

static STT_SESSION: Mutex<Option<ActiveSttSession>> = Mutex::new(None);
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

fn clear_session_if_matches(id: u64) {
    if let Ok(mut lock) = STT_SESSION.lock() {
        if lock.as_ref().map(|s| s.id) == Some(id) {
            *lock = None;
        }
    }
}

fn emit_state(app: &tauri::AppHandle, state: &str, caller: &str) {
    let _ = app.emit(
        "stt-state",
        SttStatePayload {
            state: state.to_string(),
            caller: caller.to_string(),
        },
    );
}

fn emit_error(app: &tauri::AppHandle, message: impl Into<String>, caller: &str) {
    let _ = app.emit(
        "stt-error",
        serde_json::json!({ "message": message.into(), "caller": caller }),
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

fn decode_samples(recognizer: &OfflineRecognizer, sample_rate: i32, samples: &[f32]) -> String {
    if samples.is_empty() {
        return String::new();
    }
    let stream = recognizer.create_stream();
    stream.accept_waveform(sample_rate, samples);
    recognizer.decode(&stream);
    stream
        .get_result()
        .map(|r| r.text.trim().to_string())
        .unwrap_or_default()
}

fn resample_to_16k(samples: &[f32], src_rate: i32, channels: usize) -> Vec<f32> {
    if channels == 0 {
        return Vec::new();
    }
    let mono: Vec<f32> = if channels == 1 {
        samples.to_vec()
    } else {
        samples
            .chunks(channels)
            .map(|frame| frame.iter().copied().sum::<f32>() / channels as f32)
            .collect()
    };
    if src_rate == TARGET_SAMPLE_RATE {
        return mono;
    }
    let ratio = TARGET_SAMPLE_RATE as f64 / src_rate as f64;
    let out_len = ((mono.len() as f64) * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 / ratio;
        let idx = pos.floor() as usize;
        let frac = (pos - idx as f64) as f32;
        let s0 = *mono.get(idx).unwrap_or(&0.0);
        let s1 = *mono.get(idx + 1).unwrap_or(&s0);
        out.push(s0 + (s1 - s0) * frac);
    }
    out
}

fn normalize_i16_input(data: &[i16]) -> Vec<f32> {
    data.iter().map(|&s| s as f32 / i16::MAX as f32).collect()
}

fn normalize_u16_input(data: &[u16]) -> Vec<f32> {
    data.iter()
        .map(|&s| (s as f32 / u16::MAX as f32) * 2.0 - 1.0)
        .collect()
}

fn maybe_emit_partial(
    app: &tauri::AppHandle,
    recognizer: &OfflineRecognizer,
    current_samples: &[f32],
    last_partial: &mut String,
    last_partial_at: &mut Instant,
    caller: &str,
) {
    if current_samples.len() < (TARGET_SAMPLE_RATE as usize / 2) {
        return;
    }
    if last_partial_at.elapsed() < Duration::from_millis(450) {
        return;
    }
    let text = decode_samples(recognizer, TARGET_SAMPLE_RATE, current_samples);
    if !text.is_empty() && text != *last_partial {
        *last_partial = text.clone();
        emit_partial(app, text, caller);
    }
    *last_partial_at = Instant::now();
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
        if !is_stt_model_downloaded(&model) {
            return Err("STT モデルがまだダウンロードされていません".into());
        }

        emit_state(&app, "initializing", caller);
        let recognizer = OfflineRecognizer::create(&build_sense_voice_config(&model)?)
            .ok_or_else(|| "SenseVoice 認識器の作成に失敗しました".to_string())?;
        let vad = VoiceActivityDetector::create(&build_vad_config()?, 30.0)
            .ok_or_else(|| "VAD の初期化に失敗しました".to_string())?;

        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "利用可能なマイク入力デバイスが見つかりません".to_string())?;
        let (supported_cfg, channels) = choose_input_config(&device)?;
        let sample_rate = supported_cfg.sample_rate().0 as i32;
        let stream_config = supported_cfg.config();

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

        let mut current_utterance = Vec::<f32>::new();
        let mut last_partial = String::new();
        let mut last_partial_at = Instant::now();

        loop {
            if stop_rx.try_recv().is_ok() {
                break;
            }
            match audio_rx.recv_timeout(Duration::from_millis(120)) {
                Ok(chunk) => {
                    let resampled = resample_to_16k(&chunk, sample_rate, channels);
                    if resampled.is_empty() {
                        continue;
                    }
                    vad.accept_waveform(&resampled);
                    if vad.detected() {
                        current_utterance.extend_from_slice(&resampled);
                        maybe_emit_partial(
                            &app,
                            &recognizer,
                            &current_utterance,
                            &mut last_partial,
                            &mut last_partial_at,
                            caller,
                        );
                    }
                    while !vad.is_empty() {
                        if let Some(segment) = vad.front() {
                            let samples = segment.samples().to_vec();
                            let text = decode_samples(&recognizer, TARGET_SAMPLE_RATE, &samples);
                            if !text.is_empty() {
                                last_partial = text.clone();
                                emit_final(&app, text, caller);
                            }
                        }
                        vad.pop();
                        current_utterance.clear();
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
                let text = decode_samples(&recognizer, TARGET_SAMPLE_RATE, &samples);
                if !text.is_empty() {
                    emit_final(&app, text, caller);
                }
            }
            vad.pop();
        }
        if !current_utterance.is_empty() {
            let text = decode_samples(&recognizer, TARGET_SAMPLE_RATE, &current_utterance);
            if !text.is_empty() {
                emit_final(&app, text, caller);
            }
        }

        Ok(())
    })();

    if let Err(err) = result {
        emit_error(&app, err, caller);
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
    config.selected_model = config.selected_model.trim().to_string();
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
pub fn stt_test_model() -> Result<String, String> {
    let model = selected_model_from_config()?;
    if !is_stt_model_downloaded(&model) {
        return Err("STT モデルを先にダウンロードしてください".into());
    }
    let cfg = build_sense_voice_config(&model)?;
    OfflineRecognizer::create(&cfg)
        .map(|_| format!("OK: {}", model.name))
        .ok_or_else(|| "SenseVoice 認識器の初期化に失敗しました".into())
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
