use super::{
    sanitize_model_output, LiveChunkAiResult, LiveSummaryChunk, LiveTermExplanation,
    LiveWhiteboard, LiveWhiteboardEdge, LiveWhiteboardNode, MAX_LIVE_TERM_EXPLANATION_CHARS,
};

// ── Cumulative knowledge whiteboard ─────────────────────────────────────────
// The model is *prompted* to return the clearest current cumulative board with
// stable IDs on every segment. When a segment returns a board, it is treated as
// authoritative so the model can prune stale nodes, merge duplicates, and
// simplify edges instead of the board only ever growing.
//
//   summarize_chunk → chunk_ai.whiteboard : Option<LiveWhiteboard>
//                    ↓
//   reconcile_whiteboard(previous, model_output) → Option<LiveWhiteboard>
//                    ↓                                    │
//             replace with new board             carry-forward (None)
//
// Reading order below follows that flow: latest → reconcile → parse.

pub(super) fn latest_whiteboard(
    summaries: &[LiveSummaryChunk],
) -> Option<&LiveWhiteboard> {
    summaries
        .iter()
        .rev()
        .find_map(|chunk| chunk.whiteboard.as_ref())
}

pub(super) fn format_latest_whiteboard_context(summaries: &[LiveSummaryChunk]) -> String {
    let board = match latest_whiteboard(summaries) {
        Some(b) => b,
        None => return "なし".to_string(),
    };

    // Build a compressed structural summary instead of dumping the full JSON.
    // This keeps the prompt lighter and prevents the model from being anchored
    // to stale detail/source_excerpt text.
    //
    // Format:
    //   title | layout
    //   [main] id: label (kind)
    //     [branch] id: label (kind)   ← structure branches only
    //     terms(N): term1, term2, …   ← term children collapsed per parent
    //   edges: A→B [label], …         ← cross-structure edges only

    let mut out = String::new();
    out.push_str("title: ");
    out.push_str(if board.title.is_empty() { "—" } else { &board.title });
    out.push_str(" | layout: ");
    out.push_str(&board.layout);
    out.push('\n');

    // Group term children by parent for compact display.
    let mut terms_by_parent: std::collections::HashMap<&str, Vec<&str>> =
        std::collections::HashMap::new();
    for node in &board.nodes {
        if node.node_type == "term" {
            terms_by_parent
                .entry(node.parent_id.as_str())
                .or_default()
                .push(&node.label);
        }
    }

    // Emit structure nodes: mains first, then their branches.
    let mains: Vec<_> = board
        .nodes
        .iter()
        .filter(|n| n.node_type != "term" && n.role == "main")
        .collect();
    let mut emitted: std::collections::HashSet<&str> = std::collections::HashSet::new();

    for main in &mains {
        if main.detail.is_empty() {
            out.push_str(&format!("[main] {}: {} ({})\n", main.id, main.label, main.kind));
        } else {
            out.push_str(&format!(
                "[main] {}: {} ({}) — {}\n",
                main.id,
                main.label,
                main.kind,
                clamp_chars(&main.detail, 60)
            ));
        }
        emitted.insert(main.id.as_str());

        let branches: Vec<_> = board
            .nodes
            .iter()
            .filter(|n| n.node_type != "term" && n.role != "main" && n.parent_id == main.id)
            .collect();
        for branch in &branches {
            if branch.detail.is_empty() {
                out.push_str(&format!(
                    "  [branch] {}: {} ({})\n",
                    branch.id, branch.label, branch.kind
                ));
            } else {
                out.push_str(&format!(
                    "  [branch] {}: {} ({}) — {}\n",
                    branch.id,
                    branch.label,
                    branch.kind,
                    clamp_chars(&branch.detail, 48)
                ));
            }
            emitted.insert(branch.id.as_str());

            // Terms under this branch.
            if let Some(terms) = terms_by_parent.get(branch.id.as_str()) {
                if !terms.is_empty() {
                    out.push_str(&format!(
                        "    terms({}): {}\n",
                        terms.len(),
                        terms.join(", ")
                    ));
                }
            }
        }

        // Terms directly under this main.
        if let Some(terms) = terms_by_parent.get(main.id.as_str()) {
            if !terms.is_empty() {
                out.push_str(&format!(
                    "  terms({}): {}\n",
                    terms.len(),
                    terms.join(", ")
                ));
            }
        }
    }

    // Any orphaned structure nodes (no main parent match).
    for node in &board.nodes {
        if node.node_type == "term" || emitted.contains(node.id.as_str()) {
            continue;
        }
        if node.detail.is_empty() {
            out.push_str(&format!(
                "[{}] {}: {} ({})\n",
                node.role, node.id, node.label, node.kind
            ));
        } else {
            out.push_str(&format!(
                "[{}] {}: {} ({}) — {}\n",
                node.role,
                node.id,
                node.label,
                node.kind,
                clamp_chars(&node.detail, 48)
            ));
        }
    }

    // Cross-structure edges (parent-child links are implicit from the tree above).
    let node_by_id: std::collections::HashMap<&str, &LiveWhiteboardNode> = board
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n))
        .collect();
    let cross_edges: Vec<String> = board
        .edges
        .iter()
        .filter(|e| {
            let from_node = node_by_id.get(e.from.as_str());
            let to_node = node_by_id.get(e.to.as_str());
            match (from_node, to_node) {
                (Some(f), Some(t)) => {
                    // Skip term edges and parent-child links (already implicit).
                    f.node_type != "term"
                        && t.node_type != "term"
                        && f.parent_id != t.id
                        && t.parent_id != f.id
                }
                _ => false,
            }
        })
        .map(|e| {
            if e.label.is_empty() {
                format!("{}→{}", e.from, e.to)
            } else {
                format!("{}→{} [{}]", e.from, e.to, e.label)
            }
        })
        .collect();
    if !cross_edges.is_empty() {
        out.push_str("edges: ");
        out.push_str(&cross_edges.join(", "));
        out.push('\n');
    }

    out
}

