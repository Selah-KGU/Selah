//! Calibration: the step that makes the AI-rate number mean something.
//!
//! We build a small labelled validation set — genuinely human prose (label 0)
//! paired with text the *user's configured model* produces on the same seed
//! (label 1) — run the signals over it, fit the logistic fusion, and measure
//! accuracy. The resulting params + measured accuracy are what the report
//! displays. The user can add their own past documents as extra human samples,
//! which both grows the set and closes the genre gap (literary anchors vs.
//! actual student reports).
//!
//! Calibration fits on [feature score, judge probability] ONLY — the
//! continuation/rewrite channels (DNA-GPT, Raidar) are deliberately excluded
//! from the fit. The built-in human anchors are public-domain Aozora Bunko
//! prose that models have likely memorised, so a continuation of a known essay
//! can reproduce the original nearly verbatim: the DNA channel would score
//! those human anchors as strongly AI, poisoning the fitted weights. At
//! inference the DNA and Raidar channels still contribute as prior-normalised
//! corroborating evidence (see `score::normalize_*`), on the same 0..1 scale
//! as the judge.

use super::score::{save_params, CalibrationParams};
use super::{features, judge};
use crate::ai::{self, ChatMessage};

/// Genuinely human, pre-LLM public-domain prose — EXPOSITORY only. Because real
/// inputs are mostly Japanese student reports, the corpus is five 評論/科学随筆
/// mid-passages (寺田寅彦・中谷宇吉郎・三木清・西田幾多郎・戸坂潤, fetched
/// verbatim from Aozora Bunko — mid-document excerpts, not iconic openings, to
/// reduce verbatim memorisation) plus two English expository anchors (Mill,
/// Darwin). These are calibration anchors only, never shown to users.
///
/// Literary/classical anchors (漱石・芥川・太宰・中島敦・福沢) were removed
/// after per-sample diagnostics: their style-matched AI counterparts are
/// *style forgeries* (the model imitating 明治文語) that the judge cannot and
/// arguably should not catch (judge ≈ 0.00–0.05), and they are outside the
/// realistic threat model — students submit modern expository AI text, not
/// classical imitations. Keeping them poisoned the fitted threshold and
/// inflated the false-negative rate with unwinnable samples. User-supplied
/// samples (their own reports) remain the best way to grow and localise the
/// set; the continuation channels stay excluded from the fit regardless
/// (module docs above).
const HUMAN_SAMPLES: &[&str] = &[
    // 寺田寅彦「科学者とあたま」(1933, 科学随筆・中段)
    "少なくも自分でそういう気がする。そのためにややもすると前進する勇気を阻喪しやすい。頭の悪い人は前途に霧がかかっているためにかえって楽観的である。そうして難関に出会っても存外どうにかしてそれを切り抜けて行く。どうにも抜けられない難関というのはきわめてまれだからである。それで、研学の徒はあまり頭のいい先生にうっかり助言を請うてはいけない。きっと前途に重畳する難関を一つ一つしらみつぶしに枚挙されてそうして自分のせっかく楽しみにしている企図の絶望を宣告されるからである。委細かまわず着手してみると存外指摘された難関は楽に始末がついて、指摘されなかった意外な難点に出会うこともある。頭のよい人は、あまりに多く頭の力を過信する恐れがある。その結果として、自然がわれわれに表示する現象が自分の頭で考えたことと一致しない場合に、「自然のほうが間違っている」かのように考える恐れがある。まさかそれほどでなくても、そういったような傾向になる恐れがある。",
    // 中谷宇吉郎「科学と文化」(科学随筆・中段)
    "もしそれだったら科学というものの意味が本当に分っていないのではないかと危ぶまれる。科学は決してアルカロイドのようなものではなく、即ち極少量注射したら瀕死の病人が生き返るというようなものではなくて、実際は米かパンのようなもので、毎日喰べていて栄養のとれるものなのである。科学というものは、整理された常識なのである。もっともこんなことをいっては、この方面の議論をしておられる一部の文学者の叱責を買うかも知れない。それだったら文句なく兜をぬぐつもりである。物理学者が文学者と文章を用いて太刀打ちするのは対等の力では問題にならない。とにかく以上の議論を認めるとしたら、それでは自然科学を広い意味での文化の向上に役立たせるには差し当りどうしたら良いかという問題が残る。それに対しては極めて平凡であるが次のような解決があると思う。それは科学の既知の知識と、科学的の考え方との正常な普及をはかることである。",
    // 三木清「如何に読書すべきか」(1938, 論説・中段)
    "しかしいつまでも濫読のうちに止まっていることは好くない。真の読書家は殆どみな濫読から始めている、しかし濫読から抜け出すことのできない者は真の読書家になることができぬ。濫読はそれから脱却するための濫読であることによって意味を有するのである。濫読に止まるなということは多読してはならぬということではない。多読家でないような読書家があるであろうか。寧ろ読書家とは多読家の別名である。諺に、賢者はただ一冊の本の人間を恐れる、という。ひとは多く読まなければならぬ。読書の必要はただ一冊の本の人間にならないために、云い換えれば、一面的な人間にならないために、存在するのである。単に自分自身の時代のみでなく、また過ぎ去った時代について、単に、自分自身の国のみでなく、また世界について、全体の生活と思想について正しい見通しを得るために、多く読まなければならぬ。即ち読書において一般的教養を心掛けることが大切である。",
    // 西田幾多郎「国語の自在性」(論説・中段)
    "日本人の物の見方考え方の特色は、現実の中に無限を掴むにあるのである。しかし我々は単に俳句の如きものの美を誇とするに安んずることなく、我々の物の見方考え方を深めて、我々の心の底から雄大な文学や深遠な哲学を生み出すよう努力せなければならない。我々は腹の底から物事を深く考え大きく組織して行くと共に、我々の国語をして自ら世界歴史において他に類のない人生観、世界観を表現する特色ある言語たらしめねばならない。本当に物事を考えて真に或物を掴めば、自ら他によって表現することのできない言表が出て来るものである。日本語ほど、他の国語を取り入れてそのまま日本化する言語は少いであろう。久しい間、我々は漢文をそのままに読み、多くの学者は漢文書き下しによって、否、漢文そのものによって自己の思想を発表して来た。それは一面に純なる生きた日本語の発展を妨げたともいい得るであろう。しかし一面には我々の国語の自在性というものを考えることもできる。",
    // 戸坂潤「科学的精神とは何か」(論説・中段)
    "併しそれだけではない。引用の第三の形式は多分に対社会的な意義のあるものだ。と云うのは、例えば金融資本というテーマを検討するとすれば、金融資本についての従来の諸研究に一通り眼を通し、それに対する態度の決定とそれの消化とを用意するのが当り前だが、さて之を論文に書くなり何なりする段になると、筆者は自分がこの用意を怠ってはいなかったということを、一人の「学者」として、即ちそういう一人の世間人として、読者に示す必要のある場合もあるのである。このような意味の引用は尤も、絶対に必要なのでも何でもない。引用なしに話を進めることは常に可能だ。また相当優れた理論家にはそういうタイプも珍しくはない。だが或る程度まで一々の引用を実際に示すことは、論旨の進度を妨げたり自分自身の考察をスレッカラシにしたりしない限り、一種の親切と一種の具体味とを読者に感じさせる。そして之は科学的に云っても意味の大きいことだ。問題は示唆と啓蒙と教育とに関するからである。",
    // NOTE: no built-in English anchors. Public-domain English expository prose
    // is Victorian-era (Mill, Darwin), and diagnostics showed their
    // style-matched counterparts are the same forgery-hard case as 明治文語
    // imitations (judge ≈ 0.02) — unwinnable samples outside the threat model.
    // English coverage comes from user-supplied samples instead; the judge
    // itself is language-agnostic at inference.
];

