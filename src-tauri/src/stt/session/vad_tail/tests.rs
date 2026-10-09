use super::*;

#[test]
fn final_audio_is_completed_once_for_every_frame_remainder() {
    for remainder in 1..VAD_WINDOW_SAMPLES {
        let mut tail = VadInputTail::default();
        tail.observe(3 * VAD_WINDOW_SAMPLES + remainder);
        let mut padding = Vec::new();
        tail.complete(|samples| padding.extend_from_slice(samples));
        assert_eq!(padding.len(), VAD_WINDOW_SAMPLES - remainder);
        assert!(padding.iter().all(|&sample| sample == 0.0));
    }
}

#[test]
fn complete_frames_and_empty_input_do_not_add_silence() {
    for count in [0, VAD_WINDOW_SAMPLES, 80 * VAD_WINDOW_SAMPLES] {
        let mut tail = VadInputTail::default();
        tail.observe(count);
        tail.complete(|_| panic!("padding after a complete frame"));
    }
}

#[test]
fn callback_partitions_do_not_change_the_last_audio_frame() {
    for total in [1, 511, 512, 1280, 2561, 16_123] {
        let expected_padding =
            (VAD_WINDOW_SAMPLES - total % VAD_WINDOW_SAMPLES) % VAD_WINDOW_SAMPLES;
        for partition in [1, 7, 127, 512, 1280] {
            let mut tail = VadInputTail::default();
            let mut remaining = total;
            while remaining > 0 {
                let count = remaining.min(partition);
                tail.observe(count);
                remaining -= count;
            }
            let mut padding = Vec::new();
            tail.complete(|samples| padding.extend_from_slice(samples));
            assert_eq!(padding.len(), expected_padding);
        }
    }
}

#[test]
#[ignore = "requires a local CPU Silero model and public 16 kHz f32 speech fixture"]
fn real_vad_retains_the_incomplete_speech_frame_on_stop() {
    use crate::stt::{SileroVadModelConfig, VadModelConfig, TARGET_SAMPLE_RATE};
    let model = std::env::var("SELAH_STT_VAD_MODEL").expect("SELAH_STT_VAD_MODEL");
    let fixture = std::env::var("SELAH_STT_VAD_SAMPLES").expect("SELAH_STT_VAD_SAMPLES");
    let bytes = std::fs::read(fixture).unwrap();
    assert_eq!(bytes.len() % 4, 0);
    let samples: Vec<_> = bytes
        .chunks_exact(4)
        .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
        .collect();
    let create = || {
        VoiceActivityDetector::create(
            &VadModelConfig {
                sample_rate: TARGET_SAMPLE_RATE,
                num_threads: 1,
                provider: Some("cpu".into()),
                silero_vad: SileroVadModelConfig {
                    model: Some(model.clone()),
                    threshold: 0.5,
                    min_silence_duration: 0.45,
                    min_speech_duration: 0.25,
                    window_size: VAD_WINDOW_SAMPLES as i32,
                    max_speech_duration: 86_400.0,
                },
                ..Default::default()
            },
            30.0,
        )
        .expect("CPU VAD initialization")
    };

    for remainder in [1, 127, 256, 511] {
        let actual = create();
        let expected = create();
        let mut tail = VadInputTail::default();
        let mut offset = 0;
        while offset + VAD_WINDOW_SAMPLES <= samples.len() {
            let frame = &samples[offset..offset + VAD_WINDOW_SAMPLES];
            tail.accept(&actual, frame);
            expected.accept_waveform(frame);
            offset += VAD_WINDOW_SAMPLES;
            if actual.detected() {
                break;
            }
        }
        assert!(
            actual.detected(),
            "public fixture contains no detected speech"
        );
        assert!(actual.is_empty() && expected.is_empty());
        let last = &samples[offset..offset + remainder];
        assert!(last.iter().any(|sample| sample.abs() > 1e-5));
        tail.accept(&actual, last);
        expected.accept_waveform(last);
        // Independent reference: explicitly submit a complete final frame.
        expected.accept_waveform(&vec![0.0; VAD_WINDOW_SAMPLES - remainder]);
        expected.flush();
        tail.flush(&actual);
        let got = actual.front().expect("actual final speech segment");
        let want = expected.front().expect("reference final speech segment");
        assert_eq!(got.start(), want.start());
        assert!(
            got.samples() == want.samples(),
            "lost {remainder} trailing samples: {} vs {} samples",
            got.n(),
            want.n()
        );
        let relative = offset - got.start() as usize;
        assert_eq!(&got.samples()[relative..relative + remainder], last);
        assert!(got.samples()[relative + remainder..]
            .iter()
            .all(|sample| *sample == 0.0));
        drop(got);
        actual.pop();
        actual.flush();
        assert!(actual.is_empty(), "speech was flushed twice");
    }

    for silence_len in [0, 1, 511, 512, 1280] {
        let vad = create();
        let mut tail = VadInputTail::default();
        tail.accept(&vad, &vec![0.0; silence_len]);
        tail.flush(&vad);
        assert!(
            vad.is_empty(),
            "completing silence invented a speech segment"
        );
    }

    // Compose onset preservation and tail completion through the real VAD.
    // The reference explicitly prepends the last 200 ms of gated fixture
    // audio; it does not use OnsetBuffer to assemble its expected waveform.
    let callback_samples = 1280;
    let first_audible = samples
        .chunks(callback_samples)
        .position(|chunk| crate::stt::rms(chunk) >= crate::stt::RMS_GATE)
        .expect("audible public fixture");
    let start = first_audible * callback_samples;
    assert!(start >= TARGET_SAMPLE_RATE as usize / 5);
    let actual = create();
    let expected = create();
    let mut onset = crate::stt::OnsetBuffer::default();
    let mut tail = VadInputTail::default();
    for chunk in samples[..start].chunks(callback_samples) {
        assert!(onset.accept(chunk.to_vec(), false).is_none());
    }
    let mut reference_len = 0;
    for (i, chunk) in samples[start..start + 5 * callback_samples]
        .chunks(callback_samples)
        .enumerate()
    {
        let mut reference = if i == 0 {
            samples[start - TARGET_SAMPLE_RATE as usize / 5..start].to_vec()
        } else {
            Vec::new()
        };
        reference.extend_from_slice(chunk);
        let admitted = onset.accept(chunk.to_vec(), true).unwrap();
        assert!(admitted == reference, "onset or callback audio changed");
        tail.accept(&actual, &admitted);
        expected.accept_waveform(&reference);
        reference_len += reference.len();
    }
    assert!(actual.detected() && expected.detected());
    let padding = (VAD_WINDOW_SAMPLES - reference_len % VAD_WINDOW_SAMPLES) % VAD_WINDOW_SAMPLES;
    if padding != 0 {
        expected.accept_waveform(&vec![0.0; padding]);
    }
    expected.flush();
    tail.flush(&actual);
    let got = actual.front().expect("onset and tail speech segment");
    let want = expected.front().expect("explicit prefix reference segment");
    assert_eq!(got.start(), want.start());
    assert!(
        got.samples() == want.samples(),
        "onset preservation changed VAD audio order"
    );
}
