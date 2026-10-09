use crate::live::{LiveWhiteboard, LiveWhiteboardEdge, LiveWhiteboardNode};

pub fn board(count: usize, seed: usize) -> LiveWhiteboard {
    let nodes: Vec<_> = (0..count)
        .map(|index| LiveWhiteboardNode {
            id: if seed % 5 == 0 {
                format!("n{}", index / 2)
            } else {
                format!("n{index}")
            },
            label: format!("概念 {index} 👩🏽‍💻"),
            detail: "完全な詳細 日本語・中文\n\"引用\" 🌕".repeat(12),
            role: if index % 4 == 0 { "main" } else { "branch" }.into(),
            parent_id: if index % 4 == 0 {
                String::new()
            } else {
                format!("n{}", index / 4 * 4)
            },
            node_type: if index % 7 == 6 { "term" } else { "structure" }.into(),
            kind: ["core", "result", "question", "support"][(seed + index) % 4].into(),
            source_type: if index % 3 == 0 {
                "external"
            } else {
                "lecture"
            }
            .into(),
            source_excerpt: "根拠となる発話を保持する 👩🏽‍💻".into(),
            external_source: "参考資料の全文 🌕".into(),
        })
        .collect();
    let edges = if count > 0 {
        (0..count)
            .map(|index| LiveWhiteboardEdge {
                from: nodes[index].id.clone(),
                to: nodes[(index + 3) % count].id.clone(),
                label: format!("関係 {index} 🌕"),
            })
            .chain([LiveWhiteboardEdge {
                from: "missing".into(),
                to: nodes[0].id.clone(),
                label: "未知端点".into(),
            }])
            .collect()
    } else {
        vec![]
    };
    LiveWhiteboard {
        title: format!("累積ボード {seed}"),
        layout: "grid".into(),
        nodes,
        edges,
        schema_version: 1,
        normalized_by: "backend".into(),
    }
}