/// Longest slice of a user-supplied human sample fed to the signals — bounds
/// judge cost while keeping enough text for stable stylometry.
const MAX_USER_SAMPLE_CHARS: usize = 4000;
/// Minimum visible size for a user sample to carry any stylometric signal.
const MIN_USER_SAMPLE_CHARS: usize = 200;
/// Cap on user-supplied samples per calibration run (cost control).
const MAX_USER_SAMPLES: usize = 8;

fn sanitize_samples(samples: Vec<String>) -> Vec<String> {
    samples
        .into_iter()
        .map(|s| {
            s.trim()
                .chars()
                .take(MAX_USER_SAMPLE_CHARS)
                .collect::<String>()
        })
        .filter(|s| s.chars().filter(|c| !c.is_whitespace()).count() >= MIN_USER_SAMPLE_CHARS)
        .take(MAX_USER_SAMPLES)
        .collect()
}

/// Fit the fusion on the validation set. Heavy (one AI generation + one judge
/// pass per sample) but explicit/opt-in. `user_samples` are documents the user
/// vouches for as human-written (their own past reports); `known_ai_samples`
/// are documents the user KNOWS were AI-written — they enter label 1 directly,
/// which grounds the fit in the real threat distribution instead of relying
/// only on freshly generated counterparts. `on_progress` is called after each
/// completed step so the UI can show progress.
pub async fn run_calibration(
    user_samples: Vec<String>,
    known_ai_samples: Vec<String>,
    mut on_progress: impl FnMut(usize, usize),
) -> Result<CalibrationParams, String> {
    let cfg = ai::load_ai_config();
    if !cfg.ai_enabled {
        return Err("AIが無効です。設定でAIを有効にしてから校正してください。".to_string());
    }

    let user_texts = sanitize_samples(user_samples);
    let ai_texts = sanitize_samples(known_ai_samples);

    // Built-in anchors always contribute their (genuinely human) label-0 rows,
    // but their GENERATED counterparts are style imitations of dated prose —
    // saturated, unwinnable label-1 rows under the register-aware judge. Once
    // the user supplies real AI documents, those imitations would only dilute
    // the real threat-model data (and can drag LOO below the quality gate), so
    // they are skipped. User-seed counterparts stay: a modern academic
    // passage generated from the user's own topic is realistic label-1 data.
    let gen_anchor_counterparts = ai_texts.len() < 2;
    if !gen_anchor_counterparts {
        eprintln!(
            "[paper_check] calibration: real AI samples provided — skipping built-in anchor counterparts"
        );
    }
    let seeds: Vec<(&str, bool)> = HUMAN_SAMPLES
        .iter()
        .map(|s| (*s, gen_anchor_counterparts))
        .chain(user_texts.iter().map(|s| (s.as_str(), true)))
        .collect();

    // One step per seed (human signals), one per generated counterpart, one
    // per known-AI sample (signals only).
    let total = seeds.len() + seeds.iter().filter(|(_, gen)| *gen).count() + ai_texts.len();
    let mut done = 0usize;

    let mut feats: Vec<[f32; 2]> = Vec::new(); // [feature_score, judge_prob]
    let mut labels: Vec<f32> = Vec::new();

    for (human, gen_counterpart) in seeds {
        let tag: String = human.chars().take(10).collect();
        // Human sample → label 0.
        match signals(&cfg, human).await {
            Some((hf, hj)) => {
                eprintln!(
                    "[paper_check] calib sample label=0 feature={hf:.3} judge={hj:.3} seed={tag}"
                );
                feats.push([hf, hj]);
                labels.push(0.0);
            }
            None => eprintln!("[paper_check] calibration: judge failed, human row skipped ({tag})"),
        }
        done += 1;
        on_progress(done, total);

        if !gen_counterpart {
            continue;
        }
        // Matched AI sample from the configured model → label 1.
        match generate_ai_counterpart(&cfg, human).await {
            Ok(ai_text) if ai_text.trim().len() > 40 => match signals(&cfg, &ai_text).await {
                Some((af, aj)) => {
                    eprintln!(
                        "[paper_check] calib sample label=1 feature={af:.3} judge={aj:.3} seed={tag}"
                    );
                    feats.push([af, aj]);
                    labels.push(1.0);
                }
                None => {
                    eprintln!("[paper_check] calibration: judge failed, AI row skipped ({tag})")
                }
            },
            Ok(_) => eprintln!("[paper_check] calibration: AI counterpart too short, skipped"),
            Err(e) => eprintln!("[paper_check] calibration: generation failed: {e}"),
        }
        done += 1;
        on_progress(done, total);
    }

    // User-vouched AI documents → label 1 directly (no generation step).
    for ai_text in &ai_texts {
        let tag: String = ai_text.chars().take(10).collect();
        match signals(&cfg, ai_text).await {
            Some((af, aj)) => {
                eprintln!(
                    "[paper_check] calib sample label=1 feature={af:.3} judge={aj:.3} known-ai={tag}"
                );
                feats.push([af, aj]);
                labels.push(1.0);
            }
            None => {
                eprintln!("[paper_check] calibration: judge failed, known-AI row skipped ({tag})")
            }
        }
        done += 1;
        on_progress(done, total);
    }

    if labels.iter().filter(|&&l| l == 1.0).count() < 2
        || labels.iter().filter(|&&l| l == 0.0).count() < 2
    {
        return Err(
            "校正用サンプルが不足しています（AI生成に失敗した可能性があります）。".to_string(),
        );
    }

    // Deployment weights: fit on all data. Reported accuracy: leave-one-out, so
    // the number the UI shows is a held-out estimate, not training-set optimism.
    let (bias, w) = fit_logistic(&feats, &labels, 4000, 0.3);
    let loo = loo_probs(&feats, &labels);
    let metrics = sweep_metrics(&loo, &labels);

    // Quality gate: a fit that cannot separate its own validation set is worse
    // than the labelled-provisional prior — persisting it would silently skew
    // every report (a degenerate round once shipped threshold 0.70, under
    // which obvious AI text read as 低). Reset to the honest default instead
    // and tell the user what the fix is (real samples).
    if metrics.accuracy < 0.7 {
        let _ = save_params(&CalibrationParams::default());
        return Err(format!(
            "検証セットの分離が不十分でした(LOO精度 {:.0}%)。内蔵アンカーのみでは校正できません。設定から「自分が書いた」「AIが書いた」レポートを追加して再校正してください。数値は暫定値に戻しました。",
            metrics.accuracy * 100.0
        ));
    }

    // With n this small a point accuracy overstates certainty (n=30 ⇒ ±~9pt);
    // ship the Wilson 95% interval so the UI can show an honest range.
    let n = labels.len() as f32;
    let (accuracy_low, accuracy_high) = wilson_interval(metrics.accuracy * n, n);

    let mut params = CalibrationParams {
        bias,
        w_feature: w[0],
        w_judge: w[1],
        threshold: metrics.threshold,
        display_threshold: 0.0, // filled below, needs the weights in place
        method_accuracy: metrics.accuracy,
        accuracy_low,
        accuracy_high,
        false_positive_rate: metrics.false_positive_rate,
        false_negative_rate: metrics.false_negative_rate,
        validation_n: labels.len(),
        calibrated: true,
    };
    params.display_threshold = super::score::display_rescale(&params, params.threshold);
    save_params(&params)?;
    Ok(params)
}

