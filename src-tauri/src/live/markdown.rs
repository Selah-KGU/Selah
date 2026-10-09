#[cfg(test)]
use super::{LiveSummaryChunk, LiveTranscriptLine, LiveWhiteboardEdge, LiveWhiteboardNode};
use chrono::{DateTime, Local};
use std::fmt::Write;

use super::{
    format_datetime, LiveCourseInfo, LiveTermExplanation, LiveWhiteboard, SharedSummaryChunk,
    SharedTranscriptLine, FREE_NOTE_FOLDER_NAME,
};

fn source_excerpt_label(course: &LiveCourseInfo) -> &'static str {
    if course.is_free_note {
        "録音内根拠"
    } else {
        "講義内根拠"
    }
}

fn append_terms_markdown(out: &mut String, course: &LiveCourseInfo, terms: &[LiveTermExplanation]) {
    if terms.is_empty() {
        return;
    }
    let source_label = source_excerpt_label(course);
    out.push_str("\n\n### 用語注釈\n");
    for (index, term) in terms.iter().enumerate() {
        if index != 0 {
            out.push('\n');
        }
        write!(out, "- **{}**: {}", term.term, term.explanation).unwrap();
        if !term.source_excerpt.is_empty() {
            write!(out, "（{}: {}）", source_label, term.source_excerpt).unwrap();
        }
        if !term.external_source.is_empty() {
            write!(out, "（外部出典: {}）", term.external_source).unwrap();
        }
    }
}

fn append_whiteboard_markdown(
    out: &mut String,
    course: &LiveCourseInfo,
    whiteboard: Option<&LiveWhiteboard>,
) {
    let Some(board) = whiteboard else {
        return;
    };
    if board.nodes.is_empty() {
        return;
    }

    let title = if board.title.trim().is_empty() {
        "知識整理ボード"
    } else {
        &board.title
    };
    let source_label = source_excerpt_label(course);
    write!(out, "\n\n### 知識整理ボード: {}", title).unwrap();
    // Structured fence: the in-app Markdown reader replaces this with an
    // interactive whiteboard visualization. Plain markdown editors fall
    // through to the bullet list below — same info, text-only.
    if let Ok(json) = serde_json::to_string(board) {
        write!(out, "\n\n```live-whiteboard\n{}\n```", json).unwrap();
    }
    out.push_str("\n\n");
    for (index, node) in board.nodes.iter().enumerate() {
        if index != 0 {
            out.push('\n');
        }
        write!(out, "- **{}**", node.label).unwrap();
        if !node.detail.trim().is_empty() {
            write!(out, ": {}", node.detail).unwrap();
        }
        if node.source_type == "external" {
            out.push_str("（外部補足");
            if !node.external_source.trim().is_empty() {
                write!(out, ": {}", node.external_source).unwrap();
            }
            out.push('）');
        } else if !node.source_excerpt.trim().is_empty() {
            write!(out, "（{}: {}）", source_label, node.source_excerpt).unwrap();
        }
    }
    let mut has_edges = false;
    for edge in &board.edges {
        let Some(from) = board.nodes.iter().find(|node| node.id == edge.from) else {
            continue;
        };
        let Some(to) = board.nodes.iter().find(|node| node.id == edge.to) else {
            continue;
        };
        if has_edges {
            out.push('\n');
        } else {
            out.push_str("\n\n関係:\n");
        }
        has_edges = true;
        write!(out, "- {} ", from.label).unwrap();
        if edge.label.trim().is_empty() {
            out.push('→');
        } else {
            write!(out, "--{}-->", edge.label).unwrap();
        }
        out.push(' ');
        out.push_str(&to.label);
    }
}

pub(super) fn build_markdown(
    course: &LiveCourseInfo,
    started_at: DateTime<Local>,
    ended_at: DateTime<Local>,
    overall_summary: &str,
    summaries: &[SharedSummaryChunk],
    transcript_lines: &[SharedTranscriptLine],
) -> String {
    let mut out = String::new();
    if course.is_free_note {
        write!(
            out,
            "# {title}\n\n- 開始: {started}\n- 終了: {ended}\n\n",
            title = FREE_NOTE_FOLDER_NAME,
            started = format_datetime(started_at),
            ended = format_datetime(ended_at),
        )
        .unwrap();
    } else {
        write!(out,
            "# {course_name}\n\n- 授業コード: {course_code}\n- 教員: {teacher}\n- 教室: {room}\n- 時間帯: {time_label}\n- 開始: {started}\n- 終了: {ended}\n\n",
            course_name = course.course_name,
            course_code = if course.course_code.is_empty() {
                "不明"
            } else {
                &course.course_code
            },
            teacher = if course.teacher.is_empty() {
                "不明"
            } else {
                &course.teacher
            },
            room = if course.room.is_empty() {
                "未設定"
            } else {
                &course.room
            },
            time_label = if course.time_label.is_empty() {
                "未設定"
            } else {
                &course.time_label
            },
            started = format_datetime(started_at),
            ended = format_datetime(ended_at),
        ).unwrap();
    }
    out.push_str(overall_summary);
    append_whiteboard_markdown(
        &mut out,
        course,
        summaries
            .iter()
            .rev()
            .find_map(|chunk| chunk.whiteboard.as_deref()),
    );
    out.push_str("\n\n## 区間ごとの要約\n\n");
    for (index, chunk) in summaries.iter().enumerate() {
        if index != 0 {
            out.push_str("\n\n");
        }
        write!(
            out,
            "## {}\n{}\n\n{}",
            chunk.title, chunk.range_label, chunk.body
        )
        .unwrap();
        append_terms_markdown(&mut out, course, &chunk.terms);
    }
    out.push_str("\n\n## 全文転写\n\n");
    // Reserve the exact remaining syntax/text size once. Existing lines are
    // borrowed; no per-line strings, line Vec or joined transcript is created.
    let remaining =
        transcript_lines
            .iter()
            .fold(usize::from(transcript_lines.is_empty()), |bytes, line| {
                bytes
                    .saturating_add(line.at.len())
                    .saturating_add(line.text.len())
                    .saturating_add(6)
            });
    out.reserve_exact(remaining);
    for (index, line) in transcript_lines.iter().enumerate() {
        if index != 0 {
            out.push('\n');
        }
        out.push_str("- [");
        out.push_str(&line.at);
        out.push_str("] ");
        out.push_str(&line.text);
    }
    out.push('\n');
    out
}

