use super::safe_preview;
use crate::ai;
use crate::db::{AiScheduleResult, Database, SnapshotState};

#[path = "ai_analysis/detail_todos.rs"]
mod detail_todos;
#[path = "ai_analysis/response_json.rs"]
mod response_json;
#[path = "ai_analysis/schedule_prompt.rs"]
mod schedule_prompt;
#[path = "ai_analysis/todo_context.rs"]
mod todo_context;

#[cfg(test)]
use detail_todos::resolve_mail_source_url;
pub use detail_todos::*;
use response_json::*;
use schedule_prompt::*;
use todo_context::*;

pub(crate) const AI_CACHE_MAX_AGE: i64 = 12 * 3600; // 12 hours

const TODO_AI_CACHE_KEY: &str = "ai_todo_analysis";
// 分析結果の有効期限。これを過ぎたキャッシュは無効とみなし、「再分析」が必要。
const TODO_AI_CACHE_MAX_AGE: i64 = 12 * 3600; // 12 hours

#[tauri::command]
pub async fn ai_generate_schedule(
    db: crate::db::AccountDb,
    current_week_label: String,
    next_week_label: String,
    force: bool,
) -> Result<AiScheduleResult, String> {
    ai_generate_schedule_internal(&db, current_week_label, next_week_label, force).await
}

pub async fn ai_generate_schedule_internal(
    db: &Database,
    current_week_label: String,
    next_week_label: String,
    force: bool,
) -> Result<AiScheduleResult, String> {
    if !force {
        if let Some((cached, _)) = load_ai_cache_inner(db)? {
            if cached.current_week_label == current_week_label {
                return Ok(cached);
            }
        }
    }

    let raw = db.build_raw_data(&current_week_label, &next_week_label, Vec::new())?;
    let config = ai::load_ai_config();

    let is_local = config.provider == "local";
    let prompt = build_ai_schedule_prompt(&raw, is_local);
    let lang_hint = ai::reply_language_hint(
        &config.reply_language,
        "\n\n重要: 所有文本字段用中文（简体字）写。科目名・日付保持原数据不变。",
        "\n\nIMPORTANT: Write all text fields in English. Keep course names and dates as-is.",
        "\n\n중요: 모든 텍스트 필드를 한국어로 작성. 과목명・날짜는 원본 그대로.",
    );
    log::info!(
        "ai_generate_schedule: calling AI with {} chars prompt (local={})",
        prompt.len(),
        is_local
    );
    if !is_local {
        log::debug!("ai_generate_schedule: full prompt:\n{}", prompt);
    }
    let base_system_prompt = if config.provider == "local" {
        LOCAL_SCHEDULE_SYSTEM_PROMPT
    } else {
        SCHEDULE_SYSTEM_PROMPT
    };
    let sys = if lang_hint.is_empty() {
        base_system_prompt.to_string()
    } else {
        format!("{}{}", base_system_prompt, lang_hint)
    };
    let messages = vec![
        ai::ChatMessage {
            role: "system".into(),
            content: sys,
            images: Vec::new(),
        },
        ai::ChatMessage {
            role: "user".into(),
            content: prompt,
            images: Vec::new(),
        },
    ];

    let response = ai::chat_completion_public(&config, messages).await?;
    log::info!(
        "ai_generate_schedule: got response ({} chars)",
        response.len()
    );
    if !is_local {
        log::debug!(
            "ai_generate_schedule: response preview: {}",
            safe_preview(&response, 500)
        );
    }
    let result =
        parse_ai_schedule_response(&response, &current_week_label, &next_week_label, is_local)?;
    log::info!(
        "ai_generate_schedule: parsed OK — current_week={} items, next_week={} items",
        result.current_week.len(),
        result.next_week.len()
    );

    db.save_ai_schedule_cache(&result)?;
    Ok(result)
}

#[tauri::command]
pub async fn ai_analyze_todo(
    db: crate::db::AccountDb,
    force: bool,
) -> Result<serde_json::Value, String> {
    ai_analyze_todo_internal(&db, force).await
}