/// Wilson score 95% interval for a binomial proportion — well-behaved at the
/// tiny n and extreme p̂ this tool encounters (a normal approximation would
/// collapse to a zero-width interval at p̂ = 1).
fn wilson_interval(successes: f32, n: f32) -> (f32, f32) {
    if n <= 0.0 {
        return (0.0, 0.0);
    }
    let z = 1.96f32;
    let p = (successes / n).clamp(0.0, 1.0);
    let z2 = z * z;
    let denom = 1.0 + z2 / n;
    let centre = (p + z2 / (2.0 * n)) / denom;
    let half = (z / denom) * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt();
    (
        (centre - half).clamp(0.0, 1.0),
        (centre + half).clamp(0.0, 1.0),
    )
}

/// Feature score (deterministic) + judge probability for a text. DNA-GPT and
/// Raidar are intentionally NOT part of the calibration signals — see the
/// module docs: memorised public-domain anchors would poison the continuation
/// channels. Returns `None` when the judge could not run: writing a fallback
/// value into the judge column would silently corrupt the training row (it
/// happened — a transient provider failure once produced rows with
/// judge == feature), so a failed sample is skipped instead.
async fn signals(cfg: &ai::AiConfig, text: &str) -> Option<(f32, f32)> {
    let feature_score = features::extract(text).feature_ai_score;
    let outcome = judge::judge(cfg, text).await;
    outcome
        .available
        .then_some((feature_score, outcome.mean_prob))
}

