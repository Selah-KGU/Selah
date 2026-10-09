//! Frozen immediate predecessor for differential tests and isolated benchmarks.
use super::{LiveTermExplanation, LiveWhiteboard, LiveWhiteboardNode, SharedTranscriptLine};

pub(super) fn clamp_chars(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out = trimmed.chars().take(max_chars).collect::<String>();
    out.push('…');
    out
}

pub(super) fn normalized_excerpt_match_text(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase()
}

pub(super) fn whiteboard_excerpt_terms(node: &LiveWhiteboardNode) -> Vec<String> {
    let mut terms = Vec::new();
    for source in [&node.label, &node.detail] {
        for part in source.split(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
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
            let normalized = normalized_excerpt_match_text(part);
            let char_count = normalized.chars().count();
            if char_count >= 3 || (char_count >= 2 && normalized.is_ascii()) {
                terms.push(normalized);
            }
        }
    }
    terms.sort_by_key(|term| std::cmp::Reverse(term.chars().count()));
    terms.dedup();
    terms.truncate(8);
    terms
}

struct ExcerptLine<'a> {
    text: &'a str,
    normalized: String,
    char_count: usize,
}

/// Local to one board update, borrowing the accepted immutable speech. Build
/// lazily: supplied, inherited and term excerpts need no transcript scan.
struct TranscriptExcerpts<'a> {
    lines: Vec<ExcerptLine<'a>>,
}

impl<'a> TranscriptExcerpts<'a> {
    fn new(lines: &'a [SharedTranscriptLine]) -> Self {
        Self {
            lines: lines
                .iter()
                .map(|line| ExcerptLine {
                    text: &line.text,
                    normalized: normalized_excerpt_match_text(&line.text),
                    char_count: line.text.chars().count(),
                })
                .collect(),
        }
    }

    fn best(&self, terms: Vec<String>) -> Option<String> {
        let terms: Vec<_> = terms
            .iter()
            .map(|term| (term.as_str(), term.chars().count()))
            .collect();
        self.lines
            .iter()
            .filter_map(|line| {
                if line.normalized.is_empty() {
                    return None;
                }
                let score = terms
                    .iter()
                    .filter(|(term, _)| line.normalized.contains(*term))
                    .map(|(_, count)| count)
                    .sum::<usize>();
                if score == 0 {
                    None
                } else {
                    Some((score, line))
                }
            })
            .max_by_key(|(score, line)| (*score, line.char_count))
            .map(|(_, line)| clamp_chars(line.text, 80))
    }
}

pub(super) fn enrich_whiteboard_source_excerpts(
    mut board: Option<LiveWhiteboard>,
    previous: Option<&LiveWhiteboard>,
    terms: &[LiveTermExplanation],
    lines: &[SharedTranscriptLine],
) -> Option<LiveWhiteboard> {
    let whiteboard = board.as_mut()?;
    let previous_by_id = previous
        .map(|prev| {
            prev.nodes
                .iter()
                .filter(|node| !node.source_excerpt.trim().is_empty())
                .map(|node| (node.id.as_str(), node.source_excerpt.as_str()))
                .collect::<std::collections::HashMap<_, _>>()
        })
        .unwrap_or_default();
    let previous_by_label = previous
        .map(|prev| {
            prev.nodes
                .iter()
                .filter(|node| !node.source_excerpt.trim().is_empty())
                .map(|node| (node.label.as_str(), node.source_excerpt.as_str()))
                .collect::<std::collections::HashMap<_, _>>()
        })
        .unwrap_or_default();

    let mut excerpts = None;
    for node in &mut whiteboard.nodes {
        if node.source_type != "lecture" || !node.source_excerpt.trim().is_empty() {
            continue;
        }
        if let Some(excerpt) = previous_by_id
            .get(node.id.as_str())
            .or_else(|| previous_by_label.get(node.label.as_str()))
        {
            node.source_excerpt = clamp_chars(excerpt, 80);
            continue;
        }
        if node.node_type == "term" {
            if let Some(term) = terms.iter().find(|term| {
                !term.source_excerpt.trim().is_empty()
                    && (term.term == node.label
                        || node.label.contains(term.term.as_str())
                        || term.term.contains(node.label.as_str()))
            }) {
                node.source_excerpt = clamp_chars(&term.source_excerpt, 80);
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
