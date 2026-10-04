use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;

use crate::ai::{self, AiConfig};
use crate::background_refresh::BackendSessionStatusPayload;
use crate::db::{epoch_secs, Database};

use super::support::{cache_is_fresh, fresh_cache_json, fresh_course_names, load_cache_json};
use super::types::{
    AI_NOTIF_CACHE_KEY, FAST_INPUT_MAX_AGE_SECS, KWIC_INPUT_MAX_AGE_SECS, STABLE_INPUT_MAX_AGE_SECS,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnifiedNotif {
    source: String,
    title: String,
    category: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    course_info: String,
    date: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    section: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    kwic_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    information_type: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    person_category_cd: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    category_cd: String,
}
pub(in crate::ai_refresh) async fn refresh_notification_analysis(
    db: &Database,
    config: &AiConfig,
    session: &BackendSessionStatusPayload,
) -> Result<(), String> {
    let has_fresh_kgc =
        session.kgc_session_present && cache_is_fresh(db, "notifications", FAST_INPUT_MAX_AGE_SECS);
    let has_fresh_luna =
        session.luna_authenticated && cache_is_fresh(db, "luna_updates", FAST_INPUT_MAX_AGE_SECS);
    let has_fresh_kwic =
        session.kwic_authenticated && cache_is_fresh(db, "kwic_home", KWIC_INPUT_MAX_AGE_SECS);

    if !has_fresh_kgc && !has_fresh_luna && !has_fresh_kwic {
        return Err("通知データが最新ではないためAI分析をスキップします".to_string());
    }

    let notifications: crate::parser::NotificationsData = load_cache_json(db, "notifications")
        .unwrap_or(crate::parser::NotificationsData {
            entries: Vec::new(),
        });
    let luna_updates: Vec<crate::luna_parser::LunaNotification> =
        load_cache_json(db, "luna_updates").unwrap_or_default();
    let kwic_home: Option<crate::kwic_commands::KwicPortalHome> = load_cache_json(db, "kwic_home");
    let sources = build_unified_notifications(
        if has_fresh_kgc {
            Some(&notifications)
        } else {
            None
        },
        if has_fresh_luna {
            Some(&luna_updates)
        } else {
            None
        },
        if has_fresh_kwic {
            kwic_home.as_ref()
        } else {
            None
        },
    );

    if sources.is_empty() {
        return Err("通知データがまだありません".to_string());
    }

    let course_names = fresh_course_names(db)?;

    let todo_items: Vec<crate::luna_parser::LunaTodoItem> =
        if session.luna_authenticated && cache_is_fresh(db, "luna_todo", FAST_INPUT_MAX_AGE_SECS) {
            load_cache_json(db, "luna_todo").unwrap_or_default()
        } else {
            Vec::new()
        };
    let todo_summary = todo_items
        .iter()
        .take(8)
        .map(|item| {
            format!(
                "- {} / {} / {} / {}",
                item.course_name, item.content_type, item.content_name, item.deadline
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let profile_json =
        fresh_cache_json(db, "student_profile", STABLE_INPUT_MAX_AGE_SECS).unwrap_or_default();

    let system_prompt = build_notification_system_prompt(config);
    let user_prompt =
        build_notification_user_prompt(&profile_json, &course_names, &todo_summary, &sources);

    let response = ai::chat_completion_public(
        config,
        vec![
            ai::ChatMessage {
                role: "system".into(),
                content: system_prompt,
                images: Vec::new(),
            },
            ai::ChatMessage {
                role: "user".into(),
                content: user_prompt,
                images: Vec::new(),
            },
        ],
    )
    .await?;

    let result = parse_notification_response(&response)?;
    let payload = json!({
        "result": result,
        "sources": sources,
        "generated_at": epoch_secs(),
    });
    db.save_data_cache(AI_NOTIF_CACHE_KEY, &payload.to_string())
}
fn build_notification_system_prompt(config: &AiConfig) -> String {
    let lang_hint = ai::reply_language_hint(
        &config.reply_language,
        "\n\nsummary, important.title, important.reason, suggestions は中国語（简体字）で書く。",
        "\n\nWrite summary, important.title, important.reason, and suggestions in English.",
        "\n\nsummary, important.title, important.reason, suggestions 는 한국어로 작성.",
    );
    format!(
        r#"あなたは関西学院大学の学生向けパーソナル通知アシスタントです。
学生のプロフィール、履修科目、課題状況、通知一覧を受け取り、今この学生にとって重要な情報をJSON形式だけで返します。

判定基準:
- 日程が現在より前なら終了済みとして低優先または除外
- 学生の履修科目・学部・キャンパスと関係が強いものを優先
- 似たタイトルでも各通知を個別に扱う
- 通知文そのままの繰り返しではなく、学生が次に動ける一歩を提案する

出力JSON形式:
{{"summary":"80〜150字","important":[{{"title":"20字以内","reason":"15字以内","index":1}}],"suggestions":["10〜20字の行動提案"]}}

JSON以外の文字、Markdown、コードブロックは出力しない。{}"#,
        lang_hint
    )
}

fn build_notification_user_prompt(
    profile_json: &str,
    course_names: &str,
    todo_summary: &str,
    sources: &[UnifiedNotif],
) -> String {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    let notif_text = sources
        .iter()
        .enumerate()
        .map(|(idx, notif)| {
            format!(
                "{}. [{}] {} / {} / {}{}",
                idx + 1,
                notif.source,
                notif.date,
                notif.category,
                notif.title,
                if notif.course_info.is_empty() {
                    String::new()
                } else {
                    format!(" / 科目: {}", notif.course_info)
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "現在日時: {now}\n\n学生プロフィールJSON:\n{profile_json}\n\n履修科目:\n{course_names}\n\n未完了課題:\n{todo_summary}\n\n通知一覧:\n{notif_text}"
    )
}

fn build_unified_notifications(
    notifications: Option<&crate::parser::NotificationsData>,
    luna_updates: Option<&[crate::luna_parser::LunaNotification]>,
    kwic_home: Option<&crate::kwic_commands::KwicPortalHome>,
) -> Vec<UnifiedNotif> {
    let mut seen = HashSet::new();
    let mut merged = Vec::new();

    if let Some(notifications) = notifications {
        for notif in notifications.entries.iter().take(12) {
            push_unique(
                &mut merged,
                &mut seen,
                UnifiedNotif {
                    source: "kgc".into(),
                    title: notif.title.clone(),
                    category: notif.category.clone(),
                    course_info: String::new(),
                    date: notif.date.clone(),
                    section: String::new(),
                    url: notif.url.clone(),
                    kwic_id: String::new(),
                    information_type: String::new(),
                    person_category_cd: String::new(),
                    category_cd: String::new(),
                },
            );
        }
    }

    if let Some(luna_updates) = luna_updates {
        for notif in luna_updates.iter().take(12) {
            push_unique(
                &mut merged,
                &mut seen,
                UnifiedNotif {
                    source: "luna".into(),
                    title: notif.content.clone(),
                    category: if notif.module.is_empty() {
                        notif.course_info.clone()
                    } else {
                        notif.module.clone()
                    },
                    course_info: notif.course_info.clone(),
                    date: notif.date.clone(),
                    section: String::new(),
                    url: notif.url.clone(),
                    kwic_id: String::new(),
                    information_type: String::new(),
                    person_category_cd: String::new(),
                    category_cd: String::new(),
                },
            );
        }
    }

    if let Some(home) = kwic_home {
        for section in home.sections.iter() {
            if section.title == "メインリンク" || section.title == "注目コンテンツ" {
                continue;
            }
            for item in section.items.iter().take(8) {
                push_unique(
                    &mut merged,
                    &mut seen,
                    UnifiedNotif {
                        source: "kwic".into(),
                        title: item.title.clone(),
                        category: if item.category.is_empty() {
                            section.title.clone()
                        } else {
                            item.category.clone()
                        },
                        course_info: String::new(),
                        date: item.date.clone(),
                        section: section.title.clone(),
                        url: String::new(),
                        kwic_id: item.id.clone(),
                        information_type: item.information_type.clone(),
                        person_category_cd: item.person_category_cd.clone(),
                        category_cd: item.category_cd.clone(),
                    },
                );
            }
        }
    }

    merged.sort_by(|a, b| b.date.cmp(&a.date));
    merged.truncate(12);
    merged
}

fn push_unique(merged: &mut Vec<UnifiedNotif>, seen: &mut HashSet<String>, notif: UnifiedNotif) {
    let title_key = notif.title.split_whitespace().collect::<Vec<_>>().join("");
    let key = format!("{}|{}|{}", notif.source, title_key, notif.date);
    if seen.insert(key) {
        merged.push(notif);
    }
}

fn parse_notification_response(response: &str) -> Result<serde_json::Value, String> {
    let cleaned = strip_think_blocks(response);
    let start = cleaned
        .find('{')
        .ok_or_else(|| "AI通知分析のJSON開始位置が見つかりません".to_string())?;
    let end = cleaned
        .rfind('}')
        .ok_or_else(|| "AI通知分析のJSON終了位置が見つかりません".to_string())?;
    let mut value: serde_json::Value = serde_json::from_str(&cleaned[start..=end])
        .map_err(|e| format!("AI通知分析のJSON解析に失敗: {}", e))?;
    if let Some(obj) = value.as_object_mut() {
        obj.remove("_check");
    }
    Ok(value)
}

fn strip_think_blocks(text: &str) -> String {
    let mut out = text.to_string();
    loop {
        let lower = out.to_ascii_lowercase();
        let Some(start) = lower.find("<think") else {
            break;
        };
        let Some(rel_end) = lower[start..].find("</think>") else {
            out.replace_range(start.., "");
            break;
        };
        let end = start + rel_end + "</think>".len();
        out.replace_range(start..end, "");
    }
    out.replace("<think>", "")
        .replace("</think>", "")
        .trim()
        .to_string()
}