/// The counterpart must match the seed's GENRE and REGISTER, not be converted
/// to an "academic paragraph": otherwise label 0 (literary anchors) and label 1
/// (academic AI text) differ systematically in genre, and the fitted fusion
/// learns literary-vs-academic instead of human-vs-AI. With style matched, the
/// only systematic difference within each pair is authorship.
async fn generate_ai_counterpart(cfg: &ai::AiConfig, seed: &str) -> Result<String, String> {
    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: "You are a writing assistant. Write an ORIGINAL passage on the same \
                subject as the user's passage, in the SAME GENRE, REGISTER and STYLE \
                (literary stays literary, expository stays expository, academic stays \
                academic), roughly the same length (4-6 sentences). Do not copy sentences \
                from the input. Write in the SAME LANGUAGE as the user's passage (if it is \
                Japanese, respond in Japanese). Return only the passage."
                .to_string(),
            images: Vec::new(),
        },
        ChatMessage {
            role: "user".to_string(),
            content: seed.to_string(),
            images: Vec::new(),
        },
    ];
    let mut gen_cfg = cfg.clone();
    gen_cfg.temperature = 0.7;
    if gen_cfg.max_tokens != 0 && gen_cfg.max_tokens < 512 {
        gen_cfg.max_tokens = 512;
    }
    ai::chat_completion_public(&gen_cfg, messages).await
}