/// Decide what whiteboard to persist for the current segment.
///
/// - `Some(new)` → trust the model's full-board rewrite. Omitted nodes/edges
///   are considered intentionally removed so the structure can stay clear.
/// - `None`      → carry the previous board forward so the UI doesn't blink
///   out an existing visualization just because this chunk's AI call happened
///   to skip the field.
pub(super) fn reconcile_whiteboard(
    previous: Option<&LiveWhiteboard>,
    model_output: Option<LiveWhiteboard>,
) -> Option<LiveWhiteboard> {
    match model_output {
        Some(new_board) => {
            if let Some(prev) = previous {
                if should_keep_previous_whiteboard(prev, &new_board) {
                    // Individual guards emit their own diagnostic lines above;
                    // this just records the final carry-forward decision.
                    eprintln!(
                        "[Live whiteboard] guard triggered ({} -> {} nodes); carrying previous board forward",
                        prev.nodes.len(),
                        new_board.nodes.len()
                    );
                    return Some(prev.clone());
                }
            }
            Some(new_board)
        }
        None => {
            if previous.is_some() {
                eprintln!("[Live whiteboard] model returned no board; carrying previous forward");
            }
            previous.cloned()
        }
    }
}

fn should_keep_previous_whiteboard(previous: &LiveWhiteboard, current: &LiveWhiteboard) -> bool {
    let prev_total = previous.nodes.len();
    let curr_total = current.nodes.len();

    // Guard 1: extreme total shrink (12→5 style collapse).
    let extreme_shrink = prev_total >= 6 && curr_total < 4 && curr_total * 2 < prev_total;
    if extreme_shrink {
        return true;
    }

    // Guard 2: main-node retention. If the model output loses most structure
    // main nodes it almost certainly truncated output, not intentionally pruned.
    let prev_mains: std::collections::HashSet<&str> = previous
        .nodes
        .iter()
        .filter(|n| n.role == "main")
        .map(|n| n.id.as_str())
        .collect();
    let curr_main_ids: std::collections::HashSet<&str> = current
        .nodes
        .iter()
        .filter(|n| n.role == "main")
        .map(|n| n.id.as_str())
        .collect();
    if prev_mains.len() >= 2 {
        let retained = prev_mains.intersection(&curr_main_ids).count();
        // If fewer than half the previous main nodes survive by ID, treat as
        // suspicious. (ID-stable rewrite is an explicit model instruction.)
        if retained * 2 < prev_mains.len() {
            // Only block if the board wasn't also meaningfully growing.
            if curr_total <= prev_total {
                eprintln!(
                    "[Live whiteboard] main-node ID churn: {}/{} mains retained; blocking rewrite",
                    retained,
                    prev_mains.len()
                );
                return true;
            }
        }
    }

    // Guard 3: structure-node ID churn. If most structure node IDs vanish it
    // suggests a from-scratch rewrite rather than a targeted prune.
    let prev_struct_ids: std::collections::HashSet<&str> = previous
        .nodes
        .iter()
        .filter(|n| n.node_type != "term")
        .map(|n| n.id.as_str())
        .collect();
    if prev_struct_ids.len() >= 4 {
        let curr_struct_ids: std::collections::HashSet<&str> = current
            .nodes
            .iter()
            .filter(|n| n.node_type != "term")
            .map(|n| n.id.as_str())
            .collect();
        let retained = prev_struct_ids.intersection(&curr_struct_ids).count();
        // If fewer than 1/3 of structure node IDs are kept and the board isn't
        // growing, assume output truncation.
        if retained * 3 < prev_struct_ids.len() && curr_total <= prev_total {
            eprintln!(
                "[Live whiteboard] structure-node ID churn: {}/{} IDs retained; blocking rewrite",
                retained,
                prev_struct_ids.len()
            );
            return true;
        }
    }

    // Guard 4: cross-structure edge churn. If the board had meaningful
    // cross-group connections and the model outputs none at all while not
    // growing, the edges array was likely truncated rather than deliberately pruned.
    let prev_cross = count_cross_structure_edges(previous);
    if prev_cross >= 2 {
        let curr_cross = count_cross_structure_edges(current);
        if curr_cross == 0 && curr_total <= prev_total {
            eprintln!(
                "[Live whiteboard] edge churn: {prev_cross} cross-edges → 0; blocking rewrite"
            );
            return true;
        }
    }

    false
}

