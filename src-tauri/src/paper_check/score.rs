//! Calibration parameters and the fusion that turns raw signals (deterministic
//! feature score + LLM judge probability) into the final AI-likelihood number.
//!
//! The number is only meaningful if the fusion is calibrated against labelled
//! data. Until `calibrate` runs, we fall back to conservative prior weights and
//! flag the result as *uncalibrated* so the UI never presents a bogus accuracy.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::types::{AiRateResult, ChannelScore, FeatureStats, SentenceJudgement};

/// Logistic-fusion parameters: p = sigmoid(bias + w_feature·f + w_judge·j).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationParams {
    pub bias: f32,
    pub w_feature: f32,
    pub w_judge: f32,
    /// Decision threshold that maximised accuracy on the validation set, in
    /// RAW fused-probability space (internal).
    pub threshold: f32,
    /// `threshold` mapped through the same display rescale as the reported
    /// percentage (see `display_rescale`) — what the UI's verdict bands use.
    #[serde(default)]
    pub display_threshold: f32,
    /// Measured accuracy on the local validation set. A point estimate over a
    /// small n — always present it together with the interval below.
    pub method_accuracy: f32,
    /// Wilson 95% interval for `method_accuracy`. On the tiny validation sets
    /// this tool works with (n≈15–30) the point estimate alone overstates
    /// certainty by a wide margin; the UI shows this range instead.
    #[serde(default)]
    pub accuracy_low: f32,
    #[serde(default)]
    pub accuracy_high: f32,
    /// Rate at which human text was wrongly flagged as AI (the误伤 metric).
    #[serde(default)]
    pub false_positive_rate: f32,
    /// Rate at which AI text was missed (classified as human).
    #[serde(default)]
    pub false_negative_rate: f32,
    /// Number of labelled samples behind `method_accuracy`.
    pub validation_n: usize,
    /// True once fitted against real labelled data.
    pub calibrated: bool,
}

impl Default for CalibrationParams {
    fn default() -> Self {
        // Conservative prior: weight the reproducible feature score and the
        // judge roughly equally, centred so 0.5/0.5 inputs → ~0.5 output.
        Self {
            bias: -1.6,
            w_feature: 1.6,
            w_judge: 1.6,
            threshold: 0.5,
            display_threshold: 0.5,
            method_accuracy: 0.0,
            accuracy_low: 0.0,
            accuracy_high: 0.0,
            false_positive_rate: 0.0,
            false_negative_rate: 0.0,
            validation_n: 0,
            calibrated: false,
        }
    }
}

fn params_path() -> PathBuf {
    crate::client::data_dir().join("paper_check_calibration.json")
}

pub fn load_params() -> CalibrationParams {
    let path = params_path();
    let mut params: CalibrationParams = if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|d| serde_json::from_str(&d).ok())
            .unwrap_or_default()
    } else {
        CalibrationParams::default()
    };
    // display_threshold is derived from (bias, weights, threshold); always
    // recompute on load so persisted files stay correct when the display
    // mapping itself changes (e.g. the [20%, 95%] range introduction).
    params.display_threshold = display_rescale(&params, params.threshold);
    params
}

pub fn save_params(params: &CalibrationParams) -> Result<(), String> {
    let data = serde_json::to_string_pretty(params)
        .map_err(|e| format!("JSON serialization error: {e}"))?;
    std::fs::write(params_path(), data).map_err(|e| format!("Failed to write calibration: {e}"))
}

pub fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

// ── Channel normalisation ──
//
// The three model channels measure different raw quantities on incompatible
// scales: the judge emits a probability (0..1, roughly centred), DNA-GPT emits
// a 4-gram overlap fraction (AI text typically lands ~0.05–0.35; human text
// near 0), and Raidar emits a bigram Jaccard (AI text ~0.5–0.8; human ~0.2–0.5,
// a rewrite always changes *something*). Averaging the raw values would let the
// low-magnitude channels systematically drag the fused score down. Each raw
// value is therefore mapped through a monotone ramp onto a common 0..1
// "AI-likelihood" scale first, so the channels are exchangeable estimates of
// the same quantity — which also keeps the calibrated fusion valid when a
// channel is unavailable for a given document (short text, request failure).
// The operating points are priors; the logistic fusion fitted by `calibrate`
// absorbs residual scale error.

