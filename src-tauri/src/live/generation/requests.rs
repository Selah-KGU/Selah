//! Configuration, text assembly and course-plan IO belong to blocking workers.
use super::*;
use chrono::Datelike;

use tauri::Manager;

const FAILURE: &str = "Live生成準備処理失敗";

pub(super) struct Captured {
    course: LiveCourseInfo,
    lines: LiveTranscriptLines,
    summaries: LiveSummaryChunks,
}
impl Captured {
    pub(super) fn new(
        course: &LiveCourseInfo,
        lines: &LiveTranscriptLines,
        summaries: &LiveSummaryChunks,
    ) -> Self {
        Self {
            course: course.clone(),
            lines: lines.clone(),
            summaries: summaries.clone(),
        }
    }
}
pub(super) struct Prepared {
    pub cfg: crate::ai::AiConfig,
    pub messages: Vec<crate::ai::ChatMessage>,
}
pub(super) enum Overall {
    Ready(String),
    Model { request: Prepared, fallback: String },
}
async fn prepare<R: Send + 'static>(
    input: Captured,
    work: impl FnOnce(&Captured) -> Result<R, String> + Send + 'static,
) -> Result<R, String> {
    crate::background_ipc::run(FAILURE, move || work(&input)).await
}

pub(super) async fn chunk(input: Captured) -> Result<Prepared, String> {
    prepare(input, |input| {
        let cfg = live_ai_config()?;
        let messages = chunk_messages(&cfg, input);
        Ok(Prepared { cfg, messages })
    })
    .await
}
pub(super) async fn whiteboard(
    input: Captured,
    reply_language: String,
    parsed: LiveChunkAiResult,
    range_label: String,
) -> Result<(LiveChunkAiResult, Vec<crate::ai::ChatMessage>), String> {
    prepare(input, move |input| {
        let messages = whiteboard_messages(&reply_language, input, &parsed, &range_label);
        Ok((parsed, messages))
    })
    .await
}
pub(super) async fn overall(
    input: Captured,
    started_at: DateTime<Local>,
    ended_at: DateTime<Local>,
) -> Result<Overall, String> {
    prepare(input, move |input| {
        Ok(overall_with(
            input,
            started_at,
            ended_at,
            crate::ai::load_ai_config(),
            validate_live_ai_config,
        ))
    })
    .await
}
fn overall_with(
    input: &Captured,
    started_at: DateTime<Local>,
    ended_at: DateTime<Local>,
    settings: crate::ai::AiConfig,
    validate: impl FnOnce(crate::ai::AiConfig) -> Result<crate::ai::AiConfig, String>,
) -> Overall {
    let short = should_skip_ai_summarization(started_at, ended_at);
    let fallback = if short {
        short_session_overall_summary(&input.course, input.lines.len(), &settings.reply_language)
    } else {
        fallback_overall_summary(
            &input.course,
            input.lines.len(),
            input.summaries.len(),
            &settings.reply_language,
        )
    };
    if short || !should_run_finish_ai(&settings.provider, started_at, ended_at) {
        return Overall::Ready(fallback);
    }
    match validate(settings) {
        Ok(cfg) => {
            let messages = overall_messages(&cfg, input);
            Overall::Model {
                request: Prepared { cfg, messages },
                fallback,
            }
        }
        Err(_) => Overall::Ready(fallback),
    }
}
pub(super) async fn todo(
    input: Captured,
    app: tauri::AppHandle,
    ended_at: DateTime<Local>,
) -> Result<Prepared, String> {
    prepare(input, move |input| {
        let cfg = live_ai_config()?;
        let context = live_todo_course_plan_context(
            &app.state::<crate::db::Database>(),
            &input.course,
            ended_at,
        );
        let messages = todo_messages(&cfg, input, &context);
        Ok(Prepared { cfg, messages })
    })
    .await
}

