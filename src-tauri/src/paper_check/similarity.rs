//! Web similarity / plagiarism channel — Turnitin-style *passage coverage*.
//!
//! A single sentence matching something online does not by itself establish a
//! duplication rate, so we do not treat isolated sentences as verdicts. Instead
//! we shingle the whole document into overlapping word-windows, search a budget
//! of distinctive phrases, fetch candidate pages, and then mark every document
//! character span whose shingle appears verbatim in *some* fetched source. The
//! reported rate is the fraction of the document covered by such matched runs,
//! and each reported match is a contiguous passage tied to its dominant source.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use super::search;
use super::text::{is_cjk, tokenize, tokenize_spans};
use super::types::{SimilarityMatch, SimilarityResult};

/// Word-shingle size for Latin-script text. A run this long matching verbatim
/// is a strong copy signal while remaining robust to coincidental overlaps.
const SHINGLE_N: usize = 8;
/// Shingle size for CJK-dominant text. The tokenizer emits one token per CJK
/// character, so 8 tokens would mean just 8 characters — short enough that
/// stock academic phrasing collides constantly and inflates the duplication
/// rate. ~13 characters is comparable evidence to 8 Latin words.
const SHINGLE_N_CJK: usize = 13;
/// Wall-clock budget for the whole search+fetch phase. Without it a slow
/// backend (12 queries × search + page fetches, each with a 15s timeout) can
/// stall the report for minutes.
const SEARCH_BUDGET: Duration = Duration::from_secs(75);
/// Pause between queries on the keyless DuckDuckGo backend — bursting a dozen
/// queries at it is the fastest way to get rate-limited.
const FREE_QUERY_GAP: Duration = Duration::from_millis(400);
/// k-gram size (tokens) for winnowing fingerprints used to pick search queries.
const QUERY_K: usize = 10;
/// Winnowing window (hashes). Guarantees any matching run of ≥ WINDOW+QUERY_K−1
/// tokens contains at least one selected fingerprint (Schleimer et al., 2003).
const WINNOW_WINDOW: usize = 12;
/// Candidate pages fetched per query.
const PAGES_PER_QUERY: usize = 2;
/// Merge covered runs separated by at most this many characters into one passage.
const MERGE_GAP: usize = 10;
/// Minimum visible characters a covered run must have to be reported.
const MIN_PASSAGE_CHARS: usize = 12;
/// Cap on reported passages.
const MAX_MATCHES: usize = 20;

struct Source {
    url: String,
    title: String,
    snippet: String,
    grams: HashSet<String>,
}

