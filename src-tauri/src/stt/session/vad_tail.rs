//! Complete the final VAD frame before asking the detector to flush speech.

use super::super::{VoiceActivityDetector, VAD_WINDOW_SAMPLES};

#[derive(Default)]
pub(super) struct VadInputTail {
    pending: usize,
}

impl VadInputTail {
    pub(super) fn accept(&mut self, vad: &VoiceActivityDetector, samples: &[f32]) {
        vad.accept_waveform(samples);
        self.observe(samples.len());
    }

    fn observe(&mut self, samples: usize) {
        self.pending = (self.pending + samples % VAD_WINDOW_SAMPLES) % VAD_WINDOW_SAMPLES;
    }

    fn complete(self, accept: impl FnOnce(&[f32])) {
        if self.pending != 0 {
            // Upstream Flush ignores its incomplete input frame. Complete
            // only that frame, adding at most 511 zeros (31.94 ms at 16 kHz).
            // Consuming this clock prevents a second completion of the tail.
            let padding = [0.0; VAD_WINDOW_SAMPLES];
            accept(&padding[..VAD_WINDOW_SAMPLES - self.pending]);
        }
    }

    pub(super) fn flush(self, vad: &VoiceActivityDetector) {
        self.complete(|padding| vad.accept_waveform(padding));
        vad.flush();
    }
}

#[cfg(test)]
#[path = "vad_tail/tests.rs"]
mod tests;
