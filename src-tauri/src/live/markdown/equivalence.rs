use super::*;
use chrono::TimeZone;

fn compare(
    course: &LiveCourseInfo,
    overall: &str,
    summaries: &[SharedSummaryChunk],
    lines: &[SharedTranscriptLine],
) -> String {
    let started = Local
        .with_ymd_and_hms(2026, 10, 8, 23, 58, 59)
        .single()
        .unwrap();
    let ended = started + chrono::Duration::minutes(90);
    let original = serde_json::to_vec(&(course, summaries, lines)).unwrap();
    let expected = before::build_markdown(course, started, ended, overall, summaries, lines);
    let actual = build_markdown(course, started, ended, overall, summaries, lines);
    assert_eq!(actual.as_bytes(), expected.as_bytes());
    assert_eq!(
        serde_json::to_vec(&(course, summaries, lines)).unwrap(),
        original
    );
    actual
}

#[test]
fn complete_markdown_matches_original_across_course_metadata_empty_and_long_recordings() {
    for free in [false, true] {
        for metadata in ["empty", "whitespace", "complete"] {
            let mut course = fixtures::course(free);
            if metadata != "complete" {
                let value = if metadata == "empty" { "" } else { " \t\r\n" };
                course.course_name = value.into();
                course.course_code = value.into();
                course.teacher = value.into();
                course.room = value.into();
                course.time_label = value.into();
            }
            for count in [0, 1, 2, 119, 120, 121, 5_000] {
                let lines = fixtures::lines(count);
                for chunks in [0, 1, 3] {
                    let summaries = fixtures::summaries(chunks);
                    for overall in [
                        "",
                        "### 全体要約\n完全な概要 中文 👩🏽‍💻\n\n### 今回の論点\n点\r\n",
                    ] {
                        compare(&course, overall, &summaries, &lines);
                    }
                }
            }
        }
    }
}

#[test]
fn whiteboard_selection_json_edges_and_source_rules_match_original() {
    for free in [false, true] {
        let course = fixtures::course(free);
        for count in [0, 1, 2, 75] {
            for title in ["", " \t\r\n", " 板書 👩🏽‍💻 "] {
                let mut board = fixtures::board(count);
                board.title = title.into();
                if let Some(first) = board.nodes.first().cloned() {
                    let mut duplicate = first;
                    duplicate.label = "duplicate ID must not replace the first label".into();
                    board.nodes.push(duplicate);
                    board.edges.push(LiveWhiteboardEdge {
                        from: "node-0".into(),
                        to: "node-0".into(),
                        label: " loop\n ".into(),
                    });
                }
                let mut summaries = fixtures::summaries(3);
                std::sync::Arc::make_mut(&mut summaries[2]).whiteboard = Some(board.clone().into());
                let text = compare(
                    &course,
                    "### 全体要約\nsummary",
                    &summaries,
                    &fixtures::lines(2),
                );
                if count == 0 {
                    assert!(!text.contains("```live-whiteboard"));
                    assert!(!text.contains("関係:"));
                } else {
                    let json = text
                        .split("```live-whiteboard\n")
                        .nth(1)
                        .unwrap()
                        .split("\n```")
                        .next()
                        .unwrap();
                    assert_eq!(
                        serde_json::from_str::<serde_json::Value>(json).unwrap(),
                        serde_json::to_value(&board).unwrap()
                    );
                    assert_eq!(text.matches("```live-whiteboard").count(), 1);
                    assert!(text.contains("- 节点 0"));
                }
            }
        }
        let mut chunks = fixtures::summaries(3);
        // The last Some board wins even when empty; do not fall back to an
        // older nonempty board or change which complete JSON is persisted.
        std::sync::Arc::make_mut(&mut chunks[0]).whiteboard = Some(fixtures::board(5).into());
        std::sync::Arc::make_mut(&mut chunks[2]).whiteboard = Some(fixtures::board(0).into());
        assert!(!compare(&course, "", &chunks, &[]).contains("```live-whiteboard"));
        std::sync::Arc::make_mut(&mut chunks[2]).whiteboard = None;
        assert!(compare(&course, "", &chunks, &[]).contains("```live-whiteboard"));
    }
}

#[test]
fn a_very_long_single_line_and_empty_chunk_fields_remain_complete() {
    let mut lines = fixtures::lines(2);
    std::sync::Arc::make_mut(&mut lines[0]).text = "全文 🌕\n\r\t\0 \"引用\" \\".repeat(20_000);
    std::sync::Arc::make_mut(&mut lines[1]).at = "[]\n\0👩🏽‍💻".into();
    let mut chunks = fixtures::summaries(2);
    let empty = std::sync::Arc::make_mut(&mut chunks[0]);
    empty.title.clear();
    empty.range_label.clear();
    empty.body.clear();
    empty.terms.clear();
    let text = compare(&fixtures::course(false), "\n\n", &chunks, &lines);
    assert!(text.contains(&lines[0].text));
    assert!(text.contains(&lines[1].text));
    assert!(text.ends_with('\n'));
}
