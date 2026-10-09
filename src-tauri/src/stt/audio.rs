//! Microphone resampling, gain control, and partial-decode windows.

use super::*;

#[path = "audio/resampler.rs"]
mod resampler;
pub(super) use resampler::Resampler;

#[path = "audio/onset.rs"]
mod onset;
pub(super) use onset::OnsetBuffer;

/// Soft automatic-gain control.
///
/// Tracks a slow EMA of the peak sample observed while the VAD reports
/// speech, then scales chunks by a gain that brings that tracked peak
/// toward a conventional speech level. Gain is clamped to [1.0, 2.0] and
/// limited by the current chunk's headroom so a louder onset cannot clip
/// while the tracked peak catches up. Updates only happen during speech
/// so room tone can't pull the reference down. Already clipped input
/// cannot be restored here.
pub(super) struct Agc {
    ema_peak: f32,
    initialized: bool,
}

impl Agc {
    const TARGET_PEAK: f32 = 0.5;
    const MAX_GAIN: f32 = 2.0;
    const ATTACK: f32 = 0.15; // fast when a louder sample arrives
    const RELEASE: f32 = 0.02; // slow when the running peak decays

    pub(super) fn new() -> Self {
        Self {
            ema_peak: 0.0,
            initialized: false,
        }
    }