pub async fn analyze(text: &str, max_queries: usize) -> SimilarityResult {
    let cfg = search::load_config();
    let provider = search::active_provider_label(&cfg);
    let mut notes = vec![format!("検索バックエンド: {provider}")];

    // Exclude the reference/bibliography section: it legitimately matches sources
    // and would otherwise inflate the duplication rate (as Turnitin excludes it).
    let body = strip_references(text);
    if body.chars().count() < text.chars().count() {
        notes.push("参考文献・引用リストは重複率の対象から除外しました。".to_string());
    }
    let body = body.as_str();

    let chars: Vec<char> = body.chars().collect();
    let toks = tokenize_spans(body);
    let shingle_n = shingle_size(&chars);

    // Document shingles carrying their character span, keyed identically to the
    // page-side n-grams so a verbatim window compares equal.
    let mut shingles: Vec<(String, usize, usize)> = Vec::new();
    if toks.len() >= shingle_n {
        for i in 0..=toks.len() - shingle_n {
            let key = toks[i..i + shingle_n]
                .iter()
                .map(|t| t.0.as_str())
                .collect::<Vec<_>>()
                .join("\u{1}");
            shingles.push((key, toks[i].1, toks[i + shingle_n - 1].2));
        }
    }

    if shingles.is_empty() {
        return SimilarityResult {
            overall_pct: 0,
            matches: Vec::new(),
            available: true,
            notes: vec!["照合対象となる十分な長さの本文がありませんでした。".to_string()],
        };
    }

    // Pick search queries by winnowing fingerprints (Schleimer et al., 2003): a
    // deterministic, density-bounded selection of distinctive phrases spread
    // across the document, rather than arbitrary or longest-first sentences.
    let queries = winnow_queries(&chars, &toks, max_queries);

    // Gather candidate sources once (deduped by URL); scoring is global.
    let mut sources: Vec<Source> = Vec::new();
    let mut seen_urls: HashSet<String> = HashSet::new();
    let mut failed_queries = 0usize;
    let mut ran_queries = 0usize;
    let mut total_results = 0usize;
    let deadline = Instant::now() + SEARCH_BUDGET;

    for (qi, q) in queries.iter().enumerate() {
        if Instant::now() >= deadline {
            notes.push(format!(
                "時間の上限に達したため、{ran_queries}/{}件のクエリのみ照合しました。",
                queries.len()
            ));
            break;
        }
        // The keyless endpoint rate-limits bursts; space the queries out.
        if provider == "free" && qi > 0 {
            tokio::time::sleep(FREE_QUERY_GAP).await;
        }
        ran_queries += 1;
        let query = format!("\"{}\"", q.replace('"', ""));
        let results = match search::search(&cfg, &query, PAGES_PER_QUERY).await {
            Ok(r) => {
                total_results += r.len();
                r
            }
            Err(e) => {
                eprintln!("[paper_check] search failed: {e}");
                failed_queries += 1;
                continue;
            }
        };
        // Fetch this query's candidate pages concurrently (each has its own
        // timeout; sequential fetches were the dominant latency cost).
        let fresh: Vec<search::SearchResult> = results
            .into_iter()
            .take(PAGES_PER_QUERY)
            .filter(|r| seen_urls.insert(r.url.clone()))
            .collect();
        let pages = futures_util::future::join_all(fresh.iter().map(|r| fetch_text(&r.url))).await;
        for (r, page_text) in fresh.into_iter().zip(pages) {
            // The snippet is always available even if the page fetch is blocked.
            let page_text = page_text.unwrap_or_default();
            let haystack = if page_text.trim().is_empty() {
                r.snippet.clone()
            } else {
                page_text
            };
            let grams = ngrams(&haystack, shingle_n);
            if grams.is_empty() {
                continue;
            }
            sources.push(Source {
                url: r.url,
                title: r.title,
                snippet: truncate(&r.snippet, 240),
                grams,
            });
        }
    }

    if ran_queries > 0 && failed_queries == ran_queries {
        return SimilarityResult {
            overall_pct: 0,
            matches: Vec::new(),
            available: false,
            notes: vec![format!(
                "検索バックエンド({provider})に接続できませんでした。設定でAPIキーを追加すると安定します。"
            )],
        };
    }
    // Partial failure silently lowers the duplication rate (missed sources) —
    // that is the dangerous direction for a plagiarism check, so surface it.
    if failed_queries > 0 {
        notes.push(format!(
            "検索クエリ{ran_queries}件中{failed_queries}件が失敗しました。実際の重複率はこの値より高い可能性があります。"
        ));
    }
    if total_results == 0 && provider == "free" {
        notes.push(
            "検索結果が取得できませんでした(無料バックエンドのレート制限の可能性)。設定でAPIキーを追加すると安定します。"
                .to_string(),
        );
    }

    // Per-character attribution: the first source whose shingle covers a span
    // owns it. Isolated words never match — coverage needs an SHINGLE_N-word run.
    let mut owner: Vec<Option<usize>> = vec![None; chars.len()];
    for (si, src) in sources.iter().enumerate() {
        for (key, start, end) in &shingles {
            if src.grams.contains(key) {
                for p in *start..(*end).min(owner.len()) {
                    if owner[p].is_none() {
                        owner[p] = Some(si);
                    }
                }
            }
        }
    }

    let mut total_visible = 0usize;
    let mut covered_visible = 0usize;
    for (idx, c) in chars.iter().enumerate() {
        if !c.is_whitespace() {
            total_visible += 1;
            if owner[idx].is_some() {
                covered_visible += 1;
            }
        }
    }
    let overall_pct = if total_visible == 0 {
        0
    } else {
        ((covered_visible as f32 / total_visible as f32) * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8
    };

    let matches = build_passages(&chars, &owner, &sources);

    SimilarityResult {
        overall_pct,
        matches,
        available: true,
        notes,
    }
}

/// Collapse covered character positions into contiguous passages (merging small
/// gaps), each attributed to the source that owns the most of it.
fn build_passages(
    chars: &[char],
    owner: &[Option<usize>],
    sources: &[Source],
) -> Vec<SimilarityMatch> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut i = 0usize;
    while i < owner.len() {
        if owner[i].is_none() {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i + 1;
        let mut gap = 0usize;
        let mut j = i + 1;
        while j < owner.len() {
            if owner[j].is_some() {
                end = j + 1;
                gap = 0;
            } else {
                gap += 1;
                if gap > MERGE_GAP {
                    break;
                }
            }
            j += 1;
        }
        runs.push((start, end));
        i = end.max(start + 1);
    }

    let mut matches: Vec<SimilarityMatch> = Vec::new();
    for (start, end) in runs {
        let visible = chars[start..end]
            .iter()
            .filter(|c| !c.is_whitespace())
            .count();
        if visible < MIN_PASSAGE_CHARS {
            continue;
        }
        // Dominant source across the run.
        let mut tally: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
        let mut covered = 0usize;
        for c in &owner[start..end] {
            if let Some(si) = c {
                *tally.entry(*si).or_insert(0) += 1;
                covered += 1;
            }
        }
        let Some((&si, _)) = tally.iter().max_by_key(|(_, n)| **n) else {
            continue;
        };
        let src = &sources[si];
        let overlap = (covered as f32 / (end - start) as f32).clamp(0.0, 1.0);
        matches.push(SimilarityMatch {
            sentence: chars[start..end]
                .iter()
                .collect::<String>()
                .trim()
                .to_string(),
            source_url: src.url.clone(),
            source_title: src.title.clone(),
            snippet: src.snippet.clone(),
            overlap,
        });
    }

    matches.sort_by_key(|m| std::cmp::Reverse(m.sentence.chars().count()));
    matches.truncate(MAX_MATCHES);
    matches
}

/// Fetch a page and reduce it to visible text (best-effort; blocked/binary
/// pages yield an empty string, which the caller tolerates).
async fn fetch_text(url: &str) -> Option<String> {
    if !url.starts_with("http") {
        return None;
    }
    let resp = search::http_client().get(url).send().await.ok()?;
    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if !ctype.is_empty() && !ctype.contains("html") && !ctype.contains("text") {
        return None;
    }
    let body = resp.text().await.ok()?;
    // Drop the non-Send Html before the function's await points end.
    let text = {
        use scraper::{Html, Selector};
        let doc = Html::parse_document(&body);
        let sel = Selector::parse("p, li, h1, h2, h3, h4, td, blockquote").unwrap();
        doc.select(&sel)
            .map(|e| e.text().collect::<String>())
            .collect::<Vec<_>>()
            .join(" ")
    };
    Some(text)
}

/// Language-adaptive shingle size: CJK-dominant documents tokenize to one
/// token per character, so they need a longer run to carry the same evidence
/// as a Latin word shingle.
fn shingle_size(chars: &[char]) -> usize {
    let visible = chars.iter().filter(|c| !c.is_whitespace()).count();
    if visible == 0 {
        return SHINGLE_N;
    }
    let cjk = chars.iter().filter(|&&c| is_cjk(c)).count();
    if cjk * 2 > visible {
        SHINGLE_N_CJK
    } else {
        SHINGLE_N
    }
}

/// Set of word n-grams for a piece of text (used for containment).
fn ngrams(text: &str, n: usize) -> std::collections::HashSet<String> {
    let tokens = tokenize(text);
    let mut set = std::collections::HashSet::new();
    if tokens.len() < n {
        if !tokens.is_empty() {
            set.insert(tokens.join("\u{1}"));
        }
        return set;
    }
    for i in 0..=tokens.len() - n {
        set.insert(tokens[i..i + n].join("\u{1}"));
    }
    set
}

/// Headings that mark the start of a reference/bibliography section.
const REFERENCE_MARKERS: &[&str] = &[
    "参考文献",
    "引用文献",
    "参考資料",
    "引用・参考文献",
    "出典",
    "脚注",
    "references",
    "bibliography",
    "works cited",
    "reference list",
];

/// Drop everything from the first reference/bibliography heading onward. The
/// heading is matched only when it stands alone on a line (a real section head),
/// not when the phrase appears mid-sentence.
fn strip_references(text: &str) -> String {
    let mut offset = 0usize; // byte offset of current line start
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        let lower = trimmed.to_lowercase();
        let stripped: String = lower
            .chars()
            .filter(|c| {
                !c.is_whitespace()
                    && !matches!(
                        c,
                        '#' | ':' | '：' | '.' | '*' | '-' | '】' | '【' | '[' | ']'
                    )
            })
            .collect();
        if !stripped.is_empty()
            && trimmed.chars().count() <= 20
            && REFERENCE_MARKERS
                .iter()
                .any(|m| stripped == m.replace(' ', ""))
        {
            let body = text[..offset].trim_end();
            if !body.is_empty() {
                return body.to_string();
            }
        }
        offset += line.len();
    }
    text.to_string()
}

