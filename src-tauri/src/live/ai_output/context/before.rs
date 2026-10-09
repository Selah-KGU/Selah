// Frozen predecessor for byte-for-byte regression and local benchmarks only.
//! Cumulative whiteboard context for the next model call.

use super::*;
use crate::live::SharedSummaryChunk;

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

pub fn latest_whiteboard(summaries: &[SharedSummaryChunk]) -> Option<&LiveWhiteboard> {
    summaries
        .iter()
        .rev()
        .find_map(|chunk| chunk.whiteboard.as_deref())
}

pub fn format_latest_whiteboard_context(summaries: &[SharedSummaryChunk]) -> String {
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
    out.push_str(if board.title.is_empty() {
        "—"
    } else {
        &board.title
    });
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
            out.push_str(&format!(
                "[main] {}: {} ({})\n",
                main.id, main.label, main.kind
            ));
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
                out.push_str(&format!("  terms({}): {}\n", terms.len(), terms.join(", ")));
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
    let node_by_id: std::collections::HashMap<&str, &LiveWhiteboardNode> =
        board.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
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

fn clamp_chars(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out = trimmed.chars().take(max_chars).collect::<String>();
    out.push('…');
    out
}
