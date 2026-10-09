//! Normalize and parse the cumulative knowledge whiteboard.

use super::*;

// Model strings can be borrowed from the JSON; numbers retain the previous
// string conversion, and all other JSON kinds still behave as empty fields.
fn value_text(value: Option<&serde_json::Value>) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    match value {
        Some(serde_json::Value::String(text)) => Cow::Borrowed(text.trim()),
        Some(serde_json::Value::Number(number)) => Cow::Owned(number.to_string()),
        _ => Cow::Borrowed(""),
    }
}

fn clamped_text(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    let end = trimmed.char_indices().nth(limit).map(|(end, _)| end);
    let prefix = &trimmed[..end.unwrap_or(trimmed.len())];
    let mut out =
        String::with_capacity(prefix.len() + if end.is_some() { '…'.len_utf8() } else { 0 });
    out.push_str(prefix);
    if end.is_some() {
        out.push('…');
    }
    out
}

fn clamped_value(value: Option<&serde_json::Value>, limit: usize) -> String {
    clamped_text(&value_text(value), limit)
}

fn canonical_enum(
    value: &str,
    aliases: &[(&str, &'static str)],
    fallback: &'static str,
) -> &'static str {
    let value = value.trim();
    aliases
        .iter()
        .find(|(alias, _)| value.eq_ignore_ascii_case(alias))
        .map_or(fallback, |(_, canonical)| *canonical)
}

fn normalize_live_whiteboard_layout(layout: &str) -> &'static str {
    canonical_enum(
        layout,
        &[
            ("flow", "flow"),
            ("hub", "hub"),
            ("compare", "compare"),
            ("cycle", "cycle"),
            ("grid", "grid"),
        ],
        "grid",
    )
}

fn normalize_live_whiteboard_kind(kind: &str) -> &'static str {
    canonical_enum(
        kind,
        &[
            ("core", "core"),
            ("support", "support"),
            ("question", "question"),
            ("result", "result"),
        ],
        "support",
    )
}

fn normalize_live_whiteboard_node_type(node_type: &str) -> &'static str {
    canonical_enum(
        node_type,
        &[
            ("term", "term"),
            ("terminology", "term"),
            ("keyword", "term"),
            ("small", "term"),
        ],
        "structure",
    )
}

fn normalize_live_whiteboard_role(role: &str, kind: &str, parent_id: &str) -> &'static str {
    canonical_enum(
        role,
        &[
            ("main", "main"),
            ("primary", "main"),
            ("trunk", "main"),
            ("core", "main"),
            ("branch", "branch"),
            ("detail", "branch"),
            ("leaf", "branch"),
            ("support", "branch"),
        ],
        if kind == "core" && parent_id.trim().is_empty() {
            "main"
        } else {
            "branch"
        },
    )
}

fn normalize_live_whiteboard_source_type(source_type: &str, external_source: &str) -> &'static str {
    canonical_enum(
        source_type,
        &[
            ("external", "external"),
            ("outside", "external"),
            ("reference", "external"),
            ("lecture", "lecture"),
            ("class", "lecture"),
            ("internal", "lecture"),
        ],
        if external_source.trim().is_empty() {
            "lecture"
        } else {
            "external"
        },
    )
}

fn normalize_live_whiteboard_id(id: &str, fallback_index: usize) -> String {
    let mut out = id
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(24)
        .collect::<String>();
    if out.is_empty() {
        out = format!("n{}", fallback_index + 1);
    }
    out
}

