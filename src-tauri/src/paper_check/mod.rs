//! 論文チェッカー backend — a standalone Copilot "app" that takes a single
//! uploaded report and returns a scientifically-grounded AI-generation estimate
//! plus a web similarity/plagiarism check. No user-supplied references needed.
//!
//! Submodules: `text` (segmentation), `features` (deterministic stylometry),
//! `judge` (configured-model rubric judge), `score` (calibrated fusion),
//! `calibrate` (self-built validation set), `search`/`similarity` (查重).
//!
//! The AI-rate and similarity pipelines are separate commands so the frontend
//! can run them in parallel and render whichever finishes first.

mod calibrate;
mod dna;
mod features;
mod judge;
mod raidar;
mod score;
mod search;
mod similarity;
mod text;
mod types;

use std::time::Duration;

use base64::Engine;

use crate::ai;
use text::{clean_cjk_spaces, reflow_soft_wraps, split_sentences};
use types::{AiRateResult, ChannelScore, SimilarityResult, TextStats};

/// Upper bound for the judge channel (many batched requests on long documents).
const JUDGE_TIMEOUT: Duration = Duration::from_secs(240);
/// Upper bound for the single-request DNA-GPT / Raidar channels.
const SIDE_CHANNEL_TIMEOUT: Duration = Duration::from_secs(120);

/// Fusion weight of the judge channel: it aggregates per-sentence verdicts over
/// up to ~200 sentences, so its mean is the most stable model signal — and it
/// is the quantity the calibration was actually fitted on.
const JUDGE_WEIGHT: f32 = 1.0;
/// Fusion weight of DNA-GPT / Raidar. Both are single-sample estimators (one
/// deterministic continuation / rewrite; the original papers average ~10
/// regenerations), so they are treated as corroborating evidence rather than
/// peers of the judge — equal weighting would hand noise a third of the vote
/// and drift the deployed score away from the calibrated one.
const SIDE_CHANNEL_WEIGHT: f32 = 0.35;

/// Extract plain text from an uploaded document. The bytes arrive base64-encoded
/// (the frontend has no filesystem path for a picked file), so we stage them in
/// a temp file and reuse the app's multi-format extractor (full-document
/// variant: no PDF page cap), then clean up. Decoding + parsing is CPU-bound,
/// so it runs on a blocking thread instead of stalling the main thread.
#[tauri::command]
pub async fn paper_check_extract_text(
    file_base64: String,
    filename: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(file_base64.trim())
            .map_err(|e| format!("ファイルの読み込みに失敗しました: {e}"))?;

        let ext = std::path::Path::new(&filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("txt")
            .to_lowercase();

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tmp = std::env::temp_dir().join(format!("kwic_paper_{stamp}.{ext}"));

        std::fs::write(&tmp, &bytes)
            .map_err(|e| format!("一時ファイルの作成に失敗しました: {e}"))?;
        let result = crate::agent_tools::read_downloaded_text_full(&tmp);
        let _ = std::fs::remove_file(&tmp);

        // Strip run-boundary spaces PDF extraction leaves inside CJK words
        // (「思われ る」) — they pollute both the preview and the judge input.
        let text = clean_cjk_spaces(&result?);
        if text.trim().is_empty() {
            return Err("このファイルからテキストを抽出できませんでした。".to_string());
        }
        Ok(text)
    })
    .await
    .map_err(|e| format!("テキスト抽出の実行に失敗しました: {e}"))?
}

/// Cheap document counts over the same reflowed text the analysis channels see,
/// so the header numbers match the report.
#[tauri::command]
pub fn paper_check_text_stats(text: String) -> TextStats {
    let flowed = reflow_soft_wraps(&clean_cjk_spaces(&text));
    let trimmed = flowed.trim();
    TextStats {
        char_count: trimmed.chars().filter(|c| !c.is_whitespace()).count(),
        sentence_count: split_sentences(trimmed).len(),
    }
}

