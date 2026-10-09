//! Borrowed summary/term context, written directly into the final model input.
#[cfg(test)]
use crate::live::build_context_text;
use crate::live::{ContextPart, LiveTermExplanation, SharedSummaryChunk};
use std::fmt::Write;

const NONE: &str = "なし";
const TERMS_DETAIL_RECENT: usize = 4;
const HISTORY_HEADER: &str = "\n\nこれまでの全分割要約と用語注釈（累積素材）:\n";
const BOARD_HEADER: &str = "\n\n現在の累積知識整理ボード:\n";
const CURRENT_HEADER: &str = "\n\n今回新しく生成された区間の要約と用語:\n";
const TRANSCRIPT_HEADER: &str =
    "\n\n今回の文字起こし（補助参考、必要に応じて細部を拾う。長すぎる場合は末尾のみ表示）:\n";
const INSTRUCTION: &str = "\n\n指示: system の実行順序と構造パターン庫に従い、録音開始から現在までの累積 whiteboard JSON を返す。既出情報を失わず、必要なら既存ノードを更新・移動・アップグレード・統合・分割する。新しい具体材料は追加する。最後に parent_id、edge、term、混在タイプ分離をセルフチェックする。";

// All size calculations use UTF-8 bytes, including separators. They only read
// string lengths, not their text. No output limit or history cache is introduced.
fn term_details_bytes(terms: &[LiveTermExplanation]) -> usize {
    terms
        .iter()
        .map(|term| {
            "- ".len()
                + term.term.len()
                + ": ".len()
                + term.explanation.len()
                + 1
                + if term.external_source.is_empty() {
                    0
                } else {
                    "（出典: ".len() + term.external_source.len() + "）".len()
                }
        })
        .sum()
}

fn append_term_details(out: &mut String, terms: &[LiveTermExplanation]) {
    for term in terms {
        out.push_str("- ");
        out.push_str(&term.term);
        out.push_str(": ");
        out.push_str(&term.explanation);
        if !term.external_source.is_empty() {
            out.push_str("（出典: ");
            out.push_str(&term.external_source);
            out.push('）');
        }
        out.push('\n');
    }
}

fn full_history_bytes(summaries: &[SharedSummaryChunk]) -> usize {
    if summaries.is_empty() {
        return NONE.len();
    }
    let cutoff = summaries.len().saturating_sub(TERMS_DETAIL_RECENT);
    summaries
        .iter()
        .enumerate()
        .map(|(index, chunk)| {
            usize::from(index > 0) * 2
                + "## Chunk ".len()
                + ((index + 1).ilog10() as usize + 1).max(2)
                + " | ".len()
                + chunk.range_label.len()
                + "\n題: ".len()
                + chunk.title.len()
                + 1
                + chunk.body.len()
                + if chunk.terms.is_empty() {
                    0
                } else if index < cutoff {
                    "\n用語: ".len()
                        + chunk
                            .terms
                            .iter()
                            .map(|term| term.term.len())
                            .sum::<usize>()
                        + "、".len() * (chunk.terms.len() - 1)
                } else {
                    "\n用語:\n".len() + term_details_bytes(&chunk.terms)
                }
        })
        .sum()
}

fn append_full_history(out: &mut String, summaries: &[SharedSummaryChunk]) {
    if summaries.is_empty() {
        out.push_str(NONE);
        return;
    }
    let cutoff = summaries.len().saturating_sub(TERMS_DETAIL_RECENT);
    for (index, chunk) in summaries.iter().enumerate() {
        if index > 0 {
            out.push_str("\n\n");
        }
        let _ = write!(out, "## Chunk {:02} | ", index + 1);
        out.push_str(&chunk.range_label);
        out.push_str("\n題: ");
        out.push_str(&chunk.title);
        out.push('\n');
        out.push_str(&chunk.body);
        if chunk.terms.is_empty() {
            continue;
        }
        if index < cutoff {
            out.push_str("\n用語: ");
            for (term_index, term) in chunk.terms.iter().enumerate() {
                if term_index > 0 {
                    out.push('、');
                }
                out.push_str(&term.term);
            }
        } else {
            out.push_str("\n用語:\n");
            append_term_details(out, &chunk.terms);
        }
    }
}

fn current_chunk_bytes(body: &str, terms: &[LiveTermExplanation], range_label: &str) -> usize {
    "範囲: ".len()
        + range_label.len()
        + "\n要約:\n".len()
        + body.len()
        + if terms.is_empty() {
            0
        } else {
            "\n用語:\n".len() + term_details_bytes(terms)
        }
}

fn append_current_chunk(
    out: &mut String,
    body: &str,
    terms: &[LiveTermExplanation],
    range_label: &str,
) {
    out.push_str("範囲: ");
    out.push_str(range_label);
    out.push_str("\n要約:\n");
    out.push_str(body);
    if !terms.is_empty() {
        out.push_str("\n用語:\n");
        append_term_details(out, terms);
    }
}

#[cfg(test)]
pub fn format_recent_summary_context(summaries: &[SharedSummaryChunk], limit: usize) -> String {
    if summaries.is_empty() || limit == 0 {
        return NONE.to_owned();
    }
    build_context_text(&[ContextPart::Summaries(
        &summaries[summaries.len().saturating_sub(limit)..],
    )])
}

/// All borrowed components of Call 2's user message. Shared Arc records stay
/// immutable; only the final message String owns the assembled history.
pub struct WhiteboardContext<'a> {
    pub course: &'a str,
    pub summaries: &'a [SharedSummaryChunk],
    pub latest_board: &'a str,
    pub body: &'a str,
    pub terms: &'a [LiveTermExplanation],
    pub range: &'a str,
    pub transcript: ContextPart<'a>,
}

impl WhiteboardContext<'_> {
    pub fn build(&self) -> String {
        let capacity = self.course.len()
            + HISTORY_HEADER.len()
            + full_history_bytes(self.summaries)
            + BOARD_HEADER.len()
            + self.latest_board.len()
            + CURRENT_HEADER.len()
            + current_chunk_bytes(self.body, self.terms, self.range)
            + TRANSCRIPT_HEADER.len()
            + self.transcript.byte_len()
            + INSTRUCTION.len();
        let mut out = String::with_capacity(capacity);
        out.push_str(self.course);
        out.push_str(HISTORY_HEADER);
        append_full_history(&mut out, self.summaries);
        out.push_str(BOARD_HEADER);
        out.push_str(self.latest_board);
        out.push_str(CURRENT_HEADER);
        append_current_chunk(&mut out, self.body, self.terms, self.range);
        out.push_str(TRANSCRIPT_HEADER);
        self.transcript.append_to(&mut out);
        out.push_str(INSTRUCTION);
        out
    }
}

// Existing differential request tests use these adapters for the old call
// shape. Production writes both components directly into the final message.
#[cfg(test)]
pub fn format_full_history_for_whiteboard(summaries: &[SharedSummaryChunk]) -> String {
    let mut out = String::with_capacity(full_history_bytes(summaries));
    append_full_history(&mut out, summaries);
    out
}

#[cfg(test)]
pub fn format_current_chunk_for_whiteboard(
    body: &str,
    terms: &[LiveTermExplanation],
    range: &str,
) -> String {
    let mut out = String::with_capacity(current_chunk_bytes(body, terms, range));
    append_current_chunk(&mut out, body, terms, range);
    out
}

#[cfg(test)]
#[path = "history/tests.rs"]
mod tests;
