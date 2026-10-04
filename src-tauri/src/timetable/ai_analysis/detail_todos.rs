//! Extract actionable todos from Luna notices and student mail.

use super::*;
use std::hash::{Hash, Hasher};

// ── 詳細TODO抽出 (extract TODOs from Luna 消息/課題/通知) ──────────────────

const DETAIL_TODO_CACHE_KEY: &str = "ai_detail_todo_suggestions";
const DETAIL_TODO_CACHE_MAX_AGE: i64 = 6 * 3600; // 6 hours

const DETAIL_TODO_SYSTEM_PROMPT: &str = r#"あなたは関西学院大学のLunaシステムと学生のメール受信箱を監視するAIです。
Lunaに届いた最新の「お知らせ」「掲示板スレッド（メッセージ）」「課題」と、受信メールを読み、**明確な締切日時を含み、学生が実際に行動する必要があるTODO項目**だけを抽出してください。

## 抽出基準（厳格）
- **必ず明確な締切（日付・時刻）が文中に書かれている項目だけを採用**する。締切が読み取れない場合は採用しない。
- 単なる連絡・案内・既読推奨・補足説明・お礼などは採用しない。
- 既に提出済み・終了・回答済みと明示されているものは採用しない。
- 同じ内容が複数の通知やメールで重複している場合は、最新のものだけを採用する。
- メールは件名・本文プレビューから締切が読み取れる場合のみ採用。広告・宣伝・通知のみのメールは無視。

## 出力JSON形式（他のテキストは一切不要、マークダウンのコードブロックも使わない）
{
  "items": [
    {
      "title": "学生がやるべきこと（簡潔に1文、原文の語彙を活かす）",
      "course_name": "科目名（course_info そのまま）。メール由来でコース不明なら差出人名を入れる",
      "content_type": "課題|お知らせ|掲示板|テスト|アンケート|メール",
      "deadline": "YYYY-MM-DD HH:MM（時刻不明なら YYYY-MM-DD 23:59）",
      "source_url": "Luna 由来なら通知の url。メール由来ならメール一覧に書かれた 'mail://M数字' をそのまま",
      "source_excerpt": "通知の content や メールの subject/preview から該当箇所を 60〜140 字程度で抜粋",
      "note": "なぜこれをTODOと判断したかを 1 文（学生に提示する補足説明、20〜60字）"
    }
  ]
}

## 規則
- items が空でも `{"items": []}` を返す。
- deadlineは必ずYYYY-MM-DD HH:MM形式。元文に「6月20日 17:00まで」とあれば、現在の年を補って "2026-06-20 17:00" のように整形する。年が文中に無い場合は今日の日付から推測する（過去にならないよう来年にする）。
- title は学生視点で書く（例: 「第3回レポートを提出する」「アンケートに回答する」）。
- 1 通の通知やメールから複数のTODOが読み取れる場合は分割してよい。
- 回答は指定された言語で書くこと。"#;

const LOCAL_DETAIL_TODO_SYSTEM_PROMPT: &str = r#"Lunaの「お知らせ・掲示板・課題」と受信メールを読み、明確な締切がある実行必須項目のみJSONで抽出する。

重要:
- JSON以外の文字を出力しない
- マークダウンのコードブロックは使わない
- 締切日時が文中に無いものは絶対に出力しない
- メールの広告・案内・宣伝は無視

形式:
{
  "items": [
    {
      "title": "やることを1文",
      "course_name": "科目名 (メールならコース不明時に差出人名)",
      "content_type": "課題|お知らせ|掲示板|テスト|アンケート|メール",
      "deadline": "YYYY-MM-DD HH:MM",
      "source_url": "通知のurl または メール一覧に書かれた 'mail://M数字'",
      "source_excerpt": "原文の60〜140字抜粋",
      "note": "判断理由1文"
    }
  ]
}

ルール:
- items が無ければ {"items": []} を返す
- deadline は YYYY-MM-DD HH:MM 厳守。時刻不明は 23:59 とする
- 回答は指定された言語で書くこと"#;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DetailTodoSuggestion {
    pub title: String,
    pub course_name: String,
    pub content_type: String,
    pub deadline: String,
    pub source_url: String,
    pub source_excerpt: String,
    pub note: String,
}