/// AI-generation likelihood pipeline (features + judge + DNA-GPT + Raidar,
/// fused through the persisted calibration).
#[tauri::command]
pub async fn paper_check_analyze_ai(text: String) -> Result<AiRateResult, String> {
    let flowed = reflow_soft_wraps(&clean_cjk_spaces(&text));
    let trimmed = flowed.trim();
    if trimmed.chars().count() < 40 {
        return Err("テキストが短すぎます。40文字以上を入力してください。".to_string());
    }
    Ok(run_ai_rate(trimmed).await)
}

/// Web similarity / plagiarism pipeline.
#[tauri::command]
pub async fn paper_check_analyze_similarity(
    text: String,
    max_queries: Option<usize>,
) -> Result<SimilarityResult, String> {
    let flowed = reflow_soft_wraps(&clean_cjk_spaces(&text));
    let trimmed = flowed.trim();
    if trimmed.chars().count() < 40 {
        return Err("テキストが短すぎます。40文字以上を入力してください。".to_string());
    }
    Ok(similarity::analyze(trimmed, max_queries.unwrap_or(12)).await)
}

async fn run_ai_rate(text: &str) -> AiRateResult {
    let features = features::extract(text);
    let cfg = ai::load_ai_config();

    let mut channels = vec![ChannelScore {
        key: "feature".to_string(),
        label: "統計特徴".to_string(),
        score: features.feature_ai_score,
        raw: features.feature_ai_score,
        available: true,
    }];

    if !cfg.ai_enabled {
        for (key, label) in [
            ("judge", "モデル審査"),
            ("dna", "継続再現(DNA-GPT)"),
            ("raidar", "書換類似(Raidar)"),
        ] {
            channels.push(unavailable_channel(key, label));
        }
        let feature_only = features.feature_ai_score;
        return score::assemble(
            features,
            feature_only,
            channels,
            Vec::new(),
            vec!["AIが無効のため、判定は統計特徴のみで行われました。".to_string()],
        );
    }

    // Three model-based channels in parallel: the rubric judge, DNA-GPT
    // (continuation overlap) and Raidar (rewrite similarity). Each is bounded
    // by its own timeout so one stuck request cannot stall the whole report
    // (the underlying HTTP client allows up to 300s per request).
    let (judge_res, dna_res, raidar_res) = tokio::join!(
        tokio::time::timeout(JUDGE_TIMEOUT, judge::judge(&cfg, text)),
        tokio::time::timeout(SIDE_CHANNEL_TIMEOUT, dna::score(&cfg, text)),
        tokio::time::timeout(SIDE_CHANNEL_TIMEOUT, raidar::score(&cfg, text)),
    );

    let mut notes = Vec::new();
    let mut sentences = Vec::new();
    // (normalised score, fusion weight) per available model channel.
    let mut model_scores: Vec<(f32, f32)> = Vec::new();

    match judge_res {
        Ok(outcome) => {
            notes.extend(outcome.notes);
            sentences = outcome.sentences;
            if outcome.available {
                model_scores.push((outcome.mean_prob, JUDGE_WEIGHT));
                channels.push(ChannelScore {
                    key: "judge".to_string(),
                    label: "モデル審査".to_string(),
                    score: outcome.mean_prob,
                    raw: outcome.mean_prob,
                    available: true,
                });
            } else {
                channels.push(unavailable_channel("judge", "モデル審査"));
            }
        }
        Err(_) => {
            notes.push("モデル審査が時間切れのためスキップしました。".to_string());
            channels.push(unavailable_channel("judge", "モデル審査"));
        }
    }

    // The side channels emit raw quantities on their own scales; normalise
    // onto the common likelihood scale before they join the fusion mean.
    push_normalized(
        &mut channels,
        &mut model_scores,
        &mut notes,
        "dna",
        "継続再現(DNA-GPT)",
        dna_res,
        score::normalize_dna,
    );
    push_normalized(
        &mut channels,
        &mut model_scores,
        &mut notes,
        "raidar",
        "書換類似(Raidar)",
        raidar_res,
        score::normalize_raidar,
    );

    // Fuse the available model channels into one probability (weighted mean —
    // the judge dominates, side channels corroborate); fall back to the
    // reproducible feature score when no model channel ran.
    let model_prob = if model_scores.is_empty() {
        notes
            .push("モデル系の判定が利用できなかったため、統計特徴のみで評価しました。".to_string());
        features.feature_ai_score
    } else {
        let weight_sum: f32 = model_scores.iter().map(|(_, w)| w).sum();
        model_scores.iter().map(|(s, w)| s * w).sum::<f32>() / weight_sum
    };

    score::assemble(features, model_prob, channels, sentences, notes)
}

