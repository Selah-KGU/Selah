use super::*;

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

pub(in crate::stt) fn build_vad_config(
    profile: &SttSensitivityProfile,
) -> Result<VadModelConfig, String> {
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
            window_size: VAD_WINDOW_SAMPLES as i32,
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

pub(in crate::stt) fn selected_model_for_config(cfg: &SttConfig) -> Result<SttModelInfo, String> {
    stt_model_catalog()
        .iter()
        .find(|m| m.id == cfg.selected_model)
        .cloned()
        .ok_or_else(|| format!("不明な STT モデル: {}", cfg.selected_model))
}

pub(in crate::stt) struct RecognizerInitResult {
    pub(in crate::stt) recognizer: OfflineRecognizer,
    pub(in crate::stt) execution_backend: String,
    pub(in crate::stt) fallback_from: Option<String>,
}

pub(in crate::stt) fn create_recognizer_for_config(
    model: &SttModelInfo,
    config: &SttConfig,
) -> Result<RecognizerInitResult, String> {
    create_recognizer_with_preferences(model, &config.language, &config.execution_backend)
}

fn create_recognizer_with_preferences(
    model: &SttModelInfo,
    language: &str,
    requested_backend: &str,
) -> Result<RecognizerInitResult, String> {
    let requested_cfg = build_sense_voice_config_for_backend(model, &language, &requested_backend)?;

    if let Some(recognizer) = OfflineRecognizer::create(&requested_cfg) {
        return Ok(RecognizerInitResult {
            recognizer,
            execution_backend: requested_backend.to_string(),
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
        fallback_from: Some(requested_backend.to_string()),
    })
}
