use super::super::*;
use super::transcript::recording;

fn chunk(index: usize, node_count: usize) -> LiveSummaryChunk {
    let nodes = (0..node_count)
        .map(|i| LiveWhiteboardNode {
            id: format!("{index}-{i}"),
            label: format!("概念 {i} 👩🏽‍💻"),
            detail: "完全な詳細\n\"引用\" 日本語 ".repeat(12),
            node_type: "structure".into(),
            kind: "core".into(),
            role: "main".into(),
            parent_id: String::new(),
            source_type: "lecture".into(),
            source_excerpt: "講義内の完全な根拠".into(),
            external_source: String::new(),
        })
        .collect();
    LiveSummaryChunk {
        title: format!("Chunk {index} 🌕"),
        range_label: "10:00-10:10".into(),
        body: format!(
            "# 完全な要約 {index}\n{}",
            "本文と引用を保持する \"quoted\" 👩🏽‍💻\n".repeat(64)
        ),
        line_count: index + 1,
        terms: vec![LiveTermExplanation {
            term: "一次資料".into(),
            explanation: "全文の説明".repeat(128),
            source_excerpt: "講義の発話そのもの".into(),
            external_source: "https://example.invalid/source".into(),
        }],
        whiteboard: Some(
            LiveWhiteboard {
                title: format!("累積ボード {index}"),
                layout: "grid".into(),
                nodes,
                edges: vec![],
                schema_version: 1,
                normalized_by: "backend".into(),
            }
            .into(),
        ),
    }
}

#[test]
fn completed_chunk_moves_full_buffers_and_round_trips_legacy_cache_objects() {
    let summary = chunk(0, 64);
    let expected = serde_json::to_value(&summary).unwrap();
    let body_ptr = summary.body.as_ptr();
    let term_ptr = summary.terms[0].explanation.as_ptr();
    let nodes_ptr = summary.whiteboard.as_ref().unwrap().nodes.as_ptr();
    let detail_ptr = summary.whiteboard.as_ref().unwrap().nodes[0]
        .detail
        .as_ptr();
    let mut session = recording();
    session.append_summary(summary);
    let stored = &session.summaries[0];
    assert_eq!(stored.body.as_ptr(), body_ptr);
    assert_eq!(stored.terms[0].explanation.as_ptr(), term_ptr);
    assert_eq!(
        stored.whiteboard.as_ref().unwrap().nodes.as_ptr(),
        nodes_ptr
    );
    assert_eq!(
        stored.whiteboard.as_ref().unwrap().nodes[0].detail.as_ptr(),
        detail_ptr
    );
    let snapshot = session.snapshot();
    let value = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(value["summaries"], serde_json::json!([expected]));
    let cache_json = serde_json::json!({
        "date": "2026-10-08", "course_name": "legacy course",
        "started_at": "2026-10-08 10:00:00", "transcript_lines": [],
        "summaries": value["summaries"]
    });
    let restored: LiveDayCache = serde_json::from_value(cache_json.clone()).unwrap();
    assert_eq!(serde_json::to_value(&restored).unwrap(), cache_json);
    let borrowed = LiveDayCacheRef {
        date: restored.date.clone(),
        course_name: &restored.course_name,
        started_at: restored.started_at.clone(),
        transcript_lines: &restored.transcript_lines,
        summaries: &restored.summaries,
    };
    assert_eq!(serde_json::to_value(borrowed).unwrap(), cache_json);
    let decoded: LiveSessionSnapshot = serde_json::from_value(value).unwrap();
    assert_eq!(
        decoded.summaries[0]
            .whiteboard
            .as_ref()
            .unwrap()
            .nodes
            .len(),
        64
    );
    assert_eq!(decoded.summaries[0].body, snapshot.summaries[0].body);
    let markdown = build_markdown(
        &session.course,
        session.started_at,
        Local::now(),
        "overall",
        &decoded.summaries,
        &decoded.transcript_lines,
    );
    assert!(markdown.contains(&decoded.summaries[0].body));
    assert!(markdown.contains("完全な詳細"));
    assert!(markdown.contains("講義内の完全な根拠"));
}

