use super::*;

#[path = "before.rs"]
mod before;
#[path = "fixtures.rs"]
mod fixtures;

#[test]
fn complete_user_text_matches_frozen_assembly_for_1536_unicode_and_boundary_inputs() {
    for seed in 0..32 {
        for line_count in [0, 1, 24, 25, 80, 81, 500, 501] {
            for summary_count in [0, 1, 2, 3, 12, 128] {
                let input = fixtures::input(line_count, summary_count, seed);
                let original =
                    serde_json::to_vec(&(&input.course, &input.lines, &input.summaries)).unwrap();
                assert_eq!(
                    chunk(
                        "講義\n 🌕",
                        &input.summaries,
                        &input.lines,
                        "末尾の注記\r\n"
                    ),
                    before::chunk(
                        "講義\n 🌕",
                        &input.summaries,
                        &input.lines,
                        "末尾の注記\r\n"
                    )
                );
                assert_eq!(
                    overall(&input.course, &input.lines, &input.summaries),
                    before::overall(&input.course, &input.lines, &input.summaries)
                );
                assert_eq!(
                    todo(&input.course, &input.lines, &input.summaries, "全計画\n 👩🏽‍💻"),
                    before::todo(&input.course, &input.lines, &input.summaries, "全計画\n 👩🏽‍💻")
                );
                assert_eq!(
                    serde_json::to_vec(&(&input.course, &input.lines, &input.summaries)).unwrap(),
                    original
                );
                assert!(input
                    .lines
                    .iter()
                    .all(|line| std::sync::Arc::strong_count(line) == 1));
                assert!(input
                    .summaries
                    .iter()
                    .all(|summary| std::sync::Arc::strong_count(summary) == 1));
            }
        }
    }
}

#[test]
fn actual_requests_keep_full_chunk_and_summary_bodies_but_existing_tail_windows() {
    let input = fixtures::input(3000, 140, 5);
    let first = chunk("course", &input.summaries, &input.lines, "note");
    let last = overall(&input.course, &input.lines, &input.summaries);
    let todo = todo(&input.course, &input.lines, &input.summaries, "plan");
    for index in 0..3000 {
        assert!(first.contains(&format!("line{index}:")));
    }
    assert!(
        !first.contains("summary137:")
            && first.contains("summary138:")
            && first.contains("summary139:")
    );
    assert!(
        !last.contains("line2975:") && last.contains("line2976:") && last.contains("line2999:")
    );
    assert!(
        !todo.contains("line2919:") && todo.contains("line2920:") && todo.contains("line2999:")
    );
    for index in 0..140 {
        assert!(
            last.contains(&format!("summary{index}:")) && last.contains(&format!("body{index}:"))
        );
        assert!(
            todo.contains(&format!("summary{index}:")) && todo.contains(&format!("body{index}:"))
        );
    }
    let empty = fixtures::input(0, 0, 0);
    assert_eq!(
        chunk("course", &[], &[], "note"),
        "course\n\n直前の分割要約:\nなし\n\n今回の文字起こし:\n\n\nnote"
    );
    assert!(overall(&empty.course, &[], &[])
        .contains("\n\n分割要約:\n\n\n終盤の文字起こし:\n\n\n注記:"));
}

#[test]
fn large_full_text_survives_after_all_input_records_are_released() {
    let (first, last, todo_text, expected) = {
        let mut input = fixtures::input(256, 64, 5);
        for (index, line) in input.lines.iter_mut().enumerate() {
            std::sync::Arc::make_mut(line).text =
                format!("line{index}: {} endline{index}", "発話 🌕 ".repeat(1024));
        }
        for (index, chunk) in input.summaries.iter_mut().enumerate() {
            std::sync::Arc::make_mut(chunk).body =
                format!("body{index}: {} endbody{index}", "要約 👩🏽‍💻 ".repeat(4096));
        }
        let expected = [
            before::chunk("course", &input.summaries, &input.lines, "note"),
            before::overall(&input.course, &input.lines, &input.summaries),
            before::todo(&input.course, &input.lines, &input.summaries, "plan"),
        ];
        (
            chunk("course", &input.summaries, &input.lines, "note"),
            overall(&input.course, &input.lines, &input.summaries),
            todo(&input.course, &input.lines, &input.summaries, "plan"),
            expected,
        )
    };
    assert_eq!(
        [&first, &last, &todo_text],
        [&expected[0], &expected[1], &expected[2]]
    );
    for index in 0..256 {
        assert!(first.contains(&format!("endline{index}")));
    }
    for index in 0..64 {
        assert!(
            last.contains(&format!("endbody{index}"))
                && todo_text.contains(&format!("endbody{index}"))
        );
    }
}
