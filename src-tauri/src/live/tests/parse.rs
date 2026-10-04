use super::super::*;

#[test]
fn finish_ai_runs_only_for_non_local_provider_after_minimum_duration() {
    let now = Local::now();
    let long_session = now - chrono::Duration::seconds(120);
    let short_session = now - chrono::Duration::seconds(119);

    assert!(!should_run_finish_ai("local", long_session, now));
    assert!(should_run_finish_ai("openai", long_session, now));
    assert!(!should_run_finish_ai("openai", short_session, now));
}

#[test]
fn finish_requires_pending_chunk_ai_after_minimum_duration() {
    let now = Local::now();
    let long_session = now - chrono::Duration::seconds(120);
    let short_session = now - chrono::Duration::seconds(119);

    assert!(should_require_finish_chunk_ai(long_session, now, 3));
    assert!(!should_require_finish_chunk_ai(long_session, now, 0));
    assert!(!should_require_finish_chunk_ai(short_session, now, 3));
}

#[test]
fn parse_chunk_ai_result_extracts_terms() {
    let raw = r#"{
      "summary_markdown": "- MVC\n\n---\n\n**MVC**: 画面と処理を分ける考え方。",
          "terms": [
            {
              "term": "MVC",
              "explanation": "Model、View、Controllerに責務を分ける設計パターン。画面変更とデータ処理の責任範囲を見直す観点になる。",
              "source_excerpt": "MVCという設計",
              "external_source": "MDN Web Docs: MVC architecture"
            }
          ],
          "whiteboard": {
            "title": "MVCの責務分離",
            "layout": "flow",
            "nodes": [
              { "id": "model", "label": "Model", "detail": "データ", "kind": "core" },
              { "id": "view", "label": "View", "detail": "表示", "kind": "support" },
              { "id": "controller", "label": "Controller", "detail": "制御", "kind": "result" },
              { "id": "observer", "label": "Observer", "detail": "変更通知の関連パターン", "kind": "support", "source_type": "external", "external_source": "Gamma et al., Design Patterns" }
            ],
            "edges": [
              { "from": "model", "to": "view", "label": "反映" },
              { "from": "view", "to": "missing", "label": "無効" }
            ]
          }
        }"#;
    let parsed = parse_chunk_ai_result(raw);
    assert!(parsed.body.contains("MVC"));
    assert_eq!(parsed.terms.len(), 1);
    assert_eq!(parsed.terms[0].term, "MVC");
    assert!(parsed.terms[0].external_source.contains("MDN"));
    let board = parsed.whiteboard.expect("whiteboard should parse");
    assert_eq!(board.title, "MVCの責務分離");
    assert_eq!(board.layout, "flow");
    assert_eq!(board.nodes.len(), 4);
    assert_eq!(board.nodes[0].kind, "core");
    assert_eq!(board.nodes[0].role, "main");
    assert_eq!(board.nodes[3].source_type, "external");
    assert!(board.nodes[3].external_source.contains("Design Patterns"));
    assert_eq!(board.edges.len(), 1);
}

#[test]
fn parse_chunk_ai_result_filters_low_value_terms() {
    let raw = r#"{
      "summary_markdown": "- 重点\n\n---\n\n**重点**: 説明",
      "terms": [
        {
          "term": "授業",
          "explanation": "大学で行われる講義のこと。",
          "source_excerpt": "今日の授業"
        },
        {
          "term": "認知的不協和",
          "explanation": "矛盾する認知を同時に持つことで生じる不快感。講義では態度変容の説明に使われる。",
          "source_excerpt": "認知的不協和が起きる"
        }
      ]
    }"#;
    let parsed = parse_chunk_ai_result(raw);
    assert_eq!(parsed.terms.len(), 1);
    assert_eq!(parsed.terms[0].term, "認知的不協和");
}

#[test]
fn parse_chunk_ai_result_falls_back_to_markdown() {
    let parsed = parse_chunk_ai_result("- 重点\n\n---\n\n**重点**: 説明");
    assert!(parsed.body.starts_with("- 重点"));
    assert!(parsed.terms.is_empty());
    assert!(parsed.whiteboard.is_none());
}

#[test]
fn latest_whiteboard_context_uses_most_recent_cumulative_board() {
    let summaries = vec![
        LiveSummaryChunk {
            title: "前半".to_string(),
            range_label: "10:00-10:05".to_string(),
            body: "古い内容".to_string(),
            line_count: 3,
            terms: Vec::new(),
            whiteboard: Some(LiveWhiteboard {
                title: "古いボード".to_string(),
                layout: "grid".to_string(),
                nodes: vec![
                    LiveWhiteboardNode {
                        id: "old".to_string(),
                        label: "旧概念".to_string(),
                        detail: String::new(),
                        node_type: "structure".to_string(),
                        kind: "core".to_string(),
                        role: "main".to_string(),
                        parent_id: String::new(),
                        source_type: "lecture".to_string(),
                        source_excerpt: String::new(),
                        external_source: String::new(),
                    },
                    LiveWhiteboardNode {
                        id: "old-2".to_string(),
                        label: "旧補足".to_string(),
                        detail: String::new(),
                        node_type: "structure".to_string(),
                        kind: "support".to_string(),
                        role: "branch".to_string(),
                        parent_id: "old".to_string(),
                        source_type: "lecture".to_string(),
                        source_excerpt: String::new(),
                        external_source: String::new(),
                    },
                ],
                edges: Vec::new(),
                schema_version: 0,
                normalized_by: String::new(),
            }),
        },
        LiveSummaryChunk {
            title: "後半".to_string(),
            range_label: "10:05-10:10".to_string(),
            body: "新しい内容".to_string(),
            line_count: 4,
            terms: Vec::new(),
            whiteboard: Some(LiveWhiteboard {
                title: "更新後ボード".to_string(),
                layout: "flow".to_string(),
                nodes: vec![
                    LiveWhiteboardNode {
                        id: "old".to_string(),
                        label: "旧概念".to_string(),
                        detail: String::new(),
                        node_type: "structure".to_string(),
                        kind: "core".to_string(),
                        role: "main".to_string(),
                        parent_id: String::new(),
                        source_type: "lecture".to_string(),
                        source_excerpt: String::new(),
                        external_source: String::new(),
                    },
                    LiveWhiteboardNode {
                        id: "new".to_string(),
                        label: "新概念".to_string(),
                        detail: "追加".to_string(),
                        node_type: "structure".to_string(),
                        kind: "result".to_string(),
                        role: "branch".to_string(),
                        parent_id: "old".to_string(),
                        source_type: "lecture".to_string(),
                        source_excerpt: String::new(),
                        external_source: String::new(),
                    },
                ],
                edges: vec![LiveWhiteboardEdge {
                    from: "old".to_string(),
                    to: "new".to_string(),
                    label: "発展".to_string(),
                }],
                schema_version: 0,
                normalized_by: String::new(),
            }),
        },
    ];

    let context = format_latest_whiteboard_context(&summaries);
    assert!(context.contains("更新後ボード"));
    assert!(context.contains("新概念"));
    assert!(!context.contains("古いボード"));
}
