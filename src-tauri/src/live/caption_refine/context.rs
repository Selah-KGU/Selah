//! Bounded terminology hints from this recording's already completed notes.
use super::super::{LiveState, SharedSummaryChunk};
use tauri::{AppHandle, Manager};

const TERM_LIMIT: usize = 24;
const TERM_CHARS: usize = 1_200;

pub(super) fn capture_terms(app: &AppHandle, session_id: &str) -> String {
    let Some(state) = app.try_state::<LiveState>() else {
        return String::new();
    };
    // Retain the immutable index only; scan/format outside the session lock.
    let summaries = state.session.lock().ok().and_then(|guard| {
        guard
            .as_ref()
            .filter(|session| session.session_id == session_id)
            .map(|session| session.summaries.clone())
    });
    summaries.map_or_else(String::new, |chunks| summary_terms(&chunks))
}

fn summary_terms(chunks: &[SharedSummaryChunk]) -> String {
    let mut terms = Vec::new();
    let mut used = 0;
    let mut add = |label: &str, detail: &str| {
        let label = super::clip(label, 48);
        if label.is_empty() || terms.iter().any(|(seen, _)| seen == &label) {
            return;
        }
        let detail = super::clip(detail, 80);
        let cost = label.chars().count() + detail.chars().count() + 3;
        if terms.len() < TERM_LIMIT && used + cost <= TERM_CHARS {
            used += cost;
            terms.push((label, detail));
        }
    };
    for chunk in chunks.iter().rev().take(3) {
        for term in &chunk.terms {
            if term.external_source.trim().is_empty() && !term.source_excerpt.trim().is_empty() {
                add(&term.term, &term.explanation);
            }
        }
    }
    if let Some(board) = chunks
        .iter()
        .rev()
        .find_map(|chunk| chunk.whiteboard.as_ref())
    {
        for node in board.nodes.iter().rev().take(TERM_LIMIT * 4) {
            if node.source_type != "external"
                && node.external_source.trim().is_empty()
                && !node.source_excerpt.trim().is_empty()
            {
                add(&node.label, &node.detail);
            }
        }
    }
    terms
        .into_iter()
        .map(|(label, detail)| format!("{label}: {detail}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::{LiveSummaryChunk, LiveTermExplanation, LiveWhiteboard, LiveWhiteboardNode};
    use std::sync::Arc;

    #[test]
    fn only_recording_grounded_terms_are_hints_and_newest_definition_wins() {
        let chunk = |definition: &str| {
            Arc::new(LiveSummaryChunk {
                title: "統計学".into(),
                range_label: String::new(),
                body: String::new(),
                line_count: 3,
                terms: vec![
                    LiveTermExplanation {
                        term: "最尤推定".into(),
                        explanation: definition.into(),
                        source_excerpt: "尤度を最大にします".into(),
                        external_source: String::new(),
                    },
                    LiveTermExplanation {
                        term: "外部の用語".into(),
                        explanation: "録音外".into(),
                        source_excerpt: "引用".into(),
                        external_source: "https://example.com".into(),
                    },
                ],
                whiteboard: Some(Arc::new(LiveWhiteboard {
                    title: String::new(),
                    layout: String::new(),
                    nodes: vec![
                        node("ベイズ推定", "lecture", "事前分布を使う"),
                        node("録音外の背景", "external", "外部情報"),
                        node("根拠のない候補", "lecture", ""),
                    ],
                    edges: Vec::new(),
                    schema_version: 1,
                    normalized_by: "backend".into(),
                })),
            })
        };
        let hints = summary_terms(&[chunk("古い定義"), chunk("尤度を最大にする推定")]);
        assert!(hints.contains("最尤推定: 尤度を最大にする推定"));
        assert!(hints.contains("ベイズ推定"));
        assert!(!hints.contains("古い定義"));
        assert!(!hints.contains("外部"));
        assert!(!hints.contains("根拠のない候補"));
    }

    fn node(label: &str, source_type: &str, excerpt: &str) -> LiveWhiteboardNode {
        LiveWhiteboardNode {
            id: label.into(),
            label: label.into(),
            detail: "説明".into(),
            node_type: "term".into(),
            kind: "support".into(),
            role: "branch".into(),
            parent_id: String::new(),
            source_type: source_type.into(),
            source_excerpt: excerpt.into(),
            external_source: String::new(),
        }
    }

    #[test]
    fn large_boards_cannot_expand_the_caption_request_without_bound() {
        let chunk = Arc::new(LiveSummaryChunk {
            title: String::new(),
            range_label: String::new(),
            body: String::new(),
            line_count: 1,
            terms: Vec::new(),
            whiteboard: Some(Arc::new(LiveWhiteboard {
                title: String::new(),
                layout: String::new(),
                nodes: (0..100)
                    .map(|i| {
                        let mut entry = node(&format!("用語{i}"), "lecture", "発話内");
                        entry.detail = "説明".repeat(100);
                        entry
                    })
                    .collect(),
                edges: Vec::new(),
                schema_version: 1,
                normalized_by: "backend".into(),
            })),
        });
        let hints = summary_terms(&[chunk]);
        assert!(hints.starts_with("用語99:"));
        assert!(hints.chars().count() <= TERM_CHARS);
        assert!(hints.lines().count() <= TERM_LIMIT);
    }
}