/// Map a raw DNA-GPT n-gram overlap onto the common likelihood scale.
pub fn normalize_dna(raw_overlap: f32) -> f32 {
    ramp(raw_overlap, 0.02, 0.30)
}

/// Map a raw Raidar rewrite-Jaccard onto the common likelihood scale.
/// Operating points raised from (0.35, 0.75) after field data: a conservative
/// rewriter (minimax-m3 at temperature 0) left an author-verified HUMAN report
/// at raw Jaccard ≈ 0.63, which the old ramp displayed as 71% AI-leaning.
pub fn normalize_raidar(raw_jaccard: f32) -> f32 {
    ramp(raw_jaccard, 0.45, 0.85)
}

/// Linear ramp: `lo_at` → 0, `hi_at` → 1, clamped.
fn ramp(value: f32, lo_at: f32, hi_at: f32) -> f32 {
    ((value - lo_at) / (hi_at - lo_at)).clamp(0.0, 1.0)
}

/// Fuse feature score + judge probability into a calibrated 0..1 likelihood.
pub fn fuse_probability(params: &CalibrationParams, feature_score: f32, judge_prob: f32) -> f32 {
    sigmoid(params.bias + params.w_feature * feature_score + params.w_judge * judge_prob)
        .clamp(0.0, 1.0)
}

/// Display floor: "no detectable trace" still shows 20%, not 0%. The tool's
/// measured false-negative rate is 30–40% (style-matched AI is largely
/// invisible to it); with a plausible base rate of AI use, the posterior for
/// a clean scan sits around 15–20% — displaying 0% would certify originality,
/// which the tool cannot do.
const DISPLAY_MIN: f32 = 0.20;
/// Display ceiling: symmetric honesty in the other direction — the tool can
/// never be *certain* text is machine-written (false positives exist), so
/// saturated evidence shows 95%, not 100%.
const DISPLAY_MAX: f32 = 0.95;

/// Map a raw fused probability onto the tool's REACHABLE range for display.
///
/// The (regularised, small-n) logistic cannot reach 0 or 1: with typical
/// fitted params, zero evidence in every channel still lands at
/// sigmoid(bias) ≈ 0.35, and full evidence tops out near 0.93 — so obviously
/// human documents were displaying as "35–45% AI", which reads as an
/// accusation. The displayed number is therefore the position within
/// [sigmoid(bias), sigmoid(bias + Σw)], mapped onto [DISPLAY_MIN,
/// DISPLAY_MAX]: zero evidence → 20%, saturated evidence → 95%. Order and the
/// decision threshold's relative position are preserved (the threshold is
/// mapped through the same function, stored as `display_threshold`).
pub fn display_rescale(params: &CalibrationParams, p: f32) -> f32 {
    let floor = sigmoid(params.bias);
    let ceil = sigmoid(params.bias + params.w_feature + params.w_judge);
    if ceil - floor <= f32::EPSILON {
        return p;
    }
    let unit = ((p - floor) / (ceil - floor)).clamp(0.0, 1.0);
    DISPLAY_MIN + unit * (DISPLAY_MAX - DISPLAY_MIN)
}

/// Confidence bucket derived from how much *all* the available channels agree
/// (their spread) and whether the model is calibrated. Measuring the spread
/// across every channel — not just feature-vs-model — means genuine disagreement
/// between the judge, DNA-GPT and Raidar lowers confidence instead of being
/// hidden by averaging them into a single `model_prob` first.
fn confidence_from_channels(calibrated: bool, channels: &[ChannelScore]) -> &'static str {
    let scores: Vec<f32> = channels
        .iter()
        .filter(|c| c.available)
        .map(|c| c.score)
        .collect();
    if scores.len() < 2 {
        // A single evidence source cannot be corroborated.
        return if calibrated { "medium" } else { "low" };
    }
    let max = scores.iter().cloned().fold(f32::MIN, f32::max);
    let min = scores.iter().cloned().fold(f32::MAX, f32::min);
    let spread = max - min; // 0 = perfect agreement
    match (calibrated, spread) {
        (true, s) if s <= 0.2 => "high",
        (true, s) if s <= 0.4 => "medium",
        (false, s) if s <= 0.2 => "medium",
        _ => "low",
    }
}

