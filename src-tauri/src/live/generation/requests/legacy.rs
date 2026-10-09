//! Frozen request formatting before the worker/assembly change. Test-only.
use super::*;

pub(super) fn chunk(
    cfg: &crate::ai::AiConfig,
    input: &Captured,
    parsed_1: &LiveChunkAiResult,
    range_label: &str,
) -> (Vec<crate::ai::ChatMessage>, Vec<crate::ai::ChatMessage>) {
    let Captured {
        course,
        lines,
        summaries: recent_summaries,
    } = input;
    let language_hint = live_reply_language_hint(&cfg.reply_language);
    let whiteboard_language_instruction = live_whiteboard_language_instruction(&cfg.reply_language);
    let transcript = lines
        .iter()
        .map(|line| format!("- [{}] {}", line.at, line.text))
        .collect::<Vec<_>>()
        .join("\n");
    // Whiteboard (Call 2) gets a transcript trimmed to its tail to bound token
    // cost when a chunk window contains an unusually large number of STT lines.
    // Call 1 still sees the full transcript because summary+terms accuracy
    // depends on covering every line.
    const WHITEBOARD_TRANSCRIPT_LINE_CAP: usize = 500;
    let transcript_for_whiteboard = if lines.len() > WHITEBOARD_TRANSCRIPT_LINE_CAP {
        let elided = lines.len() - WHITEBOARD_TRANSCRIPT_LINE_CAP;
        let mut out = format!("(... 古い文字起こし {} 行を省略 ...)\n", elided);
        out.push_str(
            &lines
                .iter()
                .skip(elided)
                .map(|line| format!("- [{}] {}", line.at, line.text))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        out
    } else {
        transcript.clone()
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
    let trailing_note = if course.is_free_note {
        "注記: 自由ノートは講義とは限りません。録音内容そのものを対象に、人物・出来事・ルール・話題の流れを整理してください。文字起こしの固有名詞には STT の誤認識が混ざる可能性があります。".to_string()
    } else {
        format!(
            "注記: 文字起こしの専門用語・固有名詞は STT の誤認識が混ざる可能性があります。講義名「{}」の分野脈絡を手がかりに、明らかな誤りは自然に補正してください。",
            course.course_name
        )
    };

    // === Call 1: summary + terms ===
    let recent_summary_context = format_recent_summary_context(recent_summaries, 2);
    let messages_1 = vec![
        crate::ai::ChatMessage {
            role: "system".into(),
            content: live_chunk_system_prompt(language_hint, course.is_free_note),
            images: Vec::new(),
        },
        crate::ai::ChatMessage {
            role: "user".into(),
            content: format!(
                "{}\n\n直前の分割要約:\n{}\n\n今回の文字起こし:\n{}\n\n{}",
                course_block, recent_summary_context, transcript, trailing_note,
            ),
            images: Vec::new(),
        },
    ];
    let whiteboard_context = format_latest_whiteboard_context(recent_summaries);
    let full_history = format_full_history_for_whiteboard(recent_summaries);
    let current_chunk_brief =
        format_current_chunk_for_whiteboard(&parsed_1.body, &parsed_1.terms, range_label);
    let messages_2 = vec![
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
            content: format!(
                "{}\n\nこれまでの全分割要約と用語注釈（累積素材）:\n{}\n\n現在の累積知識整理ボード:\n{}\n\n今回新しく生成された区間の要約と用語:\n{}\n\n今回の文字起こし（補助参考、必要に応じて細部を拾う。長すぎる場合は末尾のみ表示）:\n{}\n\n指示: system の実行順序と構造パターン庫に従い、録音開始から現在までの累積 whiteboard JSON を返す。既出情報を失わず、必要なら既存ノードを更新・移動・アップグレード・統合・分割する。新しい具体材料は追加する。最後に parent_id、edge、term、混在タイプ分離をセルフチェックする。",
                course_block,
                full_history,
                whiteboard_context,
                current_chunk_brief,
                transcript_for_whiteboard,
            ),
            images: Vec::new(),
        },
    ];
    (messages_1, messages_2)
}

pub(super) fn overall(cfg: &crate::ai::AiConfig, input: &Captured) -> Vec<crate::ai::ChatMessage> {
    let Captured {
        course,
        lines: transcript_lines,
        summaries,
    } = input;
    let language_hint = live_reply_language_hint(&cfg.reply_language);
    let summary_text = summaries
        .iter()
        .map(|chunk| format!("## {}\n{}\n{}", chunk.title, chunk.range_label, chunk.body))
        .collect::<Vec<_>>()
        .join("\n\n");
    let recent_transcript = transcript_lines
        .iter()
        .rev()
        .take(24)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|line| format!("- [{}] {}", line.at, line.text))
        .collect::<Vec<_>>()
        .join("\n");
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

pub(super) fn todo(
    cfg: &crate::ai::AiConfig,
    input: &Captured,
    course_plan_context: &str,
) -> Vec<crate::ai::ChatMessage> {
    let Captured {
        course,
        lines: transcript_lines,
        summaries,
    } = input;
    let summary_text = summaries
        .iter()
        .map(|chunk| format!("## {}\n{}\n{}", chunk.title, chunk.range_label, chunk.body))
        .collect::<Vec<_>>()
        .join("\n\n");
    let transcript = transcript_lines
        .iter()
        .rev()
        .take(80)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|line| format!("- [{}] {}", line.at, line.text))
        .collect::<Vec<_>>()
        .join("\n");
    let messages = vec![
        crate::ai::ChatMessage {
            role: "system".into(),
            content: live_todo_system_prompt(&cfg.reply_language),
            images: Vec::new(),
        },
        crate::ai::ChatMessage {
            role: "user".into(),
            content: format!(
                "講義: {}\n授業コード: {}\n曜日/時限: {} {}\n教員: {}\n\n締切推定の参考情報:\n{}\n\nAIレポート/分割要約:\n{}\n\n文字起こし（終盤中心）:\n{}\n\nこの講義内で明確に指示されたTODO/課題候補だけを抽出し、必要なDDLをできるだけ補ってください。",
                course.course_name,
                course.course_code,
                course.day,
                course.period,
                if course.teacher.is_empty() { "不明" } else { &course.teacher },
                course_plan_context,
                summary_text,
                transcript,
            ),
            images: Vec::new(),
        },
    ];
    messages
}

// Frozen all-course lookup and context formatting before targeted SQL.
pub(super) fn course_plan_context(
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

    match db.get_all_session_plans() {
        Ok(plans) => {
            if let Some((_, course_plans)) =
                plans.iter().find(|(code, _)| code.trim() == course_code)
            {
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
            } else {
                lines.push("授業計画: キャッシュなし".to_string());
            }
        }
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
