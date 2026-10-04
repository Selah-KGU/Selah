use super::*;

fn node(id: &str, label: &str, role: &str, parent_id: &str) -> LiveWhiteboardNode {
    LiveWhiteboardNode {
        id: id.to_string(),
        label: label.to_string(),
        detail: String::new(),
        node_type: "structure".to_string(),
        kind: if role == "main" { "core" } else { "support" }.to_string(),
        role: role.to_string(),
        parent_id: parent_id.to_string(),
        source_type: "lecture".to_string(),
        source_excerpt: String::new(),
        external_source: String::new(),
    }
}

fn board(
    title: &str,
    nodes: Vec<LiveWhiteboardNode>,
    edges: Vec<LiveWhiteboardEdge>,
) -> LiveWhiteboard {
    LiveWhiteboard {
        title: title.to_string(),
        layout: "flow".to_string(),
        nodes,
        edges,
        schema_version: 0,
        normalized_by: String::new(),
    }
}

#[test]
fn parse_chunk_recovers_raw_newlines_in_strings() {
    // A markdown body with literal (unescaped) newlines is invalid JSON but
    // is exactly what models emit for large summaries. Repair must recover it.
    let raw = "{\"summary_markdown\": \"- 点1\n- 点2\n\n**用語**: 説明\", \"terms\": []}";
    let parsed = parse_chunk_ai_result(raw);
    assert!(parsed.body.contains("点1"));
    assert!(parsed.body.contains("点2"));
    assert!(parsed.body.contains("用語"));
}

#[test]
fn parse_chunk_recovers_truncated_object() {
    // Reply cut off mid-term (token ceiling). Repair closes the open string,
    // item, array and object so body + the completed term survive.
    let raw = "{\"summary_markdown\": \"- 重点A\\n- 重点B\", \"terms\": [{\"term\": \"認知的不協和\", \"explanation\": \"矛盾する認知を同時に持つときの不快感。途中で切れた";
    let parsed = parse_chunk_ai_result(raw);
    assert!(parsed.body.contains("重点A"));
    assert_eq!(parsed.terms.len(), 1);
    assert_eq!(parsed.terms[0].term, "認知的不協和");
}

#[test]
fn parse_chunk_salvages_body_when_unrepairable() {
    // Truncated mid-key: not validly closeable, but the body field is intact
    // and must be salvaged instead of dumping raw JSON into the note.
    let raw = "{\"summary_markdown\": \"本文は無事です\", \"ter";
    let parsed = parse_chunk_ai_result(raw);
    assert_eq!(parsed.body, "本文は無事です");
    assert!(!parsed.body.contains('{'));
}

#[test]
fn parse_chunk_never_returns_raw_json_blob() {
    // Whatever happens, a JSON-object reply must never surface its braces as
    // the rendered summary.
    let raw = "{\"summary_markdown\": broken json here, no quotes";
    let parsed = parse_chunk_ai_result(raw);
    assert!(!parsed.body.contains("broken json"));
}

#[test]
fn repair_json_object_leaves_valid_input_unchanged() {
    let valid = "{\"a\": \"b\", \"n\": [1, 2]}";
    assert_eq!(repair_json_object(valid).as_deref(), Some(valid));
}

#[test]
fn reconcile_whiteboard_trusts_model_rewrite() {
    let previous = board(
        "古い構造",
        vec![
            node("core", "中心", "main", ""),
            node("stale", "古い補足", "branch", "core"),
        ],
        vec![LiveWhiteboardEdge {
            from: "core".to_string(),
            to: "stale".to_string(),
            label: "古い関係".to_string(),
        }],
    );
    let current = board(
        "清晰化後",
        vec![
            node("core", "中心概念", "main", ""),
            node("fresh", "新しい要点", "branch", "core"),
        ],
        vec![LiveWhiteboardEdge {
            from: "core".to_string(),
            to: "fresh".to_string(),
            label: "更新".to_string(),
        }],
    );

    let reconciled = reconcile_whiteboard(Some(&previous), Some(current)).unwrap();

    assert_eq!(reconciled.title, "清晰化後");
    assert!(reconciled.nodes.iter().any(|n| n.id == "fresh"));
    assert!(!reconciled.nodes.iter().any(|n| n.id == "stale"));
    assert_eq!(reconciled.edges.len(), 1);
    assert_eq!(reconciled.edges[0].to, "fresh");
}

#[test]
fn reconcile_whiteboard_carries_previous_when_model_skips_board() {
    let previous = board(
        "前回",
        vec![
            node("core", "中心", "main", ""),
            node("branch", "補足", "branch", "core"),
        ],
        Vec::new(),
    );

    let reconciled = reconcile_whiteboard(Some(&previous), None).unwrap();

    assert_eq!(reconciled.title, "前回");
    assert_eq!(reconciled.nodes.len(), 2);
}

