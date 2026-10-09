// Frozen parser for regression and local benchmarks only.
//! Normalize and parse the cumulative knowledge whiteboard.

use super::*;

fn normalize_live_whiteboard_layout(layout: &str) -> String {
    match layout.trim().to_ascii_lowercase().as_str() {
        "flow" | "hub" | "compare" | "cycle" | "grid" => layout.trim().to_ascii_lowercase(),
        _ => "grid".to_string(),
    }
}

fn normalize_live_whiteboard_kind(kind: &str) -> String {
    match kind.trim().to_ascii_lowercase().as_str() {
        "core" | "support" | "question" | "result" => kind.trim().to_ascii_lowercase(),
        _ => "support".to_string(),
    }
}

fn normalize_live_whiteboard_node_type(node_type: &str) -> String {
    match node_type.trim().to_ascii_lowercase().as_str() {
        "term" | "terminology" | "keyword" | "small" => "term".to_string(),
        _ => "structure".to_string(),
    }
}

fn normalize_live_whiteboard_role(role: &str, kind: &str, parent_id: &str) -> String {
    match role.trim().to_ascii_lowercase().as_str() {
        "main" | "primary" | "trunk" | "core" => "main".to_string(),
        "branch" | "detail" | "leaf" | "support" => "branch".to_string(),
        _ if kind == "core" && parent_id.trim().is_empty() => "main".to_string(),
        _ => "branch".to_string(),
    }
}

fn normalize_live_whiteboard_source_type(source_type: &str, external_source: &str) -> String {
    match source_type.trim().to_ascii_lowercase().as_str() {
        "external" | "outside" | "reference" => "external".to_string(),
        "lecture" | "class" | "internal" => "lecture".to_string(),
        _ if !external_source.trim().is_empty() => "external".to_string(),
        _ => "lecture".to_string(),
    }
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
            let label = clamp_chars(&value_to_trimmed_string(item.get("label")), 36);
            if label.is_empty() {
                continue;
            }
            let mut id =
                normalize_live_whiteboard_id(&value_to_trimmed_string(item.get("id")), idx);
            if seen_ids.contains(&id) {
                id = format!("{}-{}", id, idx + 1);
            }
            seen_ids.insert(id.clone());
            let parent_id =
                normalize_live_whiteboard_id(&value_to_trimmed_string(item.get("parent_id")), idx);
            let external_source =
                clamp_chars(&value_to_trimmed_string(item.get("external_source")), 140);
            let node_type = normalize_live_whiteboard_node_type(&value_to_trimmed_string(
                item.get("node_type"),
            ));
            let mut kind =
                normalize_live_whiteboard_kind(&value_to_trimmed_string(item.get("kind")));
            if node_type == "term" {
                kind = "support".to_string();
            }
            nodes.push(LiveWhiteboardNode {
                id,
                label,
                detail: clamp_chars(&value_to_trimmed_string(item.get("detail")), 120),
                node_type: node_type.clone(),
                role: if node_type == "term" {
                    "branch".to_string()
                } else {
                    normalize_live_whiteboard_role(
                        &value_to_trimmed_string(item.get("role")),
                        &kind,
                        &value_to_trimmed_string(item.get("parent_id")),
                    )
                },
                parent_id,
                kind,
                source_type: normalize_live_whiteboard_source_type(
                    &value_to_trimmed_string(item.get("source_type")),
                    &external_source,
                ),
                source_excerpt: clamp_chars(
                    &value_to_trimmed_string(item.get("source_excerpt")),
                    80,
                ),
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
    let known_ids = nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let node_by_id = nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<std::collections::HashMap<_, _>>();
    let mut edges = Vec::new();
    let mut seen_term_edges = std::collections::HashSet::new();
    let mut seen_structure_pairs = std::collections::HashSet::new();
    if let Some(items) = board.get("edges").and_then(|v| v.as_array()) {
        for item in items.iter() {
            let from = value_to_trimmed_string(item.get("from"));
            let to = value_to_trimmed_string(item.get("to"));
            if from == to || !known_ids.contains(from.as_str()) || !known_ids.contains(to.as_str())
            {
                continue;
            }
            let from_node = node_by_id.get(from.as_str());
            let to_node = node_by_id.get(to.as_str());
            let term_node = match (from_node, to_node) {
                (Some(node), _) if node.node_type == "term" => Some(*node),
                (_, Some(node)) if node.node_type == "term" => Some(*node),
                _ => None,
            };
            if let Some(term) = term_node {
                let other = if from == term.id {
                    to.as_str()
                } else {
                    from.as_str()
                };
                if other != term.parent_id {
                    continue;
                }
                if !seen_term_edges.insert(term.id.clone()) {
                    continue;
                }
                edges.push(LiveWhiteboardEdge {
                    from: term.parent_id.clone(),
                    to: term.id.clone(),
                    label: String::new(),
                });
                continue;
            }
            let from_node = *from_node.expect("known from node");
            let to_node = *to_node.expect("known to node");
            let label = clamp_chars(&value_to_trimmed_string(item.get("label")), 32);
            let parent_link = from_node.parent_id == to || to_node.parent_id == from;
            if label.is_empty() && !parent_link {
                continue;
            }
            let pair = if from <= to {
                (from.clone(), to.clone())
            } else {
                (to.clone(), from.clone())
            };
            if !seen_structure_pairs.insert(pair) {
                continue;
            }
            edges.push(LiveWhiteboardEdge { from, to, label });
        }
    }
    for node in &nodes {
        if node.node_type != "term" || node.parent_id.is_empty() {
            continue;
        }
        if seen_term_edges.contains(&node.id) {
            continue;
        }
        edges.push(LiveWhiteboardEdge {
            from: node.parent_id.clone(),
            to: node.id.clone(),
            label: String::new(),
        });
        seen_term_edges.insert(node.id.clone());
    }

    Some(LiveWhiteboard {
        title: clamp_chars(&value_to_trimmed_string(board.get("title")), 40),
        layout: normalize_live_whiteboard_layout(&value_to_trimmed_string(board.get("layout"))),
        nodes,
        edges,
        schema_version: 1,
        normalized_by: "backend".to_string(),
    })
}

fn value_to_trimmed_string(value: Option<&serde_json::Value>) -> String {
    match value {
        Some(serde_json::Value::String(s)) => s.trim().to_string(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

pub fn clamp_chars(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out = trimmed.chars().take(max_chars).collect::<String>();
    out.push('…');
    out
}
