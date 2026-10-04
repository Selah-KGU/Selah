//! Microphone resampling, gain control, and partial-decode windows.

use super::*;

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
pub(super) struct Resampler {
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
    pub(super) fn new(src_rate: i32, channels: usize) -> Self {
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

    pub(super) fn process(&mut self, interleaved: &[f32]) -> Vec<f32> {
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

pub(super) fn normalize_i16_input(data: &[i16]) -> Vec<f32> {
    data.iter().map(|&s| s as f32 / i16::MAX as f32).collect()
}

pub(super) fn normalize_u16_input(data: &[u16]) -> Vec<f32> {
    data.iter()
        .map(|&s| (s as f32 / u16::MAX as f32) * 2.0 - 1.0)
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
    tail_silence_rms: f32,
) -> Option<Vec<f32>> {
    let partial_profile = stt_partial_throttle_profile(&load_config().partial_mode);
    partial_decode_slice_with_profile(
        current_samples,
        last_partial_at,
        stable_streak,
        window_secs,
        &partial_profile,
        tail_silence_rms,
    )
}

fn partial_decode_slice_with_profile(
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
        let skipped = partial_decode_slice_with_profile(
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
        let kept = partial_decode_slice_with_profile(
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
        let slice = partial_decode_slice_with_profile(
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
