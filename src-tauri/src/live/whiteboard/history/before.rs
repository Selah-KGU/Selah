// Frozen pre-change formatter, used only by tests and the isolated benchmark.
use super::WhiteboardContext;
use crate::live::{LiveTermExplanation, SharedSummaryChunk};

pub fn format_recent_summary_context(summaries: &[SharedSummaryChunk], limit: usize) -> String {
    if summaries.is_empty() || limit == 0 {
        return "なし".to_string();
    }

    summaries[summaries.len().saturating_sub(limit)..]
        .iter()
        .map(|chunk| format!("## {}\n{}\n{}", chunk.title, chunk.range_label, chunk.body))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Emit the full prior-chunk history (summary bodies + term explanations) so
/// the whiteboard-only call can build the cumulative board from the already
/// distilled record instead of re-parsing every raw transcript. Used as the
/// auxiliary "前面的所有总结和词条" context for the whiteboard call.

pub fn format_full_history_for_whiteboard(summaries: &[SharedSummaryChunk]) -> String {
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

pub fn format_current_chunk_for_whiteboard(
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

pub fn build(context: &WhiteboardContext<'_>) -> String {
    // Adapt only the input type: the frozen baseline receives already-formatted
    // transcript text, exactly as it did before the borrowed-piece interface.
    let transcript = match context.transcript {
        crate::live::ContextPart::Text(text) => text,
        _ => panic!("frozen history fixture requires already-formatted transcript"),
    };
    let full_history = format_full_history_for_whiteboard(context.summaries);
    let current_chunk_brief =
        format_current_chunk_for_whiteboard(context.body, context.terms, context.range);
    format!(
                "{}\n\nこれまでの全分割要約と用語注釈（累積素材）:\n{}\n\n現在の累積知識整理ボード:\n{}\n\n今回新しく生成された区間の要約と用語:\n{}\n\n今回の文字起こし（補助参考、必要に応じて細部を拾う。長すぎる場合は末尾のみ表示）:\n{}\n\n指示: system の実行順序と構造パターン庫に従い、録音開始から現在までの累積 whiteboard JSON を返す。既出情報を失わず、必要なら既存ノードを更新・移動・アップグレード・統合・分割する。新しい具体材料は追加する。最後に parent_id、edge、term、混在タイプ分離をセルフチェックする。",
                context.course,
                full_history,
                context.latest_board,
                current_chunk_brief,
                transcript,
            )
}