/// L2 strength for the channel weights (not the bias). With n≈24 an
/// unregularised fit saturates (w_judge ≈ 15 ⇒ a near-hard threshold on the
/// judge) and rides sampling noise; mild shrinkage keeps the fusion a fusion.
/// Kept deliberately small: at λ=0.05 the equilibrium weights were so shrunken
/// that the pooled LOO probabilities collapsed into a ~0.13-wide band where
/// per-fold bias jitter drowned the class separation and the threshold sweep
/// degenerated to "flag everything".
const RIDGE_LAMBDA: f32 = 0.01;

/// Plain batch-gradient-descent logistic regression on 2 features, with two
/// deliberate constraints for this domain:
///
/// * **Non-negative weights** (projected gradient). Each input is an
///   AI-evidence score where higher must never LOWER the fused probability.
///   An unconstrained fit on the small anchor corpus once learned
///   w_feature < 0 — a genre artefact (Meiji-era literary prose is metrically
///   uniform, so human anchors score high on the feature channel), and
///   deploying an inverted evidence channel on modern student reports would
///   misfire. The constraint encodes the known monotonicity a priori; if a
///   channel carries no positive signal net of the others, its weight goes to
///   0 instead of negative, and the LOO accuracy reports that honestly.
/// * **Ridge penalty** on the weights (see RIDGE_LAMBDA).
fn fit_logistic(x: &[[f32; 2]], y: &[f32], iters: usize, lr: f32) -> (f32, [f32; 2]) {
    let n = x.len() as f32;
    let mut bias = 0.0f32;
    let mut w = [0.0f32, 0.0f32];
    for _ in 0..iters {
        let mut g_bias = 0.0f32;
        let mut g_w = [0.0f32, 0.0f32];
        for (xi, &yi) in x.iter().zip(y.iter()) {
            let z = bias + w[0] * xi[0] + w[1] * xi[1];
            let p = 1.0 / (1.0 + (-z).exp());
            let err = p - yi;
            g_bias += err;
            g_w[0] += err * xi[0];
            g_w[1] += err * xi[1];
        }
        bias -= lr * g_bias / n;
        w[0] -= lr * (g_w[0] / n + RIDGE_LAMBDA * w[0]);
        w[1] -= lr * (g_w[1] / n + RIDGE_LAMBDA * w[1]);
        // Projection onto the feasible region (monotone evidence).
        w[0] = w[0].max(0.0);
        w[1] = w[1].max(0.0);
    }
    (bias, w)
}

