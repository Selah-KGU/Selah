//! Bound live VAD buffering without resetting its acoustic state. Otherwise a
//! continuously speaking lecturer makes VAD copy an ever longer passage and
//! eventually sends an enormous job to the offline recognizer.

const MAX_SPEECH_SAMPLES: usize = super::super::TARGET_SAMPLE_RATE as usize * 10;
const PREFER_PAUSE_AFTER: usize = super::super::TARGET_SAMPLE_RATE as usize * 6;
const QUIET_PAUSE_SAMPLES: usize = super::super::TARGET_SAMPLE_RATE as usize * 160 / 1_000;

#[derive(Default)]
pub(super) struct LiveUtterance {
    samples: usize,
    quiet_samples: usize,
}

impl LiveUtterance {
    pub(super) fn observe(&mut self, samples: usize, speech: bool, quiet: bool) -> bool {
        if !speech {
            self.samples = 0;
            self.quiet_samples = 0;
            return false;
        }
        self.samples = self.samples.saturating_add(samples);
        self.quiet_samples = if quiet {
            self.quiet_samples.saturating_add(samples)
        } else {
            0
        };
        // Silero may keep a lecturer's whole paragraph open across short
        // breaths. Prefer one of those breaths over a ten-second cut in the
        // middle of a word, while retaining the hard limit for continuous voice.
        let at_pause =
            self.samples >= PREFER_PAUSE_AFTER && self.quiet_samples >= QUIET_PAUSE_SAMPLES;
        if self.samples < MAX_SPEECH_SAMPLES && !at_pause {
            return false;
        }
        self.samples = 0;
        self.quiet_samples = 0;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuous_speech_has_bounded_windows_for_an_hour_of_capture() {
        let callback = 1_280;
        let mut limit = LiveUtterance::default();
        let mut since = 0;
        let mut cuts = 0;
        for _ in 0..3_600 * 16_000 / callback {
            since += callback;
            if limit.observe(callback, true, false) {
                assert!((MAX_SPEECH_SAMPLES..MAX_SPEECH_SAMPLES + callback).contains(&since));
                cuts += 1;
                since = 0;
            }
        }
        assert_eq!(cuts, 360);
    }

    #[test]
    fn natural_pauses_start_a_new_budget_and_silence_never_forces_a_cut() {
        let mut limit = LiveUtterance::default();
        assert!(!limit.observe(MAX_SPEECH_SAMPLES - 1, true, false));
        assert!(!limit.observe(16_000, false, true));
        assert!(!limit.observe(1, true, false));
        assert!(!limit.observe(16_000 * 60, false, true));
    }

    #[test]
    fn a_sustained_breath_can_close_a_long_window_before_the_hard_cut() {
        let mut limit = LiveUtterance::default();
        assert!(!limit.observe(PREFER_PAUSE_AFTER, true, false));
        assert!(!limit.observe(QUIET_PAUSE_SAMPLES - 1, true, true));
        assert!(limit.observe(1, true, true));
        // A fresh short window and a brief unvoiced consonant do not cut.
        assert!(!limit.observe(QUIET_PAUSE_SAMPLES * 2, true, true));
        assert!(!limit.observe(PREFER_PAUSE_AFTER, true, false));
        assert!(!limit.observe(QUIET_PAUSE_SAMPLES - 1, true, true));
        assert!(!limit.observe(1, true, false));
        assert!(!limit.observe(QUIET_PAUSE_SAMPLES - 1, true, true));
        assert!(limit.observe(1, true, true));
    }

    #[test]
    #[ignore = "requires an explicit public WAV and local Silero VAD model"]
    fn real_vad_flush_preserves_continuing_audio_at_every_live_cut() {
        use crate::stt::{
            SileroVadModelConfig, VadModelConfig, VoiceActivityDetector, VAD_WINDOW_SAMPLES,
        };
        let wave =
            sherpa_onnx::Wave::read(&std::env::var("SELAH_STT_PUBLIC_WAV").unwrap()).unwrap();
        assert_eq!(wave.sample_rate(), 16_000);
        let input: Vec<f32> = wave
            .samples()
            .iter()
            .copied()
            .cycle()
            .take(16_000 * 65 + 127)
            .collect();
        let vad = VoiceActivityDetector::create(
            &VadModelConfig {
                sample_rate: 16_000,
                num_threads: 1,
                provider: Some("cpu".into()),
                silero_vad: SileroVadModelConfig {
                    model: Some(std::env::var("SELAH_STT_VAD_MODEL").unwrap()),
                    threshold: 0.5,
                    // Simulate uninterrupted detection through the entire replay,
                    // so the production limiter, rather than natural pauses, cuts.
                    min_silence_duration: 600.0,
                    min_speech_duration: 0.25,
                    window_size: VAD_WINDOW_SAMPLES as i32,
                    max_speech_duration: 86_400.0,
                },
                ..Default::default()
            },
            30.0,
        )
        .unwrap();
        let mut limit = LiveUtterance::default();
        let mut tail = super::super::vad_tail::VadInputTail::default();
        let mut spans = Vec::new();
        let mut collect = || {
            while !vad.is_empty() {
                let segment = vad.front().unwrap();
                spans.push((segment.start() as usize, segment.samples().to_vec()));
                drop(segment);
                vad.pop();
            }
        };
        for chunk in input.chunks(1_280) {
            tail.accept(&vad, chunk);
            if limit.observe(chunk.len(), vad.detected(), crate::stt::rms(chunk) < 0.0018) {
                vad.flush();
            }
            collect();
        }
        tail.flush(&vad);
        collect();
        assert!(
            spans.len() >= 6,
            "continuous speech never reached its bounded cuts"
        );
        for (index, (start, samples)) in spans.iter().enumerate() {
            assert!(samples.len() <= MAX_SPEECH_SAMPLES + 16_000);
            if let Some((previous_start, previous)) = index.checked_sub(1).map(|i| &spans[i]) {
                assert_eq!(
                    *start,
                    previous_start + previous.len(),
                    "audio gap or duplication at cut {index}"
                );
            }
            let real = samples.len().min(input.len() - start);
            assert_eq!(&samples[..real], &input[*start..*start + real]);
        }
        let (last, samples) = spans.last().unwrap();
        assert!(
            last + samples.len() >= input.len(),
            "last microphone frame was lost"
        );

        // Explicit opt-in also measures the real offline recognizer on these
        // exact VAD windows. It never reads account settings or a microphone.
        if let Ok(directory) = std::env::var("SELAH_STT_PUBLIC_MODEL_DIR") {
            use sherpa_onnx::{
                OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig,
            };
            let directory = std::path::Path::new(&directory);
            let mut config = OfflineRecognizerConfig::default();
            config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
                model: Some(directory.join("model.int8.onnx").to_string_lossy().into()),
                language: Some("auto".into()),
                use_itn: true,
            };
            config.model_config.tokens =
                Some(directory.join("tokens.txt").to_string_lossy().into());
            config.model_config.provider = Some("cpu".into());
            config.model_config.num_threads = 2;
            config.decoding_method = Some("greedy_search".into());
            let recognizer = OfflineRecognizer::create(&config).unwrap();
            let mut nonempty = 0;
            let mut max_ms = 0;
            let mut total_ms = 0;
            for (index, (_, samples)) in spans.iter().enumerate() {
                let stream = recognizer.create_stream();
                stream.accept_waveform(16_000, samples);
                let start = std::time::Instant::now();
                recognizer.decode(&stream);
                let elapsed_ms = start.elapsed().as_millis();
                let text = stream.get_result().unwrap().text;
                nonempty += usize::from(!text.trim().is_empty());
                max_ms = max_ms.max(elapsed_ms);
                total_ms += elapsed_ms;
                println!(
                    "public speech window {index}: {} samples, {elapsed_ms} ms, {} text characters",
                    samples.len(),
                    text.chars().count()
                );
            }
            assert!(
                nonempty >= spans.len() - 1,
                "continuous speech windows failed to decode"
            );
            println!("65-second public speech replay: {} bounded VAD/ASR windows, max decode {max_ms} ms, total decode {total_ms} ms", spans.len());
        }
    }
}
