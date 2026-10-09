// Frozen immediately preceding text assembly; tests/isolated timing only.
use crate::live::{LiveCourseInfo, SharedSummaryChunk, SharedTranscriptLine};
use std::fmt::Write;
const NONE: &str = "なし";

fn tail<T>(values: &[T], limit: usize) -> &[T] {
    &values[values.len().saturating_sub(limit)..]
}
fn format_transcript(lines: &[SharedTranscriptLine]) -> String {
    let mut out = String::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let _ = write!(out, "- [{}] {}", line.at, line.text);
    }
    out
}
fn format_summary_text(chunks: &[SharedSummaryChunk]) -> String {
    let mut out = String::new();
    for (index, chunk) in chunks.iter().enumerate() {
        if index > 0 {
            out.push_str("\n\n");
        }
        let _ = write!(
            out,
            "## {}\n{}\n{}",
            chunk.title, chunk.range_label, chunk.body
        );
    }
    out
}

pub fn format_recent_summary_context(summaries: &[SharedSummaryChunk], limit: usize) -> String {
    if summaries.is_empty() || limit == 0 {
        return NONE.to_owned();
    }
    let chunks = &summaries[summaries.len().saturating_sub(limit)..];
    let capacity = chunks
        .iter()
        .map(|chunk| {
            "## ".len() + chunk.title.len() + 1 + chunk.range_label.len() + 1 + chunk.body.len()
        })
        .sum::<usize>()
        + chunks.len().saturating_sub(1) * 2;
    let mut out = String::with_capacity(capacity);
    for (index, chunk) in chunks.iter().enumerate() {
        if index > 0 {
            out.push_str("\n\n");
        }
        out.push_str("## ");
        out.push_str(&chunk.title);
        out.push('\n');
        out.push_str(&chunk.range_label);
        out.push('\n');
        out.push_str(&chunk.body);
    }
    out
}

pub fn chunk(
    course_block: &str,
    recent_summaries: &[SharedSummaryChunk],
    lines: &[SharedTranscriptLine],
    trailing_note: &str,
) -> String {
    let transcript = format_transcript(lines);
    let recent_summary_context = format_recent_summary_context(recent_summaries, 2);
    format!(
        "{}\n\n直前の分割要約:\n{}\n\n今回の文字起こし:\n{}\n\n{}",
        course_block, recent_summary_context, transcript, trailing_note,
    )
}
pub fn overall(
    course: &LiveCourseInfo,
    transcript_lines: &[SharedTranscriptLine],
    summaries: &[SharedSummaryChunk],
) -> String {
    let summary_text = format_summary_text(summaries);
    let recent_transcript = format_transcript(tail(transcript_lines, 24));
    let user_content = if course.is_free_note {
        format!(
            "記録種別: 自由ノート\n題名: {}\n\n分割要約:\n{}\n\n終盤の文字起こし:\n{}\n\n注記: 文字起こしには STT 誤認識が含まれる可能性があります。録音内容の文脈から、明らかな誤りは自然に補正してください。自由ノートは講義とは限らないため、会話・素材記録・自習メモなど実際の内容に合わせて整理してください。",
            course.course_name, summary_text, recent_transcript,
        )
    } else {
        format!(
            "講義: {}\n授業コード: {}\n教員: {}\n\n分割要約:\n{}\n\n終盤の文字起こし:\n{}\n\n注記: 文字起こしには STT 誤認識が含まれる可能性があります。講義名「{}」の分野脈絡から、明らかな誤りは自然に補正してください。",
            course.course_name,
            course.course_code,
            if course.teacher.is_empty() {
                "不明"
            } else {
                &course.teacher
            },
            summary_text,
            recent_transcript,
            course.course_name,
        )
    };
    user_content
}
pub fn todo(
    course: &LiveCourseInfo,
    transcript_lines: &[SharedTranscriptLine],
    summaries: &[SharedSummaryChunk],
    course_plan_context: &str,
) -> String {
    let summary_text = format_summary_text(summaries);
    let transcript = format_transcript(tail(transcript_lines, 80));
    format!(
                "講義: {}\n授業コード: {}\n曜日/時限: {} {}\n教員: {}\n\n締切推定の参考情報:\n{}\n\nAIレポート/分割要約:\n{}\n\n文字起こし（終盤中心）:\n{}\n\nこの講義内で明確に指示されたTODO/課題候補だけを抽出し、必要なDDLをできるだけ補ってください。",
                course.course_name,
                course.course_code,
                course.day,
                course.period,
                if course.teacher.is_empty() { "不明" } else { &course.teacher },
                course_plan_context,
                summary_text,
                transcript,
            )
}
