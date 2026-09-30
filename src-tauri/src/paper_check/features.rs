//! Deterministic stylometric feature extraction. No model, no network, no
//! randomness — the same text always yields the same numbers, which is what
//! makes the downstream score defensible/reproducible.

use super::text::{split_sentences, tokenize, visible_len};
use super::types::FeatureStats;

/// Connectives that AI writing tends to over-use as sentence scaffolding.
/// Deliberately small and language-mixed (JA + EN) — this is a density signal,
/// not an exhaustive list.
const TRANSITIONS: &[&str] = &[
    // English
    "however",
    "moreover",
    "furthermore",
    "therefore",
    "additionally",
    "consequently",
    "nevertheless",
    "thus",
    "hence",
    "overall",
    "in conclusion",
    "firstly",
    "secondly",
    "finally",
    "importantly",
    "notably",
    // Japanese
    "しかし",
    "また",
    "さらに",
    "したがって",
    "そのため",
    "加えて",
    "すなわち",
    "つまり",
    "一方",
    "結論として",
    "重要なことに",
    "まず",
];

pub fn extract(text: &str) -> FeatureStats {
    let sentences = split_sentences(text);
    let tokens = tokenize(text);

    let burstiness = burstiness(&sentences);
    let sentence_len_cv = sentence_len_cv(&sentences);
    let lexical_diversity = moving_avg_ttr(&tokens, 50);
    let ngram_repetition = ngram_repetition(&tokens, 3);
    let transition_density = transition_density(text, sentences.len());
    let compressibility = compressibility(text);

    let feature_ai_score = combine(
        burstiness,
        lexical_diversity,
        ngram_repetition,
        transition_density,
        compressibility,
    );

    FeatureStats {
        burstiness,
        sentence_len_cv,
        lexical_diversity,
        ngram_repetition,
        transition_density,
        compressibility,
        feature_ai_score,
    }
}

/// Deflate compression ratio (compressed/raw bytes) in (0,1]. Lower-entropy,
/// more templated text — which AI output tends to be — compresses further, so a
/// *smaller* ratio leans AI. Language-agnostic and fully deterministic.
fn compressibility(text: &str) -> f32 {
    use flate2::{write::DeflateEncoder, Compression};
    use std::io::Write;

    let raw = text.as_bytes();
    if raw.len() < 64 {
        // Too short for compression statistics to be meaningful.
        return 1.0;
    }
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::best());
    if enc.write_all(raw).is_err() {
        return 1.0;
    }
    match enc.finish() {
        Ok(compressed) => (compressed.len() as f32 / raw.len() as f32).clamp(0.0, 1.0),
        Err(_) => 1.0,
    }
}

/// Burstiness B = (σ − μ)/(σ + μ) over sentence character-lengths, in [−1, 1].
/// Higher = more human-like variability.
fn burstiness(sentences: &[String]) -> f32 {
    let lens: Vec<f32> = sentences.iter().map(|s| visible_len(s) as f32).collect();
    if lens.len() < 2 {
        return 0.0;
    }
    let (mean, std) = mean_std(&lens);
    if mean + std <= f32::EPSILON {
        return 0.0;
    }
    (std - mean) / (std + mean)
}

fn sentence_len_cv(sentences: &[String]) -> f32 {
    let lens: Vec<f32> = sentences.iter().map(|s| visible_len(s) as f32).collect();
    if lens.is_empty() {
        return 0.0;
    }
    let (mean, std) = mean_std(&lens);
    if mean <= f32::EPSILON {
        return 0.0;
    }
    std / mean
}

/// Moving-average type-token ratio: average unique/total over fixed windows,
/// which (unlike raw TTR) does not decay with document length.
fn moving_avg_ttr(tokens: &[String], window: usize) -> f32 {
    if tokens.is_empty() {
        return 0.0;
    }
    if tokens.len() <= window {
        return unique_ratio(tokens);
    }
    let mut sum = 0.0f32;
    let mut count = 0usize;
    let mut i = 0usize;
    while i + window <= tokens.len() {
        sum += unique_ratio(&tokens[i..i + window]);
        count += 1;
        i += window;
    }
    if count == 0 {
        unique_ratio(tokens)
    } else {
        sum / count as f32
    }
}

fn unique_ratio(tokens: &[String]) -> f32 {
    if tokens.is_empty() {
        return 0.0;
    }
    let unique: std::collections::HashSet<&String> = tokens.iter().collect();
    unique.len() as f32 / tokens.len() as f32
}