pub async fn ai_analyze_todo_internal(
    db: &Database,
    force: bool,
) -> Result<serde_json::Value, String> {
    let config = ai::load_ai_config();

    let todo_items: Vec<crate::luna_parser::LunaTodoItem> = db
        .get_data_cache("luna_todo")
        .ok()
        .flatten()
        .and_then(|(json, _ts)| serde_json::from_str(&json).ok())
        .unwrap_or_default();

    if todo_items.is_empty() {
        return Err("TODO項目がありません。先にTODOリストを読み込んでください。".into());
    }

    // AI 補助モードは分析を自動で起動しない。force=false のときは AI を呼ばず、
    // 直近の「再分析」で保存した結果（あれば）をそのまま返す。ただし有効期限
    // (TODO_AI_CACHE_MAX_AGE) を過ぎた結果は無効とみなし、再分析を促す。
    if !force {
        if let Ok(Some((json, ts))) = db.get_data_cache(TODO_AI_CACHE_KEY) {
            if crate::db::epoch_secs() - ts > TODO_AI_CACHE_MAX_AGE {
                return Err(
                    "AI 分析結果の有効期限が切れました。「再分析」を実行してください。".into(),
                );
            }
            if let Ok(cached) = serde_json::from_str::<serde_json::Value>(&json) {
                return Ok(strip_todo_internal_fields(cached));
            }
        }
        return Err("AI 分析結果がまだありません。「再分析」を実行してください。".into());
    }

    // 期限切れ（締切超過）タスクは分析データに含めない。期限内の未提出タスクが
    // 1件も無ければ、AI を呼ばずに終了する。
    let now_naive = chrono::Local::now().naive_local();
    let has_actionable = todo_items
        .iter()
        .any(|t| !t.status.contains("提出済") && !todo_is_overdue(t, now_naive));
    if !has_actionable {
        return Err("期限内の未提出タスクがありません。".into());
    }

    let snap = db.get_snapshot_state()?.unwrap_or_default();
    let scope = crate::academic_period::visible_weeks(
        &snap.current_week_label,
        &snap.next_week_label,
        &snap.luna_year,
        &snap.luna_term,
        chrono::Local::now().date_naive(),
    );
    let raw = db.build_raw_data(&scope.current, &scope.next, Vec::new())?;
    let is_local = config.provider == "local";
    let live_notes = if is_local {
        Vec::new()
    } else {
        collect_relevant_live_notes(&todo_items)
    };

    let prompt = build_todo_ai_prompt(&todo_items, &raw, is_local, &live_notes);
    log::info!(
        "ai_analyze_todo: calling AI with {} chars prompt, {} todo items (local={})",
        prompt.len(),
        todo_items.len(),
        is_local
    );
    if !is_local {
        log::debug!("ai_analyze_todo: full prompt:\n{}", prompt);
    }

    let lang_hint = ai::reply_language_hint(
        &config.reply_language,
        "\n\n重要: background, live_note_summary, study_hints, ready_to_use_label, ready_to_use, advice, daily_plan.label, daily_plan.tasks 等所有文本用中文（简体字）写。task_name・course_name・deadline保持原数据不变。",
        "\n\nIMPORTANT: Write background, live_note_summary, study_hints, ready_to_use_label, ready_to_use, advice, daily_plan.label, daily_plan.tasks in English. Keep task_name, course_name, deadline as-is from source data.",
        "\n\n중요: background, live_note_summary, study_hints, ready_to_use_label, ready_to_use, advice, daily_plan.label, daily_plan.tasks 등 모든 텍스트를 한국어로 작성. task_name・course_name・deadline은 원본 데이터 그대로.",
    );
    let base_system_prompt = if config.provider == "local" {
        LOCAL_TODO_SYSTEM_PROMPT
    } else {
        TODO_SYSTEM_PROMPT
    };
    let sys = if lang_hint.is_empty() {
        base_system_prompt.to_string()
    } else {
        format!("{}{}", base_system_prompt, lang_hint)
    };
    let messages = vec![
        ai::ChatMessage {
            role: "system".into(),
            content: sys,
            images: Vec::new(),
        },
        ai::ChatMessage {
            role: "user".into(),
            content: prompt,
            images: Vec::new(),
        },
    ];

    let response = ai::chat_completion_public(&config, messages).await?;
    log::info!("ai_analyze_todo: got response ({} chars)", response.len());

    let json_str = if is_local {
        extract_json_from_local_response(&response)?
    } else {
        let sanitized = sanitize_ai_response_text(&response);
        if sanitized.is_empty() {
            return Err("AI応答が空です。".into());
        }
        extract_json_from_response(&sanitized).to_string()
    };
    let result: serde_json::Value = serde_json::from_str(&json_str)
        .or_else(|_| {
            log::warn!("ai todo: initial JSON parse failed, attempting truncation repair");
            let repaired = repair_truncated_json(&json_str);
            serde_json::from_str::<serde_json::Value>(&repaired)
        })
        .map_err(|e| {
            format!(
                "AI応答のJSON解析に失敗: {} — 応答: {}",
                e,
                safe_preview(&json_str, 200)
            )
        })?;

    let result = normalize_ai_todo_json(result);

    let cache_json = serde_json::to_string(&result).unwrap_or_default();
    let _ = db.save_data_cache(TODO_AI_CACHE_KEY, &cache_json);

    Ok(result)
}

pub(super) fn load_ai_cache(db: &Database) -> Result<(Option<AiScheduleResult>, bool), String> {
    let snapshot = db.get_snapshot_state()?;
    load_ai_cache_with_snapshot(db, snapshot.as_ref())
}