#[test]
fn reconcile_whiteboard_keeps_previous_on_unexpected_shrink() {
    let previous_nodes = (0..6)
        .map(|idx| {
            node(
                &format!("prev-{idx}"),
                &format!("旧概念{idx}"),
                if idx == 0 { "main" } else { "branch" },
                if idx == 0 { "" } else { "prev-0" },
            )
        })
        .collect::<Vec<_>>();
    let previous = board("前回", previous_nodes, Vec::new());
    let current = board(
        "縮小",
        vec![
            node("current-main", "新主概念", "main", ""),
            node("current-branch", "新補足", "branch", "current-main"),
        ],
        Vec::new(),
    );

    let reconciled = reconcile_whiteboard(Some(&previous), Some(current)).unwrap();

    assert_eq!(reconciled.title, "前回");
    assert_eq!(reconciled.nodes.len(), 6);
}

#[test]
fn reconcile_whiteboard_allows_growth_for_new_topics() {
    let previous = board(
        "前回",
        vec![
            node("prev-main-a", "旧主題A", "main", ""),
            node("prev-main-b", "旧主題B", "main", ""),
            node("prev-branch-a", "旧補足A", "branch", "prev-main-a"),
            node("prev-branch-b", "旧補足B", "branch", "prev-main-b"),
        ],
        Vec::new(),
    );
    let current = board(
        "新課題を追加",
        vec![
            node("new-main-a", "新課題A", "main", ""),
            node("new-main-b", "新課題B", "main", ""),
            node("new-main-c", "新課題C", "main", ""),
            node("new-branch-a", "新補足A", "branch", "new-main-a"),
            node("new-branch-b", "新補足B", "branch", "new-main-b"),
        ],
        Vec::new(),
    );

    let reconciled = reconcile_whiteboard(Some(&previous), Some(current)).unwrap();

    assert_eq!(reconciled.title, "新課題を追加");
    assert_eq!(reconciled.nodes.len(), 5);
    assert!(reconciled.nodes.iter().any(|node| node.id == "new-main-c"));
}

#[test]
fn parse_whiteboard_limits_term_node_edges_to_parent() {
    let value = serde_json::json!({
        "title": "用語テスト",
        "layout": "flow",
        "nodes": [
            {
                "id": "main",
                "label": "主概念",
                "node_type": "structure",
                "kind": "core",
                "role": "main",
                "source_type": "lecture"
            },
            {
                "id": "other",
                "label": "別概念",
                "node_type": "structure",
                "kind": "result",
                "role": "main",
                "source_type": "lecture"
            },
            {
                "id": "term",
                "label": "用語",
                "node_type": "term",
                "kind": "core",
                "role": "main",
                "parent_id": "main",
                "source_type": "lecture"
            }
        ],
        "edges": [
            { "from": "main", "to": "term", "label": "定義" },
            { "from": "term", "to": "other", "label": "横断" },
            { "from": "term", "to": "main", "label": "重複" },
            { "from": "main", "to": "other", "label": "発展" }
        ]
    });

    let board = parse_live_whiteboard(Some(&value)).expect("whiteboard should parse");
    let term = board
        .nodes
        .iter()
        .find(|node| node.id == "term")
        .expect("term node should survive");

    assert_eq!(term.node_type, "term");
    assert_eq!(term.kind, "support");
    assert_eq!(term.role, "branch");
    assert_eq!(term.parent_id, "main");
    assert_eq!(
        board
            .edges
            .iter()
            .filter(|edge| edge.from == "term" || edge.to == "term")
            .count(),
        1
    );
    let term_edge = board
        .edges
        .iter()
        .find(|edge| edge.from == "main" && edge.to == "term")
        .expect("term edge should point from parent to term");
    assert!(term_edge.label.is_empty());
    assert!(board
        .edges
        .iter()
        .any(|edge| edge.from == "main" && edge.to == "other" && edge.label == "発展"));
}

#[test]
fn parse_whiteboard_keeps_global_term_nodes_without_valid_parent() {
    let value = serde_json::json!({
        "title": "孤立用語テスト",
        "layout": "flow",
        "nodes": [
            {
                "id": "main",
                "label": "主概念",
                "node_type": "structure",
                "kind": "core",
                "role": "main",
                "source_type": "lecture"
            },
            {
                "id": "other",
                "label": "別概念",
                "node_type": "structure",
                "kind": "support",
                "role": "branch",
                "parent_id": "main",
                "source_type": "lecture"
            },
            {
                "id": "orphan-term",
                "label": "孤立用語",
                "node_type": "term",
                "kind": "support",
                "role": "branch",
                "parent_id": "missing",
                "source_type": "lecture"
            }
        ],
        "edges": [
            { "from": "main", "to": "other", "label": "補足" },
            { "from": "main", "to": "orphan-term", "label": "定義" }
        ]
    });

    let board = parse_live_whiteboard(Some(&value)).expect("whiteboard should parse");

    let global_term = board
        .nodes
        .iter()
        .find(|node| node.id == "orphan-term")
        .expect("orphan term should become a global term");
    assert_eq!(global_term.node_type, "term");
    assert!(global_term.parent_id.is_empty());
    assert!(!board
        .edges
        .iter()
        .any(|edge| edge.from == "orphan-term" || edge.to == "orphan-term"));
}

