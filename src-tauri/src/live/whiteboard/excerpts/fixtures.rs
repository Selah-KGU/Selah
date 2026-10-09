use super::{LiveTermExplanation, LiveWhiteboard, LiveWhiteboardNode, SharedTranscriptLine};

pub(super) struct Fixture {
    pub(super) board: LiveWhiteboard,
    pub(super) previous: LiveWhiteboard,
    pub(super) terms: Vec<LiveTermExplanation>,
    pub(super) lines: Vec<SharedTranscriptLine>,
}

pub(super) fn board(nodes: Vec<LiveWhiteboardNode>) -> LiveWhiteboard {
    LiveWhiteboard {
        title: "原文の引用 👩🏽‍💻".into(),
        layout: "grid".into(),
        nodes,
        edges: vec![],
        schema_version: 1,
        normalized_by: "backend".into(),
    }
}

pub(super) fn fixture(node_count: usize, line_count: usize, seed: usize) -> Fixture {
    let nodes: Vec<LiveWhiteboardNode> = (0..node_count).map(|index| {
        let mut node: LiveWhiteboardNode = serde_json::from_value(serde_json::json!({
            "id": format!("n{}", if seed%5==0 {index/2} else {index}),
            "label": format!("API{} 方法論 👩🏽‍💻 ÉCOLE", index%7),
            "detail": format!("api{} の 一次資料/ABC{}・論点abc論点。école 😸😸 \"引用\"", index%7,seed%3),
            "source_type": if (index+seed)%11==0 {"external"} else {"lecture"},
            "node_type": if index%3==0 {"term"} else {"structure"},
            "source_excerpt": if (index+seed)%13==0 {"供給した全文 👩🏽‍💻"} else {""},
            "external_source": "外部参照は変更しない",
            "parent_id": "n0"
        })).unwrap();
        if (index+seed)%17==0 {node.label.clear();}
        node
    }).collect();
    let previous = board(
        nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                let mut node = node.clone();
                node.source_excerpt = if (index + seed) % 3 == 0 {
                    format!(" prior {index} {} ", "日本語 👩🏽‍💻 ".repeat(seed % 4 + 1))
                } else {
                    String::new()
                };
                node
            })
            .collect(),
    );
    let terms = (0..12)
        .map(|index| LiveTermExplanation {
            term: if index == 0 && seed % 3 == 0 {
                String::new()
            } else {
                format!("API{}", index % 7)
            },
            explanation: "意味の全文".into(),
            source_excerpt: if index % 4 == 0 {
                " \t　 ".into()
            } else {
                format!(" term {index} {} ", "日本語 👩🏽‍💻 ".repeat(seed % 4 + 1))
            },
            external_source: "ref".into(),
        })
        .collect();
    let lines = (0..line_count)
        .map(|index| {
            crate::live::LiveTranscriptLine {
                text: if index % 19 == 0 {
                    "　\t\n ".into()
                } else {
                    format!(
                        "{index} 一次資料の方法論 API{} ABC{} ÉCOLE école 😸😸 👩🏽‍💻\n{}",
                        index % 7,
                        seed % 3,
                        "字幕の全文を保持する　引用\" ".repeat(index % 5)
                    )
                },
                at: format!("10:{:02}:{:02}", index / 60, index % 60),
            }
            .into()
        })
        .collect();
    Fixture {
        board: board(nodes),
        previous,
        terms,
        lines,
    }
}
