//! Streaming microphone session and the background decode worker.

#[path = "session/queue.rs"]
mod queue;
#[path = "session/worker.rs"]
mod worker;

use super::*;
use worker::{drain_vad_segments, SttDecodeWorker};

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