    pub(super) fn apply(&mut self, samples: &mut [f32], in_speech: bool) {
        if samples.is_empty() {
            return;
        }
        let chunk_peak: f32 = samples.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        if in_speech {
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
        let requested_gain = (Self::TARGET_PEAK / self.ema_peak).clamp(1.0, Self::MAX_GAIN);
        // Protect both detected speech and the first chunk of a new onset,
        // which can arrive before VAD updates its speech state. Use one gain
        // for the whole chunk rather than flattening individual peaks.
        let gain = requested_gain.min((1.0 / chunk_peak).max(1.0));
        if gain <= 1.001 {
            return;
        }
        for s in samples.iter_mut() {
            *s = (*s * gain).clamp(-1.0, 1.0);
        }
    }
}

/// Root-mean-square of a slice. Returns 0 for empty input.
pub(super) fn rms(samples: &[f32]) -> f32 {
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
pub(super) const RMS_GATE: f32 = 0.0018;

/// CPAL's conversion preserves signed PCM scale and unsigned PCM's exact
/// midpoint. Convert before downmixing or resampling, retaining channel order.
pub(super) fn normalize_input<T: cpal::Sample>(data: &[T]) -> Vec<f32>
where
    f32: cpal::FromSample<T>,
{
    use cpal::Sample;
    data.iter()
        .map(|&sample| f32::from_sample(sample))
        .collect()
}

pub(super) const PARTIAL_WINDOW_SECS: usize = 5;
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

pub(super) fn partial_tail_silence_rms(rms_gate: f32) -> f32 {
    rms_gate.min(PARTIAL_TAIL_SILENCE_RMS_CAP)
}

pub(super) fn arm_utterance_restart(now: Instant) -> (Instant, Instant) {
    (now + UTTERANCE_RESTART_GRACE, now + UTTERANCE_RESTART_LIMIT)
}

/// Keep the restart window open while sound is still arriving, but never
/// past the deadline captured when the utterance ended.
pub(super) fn extend_restart_grace(
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

pub(super) fn trim_live_utterance(samples: &mut Vec<f32>) {
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
pub(super) fn partial_decode_slice(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_microphone_pcm_preserves_full_scale_and_half_scale() {
        assert_eq!(
            normalize_input(&[i16::MIN, -16384, 0, 16384, i16::MAX]),
            [-1.0, -0.5, 0.0, 0.5, 32767.0 / 32768.0]
        );
    }

    #[test]
    fn unsigned_microphone_pcm_has_exact_silence_and_symmetric_amplitude() {
        assert_eq!(
            normalize_input(&[0u16, 16384, 32768, 49152, u16::MAX]),
            [-1.0, -0.5, 0.0, 0.5, 32767.0 / 32768.0]
        );
    }

    #[test]
    fn microphone_pcm_accepts_every_integer_width_with_the_same_scale() {
        macro_rules! signed {
            ($sample:ty) => {
                let half = <$sample>::MIN / 2;
                let output = normalize_input(&[<$sample>::MIN, half, 0, -half, <$sample>::MAX]);
                assert_eq!(&output[..4], &[-1.0, -0.5, 0.0, 0.5]);
                assert!(output[4] > 0.99 && output[4] <= 1.0);
                assert!(normalize_input::<$sample>(&[]).is_empty());
            };
        }
        macro_rules! unsigned {
            ($sample:ty, $bits:literal) => {
                let midpoint: $sample = 1 << ($bits - 1);
                let output = normalize_input(&[
                    0,
                    midpoint / 2,
                    midpoint,
                    midpoint + midpoint / 2,
                    <$sample>::MAX,
                ]);
                assert_eq!(&output[..4], &[-1.0, -0.5, 0.0, 0.5]);
                assert!(output[4] > 0.99 && output[4] <= 1.0);
                assert!(normalize_input::<$sample>(&[]).is_empty());
            };
        }
        signed!(i8);
        signed!(i16);
        signed!(i32);
        signed!(i64);
        unsigned!(u8, 8);
        unsigned!(u16, 16);
        unsigned!(u32, 32);
        unsigned!(u64, 64);
    }

    #[test]
    fn floating_microphone_samples_keep_their_amplitude() {
        let samples = [-1.0f32, -0.5, -0.0, 0.0, 0.25, 0.5, 1.0];
        let output = normalize_input(&samples);
        for (source, converted) in samples.iter().zip(output) {
            assert_eq!(source.to_bits(), converted.to_bits());
        }
        let doubles = samples.map(f64::from);
        assert_eq!(normalize_input(&doubles), samples);
        assert!(normalize_input::<f32>(&[]).is_empty());
        assert!(normalize_input::<f64>(&[]).is_empty());
    }

    #[test]
    fn microphone_pcm_conversion_preserves_split_stereo_resampling() {
        fn convert_callbacks<T: cpal::Sample>(input: &[T], partition: usize) -> Vec<f32>
        where
            f32: cpal::FromSample<T>,
        {
            let mut resampler = Resampler::new(44100, 2);
            let mut actual = Vec::new();
            for chunk in input.chunks(partition) {
                actual.extend(resampler.process(&normalize_input(chunk)));
            }
            actual.extend(resampler.finish());
            actual
        }

        // An independent PCM scale reference, including interleaved channel
        // boundaries. Callback partitions deliberately split stereo frames.
        let signed: Vec<i16> = (0..8000)
            .map(|index| ((index * 997 % 65536) - 32768) as i16)
            .collect();
        let unsigned: Vec<u16> = signed
            .iter()
            .map(|&sample| (i32::from(sample) + 32768) as u16)
            .collect();
        let reference: Vec<f32> = signed
            .iter()
            .map(|&sample| sample as f32 / 32768.0)
            .collect();
        let mut reference_resampler = Resampler::new(44100, 2);
        let mut expected = reference_resampler.process(&reference);
        expected.extend(reference_resampler.finish());
        for partition in [1, 7, 511, 1280, 8000] {
            for actual in [
                convert_callbacks(&signed, partition),
                convert_callbacks(&unsigned, partition),
            ] {
                assert_eq!(actual, expected, "callback partition {partition}");
            }
        }
    }

    #[test]
    fn gain_does_not_flatten_a_loud_onset_after_quiet_speech() {
        let mut agc = Agc::new();
        let mut quiet = vec![0.05, -0.05];
        agc.apply(&mut quiet, true);
        assert_eq!(quiet, vec![0.1, -0.1]);

        // The tracked peak still permits 2x gain. A louder syllable must
        // retain its waveform rather than saturate its largest samples.
        let original = [0.8, -0.8, 0.6, -0.6, 0.2, -0.2];
        let mut onset = original;
        agc.apply(&mut onset, true);
        let gain = onset[0] / original[0];
        assert!(gain >= 1.0 && gain <= 1.25);
        for (sample, source) in onset.into_iter().zip(original) {
            assert!((sample - source * gain).abs() < 1e-6);
        }
    }

    #[test]
    fn stale_speech_gain_cannot_clip_a_chunk_before_vad_detects_it() {
        let mut agc = Agc::new();
        agc.apply(&mut [0.03, -0.03], true);
        let mut onset = [0.95, -0.95, 0.7, -0.7];
        agc.apply(&mut onset, false);
        let gain = onset[0] / 0.95;
        assert!((onset[2] - 0.7 * gain).abs() < 1e-6);
        assert!(onset.iter().all(|sample| sample.abs() <= 1.0));

        let mut full_scale = [1.0, -1.0, 0.5];
        agc.apply(&mut full_scale, true);
        assert_eq!(full_scale, [1.0, -1.0, 0.5]);
    }

    #[test]
    fn quiet_speech_keeps_its_boost_and_silence_does_not_set_the_reference() {
        let mut agc = Agc::new();
        let mut room = [0.001, -0.001];
        agc.apply(&mut room, false);
        assert_eq!(room, [0.001, -0.001]);
        let mut quiet = [0.04, -0.04, 0.02];
        agc.apply(&mut quiet, true);
        assert_eq!(quiet, [0.08, -0.08, 0.04]);
        for _ in 0..100 {
            agc.apply(&mut [0.0; 16], false);
        }
        agc.apply(&mut [], true);
        let mut later = [0.04, -0.02];
        agc.apply(&mut later, true);
        assert_eq!(later, [0.08, -0.04]);
    }

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
        let skipped = partial_decode_slice(
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
        let kept = partial_decode_slice(
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
        let slice = partial_decode_slice(
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
}
