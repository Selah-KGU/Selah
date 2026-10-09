//! Streaming microphone session and the background decode worker.

#[path = "session/input.rs"]
mod input;
#[path = "session/partial.rs"]
mod partial;
#[path = "session/queue.rs"]
mod queue;
#[path = "session/vad_tail.rs"]
mod vad_tail;
#[path = "session/worker.rs"]
mod worker;

use super::*;
use partial::{initialize_if_active, preload_partial, PartialMode};
use tauri::Manager;
use vad_tail::VadInputTail;
use worker::{drain_vad_segments, LivePartialSetup, SttDecodeContext, SttDecodeWorker};

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

pub(super) fn run_stt_session(
    app: tauri::AppHandle,
    session_id: u64,
    control: Arc<SttSessionControl>,
    caller: &str,
    live_session_id: Option<String>,
    input_session_id: Option<String>,
) {
    let _cleanup = SttSessionCleanup {
        app: app.clone(),
        session_id,
        caller: caller.to_string(),
        control: Arc::clone(&control),
        live_session_id: live_session_id.clone(),
        input_session_id: input_session_id.clone(),
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        capture_stt_session(
            &app,
            session_id,
            &control,
            caller,
            live_session_id.as_deref(),
            input_session_id.as_deref(),
        )
    }));

    match result {
        Ok(Ok(())) => {}
        Ok(Err(err)) => emit_error(
            &app,
            err,
            caller,
            live_session_id.as_deref(),
            input_session_id.as_deref(),
        ),
        Err(_) => {
            log::error!("[stt] capture thread panicked");
            emit_error(
                &app,
                "音声入力が中断されました。もう一度開始してください",
                caller,
                live_session_id.as_deref(),
                input_session_id.as_deref(),
            );
        }
    }
}