fn chunk_messages(cfg: &crate::ai::AiConfig, input: &Captured) -> Vec<crate::ai::ChatMessage> {
    let Captured {
        course,
        lines,
        summaries: recent_summaries,
    } = input;
    let language_hint = live_reply_language_hint(&cfg.reply_language);
    let course_block = if course.is_free_note {
        format!("記録種別: 自由ノート\n題名: {}", course.course_name)
    } else {
        format!(
            "講義: {}\n授業コード: {}\n教員: {}\n教室: {}\n時間帯: {}",
            course.course_name,
            course.course_code,
            if course.teacher.is_empty() {
                "不明"
            } else {
                &course.teacher
            },
            if course.room.is_empty() {
                "未設定"
            } else {
                &course.room
            },
            course.time_label,
        )
    };
    let trailing_note = if course.is_free_note {
        "注記: 自由ノートは講義とは限りません。録音内容そのものを対象に、人物・出来事・ルール・話題の流れを整理してください。文字起こしの固有名詞には STT の誤認識が混ざる可能性があります。".to_string()
    } else {
        format!(
            "注記: 文字起こしの専門用語・固有名詞は STT の誤認識が混ざる可能性があります。講義名「{}」の分野脈絡を手がかりに、明らかな誤りは自然に補正してください。",
            course.course_name
        )
    };

    let messages = vec![
        crate::ai::ChatMessage {
            role: "system".into(),
            content: live_chunk_system_prompt(language_hint, course.is_free_note),
            images: Vec::new(),
        },
        crate::ai::ChatMessage {
            role: "user".into(),
            content: super::request_text::chunk(
                &course_block,
                recent_summaries,
                lines,
                &trailing_note,
            ),
            images: Vec::new(),
        },
    ];
    messages
}

fn whiteboard_messages(
    reply_language: &str,
    input: &Captured,
    parsed: &LiveChunkAiResult,
    range_label: &str,
) -> Vec<crate::ai::ChatMessage> {
    let Captured {
        course,
        lines,
        summaries: recent_summaries,
    } = input;
    let whiteboard_language_instruction = live_whiteboard_language_instruction(reply_language);
    // Whiteboard (Call 2) gets a transcript trimmed to its tail to bound token
    // cost when a chunk window contains an unusually large number of STT lines.
    // Call 1 still sees the full transcript because summary+terms accuracy
    // depends on covering every line.
    const WHITEBOARD_TRANSCRIPT_LINE_CAP: usize = 500;
    let elided = lines.len().saturating_sub(WHITEBOARD_TRANSCRIPT_LINE_CAP);
    let transcript_for_whiteboard = ContextPart::Transcript {
        lines: &lines[elided..],
        elided,
    };
    let course_block = if course.is_free_note {
        format!("記録種別: 自由ノート\n題名: {}", course.course_name)
    } else {
        format!(
            "講義: {}\n授業コード: {}\n教員: {}\n教室: {}\n時間帯: {}",
            course.course_name,
            course.course_code,
            if course.teacher.is_empty() {
                "不明"
            } else {
                &course.teacher
            },
            if course.room.is_empty() {
                "未設定"
            } else {
                &course.room
            },
            course.time_label,
        )
    };

    let whiteboard_context = format_latest_whiteboard_context(recent_summaries);
    let messages = vec![
        crate::ai::ChatMessage {
            role: "system".into(),
            content: live_whiteboard_system_prompt(
                whiteboard_language_instruction,
                course.is_free_note,
            ),
            images: Vec::new(),
        },
        crate::ai::ChatMessage {
            role: "user".into(),
            content: WhiteboardContext {
                course: &course_block,
                summaries: recent_summaries,
                latest_board: &whiteboard_context,
                body: &parsed.body,
                terms: &parsed.terms,
                range: range_label,
                transcript: transcript_for_whiteboard,
            }
            .build(),
            images: Vec::new(),
        },
    ];
    messages
}

fn overall_messages(cfg: &crate::ai::AiConfig, input: &Captured) -> Vec<crate::ai::ChatMessage> {
    let Captured {
        course,
        lines: transcript_lines,
        summaries,
    } = input;
    let language_hint = live_reply_language_hint(&cfg.reply_language);
    let user_content = super::request_text::overall(course, transcript_lines, summaries);
    let messages = vec![
        crate::ai::ChatMessage {
            role: "system".into(),
            content: live_overall_system_prompt(
                &cfg.reply_language,
                language_hint,
                course.is_free_note,
            ),
            images: Vec::new(),
        },
        crate::ai::ChatMessage {
            role: "user".into(),
            content: user_content,
            images: Vec::new(),
        },
    ];
    messages
}