fn unavailable_channel(key: &str, label: &str) -> ChannelScore {
    ChannelScore {
        key: key.to_string(),
        label: label.to_string(),
        score: 0.0,
        raw: 0.0,
        available: false,
    }
}

/// Fold a timed-out/optional raw side-channel result into the channel list.
fn push_normalized(
    channels: &mut Vec<ChannelScore>,
    model_scores: &mut Vec<(f32, f32)>,
    notes: &mut Vec<String>,
    key: &str,
    label: &str,
    result: Result<Option<f32>, tokio::time::error::Elapsed>,
    normalize: fn(f32) -> f32,
) {
    match result {
        Ok(Some(raw)) => {
            let score = normalize(raw);
            model_scores.push((score, SIDE_CHANNEL_WEIGHT));
            channels.push(ChannelScore {
                key: key.to_string(),
                label: label.to_string(),
                score,
                raw,
                available: true,
            });
        }
        Ok(None) => channels.push(unavailable_channel(key, label)),
        Err(_) => {
            notes.push(format!("{label}が時間切れのためスキップしました。"));
            channels.push(unavailable_channel(key, label));
        }
    }
}

/// Build/refit the local validation set and fit the fusion. Heavy + opt-in.
/// `human_samples` are user-supplied documents vouched for as human-written;
/// `ai_samples` are documents the user knows were AI-written (label 1,
/// grounding the fit in real threat-model data). Progress is streamed to the
/// UI through the channel.
#[tauri::command]
pub async fn paper_check_calibrate(
    human_samples: Option<Vec<String>>,
    ai_samples: Option<Vec<String>>,
    on_progress: tauri::ipc::Channel<types::CalibrationProgress>,
) -> Result<score::CalibrationParams, String> {
    calibrate::run_calibration(
        human_samples.unwrap_or_default(),
        ai_samples.unwrap_or_default(),
        move |done, total| {
            let _ = on_progress.send(types::CalibrationProgress { done, total });
        },
    )
    .await
}

/// Current calibration state (for the UI to show "calibrated / accuracy" badges).
#[tauri::command]
pub fn paper_check_get_calibration() -> score::CalibrationParams {
    score::load_params()
}

/// Read/write the pluggable search backend config (settings surface).
#[tauri::command]
pub fn paper_check_get_search_config() -> search::SearchConfig {
    search::load_config()
}

/// Write an exported report (Markdown) into the managed download folder and
/// reveal it in the OS file manager. Returns the saved path.
#[tauri::command]
pub fn paper_check_save_report(
    app: tauri::AppHandle,
    markdown: String,
    filename: String,
) -> Result<String, String> {
    use tauri_plugin_opener::OpenerExt;

    let dir = crate::commands::default_download_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("フォルダの作成に失敗しました: {e}"))?;

    // Strip anything path-like from the supplied name and force a .md extension.
    let base: String = filename
        .chars()
        .map(|c| if "/\\:*?\"<>|".contains(c) { '_' } else { c })
        .collect();
    let base = base.trim();
    let name = if base.is_empty() {
        "paper_check_report.md".to_string()
    } else if base.to_lowercase().ends_with(".md") {
        base.to_string()
    } else {
        format!("{base}.md")
    };

    let path = dir.join(name);
    std::fs::write(&path, markdown).map_err(|e| format!("保存に失敗しました: {e}"))?;
    let _ = app.opener().reveal_item_in_dir(&path);
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn paper_check_save_search_config(config: search::SearchConfig) -> Result<(), String> {
    search::save_config(&config)
}