#[cfg(test)]
#[path = "markdown/before.rs"]
mod before;

#[cfg(test)]
#[path = "markdown/fixtures.rs"]
mod fixtures;

#[cfg(test)]
#[path = "markdown/equivalence.rs"]
mod equivalence_tests;

#[cfg(test)]
mod tests {
    use super::super::LiveWhiteboardNode;
    use super::*;
    use chrono::Local;

    fn course() -> LiveCourseInfo {
        LiveCourseInfo {
            course_name: "Test Course".to_string(),
            course_code: "TC101".to_string(),
            room: "101".to_string(),
            teacher: "Teacher".to_string(),
            day: 1,
            period: 1,
            time_label: "1限".to_string(),
            is_free_note: false,
        }
    }

    fn node(id: &str, label: &str, role: &str, parent_id: &str) -> LiveWhiteboardNode {
        LiveWhiteboardNode {
            id: id.to_string(),
            label: label.to_string(),
            detail: String::new(),
            node_type: "structure".to_string(),
            kind: "core".to_string(),
            role: role.to_string(),
            parent_id: parent_id.to_string(),
            source_type: "lecture".to_string(),
            source_excerpt: String::new(),
            external_source: String::new(),
        }
    }

    fn board(title: &str, labels: &[&str]) -> LiveWhiteboard {
        let mut nodes = Vec::new();
        for (idx, label) in labels.iter().enumerate() {
            nodes.push(node(
                &format!("n{}", idx + 1),
                label,
                if idx == 0 { "main" } else { "branch" },
                if idx == 0 { "" } else { "n1" },
            ));
        }
        LiveWhiteboard {
            title: title.to_string(),
            layout: "grid".to_string(),
            nodes,
            edges: Vec::new(),
            schema_version: 1,
            normalized_by: "backend".to_string(),
        }
    }

    fn chunk(title: &str, body: &str, whiteboard: Option<LiveWhiteboard>) -> LiveSummaryChunk {
        LiveSummaryChunk {
            title: title.to_string(),
            range_label: "09:00-09:10".to_string(),
            body: body.to_string(),
            line_count: 1,
            terms: Vec::new(),
            whiteboard: whiteboard.map(Into::into),
        }
    }

    #[test]
    fn build_markdown_writes_only_latest_cumulative_whiteboard_once() {
        let summaries = vec![
            chunk("Chunk 1", "first body", Some(board("Old Board", &["Old"]))).into(),
            chunk(
                "Chunk 2",
                "second body",
                Some(board("Final Board", &["Final", "Detail"])),
            )
            .into(),
        ];
        let markdown = build_markdown(
            &course(),
            Local::now(),
            Local::now(),
            "### 全体要約\nsummary",
            &summaries,
            &[LiveTranscriptLine {
                text: "transcript".to_string(),
                at: "09:00".to_string(),
            }
            .into()],
        );

        assert_eq!(markdown.matches("```live-whiteboard").count(), 1);
        assert_eq!(markdown.matches("### 知識整理ボード").count(), 1);
        assert!(markdown.contains("Final Board"));
        assert!(!markdown.contains("Old Board"));
        assert!(markdown.contains("## Chunk 1"));
        assert!(markdown.contains("## Chunk 2"));
    }

    #[test]
    fn build_markdown_uses_recording_source_label_for_free_notes() {
        let mut free_course = course();
        free_course.is_free_note = true;

        let mut board = board("Free Board", &["Scene"]);
        board.nodes[0].source_excerpt = "録音で出た根拠".to_string();

        let mut summary = chunk("Chunk", "body", Some(board));
        summary.terms.push(LiveTermExplanation {
            term: "用語".to_string(),
            explanation: "説明".to_string(),
            source_excerpt: "用語の根拠".to_string(),
            external_source: String::new(),
        });

        let markdown = build_markdown(
            &free_course,
            Local::now(),
            Local::now(),
            "### 全体要約\nsummary",
            &[summary.into()],
            &[],
        );

        assert!(markdown.contains("録音内根拠: 録音で出た根拠"));
        assert!(markdown.contains("録音内根拠: 用語の根拠"));
        assert!(!markdown.contains("講義内根拠"));
    }
}