fn detail_module_kind(module: &str) -> Option<&'static str> {
    let m = module.trim();
    if m.contains("掲示板") || m.contains("スレッド") || m.contains("メッセージ") {
        Some("掲示板")
    } else if m.contains("課題") || m.contains("レポート") {
        Some("課題")
    } else if m.contains("お知らせ") || m.contains("通知") {
        Some("お知らせ")
    } else if m.contains("テスト") || m.contains("試験") {
        Some("テスト")
    } else if m.contains("アンケート") || m.contains("調査") {
        Some("アンケート")
    } else {
        None
    }
}

fn build_detail_todo_fingerprint(
    notifs: &[(usize, &crate::luna_parser::LunaNotification)],
    mails: &[(usize, &crate::mail::MailMessage)],
) -> String {
    let mut parts: Vec<String> = notifs
        .iter()
        .map(|(_, n)| {
            format!(
                "L|{}|{}|{}|{}|{}",
                n.date, n.course_info, n.module, n.idnumber, n.content
            )
        })
        .collect();
    parts.extend(mails.iter().map(|(_, m)| {
        let body_sig = m
            .body
            .as_ref()
            .and_then(|b| b.content.as_deref())
            .unwrap_or("");
        format!(
            "M|{}|{}|{}|{}|{}",
            m.id,
            m.received_date_time.clone().unwrap_or_default(),
            m.subject.clone().unwrap_or_default(),
            m.body_preview.clone().unwrap_or_default(),
            body_sig.len()
        )
    }));
    parts.sort();
    let payload = parts.join("||");
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    payload.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn build_detail_todo_prompt(
    notifs: &[(usize, &crate::luna_parser::LunaNotification)],
    mails: &[(usize, &crate::mail::MailMessage)],
) -> String {
    let today = chrono::Local::now();
    let mut text = String::new();
    text.push_str(&format!(
        "## 今日: {} ({})\n",
        today.format("%Y年%m月%d日"),
        today.format("%A")
    ));
    text.push_str(&format!(
        "## 現在時刻: {}\n",
        today.format("%Y-%m-%d %H:%M")
    ));

    if !notifs.is_empty() {
        text.push_str("\n## Luna 通知一覧\n");
        text.push_str("以下は Luna の updateinfo に表示されている最新の お知らせ／掲示板／課題 です。各項目から **明確な締切がある TODO のみ** を抽出してください。\n\n");

        for (idx, n) in notifs {
            let kind = detail_module_kind(&n.module).unwrap_or("その他");
            text.push_str(&format!(
                "### [L{idx}] {date} | {course} | {module}（種別: {kind}）\n",
                idx = idx,
                date = n.date,
                course = if n.course_info.is_empty() {
                    "(コース不明)"
                } else {
                    n.course_info.as_str()
                },
                module = n.module,
                kind = kind,
            ));
            if !n.content.is_empty() {
                text.push_str(&format!("content: {}\n", n.content));
            }
            if !n.url.is_empty() {
                text.push_str(&format!("url: {}\n", n.url));
            }
            text.push('\n');
        }
    }

    if !mails.is_empty() {
        text.push_str("\n## メール一覧\n");
        text.push_str("以下は受信箱の最新メールです。**学生が締切までに行動する必要があるもの** (申請、提出、回答など) のみ TODO として抽出してください。広告／案内のみのメールは無視。\n\n");

        for (idx, m) in mails {
            let from_str = m
                .from
                .as_ref()
                .and_then(|f| {
                    let n = f.email_address.name.clone().unwrap_or_default();
                    let a = f.email_address.address.clone().unwrap_or_default();
                    if !n.is_empty() && !a.is_empty() {
                        Some(format!("{} <{}>", n, a))
                    } else if !n.is_empty() {
                        Some(n)
                    } else if !a.is_empty() {
                        Some(a)
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| "(差出人不明)".to_string());
            text.push_str(&format!(
                "### [M{idx}] {date} | {from} | 種別: メール\n",
                idx = idx,
                date = m.received_date_time.clone().unwrap_or_default(),
                from = from_str,
            ));
            if let Some(subj) = &m.subject {
                if !subj.is_empty() {
                    text.push_str(&format!("subject: {}\n", subj));
                }
            }
            // Prefer the full plain-text body; fall back to the short preview when
            // older cache entries lack it. Truncate so a single huge mail doesn't
            // dominate the prompt budget.
            let full_body = m
                .body
                .as_ref()
                .and_then(|b| b.content.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            match full_body {
                Some(content) => {
                    let truncated: String = content.chars().take(1200).collect();
                    text.push_str(&format!("body: {}\n", truncated));
                    if content.chars().count() > 1200 {
                        text.push_str("body: ...(truncated)\n");
                    }
                }
                None => {
                    if let Some(preview) = &m.body_preview {
                        if !preview.is_empty() {
                            text.push_str(&format!("preview: {}\n", preview));
                        }
                    }
                }
            }
            text.push_str(&format!("source_url: mail://M{idx}\n\n", idx = idx));
        }
    }

    text
}

fn common_ascii_suffix_len(a: &str, b: &str) -> usize {
    a.as_bytes()
        .iter()
        .rev()
        .zip(b.as_bytes().iter().rev())
        .take_while(|(left, right)| left == right)
        .count()
}

fn has_matching_id_prefix(partial: &str, full: &str) -> bool {
    let prefix_len = partial.len().min(24);
    prefix_len >= 8 && full.starts_with(&partial[..prefix_len])
}

pub(super) fn resolve_mail_source_url(
    source_url: &str,
    mails: &[(usize, &crate::mail::MailMessage)],
) -> String {
    let trimmed = source_url.trim();
    let Some(raw) = trimmed.strip_prefix("mail://") else {
        return trimmed.to_string();
    };
    let raw = raw.trim();
    if let Some(label) = raw.strip_prefix('M') {
        if let Ok(idx) = label.parse::<usize>() {
            if let Some((_, mail)) = mails.iter().find(|(mail_idx, _)| *mail_idx == idx) {
                return format!("mail://{}", mail.id);
            }
        }
    }
    if mails.iter().any(|(_, mail)| mail.id == raw) {
        return format!("mail://{}", raw);
    }

    let mut best: Option<(&str, usize)> = None;
    let mut tied = false;
    for (_, mail) in mails {
        if !has_matching_id_prefix(raw, &mail.id) {
            continue;
        }
        let score = common_ascii_suffix_len(raw, &mail.id);
        if score < 12 {
            continue;
        }
        match best {
            Some((_, best_score)) if score == best_score => tied = true,
            Some((_, best_score)) if score <= best_score => {}
            _ => {
                best = Some((mail.id.as_str(), score));
                tied = false;
            }
        }
    }
    if !tied {
        if let Some((id, _)) = best {
            return format!("mail://{}", id);
        }
    }
    trimmed.to_string()
}

fn normalize_detail_todo_source_urls(
    items: &mut [DetailTodoSuggestion],
    mails: &[(usize, &crate::mail::MailMessage)],
) {
    for item in items {
        item.source_url = resolve_mail_source_url(&item.source_url, mails);
    }
}

#[tauri::command]
pub async fn ai_extract_detail_todos(
    db: State<'_, Database>,
    force: bool,
) -> Result<Vec<DetailTodoSuggestion>, String> {
    let config = ai::load_ai_config();

    let notifs: Vec<crate::luna_parser::LunaNotification> = db
        .get_data_cache("luna_updates")
        .ok()
        .flatten()
        .and_then(|(json, _)| serde_json::from_str(&json).ok())
        .unwrap_or_default();

    let filtered: Vec<(usize, &crate::luna_parser::LunaNotification)> = notifs
        .iter()
        .enumerate()
        .filter(|(_, n)| detail_module_kind(&n.module).is_some())
        .collect();

    let mails: Vec<crate::mail::MailMessage> = db
        .get_data_cache("mail_inbox")
        .ok()
        .flatten()
        .and_then(|(json, _)| serde_json::from_str(&json).ok())
        .unwrap_or_default();

    // Limit mail volume to keep prompt size manageable — most TODO-bearing mail
    // is recent; older items are usually already-actioned or noise.
    let mail_refs: Vec<(usize, &crate::mail::MailMessage)> =
        mails.iter().enumerate().take(40).collect();

    if filtered.is_empty() && mail_refs.is_empty() {
        return Ok(Vec::new());
    }

    let fingerprint = build_detail_todo_fingerprint(&filtered, &mail_refs);

    if !force {
        if let Ok(Some((json, ts))) = db.get_data_cache(DETAIL_TODO_CACHE_KEY) {
            let now = crate::db::epoch_secs();
            if now - ts < DETAIL_TODO_CACHE_MAX_AGE {
                if let Ok(cached) = serde_json::from_str::<serde_json::Value>(&json) {
                    if cached.get("_fingerprint").and_then(|v| v.as_str())
                        == Some(fingerprint.as_str())
                    {
                        if let Some(items) = cached.get("items") {
                            if let Ok(mut parsed) =
                                serde_json::from_value::<Vec<DetailTodoSuggestion>>(items.clone())
                            {
                                normalize_detail_todo_source_urls(&mut parsed, &mail_refs);
                                log::info!(
                                    "ai_extract_detail_todos: cache hit (age={}s, items={})",
                                    now - ts,
                                    parsed.len()
                                );
                                return Ok(parsed);
                            }
                        }
                    }
                }
            }
        }
    }

    let is_local = config.provider == "local";
    let prompt = build_detail_todo_prompt(&filtered, &mail_refs);

    log::info!(
        "ai_extract_detail_todos: calling AI ({} notifications, {} mails, {} chars prompt, local={})",
        filtered.len(),
        mail_refs.len(),
        prompt.len(),
        is_local
    );

    let lang_hint = ai::reply_language_hint(
        &config.reply_language,
        "\n\n重要: title, source_excerpt, note 等の文章は中文（简体字）で書く。course_name, deadline, source_url は元データのまま。",
        "\n\nIMPORTANT: Write title, source_excerpt, note in English. Keep course_name, deadline, source_url as-is.",
        "\n\n중요: title, source_excerpt, note는 한국어로 작성. course_name, deadline, source_url는 원본 그대로.",
    );
    let base_system = if is_local {
        LOCAL_DETAIL_TODO_SYSTEM_PROMPT
    } else {
        DETAIL_TODO_SYSTEM_PROMPT
    };
    let sys = if lang_hint.is_empty() {
        base_system.to_string()
    } else {
        format!("{}{}", base_system, lang_hint)
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
        "ai_extract_detail_todos: got response ({} chars)",
        response.len()
    );

    let json_str = if is_local {
        extract_json_from_local_response(&response)?
    } else {
        let sanitized = sanitize_ai_response_text(&response);
        if sanitized.is_empty() {
            return Err("AI応答が空です。".into());
        }
        extract_json_from_response(&sanitized).to_string()
    };

    let parsed: serde_json::Value = serde_json::from_str(&json_str)
        .or_else(|_| {
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

    let mut items: Vec<DetailTodoSuggestion> = parsed
        .get("items")
        .and_then(|v| serde_json::from_value::<Vec<DetailTodoSuggestion>>(v.clone()).ok())
        .unwrap_or_default();

    // Drop items without a parseable deadline — the prompt requires one, this enforces it.
    items.retain(|item| !item.deadline.trim().is_empty() && !item.title.trim().is_empty());
    normalize_detail_todo_source_urls(&mut items, &mail_refs);

    let cache_value = serde_json::json!({
        "items": items,
        "_fingerprint": fingerprint,
    });
    let _ = db.save_data_cache(
        DETAIL_TODO_CACHE_KEY,
        &serde_json::to_string(&cache_value).unwrap_or_default(),
    );

    Ok(items)
}
