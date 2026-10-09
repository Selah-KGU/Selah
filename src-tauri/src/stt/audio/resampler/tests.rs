use super::*;

#[path = "reference.rs"]
mod reference;
use reference::LegacyResampler;

fn tone(rate: i32, hz: f64, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| (i as f64 * hz * std::f64::consts::TAU / rate as f64).sin() as f32)
        .collect()
}

fn partitioned(rate: i32, channels: usize, input: &[f32], sizes: &[usize]) -> Vec<f32> {
    let mut resampler = Resampler::new(rate, channels);
    let mut out = Vec::new();
    let mut offset = 0;
    for size in sizes.iter().cycle() {
        if offset == input.len() {
            break;
        }
        let end = (offset + size).min(input.len());
        out.extend(resampler.process(&input[offset..end]));
        offset = end;
    }
    out.extend(resampler.finish());
    assert!(
        resampler.finish().is_empty(),
        "final interpolation emitted twice"
    );
    out
}

#[test]
fn callbacks_do_not_change_the_waveform_or_duration_even_across_partial_stereo_frames() {
    for rate in [8_000, 12_000, 16_000, 22_050, 44_100, 48_000, 96_000] {
        let mono = tone(rate, 1000.0, rate as usize / 5 + 17);
        let stereo: Vec<_> = mono
            .iter()
            .flat_map(|sample| [sample * 0.6, sample * 1.4])
            .collect();
        for (channels, input) in [(1, &mono), (2, &stereo)] {
            let whole = partitioned(rate, channels, input, &[input.len()]);
            let fragmented = partitioned(rate, channels, input, &[1, 17, 509, 2, 127]);
            let expected =
                (mono.len() as u64 * TARGET_SAMPLE_RATE as u64).div_ceil(rate as u64) as usize;
            assert_eq!(whole.len(), expected, "rate={rate}, channels={channels}");
            assert_eq!(
                fragmented, whole,
                "callback boundary changed samples at {rate} Hz, {channels} channels"
            );
        }
    }
}

#[test]
fn odd_rate_callbacks_keep_the_exact_sample_clock_for_a_long_stream() {
    let mut resampler = Resampler::new(44_100, 1);
    let chunk = vec![0.25; 1024];
    let mut count = 0usize;
    for _ in 0..1000 {
        count += resampler.process(&chunk).len();
        assert!(resampler.scratch.len() <= chunk.len() + 63);
        assert_eq!(resampler.history.len(), 63);
        assert!(
            resampler.phase >= -i64::from(TARGET_SAMPLE_RATE)
                && resampler.phase <= resampler.src_rate
        );
    }
    count += resampler.finish().len();
    let expected = (1_024_000_u64 * TARGET_SAMPLE_RATE as u64).div_ceil(44_100) as usize;
    assert_eq!(count, expected);
    // The old per-callback rounding accumulates 480 excess samples here.
    assert_ne!(
        count,
        (1024.0_f64 * 16_000.0 / 44_100.0).round() as usize * 1000
    );
}

#[test]
fn low_rate_input_is_interpolated_and_the_last_fractional_frame_is_flushed_once() {
    let mut resampler = Resampler::new(8_000, 1);
    assert_eq!(resampler.process(&[0.0]), [0.0]);
    assert_eq!(resampler.process(&[1.0]), [0.5, 1.0]);
    assert_eq!(resampler.process(&[2.0]), [1.5, 2.0]);
    assert_eq!(resampler.finish(), [2.0]);
    assert!(resampler.finish().is_empty());
    assert!(Resampler::new(8_000, 1).finish().is_empty());
}

#[test]
fn native_rate_passes_samples_through_and_preserves_downmix_across_boundaries() {
    let mut mono = Resampler::new(TARGET_SAMPLE_RATE, 1);
    assert!(mono.process(&[]).is_empty());
    assert_eq!(mono.process(&[0.5, -0.25, 0.1]), [0.5, -0.25, 0.1]);
    assert!(mono.finish().is_empty());
    assert!(
        mono.scratch.is_empty(),
        "native mono path copied to scratch"
    );
    let mut stereo = Resampler::new(TARGET_SAMPLE_RATE, 2);
    assert!(stereo.process(&[1.0]).is_empty());
    assert_eq!(stereo.process(&[-1.0, 0.75]), [0.0]);
    assert_eq!(stereo.process(&[0.25]), [0.5]);
}

#[test]
fn output_only_filter_matches_the_previous_48khz_waveform() {
    let input = tone(48_000, 2300.0, 48_000);
    let mut reference = LegacyResampler::new(48_000, 1);
    let mut current = Resampler::new(48_000, 1);
    for chunk in input.chunks(3840) {
        let before = reference.process(chunk);
        let after = current.process(chunk);
        assert_eq!(before.len(), after.len());
        for (a, b) in before.iter().zip(&after) {
            assert!((a - b).abs() < 1e-6, "filter changed: {a} vs {b}");
        }
    }
}

#[test]
fn anti_alias_filter_preserves_speech_band_and_rejects_above_nyquist() {
    for rate in [44_100, 48_000, 96_000] {
        let low = partitioned(rate, 1, &tone(rate, 1000.0, rate as usize), &[101, 997, 13]);
        let high = partitioned(
            rate,
            1,
            &tone(rate, 12000.0, rate as usize),
            &[101, 997, 13],
        );
        let rms = |samples: &[f32]| {
            (samples
                .iter()
                .map(|sample| f64::from(*sample).powi(2))
                .sum::<f64>()
                / samples.len() as f64)
                .sqrt()
        };
        let low_rms = rms(&low[256..]);
        let high_rms = rms(&high[256..]);
        assert!(
            (low_rms - 0.5_f64.sqrt()).abs() < 0.025,
            "speech band attenuated at {rate}: {low_rms}"
        );
        assert!(
            high_rms / low_rms < 0.025,
            "high-frequency alias at {rate}: {high_rms}"
        );
    }
}

#[test]
#[ignore = "CPU microbenchmark; run with an optimized standalone test harness"]
fn benchmark_microphone_resampling() {
    use std::hint::black_box;
    use std::time::Instant;
    fn elapsed(mut run: impl FnMut()) -> std::time::Duration {
        let now = Instant::now();
        for _ in 0..1000 {
            run();
        }
        now.elapsed()
    }
    for rate in [16_000, 44_100, 48_000, 96_000] {
        let input = tone(rate, 1000.0, rate as usize * 80 / 1000);
        let iterations = 1000;
        let mut before = LegacyResampler::new(rate, 1);
        let mut after = Resampler::new(rate, 1);
        for _ in 0..100 {
            black_box(before.process(black_box(&input)));
            black_box(after.process(black_box(&input)));
        }
        let mut previous_times = Vec::new();
        let mut current_times = Vec::new();
        for round in 0..5 {
            let mut previous = || {
                black_box(before.process(black_box(&input)));
            };
            let mut current = || {
                black_box(after.process(black_box(&input)));
            };
            if round % 2 == 0 {
                previous_times.push(elapsed(&mut previous));
                current_times.push(elapsed(&mut current));
            } else {
                current_times.push(elapsed(&mut current));
                previous_times.push(elapsed(&mut previous));
            }
        }
        previous_times.sort_unstable();
        current_times.sort_unstable();
        let legacy = previous_times[2];
        let current = current_times[2];
        println!("{rate} Hz, 80 ms frames x {iterations}, median of 5 alternating rounds: previous={legacy:?}, current={current:?}, ratio={:.2}", legacy.as_secs_f64() / current.as_secs_f64());
    }
}
