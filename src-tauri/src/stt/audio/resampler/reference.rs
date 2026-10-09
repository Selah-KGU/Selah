//! Previous implementation retained only as a DSP regression/benchmark baseline.
//! Its per-callback rounding and low-rate passthrough are known defects; tests
//! compare matching 48 kHz waveforms and performance, not those behaviors.

use super::super::{design_lowpass_fir, TARGET_SAMPLE_RATE};

/// Stateful resampler: stereo/mono-interleaved input → 16 kHz mono.
///
/// For source rates above the target we apply a windowed-sinc low-pass
/// before decimation to avoid aliasing (the previous pure-linear path
/// folded the 8-24 kHz band into speech, hurting sibilants). State is
/// carried across chunks so the filter has no boundary transients.
pub(super) struct LegacyResampler {
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

impl LegacyResampler {
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
