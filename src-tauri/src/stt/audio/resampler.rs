//! Streaming sample-rate conversion. Work follows output samples, not discarded
//! input samples; an integer cursor carries timing across microphone callbacks.

use super::super::TARGET_SAMPLE_RATE;

fn design_lowpass_fir(fs: f32, fc: f32, m: usize) -> Vec<f32> {
    let mid = (m as f32 - 1.0) / 2.0;
    let fc_norm = fc / fs;
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
        for v in &mut taps {
            *v /= sum;
        }
    }
    taps
}

pub(in crate::stt) struct Resampler {
    src_rate: i64,
    channels: usize,
    taps: Vec<f32>,
    history: Vec<f32>,
    scratch: Vec<f32>,
    // Position relative to the next chunk's first frame, in 1/16000-frame units.
    // May be slightly negative when fractional interpolation needs a next frame.
    phase: i64,
    channel_sum: f32,
    channel_count: usize,
}

impl Resampler {
    pub(in crate::stt) fn new(src_rate: i32, channels: usize) -> Self {
        assert!(src_rate > 0, "microphone sample rate must be positive");
        let taps = if src_rate > TARGET_SAMPLE_RATE {
            // Preserve the existing 63-tap 7.5 kHz anti-alias filter and delay.
            design_lowpass_fir(src_rate as f32, 7500.0, 63)
        } else {
            Vec::new()
        };
        // One extra frame lets the fractional point before a callback boundary
        // retain its entire filter window until the following frame arrives.
        let history_len = taps.len().max(1);
        Self {
            src_rate: i64::from(src_rate),
            channels: channels.max(1),
            taps,
            history: vec![0.0; history_len],
            scratch: Vec::new(),
            phase: 0,
            channel_sum: 0.0,
            channel_count: 0,
        }
    }

    pub(in crate::stt) fn process(&mut self, interleaved: &[f32]) -> Vec<f32> {
        if interleaved.is_empty() {
            return Vec::new();
        }
        if self.src_rate == i64::from(TARGET_SAMPLE_RATE) && self.channels == 1 {
            self.history[0] = *interleaved.last().unwrap();
            return interleaved.to_vec();
        }
        let history_len = self.history.len();
        self.scratch.clear();
        self.scratch.extend_from_slice(&self.history);
        if self.channels == 1 {
            self.scratch.extend_from_slice(interleaved);
        } else {
            let inv = 1.0 / self.channels as f32;
            for &sample in interleaved {
                self.channel_sum += sample;
                self.channel_count += 1;
                if self.channel_count == self.channels {
                    self.scratch.push(self.channel_sum * inv);
                    self.channel_sum = 0.0;
                    self.channel_count = 0;
                }
            }
        }
        let frames = self.scratch.len() - history_len;
        if frames == 0 {
            return Vec::new();
        }
        let target = i64::from(TARGET_SAMPLE_RATE);
        let out = if self.src_rate == target {
            self.scratch[history_len..].to_vec()
        } else {
            let capacity = ((frames as i64 + 1) * target / self.src_rate + 1) as usize;
            let mut out = Vec::with_capacity(capacity);
            let mut last_filtered = None;
            loop {
                let base = self.phase.div_euclid(target);
                let fraction = self.phase.rem_euclid(target);
                if base >= frames as i64 || (fraction != 0 && base + 1 >= frames as i64) {
                    break;
                }
                let index = (history_len as i64 + base) as usize;
                let low = filtered_sample(&self.scratch, &self.taps, index, &mut last_filtered);
                let value = if fraction == 0 {
                    low
                } else {
                    let high =
                        filtered_sample(&self.scratch, &self.taps, index + 1, &mut last_filtered);
                    low + (high - low) * (fraction as f32 / target as f32)
                };
                out.push(value);
                self.phase += self.src_rate;
            }
            self.phase -= frames as i64 * target;
            out
        };
        // A callback can be shorter than the filter. History + new frames is
        // still long enough, so retaining its suffix works for every partition.
        self.history
            .copy_from_slice(&self.scratch[self.scratch.len() - history_len..]);
        out
    }

    /// Complete the last fractional point when the microphone has stopped.
    /// Hold its last filtered frame; do not synthesize another chunk of silence.
    pub(in crate::stt) fn finish(&mut self) -> Vec<f32> {
        let mut out = Vec::new();
        if self.phase < 0 {
            let last =
                filtered_sample(&self.history, &self.taps, self.history.len() - 1, &mut None);
            while self.phase < 0 {
                out.push(last);
                self.phase += self.src_rate;
            }
        }
        out
    }
}

fn filtered_sample(
    samples: &[f32],
    taps: &[f32],
    index: usize,
    cached: &mut Option<(usize, f32)>,
) -> f32 {
    if let Some((previous_index, value)) = cached {
        if *previous_index == index {
            return *value;
        }
    }
    let value = if taps.is_empty() {
        samples[index]
    } else {
        taps.iter()
            .zip(&samples[index + 1 - taps.len()..=index])
            .fold(0.0, |value, (tap, sample)| value + tap * sample)
    };
    *cached = Some((index, value));
    value
}

#[cfg(test)]
#[path = "resampler/tests.rs"]
mod tests;