struct Metrics {
    accuracy: f32,
    threshold: f32,
    false_positive_rate: f32,
    false_negative_rate: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cheap preflight for the CLI calibration below: verifies the AI config
    /// (and keychain-backed API key) is readable from a test process.
    /// Run: cargo test --lib probe_ai_config_available -- --ignored --nocapture
    #[test]
    #[ignore = "touches the OS keychain; run explicitly"]
    fn probe_ai_config_available() {
        let cfg = crate::ai::load_ai_config();
        eprintln!(
            "ai_enabled={} provider={} model={} key_present={}",
            cfg.ai_enabled,
            cfg.provider,
            cfg.model,
            !cfg.api_key.is_empty()
        );
        assert!(cfg.ai_enabled, "AI disabled in settings");
        assert!(
            !cfg.api_key.is_empty() || cfg.provider == "local",
            "API key not readable from this process (keychain denied?)"
        );
    }

    /// Real calibration from the CLI, writing the same params file the app
    /// reads. Costs one generation + judge pass per seed against the
    /// configured provider.
    /// Run: cargo test --lib run_real_calibration_cli -- --ignored --nocapture
    #[test]
    #[ignore = "network + provider cost; run explicitly"]
    fn run_real_calibration_cli() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        match rt.block_on(run_calibration(Vec::new(), Vec::new(), |done, total| {
            eprintln!("[calibration] {done}/{total}");
        })) {
            Ok(params) => {
                eprintln!("fitted params: {params:#?}");
                assert!(params.calibrated);
                assert!(params.validation_n >= 4);
            }
            // The quality gate rejecting an anchor-only run is a valid
            // outcome (the built-in anchors saturate under the register-aware
            // judge); real user samples are the fix, not the anchors.
            Err(e) if e.contains("分離が不十分") => {
                eprintln!("quality gate rejected anchor-only calibration: {e}");
            }
            Err(e) => panic!("calibration failed: {e}"),
        }
    }

    #[test]
    fn fitted_weights_stay_non_negative() {
        // Feature 0 anti-correlates with the label (the genre-artefact shape
        // that produced w_feature < 0 on real anchors); feature 1 correlates.
        // The projected fit must clamp the inverted channel to 0, not go
        // negative, while still learning the informative channel.
        let x: Vec<[f32; 2]> = vec![
            [0.8, 0.1],
            [0.7, 0.2],
            [0.9, 0.15],
            [0.75, 0.1], // label 0
            [0.2, 0.9],
            [0.3, 0.8],
            [0.25, 0.85],
            [0.1, 0.9], // label 1
        ];
        let y = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
        let (_bias, w) = fit_logistic(&x, &y, 4000, 0.3);
        assert!(
            w[0] >= 0.0 && w[1] >= 0.0,
            "weights must be non-negative: {w:?}"
        );
        assert!(
            w[1] > 1.0,
            "informative channel should carry the fit: {w:?}"
        );
    }

    #[test]
    fn sweep_finds_split_inside_compressed_band() {
        // Ridge-shrunken fits can compress all probabilities into a band
        // narrower than any fixed grid step; the adaptive sweep must still
        // find the separating threshold.
        let probs = vec![0.45, 0.46, 0.47, 0.48, 0.49, 0.50];
        let y = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let m = sweep_metrics(&probs, &y);
        assert!((m.accuracy - 1.0).abs() < 1e-6, "accuracy {}", m.accuracy);
        assert!(
            m.threshold > 0.47 && m.threshold <= 0.48,
            "threshold {}",
            m.threshold
        );
        assert_eq!(m.false_positive_rate, 0.0);
        assert_eq!(m.false_negative_rate, 0.0);
    }

    #[test]
    fn wilson_interval_is_wide_on_small_n() {
        // 28/30 correct (93%): the interval must be a visible range, not a point.
        let (lo, hi) = wilson_interval(28.0, 30.0);
        assert!(lo > 0.7 && lo < 0.93, "low {lo}");
        assert!(hi > 0.93 && hi <= 1.0, "high {hi}");
        // A perfect score on tiny n must not collapse to [1, 1].
        let (lo2, hi2) = wilson_interval(10.0, 10.0);
        assert!(lo2 < 0.85, "low2 {lo2}");
        assert!((hi2 - 1.0).abs() < 1e-6, "high2 {hi2}");
    }
}