/// Assemble the final AiRateResult from the raw signals + persisted calibration.
pub fn assemble(
    features: FeatureStats,
    model_prob: f32,
    channels: Vec<ChannelScore>,
    sentences: Vec<SentenceJudgement>,
    mut notes: Vec<String>,
) -> AiRateResult {
    let params = load_params();
    let raw_probability = fuse_probability(&params, features.feature_ai_score, model_prob);
    // The user-facing number is the display-rescaled value; verdict bands use
    // display_threshold, so the decision point stays aligned.
    let probability = display_rescale(&params, raw_probability);
    let confidence = confidence_from_channels(params.calibrated, &channels);

    if !params.calibrated {
        notes.push(
            "未校正:この数値は暫定的な事前推定です。検証セットで校正すると精度が測定されます。"
                .to_string(),
        );
    }

    // Liang et al. (2023): detectors systematically over-flag plain / non-native
    // writing. When we lean AI but the signals are weak/disagreeing, say so.
    // "Lean AI" is judged at the fitted decision threshold, not a fixed 0.5.
    if raw_probability >= params.threshold && confidence != "high" {
        notes.push(
            "注意:平易な文章や非母語話者の人間による文章は、AIと誤判定されやすいことが報告されています(Liang et al., 2023)。この結果だけで判断しないでください。"
                .to_string(),
        );
    }

    AiRateResult {
        probability,
        percent: (probability * 100.0).round().clamp(0.0, 100.0) as u8,
        confidence: confidence.to_string(),
        calibrated: params.calibrated,
        method_accuracy: if params.calibrated {
            Some(params.method_accuracy)
        } else {
            None
        },
        // Interval fields are zero on calibrations saved before they existed.
        accuracy_low: (params.calibrated && params.accuracy_high > 0.0)
            .then_some(params.accuracy_low),
        accuracy_high: (params.calibrated && params.accuracy_high > 0.0)
            .then_some(params.accuracy_high),
        validation_n: if params.calibrated {
            Some(params.validation_n)
        } else {
            None
        },
        features,
        channels,
        sentences,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fusion_monotonic_in_both_channels() {
        let p = CalibrationParams::default();
        let low = fuse_probability(&p, 0.1, 0.1);
        let mid = fuse_probability(&p, 0.5, 0.5);
        let high = fuse_probability(&p, 0.9, 0.9);
        assert!(low < mid && mid < high);
    }

    #[test]
    fn display_rescale_spans_twenty_to_ninety_five() {
        // Shape of the round-5 fitted params: sigmoid floor ≈ 0.35 meant
        // zero-evidence text displayed as "35% AI". After rescale the floor
        // shows the 20% detection-limit, the ceiling 95%, and the threshold
        // maps consistently in between.
        let p = CalibrationParams {
            bias: -0.628,
            w_feature: 0.0,
            w_judge: 3.297,
            threshold: 0.55,
            ..CalibrationParams::default()
        };
        let floor = fuse_probability(&p, 0.0, 0.0);
        let ceil = fuse_probability(&p, 1.0, 1.0);
        assert!((display_rescale(&p, floor) - 0.20).abs() < 0.005);
        assert!((display_rescale(&p, ceil) - 0.95).abs() < 0.005);
        let dt = display_rescale(&p, p.threshold);
        assert!(dt > 0.20 && dt < 0.95);
        // Monotone.
        assert!(display_rescale(&p, 0.4) < display_rescale(&p, 0.6));
    }

    #[test]
    fn channel_normalisation_maps_typical_ranges() {
        // Human-typical raw values must land low, AI-typical high — so the
        // normalised channels are comparable to a judge probability.
        assert!(normalize_dna(0.0) < 0.1 && normalize_dna(0.30) > 0.9);
        assert!(normalize_raidar(0.40) < 0.1 && normalize_raidar(0.85) > 0.9);
        // A conservative rewrite of human text (field-observed raw ≈ 0.63)
        // must stay below the AI-leaning half.
        assert!(normalize_raidar(0.63) < 0.5);
        // Monotone in between.
        assert!(normalize_dna(0.10) < normalize_dna(0.20));
        assert!(normalize_raidar(0.55) < normalize_raidar(0.75));
    }
}