fn count_cross_structure_edges(board: &LiveWhiteboard) -> usize {
    let node_by_id: std::collections::HashMap<&str, &LiveWhiteboardNode> =
        board.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    board
        .edges
        .iter()
        .filter(|e| {
            let from = node_by_id.get(e.from.as_str());
            let to = node_by_id.get(e.to.as_str());
            match (from, to) {
                (Some(f), Some(t)) => {
                    f.node_type != "term"
                        && t.node_type != "term"
                        && f.parent_id != t.id
                        && t.parent_id != f.id
                }
                _ => false,
            }
        })
        .count()
}

pub(super) fn extract_json_object(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let start = bytes.iter().position(|b| *b == b'{')?;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (idx, b) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if *b == b'\\' {
                escaped = true;
            } else if *b == b'"' {
                in_string = false;
            }
            continue;
        }
        match *b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return text.get(start..=idx);
                }
            }
            _ => {}
        }
    }
    None
}

pub(super) fn value_to_trimmed_string(value: Option<&serde_json::Value>) -> String {
    match value {
        Some(serde_json::Value::String(s)) => s.trim().to_string(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

pub(super) fn clamp_chars(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out = trimmed.chars().take(max_chars).collect::<String>();
    out.push('…');
    out
}

fn is_low_value_live_term(term: &str) -> bool {
    let normalized = term
        .trim()
        .trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\''
                    | '`'
                    | '「'
                    | '」'
                    | '『'
                    | '』'
                    | '（'
                    | '）'
                    | '('
                    | ')'
                    | '【'
                    | '】'
                    | '['
                    | ']'
            )
        })
        .to_lowercase();
    if normalized.chars().count() <= 1 {
        return true;
    }
    const LOW_VALUE_TERMS: &[&str] = &[
        "授業",
        "講義",
        "先生",
        "教員",
        "教授",
        "学生",
        "大学",
        "教室",
        "出席",
        "欠席",
        "課題",
        "宿題",
        "レポート",
        "資料",
        "教科書",
        "スライド",
        "今日",
        "次回",
        "明日",
        "来週",
        "学校",
        "勉強",
        "学習",
        "考试",
        "作业",
        "报告",
        "老师",
        "学生",
        "大学",
        "教室",
        "今天",
        "下次",
        "tomorrow",
        "today",
        "class",
        "lecture",
        "teacher",
        "student",
        "assignment",
        "report",
        "homework",
        "textbook",
        "slides",
    ];
    LOW_VALUE_TERMS
        .iter()
        .any(|candidate| normalized == *candidate)
}

