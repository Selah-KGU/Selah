//! Synthetic input shared by the regression tests and isolated benchmark.
use super::{
    LiveCourseInfo, LiveSummaryChunk, LiveTermExplanation, LiveTranscriptLine, LiveWhiteboard,
    LiveWhiteboardEdge, LiveWhiteboardNode, SharedSummaryChunk, SharedTranscriptLine,
};

pub(super) fn course(free: bool) -> LiveCourseInfo {
    LiveCourseInfo {
        course_name: "授業 中文 한국어 👩🏽‍💻\n\"引用\"".into(),
        course_code: " code\n1 ".into(),
        teacher: "教員**🌕**".into(),
        room: " 第1教室 ".into(),
        time_label: "木3\t13:20-15:00".into(),
        day: 4,
        period: 3,
        is_free_note: free,
    }
}

pub(super) fn lines(count: usize) -> Vec<SharedTranscriptLine> {
    (0..count)
        .map(|i| {
            LiveTranscriptLine {
                at: format!("10:{:02}:00", i % 60),
                text: format!("{i}: 完全な発話 中文 한국어 👩🏽‍💻\n\"引用\" \\\t\r\0 **段落**"),
            }
            .into()
        })
        .collect()
}

pub(super) fn board(count: usize) -> LiveWhiteboard {
    let nodes = (0..count)
        .map(|i| LiveWhiteboardNode {
            id: format!("node-{i}"),
            label: format!("节点 {i} 👩🏽‍💻\n\"引用\""),
            detail: match i % 3 {
                0 => "  \t".into(),
                _ => " 詳細\n中文 **🌕** ".into(),
            },
            source_excerpt: if i % 4 == 0 {
                " \t\r\n".into()
            } else {
                " 原文 👩🏽‍💻 ".into()
            },
            external_source: if i % 3 == 0 {
                " \t".into()
            } else {
                " https://例.example/\"🌕\" ".into()
            },
            source_type: if i % 2 == 0 {
                "external".into()
            } else {
                "lecture".into()
            },
            node_type: "structure".into(),
            kind: "core".into(),
            role: "branch".into(),
            parent_id: "node-0".into(),
        })
        .collect();
    let mut edges: Vec<_> = (1..count)
        .map(|i| LiveWhiteboardEdge {
            from: "node-0".into(),
            to: format!("node-{i}"),
            label: if i % 2 == 0 {
                " \t\r\n".into()
            } else {
                " 続く 🌕\n ".into()
            },
        })
        .collect();
    edges.insert(
        0,
        LiveWhiteboardEdge {
            from: "missing".into(),
            to: "node-0".into(),
            label: "skip".into(),
        },
    );
    edges.push(LiveWhiteboardEdge {
        from: "node-0".into(),
        to: "missing".into(),
        label: "skip".into(),
    });
    LiveWhiteboard {
        title: " 板書 🌕 ".into(),
        layout: "grid".into(),
        nodes,
        edges,
        schema_version: 1,
        normalized_by: "backend".into(),
    }
}

pub(super) fn summaries(count: usize) -> Vec<SharedSummaryChunk> {
    (0..count)
        .map(|i| {
            LiveSummaryChunk {
                title: format!("段落 {i} 🌕"),
                range_label: "10:00-10:05\n".into(),
                body: "- 点\n\n---\n\n**用語**: 説明 🌕\n".repeat(3),
                line_count: 80,
                terms: vec![
                    LiveTermExplanation {
                        term: "术语 🌕".into(),
                        explanation: "完整な説明\n\"引用\"".into(),
                        source_excerpt: " 原文 中文 👩🏽‍💻 ".into(),
                        external_source: "https://例.example".into(),
                    },
                    LiveTermExplanation {
                        term: "空の項目".into(),
                        explanation: "".into(),
                        source_excerpt: "".into(),
                        external_source: "".into(),
                    },
                    LiveTermExplanation {
                        term: "空白".into(),
                        explanation: " \t ".into(),
                        source_excerpt: " \t ".into(),
                        external_source: " \t ".into(),
                    },
                ],
                whiteboard: if i + 1 == count {
                    Some(board(75).into())
                } else {
                    None
                },
            }
            .into()
        })
        .collect()
}
