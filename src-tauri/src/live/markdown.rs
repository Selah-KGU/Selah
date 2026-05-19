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
                "## {}\n{}\n\n{}{}{}",
                chunk.title,
                chunk.range_label,
                chunk.body,
                format_whiteboard_markdown(chunk.whiteboard.as_ref()),
                format_terms_markdown(&chunk.terms),
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    if course.is_free_note {
        format!(
            "# {title}\n\n- 開始: {started}\n- 終了: {ended}\n\n{overall_summary}\n\n## 区間ごとの要約\n\n{chunk_markdown}\n\n## 全文転写\n\n{transcript}\n",
            title = FREE_NOTE_FOLDER_NAME,
            started = format_datetime(started_at),
            ended = format_datetime(ended_at),
        )
    } else {
        format!(
            "# {course_name}\n\n- 授業コード: {course_code}\n- 教員: {teacher}\n- 教室: {room}\n- 時間帯: {time_label}\n- 開始: {started}\n- 終了: {ended}\n\n{overall_summary}\n\n## 区間ごとの要約\n\n{chunk_markdown}\n\n## 全文転写\n\n{transcript}\n",
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
