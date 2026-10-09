//! One-update borrowed speech matching and source inheritance.
use super::{LiveTermExplanation, LiveWhiteboard, LiveWhiteboardNode, SharedTranscriptLine};
use std::borrow::Cow;
use std::collections::HashMap;

/// Count original scalars while locating normalization changes. Already
/// normalized text remains borrowed; changed text has one exact-size buffer.
fn normalized_excerpt_match_text(value: &str) -> (Cow<'_, str>, usize) {
    let mut original_chars = 0;
    let mut normalized_bytes = 0;
    let mut changed = false;
    for ch in value.chars() {
        original_chars += 1;
        if ch.is_whitespace() {
            changed = true;
        } else {
            normalized_bytes += ch.len_utf8();
            changed |= ch.is_ascii_uppercase();
        }
    }
    let normalized = if changed {
        let mut output = String::with_capacity(normalized_bytes);
        for ch in value.chars().filter(|ch| !ch.is_whitespace()) {
            output.push(ch.to_ascii_lowercase());
        }
        Cow::Owned(output)
    } else {
        Cow::Borrowed(value)
    };
    (normalized, original_chars)
}

struct ExcerptTerm<'a> {
    text: Cow<'a, str>,
    char_count: usize,
}

const MAX_EXCERPT_TERMS: usize = 8;

fn whiteboard_excerpt_terms(node: &LiveWhiteboardNode) -> Vec<ExcerptTerm<'_>> {
    let mut terms = Vec::new();
    for source in [&node.label, &node.detail] {
        for part in source.split(|ch: char| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    'の' | 'と'
                        | 'や'
                        | '・'
                        | '、'
                        | '。'
                        | '，'
                        | ','
                        | '.'
                        | ':'
                        | '：'
                        | ';'
                        | '；'
                        | '('
                        | ')'
                        | '（'
                        | '）'
                        | '['
                        | ']'
                        | '【'
                        | '】'
                        | '/'
                        | '／'
                        | '-'
                        | '_'
                        | '+'
                        | '＋'
                        | '='
                )
        }) {
            // split removed all whitespace; ASCII case conversion preserves
            // this scalar count. No recount is needed for sorting or scoring.
            let (text, char_count) = normalized_excerpt_match_text(part);
            if char_count >= 3 || (char_count >= 2 && text.is_ascii()) {
                terms.push(ExcerptTerm { text, char_count });
            }
        }
    }
    terms.sort_by_key(|term| std::cmp::Reverse(term.char_count));
    // Keep stable length ordering and adjacent-only deduplication, including
    // equal-length legacy terms that appear again after another term.
    terms.dedup_by(|a, b| a.text == b.text);
    terms.truncate(MAX_EXCERPT_TERMS);
    terms
}

fn source_excerpt(text: &str) -> String {
    let trimmed = text.trim();
    let end = trimmed.char_indices().nth(80).map(|(end, _)| end);
    let prefix = &trimmed[..end.unwrap_or(trimmed.len())];
    let mut output =
        String::with_capacity(prefix.len() + usize::from(end.is_some()) * '…'.len_utf8());
    output.push_str(prefix);
    if end.is_some() {
        output.push('…');
    }
    output
}

struct ExcerptLine<'a> {
    text: &'a str,
    normalized: Cow<'a, str>,
    char_count: usize,
}

/// Local to one board update. Supplied, inherited and term excerpts do not
/// build a transcript index. No normalized speech survives this invocation.
struct TranscriptExcerpts<'a> {
    lines: Vec<ExcerptLine<'a>>,
}

impl<'a> TranscriptExcerpts<'a> {
    fn new(lines: &'a [SharedTranscriptLine]) -> Self {
        Self {
            lines: lines
                .iter()
                .map(|line| {
                    let (normalized, char_count) = normalized_excerpt_match_text(&line.text);
                    ExcerptLine {
                        text: &line.text,
                        normalized,
                        char_count,
                    }
                })
                .collect(),
        }
    }

    fn best(&self, terms: Vec<ExcerptTerm<'_>>) -> Option<String> {
        // Resolve borrowed/owned text once per node, rather than in every
        // candidate-line/term comparison. The existing eight-term limit fits
        // this small stack array without another scoring Vec allocation.
        let mut weighted = [("", 0); MAX_EXCERPT_TERMS];
        for (slot, term) in weighted.iter_mut().zip(&terms) {
            *slot = (term.text.as_ref(), term.char_count);
        }
        let weighted = &weighted[..terms.len()];
        self.lines
            .iter()
            .filter_map(|line| {
                let normalized = line.normalized.as_ref();
                if normalized.is_empty() {
                    return None;
                }
                let score = weighted
                    .iter()
                    .filter(|(term, _)| normalized.contains(*term))
                    .map(|(_, count)| count)
                    .sum::<usize>();
                (score > 0).then_some((score, line))
            })
            .max_by_key(|(score, line)| (*score, line.char_count))
            .map(|(_, line)| source_excerpt(line.text))
    }
}

struct PreviousExcerpts<'a> {
    by_id: HashMap<&'a str, &'a str>,
    by_label: HashMap<&'a str, &'a str>,
}
impl<'a> PreviousExcerpts<'a> {
    fn new(previous: &'a LiveWhiteboard) -> Self {
        let mut by_id = HashMap::new();
        let mut by_label = HashMap::new();
        for node in &previous.nodes {
            if !node.source_excerpt.trim().is_empty() {
                // Later nonempty duplicate IDs/labels remain authoritative.
                by_id.insert(node.id.as_str(), node.source_excerpt.as_str());
                by_label.insert(node.label.as_str(), node.source_excerpt.as_str());
            }
        }
        Self { by_id, by_label }
    }
    fn find(&self, node: &LiveWhiteboardNode) -> Option<&str> {
        self.by_id
            .get(node.id.as_str())
            .or_else(|| self.by_label.get(node.label.as_str()))
            .copied()
    }
}

pub fn enrich_whiteboard_source_excerpts(
    mut board: Option<LiveWhiteboard>,
    previous: Option<&LiveWhiteboard>,
    terms: &[LiveTermExplanation],
    lines: &[SharedTranscriptLine],
) -> Option<LiveWhiteboard> {
    let whiteboard = board.as_mut()?;
    let mut inherited = None;
    let mut excerpts = None;
    for node in &mut whiteboard.nodes {
        if node.source_type != "lecture" || !node.source_excerpt.trim().is_empty() {
            continue;
        }
        if let Some(previous) = previous {
            if let Some(excerpt) = inherited
                .get_or_insert_with(|| PreviousExcerpts::new(previous))
                .find(node)
            {
                node.source_excerpt = source_excerpt(excerpt);
                continue;
            }
        }
        if node.node_type == "term" {
            if let Some(term) = terms.iter().find(|term| {
                !term.source_excerpt.trim().is_empty()
                    && (term.term == node.label
                        || node.label.contains(term.term.as_str())
                        || term.term.contains(node.label.as_str()))
            }) {
                node.source_excerpt = source_excerpt(&term.source_excerpt);
                continue;
            }
        }
        let match_terms = whiteboard_excerpt_terms(node);
        if match_terms.is_empty() {
            continue;
        }
        if let Some(excerpt) = excerpts
            .get_or_insert_with(|| TranscriptExcerpts::new(lines))
            .best(match_terms)
        {
            node.source_excerpt = excerpt;
        }
    }
    board
}

#[cfg(test)]
#[path = "excerpts_tests.rs"]
mod legacy_tests;
#[cfg(test)]
#[path = "excerpts/tests.rs"]
mod tests;
