//! Borrowed pieces of model input. Reserve once and copy text into the final
//! owned message, without intermediate transcript or summary Strings.
use super::{SharedSummaryChunk, SharedTranscriptLine};
use std::fmt::Write;

#[derive(Clone, Copy)]
pub enum ContextPart<'a> {
    Text(&'a str),
    Integer(i32),
    Transcript {
        lines: &'a [SharedTranscriptLine],
        elided: usize,
    },
    Summaries(&'a [SharedSummaryChunk]),
}

const ELIDED_PREFIX: &str = "(... 古い文字起こし ";
const ELIDED_SUFFIX: &str = " 行を省略 ...)\n";

impl ContextPart<'_> {
    pub fn byte_len(&self) -> usize {
        match self {
            Self::Text(text) => text.len(),
            Self::Integer(value) => {
                usize::from(*value < 0)
                    + value
                        .unsigned_abs()
                        .checked_ilog10()
                        .map_or(1, |digits| digits as usize + 1)
            }
            Self::Transcript { lines, elided } => {
                lines
                    .iter()
                    .map(|line| "- [".len() + line.at.len() + "] ".len() + line.text.len())
                    .sum::<usize>()
                    + lines.len().saturating_sub(1)
                    + if *elided == 0 {
                        0
                    } else {
                        ELIDED_PREFIX.len() + elided.ilog10() as usize + 1 + ELIDED_SUFFIX.len()
                    }
            }
            Self::Summaries(chunks) => {
                chunks
                    .iter()
                    .map(|chunk| {
                        "## ".len()
                            + chunk.title.len()
                            + 1
                            + chunk.range_label.len()
                            + 1
                            + chunk.body.len()
                    })
                    .sum::<usize>()
                    + chunks.len().saturating_sub(1) * 2
            }
        }
    }

    pub fn append_to(&self, out: &mut String) {
        match self {
            Self::Text(text) => out.push_str(text),
            Self::Integer(value) => {
                let _ = write!(out, "{value}");
            }
            Self::Transcript { lines, elided } => {
                if *elided > 0 {
                    out.push_str(ELIDED_PREFIX);
                    let _ = write!(out, "{elided}");
                    out.push_str(ELIDED_SUFFIX);
                }
                for (index, line) in lines.iter().enumerate() {
                    if index > 0 {
                        out.push('\n');
                    }
                    out.push_str("- [");
                    out.push_str(&line.at);
                    out.push_str("] ");
                    out.push_str(&line.text);
                }
            }
            Self::Summaries(chunks) => {
                for (index, chunk) in chunks.iter().enumerate() {
                    if index > 0 {
                        out.push_str("\n\n");
                    }
                    out.push_str("## ");
                    out.push_str(&chunk.title);
                    out.push('\n');
                    out.push_str(&chunk.range_label);
                    out.push('\n');
                    out.push_str(&chunk.body);
                }
            }
        }
    }
}

pub fn build_context_text(parts: &[ContextPart<'_>]) -> String {
    let mut out = String::with_capacity(parts.iter().map(ContextPart::byte_len).sum());
    for part in parts {
        part.append_to(&mut out);
    }
    out
}

#[cfg(test)]
#[path = "context_text/tests.rs"]
mod tests;