/// Winnowing fingerprint selection (Schleimer, Wilkerson & Aiken, SIGMOD 2003).
/// Hashes every QUERY_K-token window, keeps the minimum hash in each sliding
/// window of WINNOW_WINDOW, and turns each selected fingerprint back into a
/// human-readable search phrase. Capped to `budget` by even sampling.
fn winnow_queries(chars: &[char], toks: &[(String, usize, usize)], budget: usize) -> Vec<String> {
    if toks.len() < QUERY_K {
        let phrase: String = chars.iter().collect::<String>().trim().to_string();
        return if phrase.chars().filter(|c| !c.is_whitespace()).count() >= 12 {
            vec![phrase]
        } else {
            Vec::new()
        };
    }

    let hashes: Vec<u64> = (0..=toks.len() - QUERY_K)
        .map(|i| {
            let gram = toks[i..i + QUERY_K]
                .iter()
                .map(|t| t.0.as_str())
                .collect::<Vec<_>>()
                .join("\u{1}");
            hash64(&gram)
        })
        .collect();

    let mut selected: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    if hashes.len() <= WINNOW_WINDOW {
        if let Some((idx, _)) = hashes.iter().enumerate().min_by_key(|(_, h)| **h) {
            selected.insert(idx);
        }
    } else {
        for start in 0..=hashes.len() - WINNOW_WINDOW {
            let window = &hashes[start..start + WINNOW_WINDOW];
            // Rightmost minimum (standard winnowing tie-break).
            let mut min_pos = 0usize;
            for (j, h) in window.iter().enumerate() {
                if *h <= window[min_pos] {
                    min_pos = j;
                }
            }
            selected.insert(start + min_pos);
        }
    }

    let mut phrases: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for &i in &selected {
        // Expand the k-token window until the phrase is a usable query length.
        let start_char = toks[i].1;
        let mut end_tok = i + QUERY_K - 1;
        while end_tok + 1 < toks.len() && visible_between(chars, start_char, toks[end_tok].2) < 20 {
            end_tok += 1;
        }
        let end_char = toks[end_tok].2.min(chars.len());
        let phrase: String = chars[start_char..end_char]
            .iter()
            .collect::<String>()
            .trim()
            .to_string();
        if phrase.chars().filter(|c| !c.is_whitespace()).count() >= 12
            && seen.insert(phrase.clone())
        {
            phrases.push(phrase);
        }
    }

    if budget > 0 && phrases.len() > budget {
        (0..budget)
            .map(|k| phrases[k * phrases.len() / budget].clone())
            .collect()
    } else {
        phrases
    }
}

