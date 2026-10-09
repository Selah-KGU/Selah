use super::*;
use crate::live::LiveSummaryChunk;

#[path = "before.rs"]
mod before;
#[path = "fixtures.rs"]
mod fixtures;

fn context<'a>(
    summaries: &'a [SharedSummaryChunk],
    terms: &'a [LiveTermExplanation],
) -> WhiteboardContext<'a> {
    WhiteboardContext {
        course: "講義・自由記録 | 🌕\n全部保持",
        summaries,
        latest_board: "## 知識整理\n主題（用語）\n",
        body: "\t# 今回の本文\r\n 👩🏽‍💻 e\u{301} \n",
        terms,
        range: " 10:00 → 10:10\n ",
        transcript: ContextPart::Text("- [10:00] 発話の全文\n"),
    }
}

#[test]
fn borrowed_history_and_complete_prompt_match_frozen_text_for_864_inputs() {
    for seed in 0..96 {
        for count in [0, 1, 2, 4, 5, 7, 12, 99, 100] {
            let chunks = fixtures::summaries(count, seed);
            let original = serde_json::to_vec(&chunks).unwrap();
            let terms = chunks
                .last()
                .map(|chunk| chunk.terms.as_slice())
                .unwrap_or(&[]);
            let full = format_full_history_for_whiteboard(&chunks);
            assert_eq!(
                full.as_bytes(),
                before::format_full_history_for_whiteboard(&chunks).as_bytes()
            );
            assert_eq!(full.capacity(), full.len());
            for limit in [0, 1, 2, 4, 100, usize::MAX] {
                let recent = format_recent_summary_context(&chunks, limit);
                assert_eq!(
                    recent.as_bytes(),
                    before::format_recent_summary_context(&chunks, limit).as_bytes()
                );
                assert_eq!(recent.capacity(), recent.len());
            }
            let context = context(&chunks, terms);
            let brief =
                format_current_chunk_for_whiteboard(context.body, context.terms, context.range);
            assert_eq!(
                brief.as_bytes(),
                before::format_current_chunk_for_whiteboard(
                    context.body,
                    context.terms,
                    context.range
                )
                .as_bytes()
            );
            assert_eq!(brief.capacity(), brief.len());
            let message = context.build();
            assert_eq!(
                message.as_bytes(),
                before::build(&context).as_bytes(),
                "{seed}/{count}"
            );
            assert_eq!(message.capacity(), message.len());
            assert_eq!(serde_json::to_vec(&chunks).unwrap(), original);
        }
    }
}

#[test]
fn fixed_history_preserves_four_recent_term_details_and_every_summary_body() {
    let chunks: Vec<SharedSummaryChunk> = (0..6)
        .map(|index| {
            LiveSummaryChunk {
                title: format!("題{index}"),
                range_label: format!("範囲{index}"),
                body: format!("本文{index}"),
                line_count: 1,
                terms: vec![LiveTermExplanation {
                    term: format!("名{index}"),
                    explanation: format!("説明{index}"),
                    source_excerpt: "引用はここに含めない".into(),
                    external_source: if index == 3 {
                        " ".into()
                    } else if index == 5 {
                        "資料 🌕".into()
                    } else {
                        String::new()
                    },
                }],
                whiteboard: None,
            }
            .into()
        })
        .collect();
    assert_eq!(
        format_full_history_for_whiteboard(&chunks),
        concat!(
            "## Chunk 01 | 範囲0\n題: 題0\n本文0\n用語: 名0\n\n",
            "## Chunk 02 | 範囲1\n題: 題1\n本文1\n用語: 名1\n\n",
            "## Chunk 03 | 範囲2\n題: 題2\n本文2\n用語:\n- 名2: 説明2\n\n\n",
            "## Chunk 04 | 範囲3\n題: 題3\n本文3\n用語:\n- 名3: 説明3（出典:  ）\n\n\n",
            "## Chunk 05 | 範囲4\n題: 題4\n本文4\n用語:\n- 名4: 説明4\n\n\n",
            "## Chunk 06 | 範囲5\n題: 題5\n本文5\n用語:\n- 名5: 説明5（出典: 資料 🌕）\n"
        )
    );
    assert_eq!(
        format_recent_summary_context(&chunks, 2),
        "## 題4\n範囲4\n本文4\n\n## 題5\n範囲5\n本文5"
    );
    assert_eq!(
        format_current_chunk_for_whiteboard("\n 本文 \n", &chunks[3].terms, " 範囲 "),
        "範囲:  範囲 \n要約:\n\n 本文 \n\n用語:\n- 名3: 説明3（出典:  ）\n"
    );
}

#[test]
fn chunk_number_padding_reserves_exact_bytes_across_decimal_boundaries() {
    for count in [9, 10, 99, 100, 999, 1000] {
        let chunks = fixtures::summaries(count, 5);
        let message = context(&chunks, &[]).build();
        assert_eq!(message, before::build(&context(&chunks, &[])));
        assert_eq!(message.len(), message.capacity());
        assert!(message.contains(&format!("## Chunk {count:02} | ")));
    }
}

#[test]
fn long_history_and_current_terms_are_full_owned_after_input_release() {
    let (message, expected, full, recent) = {
        let chunks: Vec<SharedSummaryChunk> = (0..64)
            .map(|index| {
                LiveSummaryChunk {
                    title: format!("題{index}"),
                    range_label: "範囲\n".into(),
                    body: format!("start{index} {} end{index}", "本文 🌕\n".repeat(4096)),
                    line_count: index,
                    whiteboard: None,
                    terms: vec![LiveTermExplanation {
                        term: format!("名{index}"),
                        explanation: "長い説明 👩🏽‍💻".repeat(8192),
                        source_excerpt: String::new(),
                        external_source: "出典 🌕".repeat(1024),
                    }],
                }
                .into()
            })
            .collect();
        let context = context(&chunks, &chunks[63].terms);
        (
            context.build(),
            before::build(&context),
            format_full_history_for_whiteboard(&chunks),
            format_recent_summary_context(&chunks, usize::MAX),
        )
    };
    assert_eq!(message.as_bytes(), expected.as_bytes());
    assert_eq!(message.capacity(), message.len());
    for index in 0..64 {
        assert!(full.contains(&format!("start{index} ")) && full.contains(&format!(" end{index}")));
        assert!(
            recent.contains(&format!("start{index} ")) && recent.contains(&format!(" end{index}"))
        );
    }
    assert!(message.contains(&"長い説明 👩🏽‍💻".repeat(8192)));
}
