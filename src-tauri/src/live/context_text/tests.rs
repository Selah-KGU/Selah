use super::*;
use crate::live::{LiveSummaryChunk, LiveTranscriptLine};

#[test]
fn mixed_borrowed_parts_preserve_empty_unicode_whitespace_and_elision_bytes() {
    let lines: Vec<SharedTranscriptLine> = ["", "\t\r\n　", "日本語 中文 👩🏽‍💻 e\u{301}", "| [ ]\n"]
        .into_iter()
        .map(|text| {
            LiveTranscriptLine {
                text: text.into(),
                at: " \n時刻 | ".into(),
            }
            .into()
        })
        .collect();
    let chunks: Vec<SharedSummaryChunk> = ["", "\t\r\n　", "🌕 中文"]
        .into_iter()
        .map(|body| {
            LiveSummaryChunk {
                title: "題\n".into(),
                range_label: " 範囲 ".into(),
                body: body.into(),
                line_count: 0,
                terms: vec![],
                whiteboard: None,
            }
            .into()
        })
        .collect();
    for line_count in 0..=lines.len() {
        for chunk_count in 0..=chunks.len() {
            for elided in [0, 1, 9, 10, 99, 100, usize::MAX] {
                let line_slice = &lines[..line_count];
                let chunk_slice = &chunks[..chunk_count];
                let transcript = line_slice
                    .iter()
                    .map(|line| format!("- [{}] {}", line.at, line.text))
                    .collect::<Vec<_>>()
                    .join("\n");
                let previous_summaries = chunk_slice
                    .iter()
                    .map(|chunk| {
                        format!("## {}\n{}\n{}", chunk.title, chunk.range_label, chunk.body)
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                let notice = if elided == 0 {
                    String::new()
                } else {
                    format!("(... 古い文字起こし {elided} 行を省略 ...)\n")
                };
                let parts = [
                    ContextPart::Text("head\n"),
                    ContextPart::Transcript {
                        lines: line_slice,
                        elided,
                    },
                    ContextPart::Text("\nseparator\n"),
                    ContextPart::Summaries(chunk_slice),
                    ContextPart::Text("\nend 🌕"),
                    ContextPart::Integer(i32::MIN),
                    ContextPart::Text(" | "),
                    ContextPart::Integer(i32::MAX),
                ];
                let result = build_context_text(&parts);
                assert_eq!(
                    result,
                    format!("head\n{notice}{transcript}\nseparator\n{previous_summaries}\nend 🌕{} | {}", i32::MIN, i32::MAX)
                );
                assert_eq!(result.len(), result.capacity());
            }
        }
    }
}

#[test]
fn empty_parts_have_no_implicit_defaults_and_append_preserves_existing_prefix() {
    for value in [i32::MIN, -100, -10, -1, 0, 1, 9, 10, 99, 100, i32::MAX] {
        let number = build_context_text(&[ContextPart::Integer(value)]);
        assert_eq!(number, value.to_string());
        assert_eq!(number.capacity(), number.len());
    }
    assert_eq!(build_context_text(&[]), "");
    assert_eq!(
        build_context_text(&[
            ContextPart::Transcript {
                lines: &[],
                elided: 0
            },
            ContextPart::Summaries(&[])
        ]),
        ""
    );
    let mut existing = "prefix\r\n".to_owned();
    let part = ContextPart::Transcript {
        lines: &[],
        elided: 100,
    };
    part.append_to(&mut existing);
    assert_eq!(
        existing,
        "prefix\r\n(... 古い文字起こし 100 行を省略 ...)\n"
    );
    assert_eq!(part.byte_len(), existing.len() - "prefix\r\n".len());
}
