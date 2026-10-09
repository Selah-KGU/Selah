use super::*;
use serde_json::{json, Value};
#[path = "before.rs"]
mod before;
#[path = "fixtures.rs"]
mod fixtures;

fn parsed(value: &Value) -> Value {
    serde_json::to_value(parse_live_whiteboard(Some(value))).unwrap()
}

#[test]
fn parser_matches_frozen_output_for_672_generated_model_boards() {
    for seed in 0..96 {
        for count in [0, 1, 2, 7, 31, 128, 512] {
            let input = fixtures::board(count, seed);
            let original = serde_json::to_vec(&input).unwrap();
            assert_eq!(
                serde_json::to_vec(&parse_live_whiteboard(Some(&input))).unwrap(),
                serde_json::to_vec(&before::parse_live_whiteboard(Some(&input))).unwrap(),
                "count={count} seed={seed}"
            );
            assert_eq!(serde_json::to_vec(&input).unwrap(), original);
        }
    }
}

#[test]
fn parser_preserves_aliases_parent_normalization_and_term_edge_priority() {
    let input = json!({"layout":" FLOW ","title":" board ","nodes":[
        {"id":" m!1 ","label":" 中心 ","kind":" CORE ","role":" PRIMARY ","node_type":"unknown"},
        {"id":"b1","label":" 枝 ","kind":"QUESTION","role":" LEAF ","parent_id":"m1"},
        {"id":"t1","label":" 用語 ","kind":"result","node_type":"KEYWORD","role":"MAIN","parent_id":"b1","source_type":"OUTSIDE","source_excerpt":123}
    ],"edges":[
        {"from":" m1 ","to":"b1","label":" "},
        {"from":"b1","to":"m1","label":"reversed duplicate"},
        {"from":"t1","to":"b1","label":"ignored term label"},
        {"from":"t1","to":"m1","label":"invalid term link"},
        {"from":"absent","to":"b1","label":"missing"}
    ]});
    assert_eq!(
        parsed(&input),
        json!({"title":"board","layout":"flow","schema_version":1,"normalized_by":"backend","nodes":[
        {"id":"m1","label":"中心","detail":"","kind":"core","role":"main","node_type":"structure","parent_id":"","source_type":"lecture","source_excerpt":"","external_source":""},
        {"id":"b1","label":"枝","detail":"","kind":"question","role":"branch","node_type":"structure","parent_id":"m1","source_type":"lecture","source_excerpt":"","external_source":""},
        {"id":"t1","label":"用語","detail":"","kind":"support","role":"branch","node_type":"term","parent_id":"b1","source_type":"external","source_excerpt":"123","external_source":""}
    ],"edges":[{"from":"m1","to":"b1","label":""},{"from":"b1","to":"t1","label":""}]})
    );
}

#[test]
fn parser_keeps_unicode_scalar_boundaries_and_number_field_conversion() {
    for length in [
        0, 1, 31, 32, 33, 35, 36, 37, 39, 40, 41, 79, 80, 81, 119, 120, 121, 139, 140, 141,
    ] {
        let text: String = "日🙂e\u{301}👩🏽‍💻".chars().cycle().take(length).collect();
        let raw = format!("\u{2003}{text}\t\n");
        let expected = |limit| {
            let mut s: String = text.chars().take(limit).collect();
            if length > limit {
                s.push('…');
            }
            s
        };
        let input = json!({"title":raw,"nodes":[
            {"id":"main","label": if length == 0 { "名前" } else { &raw },"detail":raw,"source_excerpt":raw,"external_source":raw,"role":"main"},
            {"id":"branch","label":1.25,"detail":-123,"parent_id":"main"}
        ],"edges":[{"from":"main","to":"branch","label":raw}]});
        let result = parsed(&input);
        assert_eq!(result["title"], expected(40));
        let first = &result["nodes"][0];
        if length > 0 {
            assert_eq!(first["label"], expected(36));
        }
        assert_eq!(first["detail"], expected(120));
        assert_eq!(first["source_excerpt"], expected(80));
        assert_eq!(first["external_source"], expected(140));
        assert_eq!(result["nodes"][1]["label"], "1.25");
        assert_eq!(result["nodes"][1]["detail"], "-123");
        assert_eq!(result["edges"][0]["label"], expected(32));
    }
}

