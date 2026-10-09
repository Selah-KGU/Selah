//! Generated model JSON only; no application or user data.
use serde_json::{json, Value};

pub(super) fn board(count: usize, seed: usize) -> Value {
    let layouts = [
        json!(" FLOW "),
        json!("Hub"),
        json!("compare"),
        json!("CYCLE"),
        json!("grid"),
        json!("unknown"),
        json!(null),
    ];
    let kinds = [
        json!(" CORE "),
        json!("support"),
        json!("Question"),
        json!("RESULT"),
        json!("unknown"),
        json!(123),
        json!(true),
    ];
    let types = [
        json!("structure"),
        json!(" TERM "),
        json!("Terminology"),
        json!("keyword"),
        json!("small"),
        json!(null),
        json!([]),
    ];
    let roles = [
        json!(" MAIN "),
        json!("primary"),
        json!("trunk"),
        json!("core"),
        json!("branch"),
        json!("detail"),
        json!("leaf"),
        json!("support"),
        json!(null),
        json!({}),
    ];
    let sources = [
        json!("Lecture"),
        json!("CLASS"),
        json!("internal"),
        json!(" external "),
        json!("outside"),
        json!("reference"),
        json!(false),
        json!(99),
    ];
    let mut nodes: Vec<_> = (0..count).map(|i| {
        let mut node = json!({
            "id": if seed % 5 == 0 { format!(" n!{} ", i % 3) } else { format!("n{i}") },
            "label": format!(" 概念 {i} 日本語 🌕 "),
            "parent_id": if i % 4 == 0 { String::new() } else { format!("n{}", i / 4 * 4) },
            "node_type": types[(i + seed) % types.len()], "kind": kinds[(i + seed) % kinds.len()],
            "role": roles[(i + seed) % roles.len()], "source_type": sources[(i + seed) % sources.len()],
            "detail": " 前後空白 中文・日本語 👩🏽‍💻\n".repeat((i + seed) % 20),
            "source_excerpt": "\t 引用\" 日本語\n".repeat((i + seed) % 12),
            "external_source": " 出典 🌕 ".repeat((i + seed) % 30)
        });
        if seed % 3 == 0 && i % 5 == 0 { node["parent_id"] = json!("missing"); }
        if seed % 7 == 0 && i % 6 == 0 { node["label"] = json!(null); }
        if seed % 11 == 0 && i % 4 == 0 { node["id"] = json!(i); node["parent_id"] = json!(i / 4 * 4); }
        if seed % 13 == 0 && i % 3 == 0 { node["label"] = json!(123.456); node["detail"] = json!(-10); }
        if seed % 17 == 0 && i % 7 == 0 { node = json!("invalid node"); }
        node
    }).collect();
    if count > 0 {
        nodes.rotate_left(seed % count);
    }
    let mut edges = Vec::new();
    for i in 0..count {
        let from = format!("n{i}");
        let to = format!("n{}", (i + 1) % count);
        edges.push(json!({"from": from, "to": to, "label": if i % 3 == 0 { "".into() } else { "関係 中文 🌕 ".repeat(i % 10 + 1) }}));
        edges.push(json!({"from": to, "to": from, "label": "reverse duplicate"}));
        edges.push(json!({"from": from, "to": from, "label": "self"}));
        if seed % 2 == 0 {
            edges.push(json!({"from": i, "to": i + 1, "label": 123}));
        }
    }
    edges.extend([
        json!({"from":"missing","to":"n0","label":"invalid"}),
        json!(false),
        json!(null),
    ]);
    json!({"title": " 累積ボード 🌕 ", "layout": layouts[seed % 7], "nodes": nodes, "edges": edges})
}
