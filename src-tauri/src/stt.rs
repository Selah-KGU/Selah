#[allow(unused_imports)]
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
#[allow(unused_imports)]
use bzip2::read::BzDecoder;
#[allow(unused_imports)]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig, SileroVadModelConfig,
    VadModelConfig, VoiceActivityDetector,
};
#[allow(unused_imports)]
use std::collections::VecDeque;
#[allow(unused_imports)]
use std::fs::File;
#[allow(unused_imports)]
use std::io::{BufRead, Read, Write};
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[allow(unused_imports)]
use std::sync::{mpsc, Arc, Condvar, LazyLock, Mutex, OnceLock};
#[allow(unused_imports)]
use std::time::{Duration, Instant};
#[allow(unused_imports)]
use tauri::Emitter;

#[path = "stt/audio.rs"]
mod audio;
#[path = "stt/commands.rs"]
mod commands;
#[path = "stt/config.rs"]
mod config;
#[path = "stt/download.rs"]
mod download;
#[path = "stt/helper.rs"]
mod helper;
#[path = "stt/ipc_status.rs"]
pub(crate) mod ipc_status;
#[path = "stt/lifecycle.rs"]
mod lifecycle;
#[path = "stt/model.rs"]
mod model;
#[path = "stt/native_stop.rs"]
mod native_stop;
#[path = "stt/recognizer.rs"]
mod recognizer;
#[path = "stt/reservation.rs"]
pub(crate) mod reservation;
#[path = "stt/runtime.rs"]
mod runtime;
#[path = "stt/session.rs"]
mod session;

#[allow(unused_imports)]
use audio::{
    arm_utterance_restart, extend_restart_grace, normalize_input, partial_decode_slice,
    partial_tail_silence_rms, rms, trim_live_utterance, Agc, OnsetBuffer, Resampler,
    PARTIAL_WINDOW_SECS, RMS_GATE,
};
pub use commands::*;
pub(in crate::stt) use config::{
    current_partial_version, load_config, normalize_stt_language, normalize_stt_model_id,
    normalize_stt_partial_mode, normalize_stt_sensitivity, partial_runtime_preferences,
    save_config, stt_execution_backend_catalog, stt_execution_backend_label, stt_fallback_message,
    stt_partial_mode_label, stt_sensitivity_label, stt_sensitivity_profile,
    validate_stt_execution_backend, SttPartialThrottleProfile, SttSensitivityProfile,
};
pub use config::{SttConfig, SttExecutionBackendInfo};
pub use download::{cancel_stt_download, download_stt_model_blocking};
#[allow(unused_imports)]
use helper::decode_samples;
pub use helper::run_decode_helper_from_args;
pub(in crate::stt) use lifecycle::SttSessionControl;
pub(in crate::stt) use model::{
    ensure_stt_model_downloaded, stt_archive_path, stt_config_path, stt_model_dir, stt_models_dir,
    vad_model_path,
};
pub use model::{is_stt_model_downloaded, stt_model_catalog, SttModelInfo};
pub(in crate::stt) use recognizer::{
    build_vad_config, create_recognizer_for_config, selected_model_for_config,
};
pub use runtime::stt_runtime_debug_info;
pub(in crate::stt) use runtime::{
    clear_session_if_matches, emit_error, emit_final_once, emit_info, emit_partial,
    emit_runtime_debug_changed, emit_state, update_runtime_debug_message,
    update_runtime_debug_state, ActiveSttSession, FinalTranscriptGate, NEXT_SESSION_ID,
    STT_SESSION, STT_SHUTDOWN_REQUESTED,
};
pub use runtime::{SttStreamOwner, SttStreamPhase, SttStreamState};
#[allow(unused_imports)]
use session::run_stt_session;

const TARGET_SAMPLE_RATE: i32 = 16_000;
const VAD_WINDOW_SAMPLES: usize = 512;
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