#[test]
fn parser_keeps_missing_nonobject_all_term_and_filtered_node_behavior() {
    assert!(parse_live_whiteboard(None).is_none());
    for input in [
        json!(null),
        json!([]),
        json!(true),
        json!({}),
        json!({"nodes":[{"label":"one"}]}),
        json!({"nodes":[{"label":"","id":"empty"},{"label":"one"}]}),
        json!({"nodes":[{"label":"one","node_type":"term"},{"label":"two","node_type":"term"}]}),
    ] {
        assert!(parse_live_whiteboard(Some(&input)).is_none());
    }
    let input = json!({"nodes":[{"id":"1","label":"one"},{"id":"2","label":"two"}],"edges":[
        {"from":1,"to":2,"label":123},{"from":"2","to":"1","label":"duplicate"}]});
    let result = parsed(&input);
    assert_eq!(result["nodes"][0]["role"], "main");
    assert_eq!(result["nodes"][1]["parent_id"], "1");
    assert_eq!(
        result["edges"],
        json!([{"from":"1","to":"2","label":"123"}])
    );
}

#[test]
fn parser_resolves_suffix_collisions_without_reassigning_existing_endpoint_ids() {
    let input = json!({"nodes":[
        {"id":"a-3","label":"first","role":"main"},
        {"id":"a","label":"second","role":"main"},
        {"id":"a","label":"collision","role":"branch"},
        {"id":"term","label":"term","node_type":"term","parent_id":"a-3"}
    ],"edges":[
        {"from":"a-3","to":"a","label":"collision edge"},
        {"from":"a","to":"a-3","label":"reversed duplicate"},
        {"from":"term","to":"a-3","label":"term edge"}
    ]});
    let result = parsed(&input);
    let previous = serde_json::to_value(before::parse_live_whiteboard(Some(&input))).unwrap();
    assert_eq!(previous["nodes"][2]["id"], "a-3");
    let ids: Vec<_> = result["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["a-3", "a", "a-3-2", "term"]);
    assert_eq!(result["nodes"][2]["parent_id"], "a-3");
    assert_eq!(
        result["edges"],
        json!([
            {"from":"a-3","to":"a","label":"collision edge"},
            {"from":"a-3","to":"term","label":""}
        ])
    );
}

#[test]
fn parser_retries_suffixes_until_unique_without_dropping_nodes() {
    let input = json!({"nodes":[
        {"id":"a-4","label":"first","role":"main"},
        {"id":"a-4-2","label":"reserved suffix","role":"main"},
        {"id":"a","label":"original","role":"main"},
        {"id":"a","label":"collision","role":"branch"}
    ]});
    let result = parse_live_whiteboard(Some(&input)).unwrap();
    let ids: Vec<_> = result.nodes.iter().map(|node| node.id.as_str()).collect();
    assert_eq!(ids, ["a-4", "a-4-2", "a", "a-4-3"]);
    assert_eq!(
        ids.iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        4
    );
    assert_eq!(result.nodes[3].parent_id, "a-4");
    assert_eq!(result.nodes[3].label, "collision");
}

#[test]
fn parser_keeps_every_node_in_a_large_model_board_and_owns_result_after_input_drop() {
    let input = fixtures::board(4096, 1);
    let expected = serde_json::to_vec(&before::parse_live_whiteboard(Some(&input))).unwrap();
    let result = parse_live_whiteboard(Some(&input)).unwrap();
    assert_eq!(result.nodes.len(), 4096);
    drop(input);
    assert_eq!(serde_json::to_vec(&result).unwrap(), expected);
}
