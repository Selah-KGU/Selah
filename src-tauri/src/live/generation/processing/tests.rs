use super::*;
use crate::live::{tests::transcript::recording, LiveWhiteboardNode};

fn board(count: usize) -> LiveWhiteboard {
    let nodes = (0..count)
        .map(|i| {
            serde_json::from_value::<LiveWhiteboardNode>(serde_json::json!({
                "id": format!("prior-{i}"),
                "label": format!("API{i}"),
                "source_type": "lecture",
                "source_excerpt": "old exact source",
                "node_type": "structure",
                "role": "main"
            }))
            .unwrap()
        })
        .collect();
    LiveWhiteboard {
        title: "prior".into(),
        layout: "grid".into(),
        nodes,
        edges: vec![],
        schema_version: 1,
        normalized_by: "backend".into(),
    }
}
fn history(board: LiveWhiteboard) -> LiveSummaryChunks {
    Arc::new(vec![LiveSummaryChunk {
        title: "previous chunk".into(),
        range_label: "10:00-10:10".into(),
        body: "prior full text".into(),
        line_count: 1,
        terms: vec![],
        whiteboard: Some(board.into()),
    }
    .into()])
}
fn result_value<Board: serde::Serialize>(parsed: &LiveChunkAiResult<Board>) -> serde_json::Value {
    serde_json::json!({"body":parsed.body,"terms":parsed.terms,"whiteboard":parsed.whiteboard})
}