/// Reuse metadata already read for a response, preserving None versus a saved
/// default row. Standalone callers still read metadata through load_ai_cache.
pub(super) fn load_ai_cache_with_snapshot(
    db: &Database,
    snapshot: Option<&SnapshotState>,
) -> Result<(Option<AiScheduleResult>, bool), String> {
    load_ai_cache_inner(db).map(|opt| match opt {
        Some((mut result, ts)) => {
            if let Some(snapshot) = snapshot {
                let scope = crate::academic_period::visible_weeks(
                    &snapshot.current_week_label,
                    &snapshot.next_week_label,
                    &snapshot.luna_year,
                    &snapshot.luna_term,
                    chrono::Local::now().date_naive(),
                );
                let current_mismatch =
                    week_label_mismatch(&scope.current, &result.current_week_label);
                let next_mismatch = week_label_mismatch(&scope.next, &result.next_week_label);
                if current_mismatch {
                    result.current_week.clear();
                    result.current_week_label.clear();
                }
                if next_mismatch {
                    result.next_week.clear();
                    result.next_week_label.clear();
                }
                if current_mismatch || next_mismatch {
                    log::info!(
                        "load_ai_cache: week label mismatch, ignoring cached AI schedule for that week"
                    );
                }
                if result.current_week.is_empty() && result.next_week.is_empty() {
                    return (None, true);
                }
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let stale = now - ts > AI_CACHE_MAX_AGE;
            (Some(result), stale)
        }
        None => (None, true),
    })
}

fn week_label_mismatch(snapshot_label: &str, cached_label: &str) -> bool {
    !crate::academic_period::week_belongs_to_visible_label(snapshot_label, cached_label)
}

fn load_ai_cache_inner(db: &Database) -> Result<Option<(AiScheduleResult, i64)>, String> {
    db.get_ai_schedule_cache()
}

#[cfg(test)]
#[path = "ai_analysis/cache_tests.rs"]
mod cache_tests;
#[cfg(test)]
#[path = "ai_analysis/legacy_cache.rs"]
mod legacy_cache;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_live_note_markdown_omits_full_transcript() {
        let markdown = "# Course\n\n- 授業コード: TEST\n- 開始: 2026-04-21 09:00:00\n\n### 全体要約\n今日は4P分析とSTPを扱った。\n\n## 区間ごとの要約\n\n## 1つ目\n導入と事例整理。\n\n## 全文転写\n\n- [09:00] transcript";
        let compact = compact_live_note_markdown(markdown, 500);
        assert!(compact.contains("今日は4P分析とSTPを扱った。"));
        assert!(!compact.contains("transcript"));
    }

    #[test]
    fn normalize_ai_todo_json_fills_new_api_fields() {
        let normalized = normalize_ai_todo_json(serde_json::json!({
            "task_guides": [{
                "task_name": "課題A",
                "course_name": "科目A",
                "deadline": "2026/04/22 23:59",
                "urgency": "urgent",
                "background": "背景",
                "note_context": "授業ではSTPを扱った",
                "steps": ["整理する"],
                "draft_label": "提纲",
                "draft": "1. 導入",
                "minutes": "45"
            }],
            "daily_plan": [{
                "day": "今日（4/21）",
                "items": ["課題A（45分）"],
                "hours": "3.5"
            }],
            "summary": "先に着手する。"
        }));

        assert_eq!(normalized["task_guides"][0]["urgency"], "critical");
        assert_eq!(
            normalized["task_guides"][0]["live_note_summary"],
            "授業ではSTPを扱った"
        );
        assert_eq!(normalized["task_guides"][0]["ready_to_use_label"], "提纲");
        assert_eq!(normalized["daily_plan"][0]["free_hours"], 3.5);
        assert_eq!(normalized["advice"], "先に着手する。");
    }

    #[test]
    fn resolve_mail_source_url_maps_labels_and_ai_shortened_ids() {
        let mail_a = crate::mail::MailMessage {
            id: "AAMkPREFIX0123456789ABCDEFAAAAAAEMAABMIDDLEAAAlqnniAAA=".into(),
            subject: None,
            body_preview: None,
            body: None,
            from: None,
            received_date_time: None,
            is_read: Some(false),
            has_attachments: None,
        };
        let mail_b = crate::mail::MailMessage {
            id: "AAMkPREFIX0123456789ABCDEFAAAAAAEMAABMIDDLEAAAhuNzfAAA=".into(),
            subject: None,
            body_preview: None,
            body: None,
            from: None,
            received_date_time: None,
            is_read: Some(false),
            has_attachments: None,
        };
        let mails = vec![(2, &mail_a), (18, &mail_b)];

        assert_eq!(
            resolve_mail_source_url("mail://M18", &mails),
            "mail://AAMkPREFIX0123456789ABCDEFAAAAAAEMAABMIDDLEAAAhuNzfAAA="
        );
        assert_eq!(
            resolve_mail_source_url("mail://AAMkPREFIX0123456789ABCDEFAAAhuNzfAAA=", &mails),
            "mail://AAMkPREFIX0123456789ABCDEFAAAAAAEMAABMIDDLEAAAhuNzfAAA="
        );
        assert_eq!(
            resolve_mail_source_url("mail://WRONGPREFIX0123456789ABCDEFAAAhuNzfAAA=", &mails),
            "mail://WRONGPREFIX0123456789ABCDEFAAAhuNzfAAA="
        );
    }
}