pub(super) fn parse_chunk_ai_result(raw: &str) -> LiveChunkAiResult {
    let sanitized = sanitize_model_output(raw);
    let Some(json_text) = extract_json_object(&sanitized) else {
        return LiveChunkAiResult {
            body: sanitized,
            terms: Vec::new(),
            whiteboard: None,
        };
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json_text) else {
        return LiveChunkAiResult {
            body: sanitized,
            terms: Vec::new(),
            whiteboard: None,
        };
    };

    let body = value_to_trimmed_string(
        value
            .get("summary_markdown")
            .or_else(|| value.get("summary"))
            .or_else(|| value.get("body")),
    );
    let mut terms = Vec::new();
    if let Some(items) = value.get("terms").and_then(|v| v.as_array()) {
        for item in items.iter().take(5) {
            let term = clamp_chars(&value_to_trimmed_string(item.get("term")), 40);
            let explanation = clamp_chars(
                &value_to_trimmed_string(item.get("explanation")),
                MAX_LIVE_TERM_EXPLANATION_CHARS,
            );
            if term.is_empty() || explanation.is_empty() || is_low_value_live_term(&term) {
                continue;
            }
            terms.push(LiveTermExplanation {
                term,
                explanation,
                source_excerpt: clamp_chars(
                    &value_to_trimmed_string(item.get("source_excerpt")),
                    80,
                ),
                external_source: clamp_chars(
                    &value_to_trimmed_string(item.get("external_source")),
                    180,
                ),
            });
        }
    }
    let whiteboard = parse_live_whiteboard(value.get("whiteboard"));

    LiveChunkAiResult {
        body: if body.is_empty() { sanitized } else { body },
        terms,
        whiteboard,
    }
}

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

fn parse_live_whiteboard(value: Option<&serde_json::Value>) -> Option<LiveWhiteboard> {
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
    let mut cross_structure_edges = 0usize;
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
            let from_group = if from_node.role == "main" {
                from_node.id.as_str()
            } else {
                from_node.parent_id.as_str()
            };
            let to_group = if to_node.role == "main" {
                to_node.id.as_str()
            } else {
                to_node.parent_id.as_str()
            };
            let cross_group = !parent_link
                && !from_group.is_empty()
                && !to_group.is_empty()
                && from_group != to_group;
            if cross_group {
                if cross_structure_edges >= 3 {
                    continue;
                }
                cross_structure_edges += 1;
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

#[cfg(test)]
mod tests {
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
    fn parse_whiteboard_limits_noisy_structure_edges() {
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

        assert_eq!(board.edges.len(), 3);
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
        assert!(!board.edges.iter().any(|edge| edge.label == "関係4"));
        assert!(!board.edges.iter().any(|edge| edge.label == "重複"));
    }
}
