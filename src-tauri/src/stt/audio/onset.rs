//! Keep a bounded prefix of gated audio for the next audible onset.

use super::super::TARGET_SAMPLE_RATE;

const PRE_ROLL_SAMPLES: usize = TARGET_SAMPLE_RATE as usize / 5; // 200 ms

#[derive(Default)]
pub(in crate::stt) struct OnsetBuffer {
    retained: Vec<f32>,
}

impl OnsetBuffer {
    pub(in crate::stt) fn accept(
        &mut self,
        mut samples: Vec<f32>,
        allowed: bool,
    ) -> Option<Vec<f32>> {
        if samples.is_empty() {
            return None;
        }
        if !allowed {
            // Keep only the recent suffix, even if an input callback is huge.
            // Allocate this fixed buffer once; silence never grows it.
            let incoming = &samples[samples.len().saturating_sub(PRE_ROLL_SAMPLES)..];
            let keep = self.retained.len().min(PRE_ROLL_SAMPLES - incoming.len());
            let start = self.retained.len() - keep;
            self.retained.copy_within(start.., 0);
            self.retained.truncate(keep);
            if self.retained.capacity() == 0 {
                self.retained.reserve_exact(PRE_ROLL_SAMPLES);
            }
            self.retained.extend_from_slice(incoming);
            return None;
        }
        if !self.retained.is_empty() {
            let prefix = self.retained.len();
            let incoming_len = samples.len();
            samples.reserve_exact(prefix);
            samples.resize(incoming_len + prefix, 0.0);
            samples.copy_within(..incoming_len, prefix);
            samples[..prefix].copy_from_slice(&self.retained);
            self.retained.clear();
        }
        Some(samples)
    }
}

#[cfg(test)]
#[path = "onset/tests.rs"]
mod tests;
