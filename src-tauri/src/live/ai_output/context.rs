//! Cumulative whiteboard context for the next model call.

use super::*;
use crate::live::SharedSummaryChunk;

// ── Cumulative knowledge whiteboard ─────────────────────────────────────────
// The model is *prompted* to return the clearest current cumulative board with
// stable IDs on every segment. When a segment returns a board, it is treated as
// authoritative so the model can prune stale nodes, merge duplicates, and
// simplify edges instead of the board only ever growing.
//
//   summarize_chunk → chunk_ai.whiteboard : Option<SharedWhiteboard>
//                    ↓
//   reconcile_whiteboard(previous, model_output) → Option<SharedWhiteboard>
//                    ↓                                    │
//             replace with new board             carry-forward (None)
//
// Reading order below follows that flow: latest → reconcile → parse.

pub fn latest_shared_whiteboard(summaries: &[SharedSummaryChunk]) -> Option<&SharedWhiteboard> {
    summaries
        .iter()
        .rev()
        .find_map(|chunk| chunk.whiteboard.as_ref())
}

pub fn latest_whiteboard(summaries: &[SharedSummaryChunk]) -> Option<&LiveWhiteboard> {
    latest_shared_whiteboard(summaries).map(|board| board.as_ref())
}

// Write scalar-limited detail directly, matching clamp_chars without owning a copy.
fn append_detail(out: &mut String, detail: &str, max_chars: usize) {
    let trimmed = detail.trim();
    if let Some((end, _)) = trimmed.char_indices().nth(max_chars) {
        out.push_str(&trimmed[..end]);
        out.push('…');
    } else {
        out.push_str(trimmed);
    }
}

fn append_node(
    out: &mut String,
    node: &LiveWhiteboardNode,
    indent: &str,
    role: &str,
    limit: usize,
) {
    use std::fmt::Write;
    let _ = write!(
        out,
        "{indent}[{role}] {}: {} ({})",
        node.id, node.label, node.kind
    );
    // Preserve the original pre-trim emptiness check, including whitespace-only detail.
    if !node.detail.is_empty() {
        out.push_str(" — ");
        append_detail(out, &node.detail, limit);
    }
    out.push('\n');
}

fn append_terms(out: &mut String, terms: Option<&Vec<&str>>, indent: &str) {
    use std::fmt::Write;
    let Some(terms) = terms.filter(|terms| !terms.is_empty()) else {
        return;
    };
    let _ = write!(out, "{indent}terms({}): ", terms.len());
    for (index, term) in terms.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str(term);
    }
    out.push('\n');
}

pub fn format_latest_whiteboard_context(summaries: &[SharedSummaryChunk]) -> String {
    use std::collections::{HashMap, HashSet};
    use std::fmt::Write;
    let Some(board) = latest_whiteboard(summaries) else {
        return "なし".to_string();
    };

    // Preserve the compressed prompt format and input order. Build borrowed
    // indexes once rather than searching all nodes separately for every main.
    let mut terms_by_parent: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut branches_by_parent: HashMap<&str, Vec<&LiveWhiteboardNode>> = HashMap::new();
    let mut node_by_id: HashMap<&str, &LiveWhiteboardNode> = HashMap::new();
    for node in &board.nodes {
        node_by_id.insert(&node.id, node);
        if node.node_type == "term" {
            terms_by_parent
                .entry(&node.parent_id)
                .or_default()
                .push(&node.label);
        } else if node.role != "main" {
            branches_by_parent
                .entry(&node.parent_id)
                .or_default()
                .push(node);
        }
    }

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
    let mut emitted: HashSet<&str> = HashSet::new();
    for main in board
        .nodes
        .iter()
        .filter(|node| node.node_type != "term" && node.role == "main")
    {
        append_node(&mut out, main, "", "main", 60);
        emitted.insert(&main.id);
        if let Some(branches) = branches_by_parent.get(main.id.as_str()) {
            for branch in branches {
                append_node(&mut out, branch, "  ", "branch", 48);
                emitted.insert(&branch.id);
                append_terms(&mut out, terms_by_parent.get(branch.id.as_str()), "    ");
            }
        }
        append_terms(&mut out, terms_by_parent.get(main.id.as_str()), "  ");
    }
    // Preserve ID-based orphan suppression, including legacy duplicate IDs.
    for node in &board.nodes {
        if node.node_type != "term" && !emitted.contains(node.id.as_str()) {
            append_node(&mut out, node, "", &node.role, 48);
        }
    }

    // Parent-child and term links are implicit. Append cross edges directly
    // instead of building a vector of edge strings and a second joined string.
    let mut has_edges = false;
    for edge in &board.edges {
        let (Some(from), Some(to)) = (
            node_by_id.get(edge.from.as_str()),
            node_by_id.get(edge.to.as_str()),
        ) else {
            continue;
        };
        if from.node_type == "term"
            || to.node_type == "term"
            || from.parent_id == to.id
            || to.parent_id == from.id
        {
            continue;
        }
        out.push_str(if has_edges { ", " } else { "edges: " });
        has_edges = true;
        let _ = write!(out, "{}→{}", edge.from, edge.to);
        if !edge.label.is_empty() {
            let _ = write!(out, " [{}]", edge.label);
        }
    }
    if has_edges {
        out.push('\n');
    }
    out
}

#[cfg(test)]
#[path = "context/tests.rs"]
mod tests;