pub(super) fn parse_live_whiteboard(value: Option<&serde_json::Value>) -> Option<LiveWhiteboard> {
    let board = value?.as_object()?;
    let mut nodes = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();
    if let Some(items) = board.get("nodes").and_then(|v| v.as_array()) {
        for (idx, item) in items.iter().enumerate() {
            let label = clamped_value(item.get("label"), 36);
            if label.is_empty() {
                continue;
            }
            let mut id = normalize_live_whiteboard_id(&value_text(item.get("id")), idx);
            if seen_ids.contains(&id) {
                id = format!("{}-{}", id, idx + 1);
                // The usual suffix may itself be an existing model ID. Keep
                // every node, but never publish duplicate renderer keys.
                if seen_ids.contains(&id) {
                    let candidate = id;
                    let mut collision = 2;
                    loop {
                        id = format!("{candidate}-{collision}");
                        if !seen_ids.contains(&id) {
                            break;
                        }
                        collision += 1;
                    }
                }
            }
            seen_ids.insert(id.clone());
            let raw_parent = value_text(item.get("parent_id"));
            let parent_id = normalize_live_whiteboard_id(&raw_parent, idx);
            let external_source = clamped_value(item.get("external_source"), 140);
            let node_type = normalize_live_whiteboard_node_type(&value_text(item.get("node_type")));
            let kind = if node_type == "term" {
                "support"
            } else {
                normalize_live_whiteboard_kind(&value_text(item.get("kind")))
            };
            let role = if node_type == "term" {
                "branch"
            } else {
                normalize_live_whiteboard_role(&value_text(item.get("role")), kind, &raw_parent)
            };
            nodes.push(LiveWhiteboardNode {
                id,
                label,
                detail: clamped_value(item.get("detail"), 120),
                node_type: node_type.into(),
                role: role.into(),
                parent_id,
                kind: kind.into(),
                source_type: normalize_live_whiteboard_source_type(
                    &value_text(item.get("source_type")),
                    &external_source,
                )
                .into(),
                source_excerpt: clamped_value(item.get("source_excerpt"), 80),
                external_source,
            });
        }
    }
    if nodes.len() < 2 {
        return None;
    }
    if !nodes.iter().any(|node| node.role == "main") {
        let first_structure_idx = nodes.iter().position(|node| node.node_type != "term")?;
        if let Some(first) = nodes.get_mut(first_structure_idx) {
            first.role = "main".to_string();
            first.parent_id.clear();
        }
    }

    let main_ids = nodes
        .iter()
        .filter(|node| node.role == "main")
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    let main_id_set = main_ids
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    let structure_id_set = nodes
        .iter()
        .filter(|node| node.node_type != "term")
        .map(|node| node.id.clone())
        .collect::<std::collections::HashSet<_>>();
    let fallback_main = main_ids.first().cloned();
    for node in &mut nodes {
        if node.role == "main" {
            node.parent_id.clear();
            continue;
        }
        if node.node_type == "term" {
            if node.parent_id == node.id || !structure_id_set.contains(&node.parent_id) {
                node.parent_id.clear();
            }
        } else if node.parent_id == node.id || !main_id_set.contains(&node.parent_id) {
            node.parent_id = fallback_main.clone().unwrap_or_default();
        }
    }
    if nodes.len() < 2 {
        return None;
    }
    let node_by_id = nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<std::collections::HashMap<_, _>>();
    let mut edges = Vec::new();
    let mut seen_term_edges = std::collections::HashSet::new();
    let mut seen_structure_pairs = std::collections::HashSet::new();
    if let Some(items) = board.get("edges").and_then(|v| v.as_array()) {
        for item in items.iter() {
            let from = value_text(item.get("from"));
            let to = value_text(item.get("to"));
            if from == to {
                continue;
            }
            let (Some(&from_node), Some(&to_node)) =
                (node_by_id.get(from.as_ref()), node_by_id.get(to.as_ref()))
            else {
                continue;
            };
            let term_node = if from_node.node_type == "term" {
                Some(from_node)
            } else if to_node.node_type == "term" {
                Some(to_node)
            } else {
                None
            };
            if let Some(term) = term_node {
                let other = if from.as_ref() == term.id {
                    to.as_ref()
                } else {
                    from.as_ref()
                };
                if other != term.parent_id || !seen_term_edges.insert(term.id.as_str()) {
                    continue;
                }
                edges.push(LiveWhiteboardEdge {
                    from: term.parent_id.clone(),
                    to: term.id.clone(),
                    label: String::new(),
                });
                continue;
            }
            let raw_label = value_text(item.get("label"));
            let parent_link =
                from_node.parent_id == to.as_ref() || to_node.parent_id == from.as_ref();
            if raw_label.is_empty() && !parent_link {
                continue;
            }
            // Borrow canonical node IDs for dedup; numeric JSON endpoints may
            // have temporary owned text, while these IDs outlive this traversal.
            let pair = if from <= to {
                (from_node.id.as_str(), to_node.id.as_str())
            } else {
                (to_node.id.as_str(), from_node.id.as_str())
            };
            if !seen_structure_pairs.insert(pair) {
                continue;
            }
            edges.push(LiveWhiteboardEdge {
                from: from.into_owned(),
                to: to.into_owned(),
                label: clamped_text(&raw_label, 32),
            });
        }
    }
    for node in &nodes {
        if node.node_type != "term" || node.parent_id.is_empty() {
            continue;
        }
        if seen_term_edges.contains(node.id.as_str()) {
            continue;
        }
        edges.push(LiveWhiteboardEdge {
            from: node.parent_id.clone(),
            to: node.id.clone(),
            label: String::new(),
        });
        seen_term_edges.insert(node.id.as_str());
    }

    Some(LiveWhiteboard {
        title: clamped_value(board.get("title"), 40),
        layout: normalize_live_whiteboard_layout(&value_text(board.get("layout"))).into(),
        nodes,
        edges,
        schema_version: 1,
        normalized_by: "backend".to_string(),
    })
}

#[cfg(test)]
#[path = "board/tests.rs"]
mod tests;
