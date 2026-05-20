use chrono::{DateTime, Local};

use super::{
    format_datetime, LiveCourseInfo, LiveSummaryChunk, LiveTermExplanation, LiveTranscriptLine,
    LiveWhiteboard, FREE_NOTE_FOLDER_NAME,
};

fn format_terms_markdown(terms: &[LiveTermExplanation]) -> String {
    if terms.is_empty() {
        return String::new();
    }
    let lines = terms
        .iter()
        .map(|term| {
            let mut detail = String::new();
            if !term.source_excerpt.is_empty() {
                detail.push_str(&format!("（講義内根拠: {}）", term.source_excerpt));
            }
            if !term.external_source.is_empty() {
                detail.push_str(&format!("（外部出典: {}）", term.external_source));
            }
            format!("- **{}**: {}{}", term.term, term.explanation, detail)
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("\n\n### 用語注釈\n{}", lines)
}

fn format_whiteboard_markdown(whiteboard: Option<&LiveWhiteboard>) -> String {
    let Some(board) = whiteboard else {
        return String::new();
    };
    if board.nodes.is_empty() {
        return String::new();
    }

    let title = if board.title.trim().is_empty() {
        "知識整理ボード"
    } else {
        &board.title
    };
    let nodes = board
        .nodes
        .iter()
        .map(|node| {
            let mut source = String::new();
            if node.source_type == "external" {
                source.push_str("（外部補足");
                if !node.external_source.trim().is_empty() {
                    source.push_str(&format!(": {}", node.external_source));
                }
                source.push('）');
            } else if !node.source_excerpt.trim().is_empty() {
                source.push_str(&format!("（講義内根拠: {}）", node.source_excerpt));
            }
            if node.detail.trim().is_empty() {
                format!("- **{}**{}", node.label, source)
            } else {
                format!("- **{}**: {}{}", node.label, node.detail, source)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let edges = board
        .edges
        .iter()
        .filter_map(|edge| {
            let from = board.nodes.iter().find(|node| node.id == edge.from)?;
            let to = board.nodes.iter().find(|node| node.id == edge.to)?;
            let label = if edge.label.trim().is_empty() {
                "→".to_string()
            } else {
                format!("--{}-->", edge.label)
            };
            Some(format!("- {} {} {}", from.label, label, to.label))
        })
        .collect::<Vec<_>>();

    // Structured fence: the in-app Markdown reader replaces this with an
    // interactive whiteboard visualization. Plain markdown editors fall
    // through to the bullet list below — same info, text-only.
    let data_fence = match serde_json::to_string(board) {
        Ok(json) => format!("\n\n```live-whiteboard\n{}\n```", json),
        Err(_) => String::new(),
    };

    if edges.is_empty() {
        format!(
            "\n\n### 知識整理ボード: {}{}\n\n{}",
            title, data_fence, nodes
        )
    } else {
        format!(
            "\n\n### 知識整理ボード: {}{}\n\n{}\n\n関係:\n{}",
            title,
            data_fence,
            nodes,
            edges.join("\n")
        )
    }
}

pub(super) fn build_markdown(
    course: &LiveCourseInfo,
    started_at: DateTime<Local>,
    ended_at: DateTime<Local>,
    overall_summary: &str,
    summaries: &[LiveSummaryChunk],
    transcript_lines: &[LiveTranscriptLine],
) -> String {
    let transcript = transcript_lines
        .iter()
        .map(|line| format!("- [{}] {}", line.at, line.text))
        .collect::<Vec<_>>()
        .join("\n");
    let chunk_markdown = summaries
        .iter()
        .map(|chunk| {
            format!(
                "## {}\n{}\n\n{}{}",
                chunk.title,
                chunk.range_label,
                chunk.body,
                format_terms_markdown(&chunk.terms),
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let final_whiteboard_markdown = format_whiteboard_markdown(
        summaries
            .iter()
            .rev()
            .find_map(|chunk| chunk.whiteboard.as_ref()),
    );

    if course.is_free_note {
        format!(
            "# {title}\n\n- 開始: {started}\n- 終了: {ended}\n\n{overall_summary}{final_whiteboard_markdown}\n\n## 区間ごとの要約\n\n{chunk_markdown}\n\n## 全文転写\n\n{transcript}\n",
            title = FREE_NOTE_FOLDER_NAME,
            started = format_datetime(started_at),
            ended = format_datetime(ended_at),
        )
    } else {
        format!(
            "# {course_name}\n\n- 授業コード: {course_code}\n- 教員: {teacher}\n- 教室: {room}\n- 時間帯: {time_label}\n- 開始: {started}\n- 終了: {ended}\n\n{overall_summary}{final_whiteboard_markdown}\n\n## 区間ごとの要約\n\n{chunk_markdown}\n\n## 全文転写\n\n{transcript}\n",
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
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::LiveWhiteboardNode;
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
            whiteboard,
        }
    }

    #[test]
    fn build_markdown_writes_only_latest_cumulative_whiteboard_once() {
        let summaries = vec![
            chunk("Chunk 1", "first body", Some(board("Old Board", &["Old"]))),
            chunk(
                "Chunk 2",
                "second body",
                Some(board("Final Board", &["Final", "Detail"])),
            ),
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
            }],
        );

        assert_eq!(markdown.matches("```live-whiteboard").count(), 1);
        assert_eq!(markdown.matches("### 知識整理ボード").count(), 1);
        assert!(markdown.contains("Final Board"));
        assert!(!markdown.contains("Old Board"));
        assert!(markdown.contains("## Chunk 1"));
        assert!(markdown.contains("## Chunk 2"));
    }
}