#[tokio::test(flavor = "current_thread")]
async fn background_summary_keeps_parser_output_and_retryable_empty_errors() {
    let raw=serde_json::json!({"summary_markdown":"# 完全な要約\n\"quotes\" 👩🏽‍💻".repeat(2000),"terms":[{"term":"一次資料","explanation":"元の資料","source_excerpt":"一次資料に遡る"}]}).to_string();
    let expected = parse_chunk_ai_result(&raw);
    let actual = summary(raw).await.unwrap();
    assert_eq!(result_value(&actual), result_value(&expected));
    for raw in ["", "<think></think>", "{\"terms\":[]}"] {
        assert_eq!(
            summary(raw.into()).await.err().unwrap(),
            "AI要約の本文が空でした（再試行します）"
        );
    }
    assert_eq!(
        summary("plain summary\n🌕".into()).await.unwrap().body,
        "plain summary\n🌕"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn background_whiteboard_uses_captured_speech_and_prior_history_without_changing_them() {
    let mut recording = recording();
    recording.append_line(LiveTranscriptLine {
        text: "API 方法論の根拠となる発話 👩🏽‍💻".into(),
        at: "10:00:00".into(),
    });
    let lines = recording.transcript_lines.clone();
    let prior = history(board(1));
    let prior_value = serde_json::to_value(&prior).unwrap();
    let raw=serde_json::json!({"whiteboard":{"title":"updated","layout":"grid","nodes":[{"id":"prior-0","label":"API0","source_type":"lecture","node_type":"structure","role":"main"},{"id":"new","label":"方法論","source_type":"lecture","node_type":"structure","role":"main"}],"edges":[]}}).to_string();
    let parsed = parse_chunk_ai_result("{\"summary_markdown\":\"new chunk\",\"terms\":[]}");
    let expected_board = enrich_whiteboard_source_excerpts(
        parse_chunk_ai_result(&raw).whiteboard,
        latest_whiteboard(&prior),
        &parsed.terms,
        &lines,
    );
    let expected_board = reconcile_whiteboard(latest_shared_whiteboard(&prior), expected_board);
    let task = whiteboard(parsed, Some(raw), lines.clone(), prior.clone());
    recording.append_line(LiveTranscriptLine {
        text: "方法論 replacement has a much longer but later utterance".into(),
        at: "10:00:01".into(),
    });
    let result = task.await.unwrap();
    assert_eq!(
        serde_json::to_value(&result.whiteboard).unwrap(),
        serde_json::to_value(&expected_board).unwrap()
    );
    assert_eq!(result.body, "new chunk");
    let updated = result.whiteboard.unwrap();
    assert!(updated
        .nodes
        .iter()
        .any(|n| n.source_excerpt == "API 方法論の根拠となる発話 👩🏽‍💻"));
    assert_eq!(serde_json::to_value(&prior).unwrap(), prior_value);
    assert_eq!(lines.len(), 1);
    assert_eq!(recording.transcript_lines.len(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn missing_invalid_and_collapsed_boards_keep_the_previous_complete_board() {
    let prior = history(board(6));
    let expected = serde_json::to_value(latest_whiteboard(&prior)).unwrap();
    for raw in [None,Some("invalid JSON".into()),Some("{\"whiteboard\":null}".into()),Some(serde_json::json!({"whiteboard":{"title":"collapsed","nodes":[{"id":"prior-0","label":"API0","source_type":"lecture","node_type":"structure","role":"main"}],"edges":[]}}).to_string())] {
        let parsed=parse_chunk_ai_result("{\"summary_markdown\":\"complete summary\"}");
        let result=whiteboard(parsed,raw,Arc::new(vec![]),prior.clone()).await.unwrap();
        assert!(Arc::ptr_eq(result.whiteboard.as_ref().unwrap(), latest_shared_whiteboard(&prior).unwrap()));
        assert_eq!(serde_json::to_value(result.whiteboard).unwrap(),expected);
        assert_eq!(result.body,"complete summary");
    }
    let parsed = parse_chunk_ai_result("plain summary");
    assert!(whiteboard(parsed, None, Arc::new(vec![]), Arc::new(vec![]))
        .await
        .unwrap()
        .whiteboard
        .is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn overall_result_cleanup_keeps_the_existing_full_text_and_unicode_behavior() {
    for raw in [
        "# Full summary\n\"引用\" 👩🏽‍💻".repeat(4096),
        "<think>existing parser behavior</think>\n# Summary".into(),
        "<think>unterminated reasoning".into(),
        "".into(),
    ] {
        let expected = sanitize_model_output(&raw);
        assert_eq!(overall(raw).await.unwrap(), expected);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn todo_result_parsing_keeps_complete_fields_recording_metadata_and_the_existing_item_limit()
{
    let course = recording().course;
    let mut items = vec![serde_json::json!({"title":"  "})];
    for i in 1..8 {
        items.push(serde_json::json!({
            "title": format!(" 課題 {i} 🌕 "),
            "content_type": if i == 1 { "" } else { "report" },
            "deadline": " 2026-10-15 ",
            "note": "\n詳細 \"引用\" 👩🏽‍💻".repeat(4096),
            "source_excerpt": " 完全な発話引用 ",
        }));
    }
    let parsed = todos(
        course.clone(),
        serde_json::json!({"todos":items}).to_string(),
    )
    .await
    .unwrap();
    // The original budget counts the first six array entries before filtering
    // empty titles; it does not seek six additional valid entries.
    assert_eq!(parsed.len(), 5);
    assert_eq!(parsed[0].title, "課題 1 🌕");
    assert_eq!(parsed[0].content_type, "課題");
    assert_eq!(parsed[4].title, "課題 5 🌕");
    for suggestion in &parsed {
        assert_eq!(suggestion.course_name, course.course_name);
        assert_eq!(
            (suggestion.day, suggestion.period),
            (course.day, course.period)
        );
        assert_eq!(suggestion.deadline, "2026-10-15");
        assert_eq!(suggestion.note, "\n詳細 \"引用\" 👩🏽‍💻".repeat(4096).trim());
        assert_eq!(suggestion.source_excerpt, "完全な発話引用");
    }
    for raw in ["", "invalid", "{}", "{\"todos\":null}", "{\"todos\":[{}]}"] {
        assert!(todos(course.clone(), raw.into()).await.unwrap().is_empty());
    }
}
