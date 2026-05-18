use super::{
    sanitize_model_output, LiveChunkAiResult, LiveSummaryChunk, LiveTermExplanation,
    LiveWhiteboard, LiveWhiteboardEdge, LiveWhiteboardNode, MAX_LIVE_TERM_EXPLANATION_CHARS,
    MAX_LIVE_WHITEBOARD_EDGES, MAX_LIVE_WHITEBOARD_NODES,
};

// ── Cumulative knowledge whiteboard ─────────────────────────────────────────
// The model is *prompted* to return the entire cumulative board with stable
// IDs on every segment. Nothing in the API contract forces that, though, so
// this module treats the model's output as *proposed updates* and reconciles
// them against what we already have. The data flow per segment is:
//
//   summarize_chunk → chunk_ai.whiteboard : Option<LiveWhiteboard>
//                    ↓
//   reconcile_whiteboard(previous, model_output) → Option<LiveWhiteboard>
//                    ↓                                    │
//             merge_whiteboard (Some)            carry-forward (None)
//
// Reading order below follows that flow: latest → reconcile → merge.

pub(super) fn latest_whiteboard<'a>(
    summaries: &'a [LiveSummaryChunk],
) -> Option<&'a LiveWhiteboard> {
    summaries
        .iter()
        .rev()
        .find_map(|chunk| chunk.whiteboard.as_ref())
}

pub(super) fn format_latest_whiteboard_context(summaries: &[LiveSummaryChunk]) -> String {
    latest_whiteboard(summaries)
        .and_then(|board| serde_json::to_string(board).ok())
        .unwrap_or_else(|| "なし".to_string())
}

/// Decide what whiteboard to persist for the current segment.
///
/// - `Some(new)` → merge with previous (preserve concepts the model silently
///   dropped, up to the node cap).
/// - `None`      → carry the previous board forward so the UI doesn't blink
///   out an existing visualization just because this chunk's AI call happened
///   to skip the field.
pub(super) fn reconcile_whiteboard(
    previous: Option<&LiveWhiteboard>,
    model_output: Option<LiveWhiteboard>,
) -> Option<LiveWhiteboard> {
    match model_output {
        Some(new_board) => Some(merge_whiteboard(previous, new_board)),
        None => {
            if previous.is_some() {
                eprintln!("[Live whiteboard] model returned no board; carrying previous forward");
            }
            previous.cloned()
        }
    }
}

/// Union-merge the model's proposed board into the previously accumulated one.
///
/// - Nodes the model re-emitted (same `id`) → model's version wins; labels,
///   details, kinds, and source metadata can all evolve.
/// - Nodes the model omitted → kept (oldest-first) up to the node cap;
///   concepts only fall out when we genuinely run out of room.
/// - Edges → union by `(from, to)`; edges whose endpoints didn't survive the
///   node merge are dropped.
/// - `title` and `layout` → follow the model (per-segment choices, not state).
pub(super) fn merge_whiteboard(
    previous: Option<&LiveWhiteboard>,
    current: LiveWhiteboard,
) -> LiveWhiteboard {
    let Some(prev) = previous else {
        return current;
    };

    use std::collections::HashSet;
    let current_ids: HashSet<String> = current.nodes.iter().map(|n| n.id.clone()).collect();

    let mut nodes = current.nodes;
    let mut preserved_nodes = 0usize;
    for prev_node in &prev.nodes {
        if nodes.len() >= MAX_LIVE_WHITEBOARD_NODES {
            break;
        }
        if current_ids.contains(&prev_node.id) {
            continue;
        }
        nodes.push(prev_node.clone());
        preserved_nodes += 1;
    }

    let known_ids: HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();
    let mut edges = current.edges;
    let mut seen_edges: HashSet<(String, String)> = edges
        .iter()
        .map(|e| (e.from.clone(), e.to.clone()))
        .collect();
    let mut preserved_edges = 0usize;
    for prev_edge in &prev.edges {
        if edges.len() >= MAX_LIVE_WHITEBOARD_EDGES {
            break;
        }
        let key = (prev_edge.from.clone(), prev_edge.to.clone());
        if seen_edges.contains(&key) {
            continue;
        }
        if !known_ids.contains(&prev_edge.from) || !known_ids.contains(&prev_edge.to) {
            continue;
        }
        seen_edges.insert(key);
        edges.push(prev_edge.clone());
        preserved_edges += 1;
    }

    if preserved_nodes > 0 || preserved_edges > 0 {
        eprintln!(
            "[Live whiteboard] merge: preserved {} node(s) and {} edge(s) the model omitted (now {} / {} total)",
            preserved_nodes,
            preserved_edges,
            nodes.len(),
            edges.len()
        );
    }

    LiveWhiteboard {
        title: current.title,
        layout: current.layout,
        nodes,
        edges,
    }
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
        for (idx, item) in items.iter().take(MAX_LIVE_WHITEBOARD_NODES).enumerate() {
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
            let kind = normalize_live_whiteboard_kind(&value_to_trimmed_string(item.get("kind")));
            nodes.push(LiveWhiteboardNode {
                id,
                label,
                detail: clamp_chars(&value_to_trimmed_string(item.get("detail")), 120),
                role: normalize_live_whiteboard_role(
                    &value_to_trimmed_string(item.get("role")),
                    &kind,
                    &value_to_trimmed_string(item.get("parent_id")),
                ),
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
        if let Some(first) = nodes.first_mut() {
            first.role = "main".to_string();
            first.parent_id.clear();
        }
    }

    let main_ids = nodes
        .iter()
        .filter(|node| node.role == "main")
        .map(|node| node.id.clone())
        .collect::<std::collections::HashSet<_>>();
    let fallback_main = main_ids.iter().next().cloned();
    for node in &mut nodes {
        if node.role == "main" {
            node.parent_id.clear();
            continue;
        }
        if node.parent_id == node.id || !main_ids.contains(&node.parent_id) {
            node.parent_id = fallback_main.clone().unwrap_or_default();
        }
    }
    let known_ids = nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut edges = Vec::new();
    if let Some(items) = board.get("edges").and_then(|v| v.as_array()) {
        for item in items.iter().take(MAX_LIVE_WHITEBOARD_EDGES) {
            let from = value_to_trimmed_string(item.get("from"));
            let to = value_to_trimmed_string(item.get("to"));
            if from == to || !known_ids.contains(from.as_str()) || !known_ids.contains(to.as_str())
            {
                continue;
            }
            edges.push(LiveWhiteboardEdge {
                from,
                to,
                label: clamp_chars(&value_to_trimmed_string(item.get("label")), 32),
            });
        }
    }

    Some(LiveWhiteboard {
        title: clamp_chars(&value_to_trimmed_string(board.get("title")), 40),
        layout: normalize_live_whiteboard_layout(&value_to_trimmed_string(board.get("layout"))),
        nodes,
        edges,
    })
}