fn capture_stt_session(
    app: &tauri::AppHandle,
    session_id: u64,
    control: &Arc<SttSessionControl>,
    caller: &str,
    live_session_id: Option<&str>,
    input_session_id: Option<&str>,
) -> Result<(), String> {
    let capturing = || {
        !control.is_stopping()
            && !STT_SHUTDOWN_REQUESTED.load(Ordering::SeqCst)
            && live_session_id
                .is_none_or(|id| app.state::<crate::live::LiveState>().is_session_current(id))
    };
    if !capturing() {
        return Ok(());
    }
    let config = load_config();
    let model = selected_model_for_config(&config)?;
    ensure_stt_model_downloaded(&model)?;
    if !is_stt_model_downloaded(&model) {
        return Err("STT モデルがまだダウンロードされていません".into());
    }

    emit_state(
        app,
        session_id,
        "initializing",
        caller,
        live_session_id,
        input_session_id,
    );
    let Some(recognizer_init) =
        initialize_if_active(capturing, || create_recognizer_for_config(&model, &config))?
    else {
        return Ok(());
    };
    update_runtime_debug_state(
        "initializing",
        Some(caller),
        Some(recognizer_init.execution_backend.as_str()),
        recognizer_init.fallback_from.as_deref(),
    );
    if let Some(fallback_from) = recognizer_init.fallback_from.as_ref() {
        emit_info(
            app,
            stt_fallback_message(fallback_from),
            caller,
            live_session_id,
            input_session_id,
        );
    }
    // Decode off this thread. SenseVoice is offline, so one partial or
    // final can take longer than the audio it covers. Decoding here
    // starves the VAD, and live speech stops being detected.
    // Live loads a second recognizer so a final already being decoded
    // cannot block the next partial. Agent input keeps one recognizer.
    let live_partial_recognizer = if caller == "live" {
        let seed = preload_partial(capturing, current_partial_version, || {
            create_recognizer_for_config(&model, &config)
        })
        .map_err(|err| {
            format!(
                "リアルタイム字幕用の追加認識器を読み込めませんでした: {}",
                err
            )
        })?;
        if let Some((partial_init, _)) = seed
            .as_ref()
            .filter(|(_, version)| current_partial_version() == *version && capturing())
        {
            if recognizer_init.fallback_from.is_none() {
                if let Some(fallback_from) = partial_init.fallback_from.as_ref() {
                    log::warn!(
                        "[stt] live partial recognizer fell back from {}",
                        fallback_from
                    );
                    emit_info(
                        app,
                        stt_fallback_message(fallback_from),
                        caller,
                        live_session_id,
                        input_session_id,
                    );
                }
            }
        }
        seed.map(|(init, version)| (init.recognizer, version))
    } else {
        None
    };
    if !capturing() {
        return Ok(());
    }
    let recognizer = recognizer_init.recognizer;
    let (_, initial_partial_version) = partial_runtime_preferences();
    let decode_worker = SttDecodeWorker::spawn(
        SttDecodeContext {
            app: app.clone(),
            caller: caller.to_string(),
            live_session_id: live_session_id.map(str::to_string),
            input_session_id: input_session_id.map(str::to_string),
            control: Arc::clone(control),
        },
        recognizer,
        (caller == "live").then(|| LivePartialSetup {
            model,
            config: config.clone(),
            recognizer: live_partial_recognizer,
        }),
        Arc::new(PartialMode::from_version(initial_partial_version)),
    )?;
    let sensitivity_profile = stt_sensitivity_profile(&config.sensitivity);
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
    // CPAL measures buffer size in frames, each containing all channels.
    let desired_frames = (sample_rate as u32 * 80) / 1000;
    if let cpal::SupportedBufferSize::Range { min, max } = supported_cfg.buffer_size() {
        let clamped = desired_frames.clamp(*min, *max);
        stream_config.buffer_size = cpal::BufferSize::Fixed(clamped);
    }

    let (audio_tx, audio_rx) = mpsc::channel::<Vec<f32>>();
    let err_app = app.clone();
    let err_caller = caller.to_string();
    let err_control = Arc::clone(control);
    let err_live_session_id = live_session_id.map(str::to_string);
    let err_input_session_id = input_session_id.map(str::to_string);
    let err_fn = move |err| {
        err_control.request_stop();
        log::error!("[stt] microphone stream error: {}", err);
        emit_error(
            &err_app,
            format!("マイク入力エラー: {}", err),
            &err_caller,
            err_live_session_id.as_deref(),
            err_input_session_id.as_deref(),
        );
    };

    let input_stream = input::build_input_stream(
        &device,
        &stream_config,
        supported_cfg.sample_format(),
        audio_tx,
        err_fn,
    )?;

    if control.is_stopping() || STT_SHUTDOWN_REQUESTED.load(Ordering::SeqCst) {
        return Ok(());
    }
    input_stream
        .play()
        .map_err(|e| format!("マイク入力開始失敗: {}", e))?;
    emit_state(
        app,
        session_id,
        "listening",
        caller,
        live_session_id,
        input_session_id,
    );
    if caller == "live" {
        if let Err(err) = crate::power::prevent_sleep_start(Some("KWIC live transcription".into()))
        {
            log::warn!("[stt] failed to prevent system sleep during Live: {}", err);
        }
    }

    let mut current_utterance = Vec::<f32>::new();
    let mut last_partial_at = Instant::now();
    let mut stable_streak: u32 = 0;
    let mut resampler = Resampler::new(sample_rate, channels);
    let mut agc = Agc::new();
    let mut onset = OnsetBuffer::default();
    let mut vad_tail = VadInputTail::default();
    let mut vad_feed_grace_until = Instant::now();
    let mut vad_feed_grace_deadline = Instant::now();

    let mut accept_samples =
        |resampled: Vec<f32>, partial_profile: Option<&SttPartialThrottleProfile>| {
            if resampled.is_empty() {
                return;
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
            let gated =
                !in_utterance && !in_restart_grace && chunk_rms < sensitivity_profile.rms_gate;
            let Some(mut resampled) = onset.accept(resampled, !gated) else {
                return;
            };
            agc.apply(&mut resampled, in_utterance);
            vad_tail.accept(&vad, &resampled);
            if vad.detected() {
                current_utterance.extend_from_slice(&resampled);
                trim_live_utterance(&mut current_utterance);
                if let Some(profile) = partial_profile {
                    if let Some(slice) = partial_decode_slice(
                        &current_utterance,
                        &mut last_partial_at,
                        &mut stable_streak,
                        PARTIAL_WINDOW_SECS,
                        profile,
                        partial_tail_silence_rms(sensitivity_profile.rms_gate),
                    ) {
                        decode_worker.push_partial(slice);
                    }
                }
            }
            if drain_vad_segments(&vad, &decode_worker) {
                current_utterance.clear();
                stable_streak = 0;
                let (until, deadline) = arm_utterance_restart(Instant::now());
                vad_feed_grace_until = until;
                vad_feed_grace_deadline = deadline;
            }
        };

    let should_flush_pending_audio = loop {
        let (partial_profile, version) = partial_runtime_preferences();
        decode_worker.sync_partial_version(version);
        if decode_worker.failed() {
            return Err(
                "音声認識が中断されました。マイクを一度止めて、もう一度開始してください".into(),
            );
        }
        if live_session_id
            .as_deref()
            .is_some_and(|id| !app.state::<crate::live::LiveState>().is_session_current(id))
        {
            break false;
        }
        if control.is_stopping() {
            break true;
        }
        match audio_rx.recv_timeout(Duration::from_millis(120)) {
            Ok(chunk) => accept_samples(resampler.process(&chunk), Some(&partial_profile)),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break true;
            }
        }
    };

    // Stop callbacks before draining samples already captured. Waiting
    // for decoder completion alone would still lose a queued mic tail.
    drop(input_stream);
    if should_flush_pending_audio && !decode_worker.failed() {
        while let Ok(chunk) = audio_rx.try_recv() {
            accept_samples(resampler.process(&chunk), None);
        }
        accept_samples(resampler.finish(), None);
    }
    drop(accept_samples);
    if should_flush_pending_audio && !decode_worker.failed() {
        vad_tail.flush(&vad);
        // The flushed segment already contains this utterance. Pushing
        // current_utterance as well emits the same speech twice.
        if !drain_vad_segments(&vad, &decode_worker) && !current_utterance.is_empty() {
            decode_worker.push_final(std::mem::take(&mut current_utterance));
        }
    }

    Ok(())
}

/// Completion is published last, including during unwinding. The closure has
/// already dropped its microphone and joined every decoder before this runs.
struct SttSessionCleanup {
    app: tauri::AppHandle,
    session_id: u64,
    caller: String,
    control: Arc<SttSessionControl>,
    live_session_id: Option<String>,
    input_session_id: Option<String>,
}

impl Drop for SttSessionCleanup {
    fn drop(&mut self) {
        // An idle listener may immediately resume a borrowed microphone. A
        // restart waits for this control instead of treating teardown as busy.
        self.control.request_stop();
        if self.caller == "live" {
            if let Err(err) = crate::power::prevent_sleep_stop() {
                log::warn!("[stt] failed to release Live sleep prevention: {err}");
            }
        }
        emit_state(
            &self.app,
            self.session_id,
            "idle",
            &self.caller,
            self.live_session_id.as_deref(),
            self.input_session_id.as_deref(),
        );
        clear_session_if_matches(self.session_id);
        self.control.finish();
    }
}