/// Leave-one-out held-out probabilities: for each sample, fit on all the others
/// and predict the sample that was held out. This yields an honest estimate on
/// a set far too small for a fixed train/test split.
fn loo_probs(x: &[[f32; 2]], y: &[f32]) -> Vec<f32> {
    let n = x.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let train_x: Vec<[f32; 2]> = x
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, v)| *v)
            .collect();
        let train_y: Vec<f32> = y
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, v)| *v)
            .collect();
        let (bias, w) = fit_logistic(&train_x, &train_y, 4000, 0.3);
        let z = bias + w[0] * x[i][0] + w[1] * x[i][1];
        out.push(1.0 / (1.0 + (-z).exp()));
    }
    out
}

/// Sweep candidate thresholds on the given probabilities and report the false
/// positive/negative rates at the chosen operating point — the honest "how
/// often does it misfire" numbers the report surfaces. Ties on accuracy
/// prefer the LOWER false-positive rate, then the HIGHER threshold: for a tool
/// whose output can read as an accusation, missing AI text is the acceptable
/// error direction, flagging a human is not.
///
/// Candidates are the midpoints between consecutive sorted probabilities (plus
/// sentinels outside the range) — NOT a fixed grid. A ridge-shrunken fit can
/// compress its whole output into a band narrower than any fixed grid step
/// (observed: perfectly rank-separated signals squeezed into [0.45, 0.55],
/// where a 0.05 grid found no usable split and degenerated to "flag nothing").
fn sweep_metrics(probs: &[f32], y: &[f32]) -> Metrics {
    let mut sorted: Vec<f32> = probs.to_vec();
    sorted.sort_by(f32::total_cmp);
    sorted.dedup();
    let mut candidates: Vec<f32> = Vec::with_capacity(sorted.len() + 1);
    match (sorted.first(), sorted.last()) {
        (Some(&lo), Some(&hi)) => {
            candidates.push((lo - 0.02).clamp(0.01, 0.99));
            for w in sorted.windows(2) {
                candidates.push((w[0] + w[1]) / 2.0);
            }
            candidates.push((hi + 0.02).clamp(0.01, 0.99));
        }
        _ => candidates.push(0.5),
    }

    let mut best: Option<Metrics> = None;
    for t in candidates {
        let (mut correct, mut fp, mut human, mut fneg, mut ai) =
            (0usize, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for (&p, &yi) in probs.iter().zip(y.iter()) {
            let pred_ai = p >= t;
            if pred_ai == (yi >= 0.5) {
                correct += 1;
            }
            if yi < 0.5 {
                human += 1.0;
                if pred_ai {
                    fp += 1.0;
                }
            } else {
                ai += 1.0;
                if !pred_ai {
                    fneg += 1.0;
                }
            }
        }
        let m = Metrics {
            accuracy: correct as f32 / y.len() as f32,
            threshold: t,
            false_positive_rate: if human > 0.0 { fp / human } else { 0.0 },
            false_negative_rate: if ai > 0.0 { fneg / ai } else { 0.0 },
        };
        // `t` ascends, so `<=` on FPR lands on the highest tied threshold.
        let better = match &best {
            None => true,
            Some(b) => {
                m.accuracy > b.accuracy
                    || (m.accuracy == b.accuracy && m.false_positive_rate <= b.false_positive_rate)
            }
        };
        if better {
            best = Some(m);
        }
    }
    best.unwrap_or(Metrics {
        accuracy: 0.0,
        threshold: 0.5,
        false_positive_rate: 0.0,
        false_negative_rate: 0.0,
    })
}