fn visible_between(chars: &[char], start: usize, end: usize) -> usize {
    let end = end.min(chars.len());
    if start >= end {
        return 0;
    }
    chars[start..end]
        .iter()
        .filter(|c| !c.is_whitespace())
        .count()
}

fn hash64(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn truncate(s: &str, n: usize) -> String {
    let t: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        format!("{t}…")
    } else {
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shingles_share_keys_for_verbatim_runs() {
        // A document window that appears verbatim in a source must produce a
        // shared shingle key, while unrelated text must not.
        let doc = "the quick brown fox jumps over the lazy dog every single morning";
        let source = "earlier that day the quick brown fox jumps over the lazy dog every single morning without fail";
        let unrelated = "a recipe for chocolate chip cookies with plenty of butter and sugar";

        let src_grams = ngrams(source, SHINGLE_N);
        let toks = tokenize_spans(doc);
        let mut shared = false;
        let mut shared_unrelated = false;
        let un_grams = ngrams(unrelated, SHINGLE_N);
        for i in 0..=toks.len() - SHINGLE_N {
            let key = toks[i..i + SHINGLE_N]
                .iter()
                .map(|t| t.0.as_str())
                .collect::<Vec<_>>()
                .join("\u{1}");
            if src_grams.contains(&key) {
                shared = true;
            }
            if un_grams.contains(&key) {
                shared_unrelated = true;
            }
        }
        assert!(shared, "verbatim run should share a shingle");
        assert!(!shared_unrelated, "unrelated text must not share a shingle");
    }

    #[test]
    fn strip_references_cuts_bibliography_section() {
        let text = "本研究では制度設計を検討した。結論として教育との併用が有効である。\n参考文献\n田中 (2020)『情報教育論』.\nSmith (2019) Journal of X.";
        let body = strip_references(text);
        assert!(body.contains("結論として"), "body must be kept");
        assert!(!body.contains("田中"), "reference entries must be dropped");
        assert!(
            !body.contains("参考文献"),
            "the heading itself must be dropped"
        );
    }

    #[test]
    fn strip_references_noop_without_section() {
        let text = "参考文献は重要であるという主張が本文中にある。しかし見出しは存在しない。";
        // The phrase appears mid-sentence, not as a heading → nothing stripped.
        assert_eq!(strip_references(text), text);
    }

    #[test]
    fn shingle_size_adapts_to_language() {
        let ja: Vec<char> = "本研究では大学生のレポートにおける生成AIの利用実態を調査した。"
            .chars()
            .collect();
        let en: Vec<char> =
            "This study surveyed how undergraduates actually use generative AI in their reports."
                .chars()
                .collect();
        assert_eq!(shingle_size(&ja), SHINGLE_N_CJK);
        assert_eq!(shingle_size(&en), SHINGLE_N);
    }

    #[test]
    fn winnow_queries_returns_phrases_from_the_text() {
        let text = "Academic integrity requires that every borrowed idea be attributed to its \
            original author with a precise citation, because scholarship advances only when \
            claims can be traced, verified, and built upon by later researchers over time.";
        let chars: Vec<char> = text.chars().collect();
        let toks = tokenize_spans(text);
        let queries = winnow_queries(&chars, &toks, 5);
        assert!(
            !queries.is_empty(),
            "should select at least one fingerprint phrase"
        );
        assert!(queries.len() <= 5, "must respect the budget");
        let lowered = text.to_lowercase();
        for q in &queries {
            // Phrases are drawn from the document (token-normalised words present).
            let first: String = q.split_whitespace().next().unwrap_or("").to_lowercase();
            assert!(
                lowered.contains(&first),
                "phrase should come from the text: {q}"
            );
        }
    }
}
