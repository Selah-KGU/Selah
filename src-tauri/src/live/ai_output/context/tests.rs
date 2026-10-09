use super::*;

#[path = "before.rs"]
mod before;
#[path = "fixtures.rs"]
mod fixtures;

#[test]
fn context_matches_frozen_output_for_mixed_order_duplicates_orphans_and_empty_boards() {
    for seed in 0..128 {
        for count in [0, 1, 2, 7, 31, 128] {
            let board = fixtures::board(count, seed);
            let input = vec![
                fixtures::summary(Some(fixtures::board(3, seed + 1))),
                fixtures::summary(None),
                fixtures::summary(Some(board)),
                fixtures::summary(None),
            ];
            let original = serde_json::to_vec(&input).unwrap();
            assert_eq!(
                format_latest_whiteboard_context(&input),
                before::format_latest_whiteboard_context(&input),
                "seed={seed} count={count}"
            );
            assert_eq!(serde_json::to_vec(&input).unwrap(), original);
        }
    }
}

#[test]
fn context_preserves_structure_term_and_cross_edge_order_exactly() {
    let mut branch = fixtures::node("branch", "branch", "main");
    branch.label = "分岐".into();
    let mut term = fixtures::node("term", "branch", "branch");
    term.node_type = "term".into();
    term.label = "用語".into();
    let mut main = fixtures::node("main", "main", "");
    main.label = "主題".into();
    let mut other = fixtures::node("other", "main", "");
    other.label = "第二".into();
    let mut orphan = fixtures::node("orphan", "other-role", "missing");
    orphan.label = "孤立".into();
    let edge = |a: &str, b: &str, label: &str| LiveWhiteboardEdge {
        from: a.into(),
        to: b.into(),
        label: label.into(),
    };
    let board = LiveWhiteboard {
        title: String::new(),
        layout: "flow".into(),
        nodes: vec![term, branch, other, orphan, main],
        edges: vec![
            edge("main", "branch", "implicit"),
            edge("branch", "term", "term"),
            edge("absent", "other", "missing"),
            edge("other", "main", ""),
            edge("branch", "other", "横断"),
            edge("other", "main", "repeated"),
        ],
        schema_version: 1,
        normalized_by: String::new(),
    };
    assert_eq!(format_latest_whiteboard_context(&[fixtures::summary(Some(board))]),
        "title: — | layout: flow\n[main] other: 第二 (core)\n[main] main: 主題 (core)\n  [branch] branch: 分岐 (core)\n    terms(1): 用語\n[other-role] orphan: 孤立 (core)\nedges: other→main, branch→other [横断], other→main [repeated]\n");
}

#[test]
fn context_keeps_unicode_scalar_limits_and_whitespace_detail_behavior() {
    for length in [0, 1, 47, 48, 49, 59, 60, 61, 200] {
        let raw: String = "日🙂e\u{301}👩🏽‍💻".chars().cycle().take(length).collect();
        let mut main = fixtures::node("main", "main", "");
        main.detail = format!("\u{2003}{raw}\t\n");
        let mut branch = fixtures::node("branch", "branch", "main");
        branch.detail = main.detail.clone();
        let mut orphan = fixtures::node("orphan", "branch", "absent");
        orphan.detail = main.detail.clone();
        let mut b = fixtures::board(0, 0);
        b.nodes = vec![branch, orphan, main];
        let clamp = |limit| {
            let mut s: String = raw.chars().take(limit).collect();
            if length > limit {
                s.push('…');
            }
            s
        };
        let text = format_latest_whiteboard_context(&[fixtures::summary(Some(b))]);
        assert!(text.contains(&format!(
            "[main] main: 概念 main 🌕 (core) — {}\n",
            clamp(60)
        )));
        assert!(text.contains(&format!(
            "  [branch] branch: 概念 branch 🌕 (core) — {}\n",
            clamp(48)
        )));
        assert!(text.contains(&format!(
            "[branch] orphan: 概念 orphan 🌕 (core) — {}\n",
            clamp(48)
        )));
    }
}

#[test]
fn context_uses_latest_present_board_even_when_empty_and_emits_every_large_node() {
    assert_eq!(format_latest_whiteboard_context(&[]), "なし");
    assert_eq!(
        format_latest_whiteboard_context(&[fixtures::summary(None)]),
        "なし"
    );
    let mut board = fixtures::board(4096, 1);
    for node in &mut board.nodes {
        node.node_type = "structure".into();
        node.role = "main".into();
        node.parent_id.clear();
    }
    let expected =
        before::format_latest_whiteboard_context(&[fixtures::summary(Some(board.clone()))]);
    let input = vec![fixtures::summary(Some(board)), fixtures::summary(None)];
    let text = format_latest_whiteboard_context(&input);
    assert_eq!(text, expected);
    assert_eq!(text.matches("[main] ").count(), 4096);
    let mut input = input;
    input.push(fixtures::summary(Some(fixtures::board(0, 0))));
    assert_eq!(
        format_latest_whiteboard_context(&input),
        "title: — | layout: grid\n"
    );
}
