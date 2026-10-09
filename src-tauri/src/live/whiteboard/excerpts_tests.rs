use super::*;
use crate::live::LiveTranscriptLine;
use before::clamp_chars;
#[allow(dead_code)]
#[path = "excerpts/before.rs"]
mod before;

fn node(label: &str) -> LiveWhiteboardNode {
    serde_json::from_value(serde_json::json!({"id":label,"label":label,"source_type":"lecture","node_type":"structure"})).unwrap()
}
fn speech(text: &str) -> SharedTranscriptLine {
    LiveTranscriptLine {
        text: text.into(),
        at: "10:00:00".into(),
    }
    .into()
}

// Previous product algorithm, retained only for differential checks and timing.
fn previous_best(node: &LiveWhiteboardNode, lines: &[SharedTranscriptLine]) -> Option<String> {
    let terms = before::whiteboard_excerpt_terms(node);
    if terms.is_empty() {
        return None;
    }
    lines
        .iter()
        .filter_map(|line| {
            let normalized = before::normalized_excerpt_match_text(&line.text);
            if normalized.is_empty() {
                return None;
            }
            let score = terms
                .iter()
                .filter(|term| normalized.contains(term.as_str()))
                .map(|term| term.chars().count())
                .sum::<usize>();
            (score > 0).then_some((score, line.text.as_str()))
        })
        .max_by_key(|(score, text)| (*score, text.chars().count()))
        .map(|(_, text)| clamp_chars(text, 80))
}

#[test]
fn indexed_excerpts_match_previous_selection_for_unicode_whitespace_and_terms() {
    let mut lines = vec![
        speech(""),
        speech(" \t\n　"),
        speech("APIあ"),
        speech("Apiい"),
    ];
    for i in 0..400 {
        lines.push(speech(&format!(
            "{i}: 一次資料\tの方法論　ABC{} API 東京日本語 👩🏽‍💻 ÉCOLE école\n{}",
            i % 7,
            "引用\" ".repeat(i % 13)
        )));
    }
    let indexed = TranscriptExcerpts::new(&lines);
    for label in [
        "API",
        "ABC1",
        "一次資料の方法論",
        "東京日本語",
        "👩🏽‍💻",
        "ÉCOLE",
        "école",
        "不存在",
        "aa",
        "あ",
        "ab abc abc ab",
        "東京日本語/一次資料/方法論/API/ABC2/ABC3/ABC4/ABC5/ABC6/ÉCOLE",
    ] {
        let mut node = node(label);
        node.detail = format!("{label}、方法論 : API");
        assert_eq!(
            indexed.best(whiteboard_excerpt_terms(&node)),
            previous_best(&node, &lines),
            "{label}"
        );
    }
}

#[test]
fn excerpt_ties_use_original_character_count_then_the_last_matching_line() {
    let lines = vec![speech("APIあ"), speech("Apiい")];
    let label = node("API");
    assert_eq!(
        TranscriptExcerpts::new(&lines)
            .best(whiteboard_excerpt_terms(&label))
            .as_deref(),
        Some("Apiい")
    );
    let lines = vec![speech("API      あ"), speech("Apiい")];
    assert_eq!(
        TranscriptExcerpts::new(&lines)
            .best(whiteboard_excerpt_terms(&label))
            .as_deref(),
        Some("API      あ")
    );
    let long = format!("API {}", "👩🏽‍💻 日本語\n".repeat(100));
    let lines = vec![speech(&long)];
    let excerpt = TranscriptExcerpts::new(&lines)
        .best(whiteboard_excerpt_terms(&label))
        .unwrap();
    assert_eq!(excerpt, clamp_chars(&long, 80));
}

#[test]
fn transcript_matching_preserves_supplied_inherited_term_and_external_sources() {
    let board = |nodes| LiveWhiteboard {
        title: "board".into(),
        layout: "grid".into(),
        nodes,
        edges: vec![],
        schema_version: 1,
        normalized_by: "backend".into(),
    };
    let mut inherited = node("API inherited");
    inherited.source_excerpt = "prior exact source".into();
    let previous = board(vec![inherited.clone()]);
    let mut renamed = node("renamed API");
    renamed.id = inherited.id.clone();
    let mut supplied = node("supplied API");
    supplied.source_excerpt = "supplied exact source".into();
    let mut external = node("external API");
    external.source_type = "external".into();
    let mut term = node("API term");
    term.node_type = "term".into();
    let terms = vec![LiveTermExplanation {
        term: "API term".into(),
        explanation: "meaning".into(),
        source_excerpt: "term exact source".into(),
        external_source: String::new(),
    }];
    let lines = vec![
        speech("API new source"),
        speech("API another longer new source"),
    ];
    let enriched = enrich_whiteboard_source_excerpts(
        Some(board(vec![
            renamed,
            node("API inherited"),
            supplied,
            external,
            term,
            node("API"),
        ])),
        Some(&previous),
        &terms,
        &lines,
    )
    .unwrap();
    let sources: Vec<_> = enriched
        .nodes
        .iter()
        .map(|n| n.source_excerpt.as_str())
        .collect();
    assert_eq!(
        sources,
        [
            "prior exact source",
            "prior exact source",
            "supplied exact source",
            "",
            "term exact source",
            "API another longer new source"
        ]
    );
    assert_eq!(previous.nodes[0].source_excerpt, "prior exact source");
    assert_eq!(lines[1].text, "API another longer new source");
}

#[test]
#[ignore = "manual CPU benchmark; no model, microphone, UI, or GPU"]
fn benchmark_whiteboard_excerpt_matching() {
    use std::{hint::black_box, time::Instant};
    let lines: Vec<_> = (0..500)
        .map(|i| {
            speech(&format!(
                "{i} 資料検討 ABC{} API 方法論\n{}",
                i % 15,
                "字幕の原文を保持する 👩🏽‍💻　".repeat(12)
            ))
        })
        .collect();
    let nodes: Vec<_> = (0..75)
        .map(|i| node(&format!("資料検討 ABC{} API", i % 15)))
        .collect();
    let previous = || {
        nodes
            .iter()
            .map(|node| previous_best(node, &lines))
            .collect::<Vec<_>>()
    };
    let current = || {
        let index = TranscriptExcerpts::new(&lines);
        nodes
            .iter()
            .map(|node| index.best(whiteboard_excerpt_terms(node)))
            .collect::<Vec<_>>()
    };
    assert_eq!(previous(), current());
    let mut old_times = vec![];
    let mut new_times = vec![];
    for trial in 0..5 {
        for old in if trial % 2 == 0 {
            [true, false]
        } else {
            [false, true]
        } {
            let start = Instant::now();
            let result = if old { previous() } else { current() };
            black_box(result);
            let elapsed = start.elapsed();
            if old {
                old_times.push(elapsed);
            } else {
                new_times.push(elapsed);
            }
        }
    }
    old_times.sort();
    new_times.sort();
    eprintln!("500 full speech lines / 75 nodes, default test profile, 5 alternating trials: previous {:.3} ms / indexed {:.3} ms (median)", old_times[2].as_secs_f64()*1000.0, new_times[2].as_secs_f64()*1000.0);
}
