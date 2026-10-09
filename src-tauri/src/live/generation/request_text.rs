//! Text-only model user messages. Prompt wording and selection stay explicit;
//! common borrowed text pieces avoid full temporary transcripts/summaries.
use crate::live::{
    build_context_text, ContextPart, LiveCourseInfo, SharedSummaryChunk, SharedTranscriptLine,
};

fn tail<T>(values: &[T], limit: usize) -> &[T] {
    &values[values.len().saturating_sub(limit)..]
}

pub fn chunk(
    course_block: &str,
    summaries: &[SharedSummaryChunk],
    lines: &[SharedTranscriptLine],
    trailing_note: &str,
) -> String {
    let recent = tail(summaries, 2);
    build_context_text(&[
        ContextPart::Text(course_block),
        ContextPart::Text("\n\n直前の分割要約:\n"),
        if recent.is_empty() {
            ContextPart::Text("なし")
        } else {
            ContextPart::Summaries(recent)
        },
        ContextPart::Text("\n\n今回の文字起こし:\n"),
        ContextPart::Transcript { lines, elided: 0 },
        ContextPart::Text("\n\n"),
        ContextPart::Text(trailing_note),
    ])
}

pub fn overall(
    course: &LiveCourseInfo,
    lines: &[SharedTranscriptLine],
    summaries: &[SharedSummaryChunk],
) -> String {
    let summary = ContextPart::Summaries(summaries);
    let transcript = ContextPart::Transcript {
        lines: tail(lines, 24),
        elided: 0,
    };
    if course.is_free_note {
        build_context_text(&[
            ContextPart::Text("記録種別: 自由ノート\n題名: "), ContextPart::Text(&course.course_name),
            ContextPart::Text("\n\n分割要約:\n"), summary,
            ContextPart::Text("\n\n終盤の文字起こし:\n"), transcript,
            ContextPart::Text("\n\n注記: 文字起こしには STT 誤認識が含まれる可能性があります。録音内容の文脈から、明らかな誤りは自然に補正してください。自由ノートは講義とは限らないため、会話・素材記録・自習メモなど実際の内容に合わせて整理してください。"),
        ])
    } else {
        build_context_text(&[
            ContextPart::Text("講義: "),
            ContextPart::Text(&course.course_name),
            ContextPart::Text("\n授業コード: "),
            ContextPart::Text(&course.course_code),
            ContextPart::Text("\n教員: "),
            ContextPart::Text(if course.teacher.is_empty() {
                "不明"
            } else {
                &course.teacher
            }),
            ContextPart::Text("\n\n分割要約:\n"),
            summary,
            ContextPart::Text("\n\n終盤の文字起こし:\n"),
            transcript,
            ContextPart::Text(
                "\n\n注記: 文字起こしには STT 誤認識が含まれる可能性があります。講義名「",
            ),
            ContextPart::Text(&course.course_name),
            ContextPart::Text("」の分野脈絡から、明らかな誤りは自然に補正してください。"),
        ])
    }
}

pub fn todo(
    course: &LiveCourseInfo,
    lines: &[SharedTranscriptLine],
    summaries: &[SharedSummaryChunk],
    plan: &str,
) -> String {
    build_context_text(&[
        ContextPart::Text("講義: "), ContextPart::Text(&course.course_name),
        ContextPart::Text("\n授業コード: "), ContextPart::Text(&course.course_code),
        ContextPart::Text("\n曜日/時限: "), ContextPart::Integer(course.day), ContextPart::Text(" "), ContextPart::Integer(course.period),
        ContextPart::Text("\n教員: "), ContextPart::Text(if course.teacher.is_empty() { "不明" } else { &course.teacher }), ContextPart::Text("\n\n締切推定の参考情報:\n"), ContextPart::Text(plan),
        ContextPart::Text("\n\nAIレポート/分割要約:\n"), ContextPart::Summaries(summaries),
        ContextPart::Text("\n\n文字起こし（終盤中心）:\n"), ContextPart::Transcript { lines: tail(lines, 80), elided: 0 },
        ContextPart::Text("\n\nこの講義内で明確に指示されたTODO/課題候補だけを抽出し、必要なDDLをできるだけ補ってください。"),
    ])
}

#[cfg(test)]
#[path = "request_text/tests.rs"]
mod tests;
