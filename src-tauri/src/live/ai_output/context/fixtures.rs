//! Synthetic boards shared by regression tests and the standalone benchmark.
use super::*;
use crate::live::LiveSummaryChunk;

pub(super) fn node(id: &str, role: &str, parent: &str) -> LiveWhiteboardNode {
    LiveWhiteboardNode {
        id: id.into(),
        label: format!("概念 {id} 🌕"),
        detail: String::new(),
        node_type: "structure".into(),
        kind: "core".into(),
        role: role.into(),
        parent_id: parent.into(),
        source_type: "lecture".into(),
        source_excerpt: String::new(),
        external_source: String::new(),
    }
}

pub(super) fn summary(board: Option<LiveWhiteboard>) -> SharedSummaryChunk {
    LiveSummaryChunk {
        title: "fixture".into(),
        range_label: "10:00-10:10".into(),
        body: "all summary text retained".into(),
        line_count: 12,
        terms: vec![],
        whiteboard: board.map(Into::into),
    }
    .into()
}

pub(super) fn board(count: usize, seed: usize) -> LiveWhiteboard {
    let mut nodes: Vec<_> = (0..count)
        .map(|i| {
            let mut n = node(
                &format!("n{i}"),
                if i % 4 == 0 { "main" } else { "branch" },
                if i % 4 == 0 { "" } else { "n0" },
            );
            if i % 4 != 0 {
                n.parent_id = format!("n{}", i / 4 * 4);
            }
            if i % 4 == 3 {
                n.node_type = "term".into();
            }
            n.detail = match (i + seed) % 4 {
                0 => String::new(),
                1 => " \t\n ".into(),
                2 => "短い本文 中文🙂".into(),
                _ => "  日本語 👩🏽‍💻\n".repeat(15),
            };
            if seed % 3 == 0 && i % 5 == 0 {
                n.parent_id = "missing".into();
            }
            if seed % 5 == 0 && i % 7 == 0 {
                n.id = "duplicate".into();
            }
            if seed % 7 == 0 {
                n.role = "orphan".into();
            }
            if seed % 11 == 0 {
                n.node_type = "term".into();
            }
            if seed % 13 == 0 {
                n.id.clear();
                n.parent_id.clear();
            }
            n
        })
        .collect();
    if count > 0 {
        nodes.rotate_left(seed % count);
    }
    let mut edges: Vec<_> = (0..count)
        .map(|i| LiveWhiteboardEdge {
            from: nodes[i].id.clone(),
            to: nodes[(i + 4) % count].id.clone(),
            label: if i % 3 == 0 {
                String::new()
            } else {
                format!("関係 {i} 中文 🌕\n")
            },
        })
        .collect();
    if count > 0 {
        edges.push(LiveWhiteboardEdge {
            from: "missing".into(),
            to: nodes[0].id.clone(),
            label: "invalid".into(),
        });
    }
    LiveWhiteboard {
        title: if seed % 2 == 0 {
            String::new()
        } else {
            "累積ボード 🌕\n".into()
        },
        layout: "grid".into(),
        nodes,
        edges,
        schema_version: 1,
        normalized_by: "backend".into(),
    }
}
