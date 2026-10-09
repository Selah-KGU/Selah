//! Generated immutable summaries shared by differential tests and timing.
use crate::live::{LiveSummaryChunk, LiveTermExplanation, SharedSummaryChunk};

pub fn summaries(count: usize, seed: usize) -> Vec<SharedSummaryChunk> {
    const TEXT: [&str; 6] = [
        "",
        "ASCII | : quoted \"text\"",
        "日本語 中文・\n引用",
        " 👩🏽‍💻 🌕 e\u{301} ",
        "\t\r\n　",
        "# heading\n- list\n末尾\n",
    ];
    (0..count)
        .map(|index| {
            LiveSummaryChunk {
                title: format!("{} {index}", TEXT[(seed + index) % TEXT.len()]),
                range_label: TEXT[(seed * 3 + index) % TEXT.len()].into(),
                body: TEXT[(seed * 7 + index) % TEXT.len()].repeat((index + seed) % 8),
                line_count: index,
                terms: (0..(seed + index) % 6)
                    .map(|term| LiveTermExplanation {
                        term: TEXT[(seed + term) % TEXT.len()].into(),
                        explanation: TEXT[(index + term) % TEXT.len()].repeat((seed + term) % 4),
                        source_excerpt: "not part of this prompt 🌕".into(),
                        external_source: TEXT[(seed + index + term) % TEXT.len()].into(),
                    })
                    .collect(),
                whiteboard: None,
            }
            .into()
        })
        .collect()
}