/// Fraction of word n-grams that occur more than once.
fn ngram_repetition(tokens: &[String], n: usize) -> f32 {
    if tokens.len() < n {
        return 0.0;
    }
    let mut counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let total = tokens.len() - n + 1;
    for i in 0..total {
        let gram = tokens[i..i + n].join("\u{1}");
        *counts.entry(gram).or_insert(0) += 1;
    }
    let repeated: usize = counts
        .values()
        .filter(|&&c| c > 1)
        .map(|&c| c as usize)
        .sum();
    repeated as f32 / total as f32
}

fn transition_density(text: &str, sentence_count: usize) -> f32 {
    if sentence_count == 0 {
        return 0.0;
    }
    let lower = text.to_lowercase();
    let mut hits = 0usize;
    for t in TRANSITIONS {
        hits += lower.matches(t).count();
    }
    hits as f32 / sentence_count as f32
}

/// Fold the four discriminative features into a single 0..1 feature-only AI
/// score (higher = more AI-like). Each feature is mapped through a monotonic
/// ramp with a literature-informed operating point, then averaged. These are
/// the *prior* weights; `calibrate` can override the fusion downstream.
fn combine(
    burstiness: f32,
    lexical_diversity: f32,
    ngram_repetition: f32,
    transition_density: f32,
    compressibility: f32,
) -> f32 {
    // Low burstiness → AI. burstiness in [-1,1]; map -0.6..0.2 to 1..0.
    let s_burst = ramp(burstiness, 0.2, -0.6);
    // Low lexical diversity → AI. TTR window ratio typically 0.55..0.85.
    let s_lex = ramp(lexical_diversity, 0.8, 0.5);
    // High repetition → AI.
    let s_rep = ramp(ngram_repetition, 0.02, 0.15);
    // High transition density → AI.
    let s_trans = ramp(transition_density, 0.1, 0.6);
    // High compressibility (small ratio) → AI. Deflate ratios for prose sit
    // roughly in 0.30..0.55; smaller = lower entropy = more AI-like.
    let s_comp = ramp(compressibility, 0.55, 0.32);

    let avg = (s_burst + s_lex + s_rep + s_trans + s_comp) / 5.0;
    avg.clamp(0.0, 1.0)
}

/// Linear ramp mapping `value` from `lo_at`→0 and `hi_at`→1, clamped. `lo_at`
/// and `hi_at` may be in either order (supports inverted features).
fn ramp(value: f32, lo_at: f32, hi_at: f32) -> f32 {
    if (hi_at - lo_at).abs() <= f32::EPSILON {
        return 0.5;
    }
    ((value - lo_at) / (hi_at - lo_at)).clamp(0.0, 1.0)
}

fn mean_std(xs: &[f32]) -> (f32, f32) {
    let n = xs.len() as f32;
    if n == 0.0 {
        return (0.0, 0.0);
    }
    let mean = xs.iter().sum::<f32>() / n;
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / n;
    (mean, var.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_text_scores_higher_than_varied() {
        // Uniform, repetitive → higher feature score.
        let uniform = "This is a test sentence here. This is a test sentence here. \
            This is a test sentence here. This is a test sentence here.";
        // Varied length + vocabulary → lower.
        let varied = "Rain. The committee, weary after months of deliberation, \
            reconvened at dawn to reconsider everything. Why? Nobody knew. \
            A single dissenting voice changed the vote entirely, and history turned.";
        let fu = extract(uniform).feature_ai_score;
        let fv = extract(varied).feature_ai_score;
        assert!(fu > fv, "uniform {fu} should exceed varied {fv}");
    }

    #[test]
    fn scores_stay_in_range() {
        let f = extract("Hello world. Another sentence follows here quickly.");
        assert!((0.0..=1.0).contains(&f.feature_ai_score));
    }

    #[test]
    fn repetitive_text_compresses_more_than_varied() {
        let repetitive = "the same clause repeats again and again ".repeat(30);
        let varied = "Rain fell. The weary committee reconvened at dawn to reconsider \
            everything, though nobody quite knew why, until one dissenting voice \
            abruptly changed the vote and, with it, the entire course of the inquiry."
            .repeat(3);
        let cr = compressibility(&repetitive);
        let cv = compressibility(&varied);
        assert!(
            cr < cv,
            "repetitive ({cr}) should compress smaller than varied ({cv})"
        );
        assert!((0.0..=1.0).contains(&cr) && (0.0..=1.0).contains(&cv));
    }
}