fn todo_messages(
    cfg: &crate::ai::AiConfig,
    input: &Captured,
    course_plan_context: &str,
) -> Vec<crate::ai::ChatMessage> {
    let Captured {
        course,
        lines: transcript_lines,
        summaries,
    } = input;
    let messages = vec![
        crate::ai::ChatMessage {
            role: "system".into(),
            content: live_todo_system_prompt(&cfg.reply_language),
            images: Vec::new(),
        },
        crate::ai::ChatMessage {
            role: "user".into(),
            content: super::request_text::todo(
                course,
                transcript_lines,
                summaries,
                course_plan_context,
            ),
            images: Vec::new(),
        },
    ];
    messages
}

fn live_todo_course_plan_context(
    db: &crate::db::Database,
    course: &LiveCourseInfo,
    ended_at: DateTime<Local>,
) -> String {
    let mut lines = vec![
        format!("現在日時: {}", ended_at.format("%Y-%m-%d %H:%M")),
        format!(
            "次回授業候補: {}",
            next_course_meeting_hint(course, ended_at).unwrap_or_else(|| "不明".to_string())
        ),
    ];
    let course_code = course.course_code.trim();
    if course_code.is_empty() {
        lines.push("授業計画: 授業コードなし".to_string());
        return lines.join("\n");
    }

    match db.get_session_plans_for_course(course_code) {
        Ok(Some(course_plans)) => {
            lines.push("授業計画:".to_string());
            for plan in course_plans.iter().take(18) {
                let mut parts = Vec::new();
                if !plan.th_header.trim().is_empty() {
                    parts.push(clamp_chars(&plan.th_header, 80));
                }
                if !plan.topic.trim().is_empty() {
                    parts.push(clamp_chars(&plan.topic, 160));
                }
                if !plan.study_outside.trim().is_empty() {
                    parts.push(format!(
                        "授業外学修: {}",
                        clamp_chars(&plan.study_outside, 180)
                    ));
                }
                if !parts.is_empty() {
                    lines.push(format!("第{}回: {}", plan.session_num, parts.join(" / ")));
                }
            }
        }
        Ok(None) => lines.push("授業計画: キャッシュなし".to_string()),
        Err(_) => lines.push("授業計画: 読み込み失敗".to_string()),
    }

    if let Ok(Some(detail)) = db.get_kgc_course_detail(course_code) {
        let detail_lines = detail
            .fields
            .iter()
            .filter(|(label, value)| {
                let label = label.as_str();
                !value.trim().is_empty()
                    && (label.contains("授業外")
                        || label.contains("課題")
                        || label.contains("評価")
                        || label.contains("試験"))
            })
            .take(4)
            .map(|(label, value)| format!("{}: {}", label, clamp_chars(value, 160)))
            .collect::<Vec<_>>();
        if !detail_lines.is_empty() {
            lines.push("シラバス補足:".to_string());
            lines.extend(detail_lines);
        }
    }

    lines.join("\n")
}

fn next_course_meeting_hint(course: &LiveCourseInfo, ended_at: DateTime<Local>) -> Option<String> {
    if !(1..=7).contains(&course.day) {
        return None;
    }
    let today = ended_at.weekday().number_from_monday() as i32;
    let mut days_until = (course.day - today + 7) % 7;
    if days_until == 0 {
        days_until = 7;
    }
    let date = ended_at.date_naive() + ChronoDuration::days(days_until as i64);
    let time = course_period_start_time(course.period);
    Some(match time {
        Some((hour, minute)) => format!("{} {:02}:{:02}", date.format("%Y-%m-%d"), hour, minute),
        None => date.format("%Y-%m-%d").to_string(),
    })
}

fn course_period_start_time(period: i32) -> Option<(u32, u32)> {
    if period < 1 {
        return None;
    }
    crate::config::PERIOD_TIMES
        .get((period - 1) as usize)
        .map(|(start_h, start_m, _, _)| (*start_h, *start_m))
}

#[cfg(test)]
mod tests;