#[test]
fn retained_snapshots_copy_only_indices_and_keep_all_prior_boards_immutable() {
    let state = LiveState::new();
    let mut session = recording();
    for index in 0..32 {
        session.append_summary(chunk(index, 24));
    }
    *state.session.lock().unwrap() = Some(session);
    let first = current_snapshot(&state);
    let first_json = serde_json::to_value(&first.summaries).unwrap();
    {
        let mut guard = state.session.lock().unwrap();
        guard.as_mut().unwrap().append_summary(chunk(32, 40));
    }
    let second = current_snapshot(&state);
    {
        let mut guard = state.session.lock().unwrap();
        guard.as_mut().unwrap().append_summary(chunk(33, 48));
    }
    let third = current_snapshot(&state);
    assert_eq!(
        (
            first.summaries.len(),
            second.summaries.len(),
            third.summaries.len()
        ),
        (32, 33, 34)
    );
    assert!(!Arc::ptr_eq(&first.summaries, &second.summaries));
    assert!(!Arc::ptr_eq(&second.summaries, &third.summaries));
    for index in 0..32 {
        assert!(Arc::ptr_eq(
            &first.summaries[index],
            &second.summaries[index]
        ));
        assert!(Arc::ptr_eq(
            &first.summaries[index],
            &third.summaries[index]
        ));
    }
    assert!(Arc::ptr_eq(&second.summaries[32], &third.summaries[32]));
    assert_eq!(serde_json::to_value(&first.summaries).unwrap(), first_json);
    assert_eq!(
        third.summaries[33].whiteboard.as_ref().unwrap().nodes.len(),
        48
    );
    assert_eq!(third.summaries[33].body, chunk(33, 48).body);
    *state.session.lock().unwrap() = None;
    assert_eq!(serde_json::to_value(&first.summaries).unwrap(), first_json);
}

#[test]
fn recent_prompt_context_keeps_full_bodies_in_chronological_range_order() {
    let chunks: Vec<_> = (0..7).map(|i| Arc::new(chunk(i, 8))).collect();
    for limit in [0, 1, 2, 7, usize::MAX] {
        let expected = if limit == 0 {
            "なし".into()
        } else {
            // Previous collect/reverse path; compare ordering and text against
            // the new borrowed-slice path, including unbounded/zero limits.
            chunks
                .iter()
                .rev()
                .take(limit)
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(|chunk| format!("## {}\n{}\n{}", chunk.title, chunk.range_label, chunk.body))
                .collect::<Vec<_>>()
                .join("\n\n")
        };
        assert_eq!(format_recent_summary_context(&chunks, limit), expected);
    }
    assert_eq!(format_recent_summary_context(&[], 2), "なし");
}

#[test]
#[ignore = "manual summary append CPU benchmark; no models, recording or UI"]
fn benchmark_append_summary_after_retained_snapshot() {
    use std::{hint::black_box, time::Instant};
    let chunks: LiveSummaryChunks = Arc::new((0..75).map(|i| Arc::new(chunk(i, 100))).collect());
    // Comparison uses the previous owned-chunk history, not a separate model.
    let old_base = Arc::new(
        chunks
            .iter()
            .map(|c| c.as_ref().clone())
            .collect::<Vec<_>>(),
    );
    let mut old_times = vec![];
    let mut new_times = vec![];
    for trial in 0..7 {
        for old in if trial % 2 == 0 {
            [true, false]
        } else {
            [false, true]
        } {
            let next = chunk(75, 100);
            if old {
                let history = Mutex::new(old_base.clone());
                let held = history.lock().unwrap().clone();
                let start = Instant::now();
                {
                    Arc::make_mut(&mut history.lock().unwrap()).push(next);
                }
                old_times.push(start.elapsed());
                let current = history.lock().unwrap();
                assert_eq!(current.len(), 76);
                assert_eq!(held.len(), 75);
                assert_eq!(current[0].body, held[0].body);
                black_box(&*current);
            } else {
                let state = LiveState::new();
                let mut session = recording();
                session.summaries = chunks.clone();
                *state.session.lock().unwrap() = Some(session);
                let held = current_snapshot(&state);
                let start = Instant::now();
                {
                    state
                        .session
                        .lock()
                        .unwrap()
                        .as_mut()
                        .unwrap()
                        .append_summary(next);
                }
                new_times.push(start.elapsed());
                let current = current_snapshot(&state);
                assert_eq!(current.summaries.len(), 76);
                assert_eq!(held.summaries.len(), 75);
                assert!(Arc::ptr_eq(&current.summaries[0], &held.summaries[0]));
                black_box(current);
            }
        }
    }
    old_times.sort();
    new_times.sort();
    eprintln!("75 retained chunks / 100 board nodes each, default test profile, 7 alternating trials: previous {:.3} ms / shared {:.3} ms summary append (median)", old_times[3].as_secs_f64()*1000.0, new_times[3].as_secs_f64()*1000.0);
}
