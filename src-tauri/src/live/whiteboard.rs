use super::{
    clamp_chars, LiveSummaryChunk, LiveTermExplanation, LiveTranscriptLine, LiveWhiteboard,
    LiveWhiteboardNode,
};

pub(in crate::live) fn format_recent_summary_context(
    summaries: &[LiveSummaryChunk],
    limit: usize,
) -> String {
    if summaries.is_empty() || limit == 0 {
        return "なし".to_string();
    }

    summaries
        .iter()
        .rev()
        .take(limit)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|chunk| format!("## {}\n{}\n{}", chunk.title, chunk.range_label, chunk.body))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Emit the full prior-chunk history (summary bodies + term explanations) so
/// the whiteboard-only call can build the cumulative board from the already
/// distilled record instead of re-parsing every raw transcript. Used as the
/// auxiliary "前面的所有总结和词条" context for the whiteboard call.

pub(in crate::live) fn format_full_history_for_whiteboard(
    summaries: &[LiveSummaryChunk],
) -> String {
    if summaries.is_empty() {
        return "なし".to_string();
    }
    // Every chunk keeps its segment summary (body) so coverage/ordering and the
    // running narrative stay intact. Only the heavier per-term notes are trimmed
    // for older chunks — those terms are already present as nodes in the
    // cumulative board JSON passed alongside, so re-sending their explanations
    // each time just grows the prompt linearly over a long session.
    const TERMS_DETAIL_RECENT: usize = 4;
    let cutoff = summaries.len().saturating_sub(TERMS_DETAIL_RECENT);
    let mut out = String::new();
    for (idx, chunk) in summaries.iter().enumerate() {
        if idx > 0 {
            out.push_str("\n\n");
        }
        out.push_str(&format!(
            "## Chunk {:02} | {}\n題: {}\n{}",
            idx + 1,
            chunk.range_label,
            chunk.title,
            chunk.body
        ));
        if !chunk.terms.is_empty() {
            if idx < cutoff {
                // Older chunk: list term names only (explanations live on the board).
                let names = chunk
                    .terms
                    .iter()
                    .map(|term| term.term.as_str())
                    .collect::<Vec<_>>()
                    .join("、");
                out.push_str(&format!("\n用語: {}", names));
            } else {
                out.push_str("\n用語:\n");
                for term in &chunk.terms {
                    out.push_str(&format!("- {}: {}", term.term, term.explanation));
                    if !term.external_source.is_empty() {
                        out.push_str(&format!("（出典: {}）", term.external_source));
                    }
                    out.push('\n');
                }
            }
        }
    }
    out
}

/// Emit the just-generated current-chunk summary + terms in the same shape as
/// the historical entries. Fed to the whiteboard call so it knows what this
/// segment introduced.

pub(in crate::live) fn format_current_chunk_for_whiteboard(
    body: &str,
    terms: &[LiveTermExplanation],
    range_label: &str,
) -> String {
    let mut out = format!("範囲: {}\n要約:\n{}", range_label, body);
    if !terms.is_empty() {
        out.push_str("\n用語:\n");
        for term in terms {
            out.push_str(&format!("- {}: {}", term.term, term.explanation));
            if !term.external_source.is_empty() {
                out.push_str(&format!("（出典: {}）", term.external_source));
            }
            out.push('\n');
        }
    }
    out
}

fn normalized_excerpt_match_text(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase()
}

fn whiteboard_excerpt_terms(node: &LiveWhiteboardNode) -> Vec<String> {
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

fn best_transcript_excerpt_for_node(
    node: &LiveWhiteboardNode,
    lines: &[LiveTranscriptLine],
) -> Option<String> {
    let terms = whiteboard_excerpt_terms(node);
    if terms.is_empty() {
        return None;
    }

    lines
        .iter()
        .filter_map(|line| {
            let normalized = normalized_excerpt_match_text(&line.text);
            if normalized.is_empty() {
                return None;
            }
            let score = terms
                .iter()
                .filter(|term| normalized.contains(term.as_str()))
                .map(|term| term.chars().count())
                .sum::<usize>();
            if score == 0 {
                None
            } else {
                Some((score, line.text.as_str()))
            }
        })
        .max_by_key(|(score, text)| (*score, text.chars().count()))
        .map(|(_, text)| clamp_chars(text, 80))
}

pub(in crate::live) fn enrich_whiteboard_source_excerpts(
    mut board: Option<LiveWhiteboard>,
    previous: Option<&LiveWhiteboard>,
    terms: &[LiveTermExplanation],
    lines: &[LiveTranscriptLine],
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
        if let Some(excerpt) = best_transcript_excerpt_for_node(node, lines) {
            node.source_excerpt = excerpt;
        }
    }

    board
}