#[test]
fn parse_whiteboard_allows_term_nodes_to_attach_to_structure_branch() {
    let value = serde_json::json!({
        "title": "分岐用語テスト",
        "layout": "flow",
        "nodes": [
            {
                "id": "main",
                "label": "主概念",
                "node_type": "structure",
                "kind": "core",
                "role": "main",
                "source_type": "lecture"
            },
            {
                "id": "branch",
                "label": "構造分岐",
                "node_type": "structure",
                "kind": "support",
                "role": "branch",
                "parent_id": "main",
                "source_type": "lecture"
            },
            {
                "id": "term",
                "label": "分岐用語",
                "node_type": "term",
                "kind": "support",
                "role": "branch",
                "parent_id": "branch",
                "source_type": "lecture"
            }
        ],
        "edges": [
            { "from": "branch", "to": "term", "label": "定義" },
            { "from": "main", "to": "branch", "label": "展開" }
        ]
    });

    let board = parse_live_whiteboard(Some(&value)).expect("whiteboard should parse");
    let term = board
        .nodes
        .iter()
        .find(|node| node.id == "term")
        .expect("term node should survive");

    assert_eq!(term.parent_id, "branch");
    let term_edge = board
        .edges
        .iter()
        .find(|edge| edge.from == "branch" && edge.to == "term")
        .expect("term edge should point from structure branch to term");
    assert!(term_edge.label.is_empty());
}

#[test]
fn parse_whiteboard_synthesizes_missing_term_parent_edge() {
    let value = serde_json::json!({
        "title": "用語エッジ補完テスト",
        "layout": "flow",
        "nodes": [
            {
                "id": "main",
                "label": "主概念",
                "node_type": "structure",
                "kind": "core",
                "role": "main",
                "source_type": "lecture"
            },
            {
                "id": "term",
                "label": "用語",
                "node_type": "term",
                "kind": "support",
                "role": "branch",
                "parent_id": "main",
                "source_type": "lecture"
            }
        ],
        "edges": []
    });

    let board = parse_live_whiteboard(Some(&value)).expect("whiteboard should parse");

    let term_edges = board
        .edges
        .iter()
        .filter(|edge| edge.from == "main" && edge.to == "term")
        .collect::<Vec<_>>();
    assert_eq!(term_edges.len(), 1);
    assert!(term_edges[0].label.is_empty());
}

#[test]
fn parse_whiteboard_keeps_all_valid_structure_edges() {
    let value = serde_json::json!({
        "title": "構造エッジテスト",
        "layout": "flow",
        "nodes": [
            { "id": "a", "label": "A", "node_type": "structure", "kind": "core", "role": "main", "source_type": "lecture" },
            { "id": "b", "label": "B", "node_type": "structure", "kind": "core", "role": "main", "source_type": "lecture" },
            { "id": "c", "label": "C", "node_type": "structure", "kind": "core", "role": "main", "source_type": "lecture" },
            { "id": "d", "label": "D", "node_type": "structure", "kind": "core", "role": "main", "source_type": "lecture" }
        ],
        "edges": [
            { "from": "a", "to": "b", "label": "関係1" },
            { "from": "b", "to": "a", "label": "重複" },
            { "from": "a", "to": "c", "label": "関係2" },
            { "from": "a", "to": "d", "label": "関係3" },
            { "from": "b", "to": "c", "label": "関係4" },
            { "from": "c", "to": "d", "label": "" }
        ]
    });

    let board = parse_live_whiteboard(Some(&value)).expect("whiteboard should parse");

    assert_eq!(board.edges.len(), 4);
    assert!(board
        .edges
        .iter()
        .any(|edge| edge.from == "a" && edge.to == "b"));
    assert!(board
        .edges
        .iter()
        .any(|edge| edge.from == "a" && edge.to == "c"));
    assert!(board
        .edges
        .iter()
        .any(|edge| edge.from == "a" && edge.to == "d"));
    assert!(board.edges.iter().any(|edge| edge.label == "関係4"));
    assert!(!board.edges.iter().any(|edge| edge.label == "重複"));
}
