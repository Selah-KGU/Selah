//! Decide whether a new whiteboard replaces the previous one.

use super::*;

/// Decide what whiteboard to persist for the current segment.
///
/// - `Some(new)` → trust the model's full-board rewrite. Omitted nodes/edges
///   are considered intentionally removed so the structure can stay clear.
/// - `None`      → carry the previous board forward so the UI doesn't blink
///   out an existing visualization just because this chunk's AI call happened
///   to skip the field.
pub fn reconcile_whiteboard(
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
