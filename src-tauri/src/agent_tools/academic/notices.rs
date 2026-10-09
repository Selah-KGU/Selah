use super::course_match::normalize_text;
use super::*;

// ── Luna activity tools ──

pub async fn list_luna_todos(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>().scope();
    let acts = db.get_all_luna_activities().unwrap_or_default();
    // Name lookup for luna courses
    let luna_courses = db.get_luna_courses().unwrap_or_default();
    let mut items: Vec<Value> = Vec::new();
    for a in acts.iter() {
        if !matches!(a.activity_type.as_str(), "report" | "exam" | "discussion") {
            continue;
        }
        if a.status.contains("提出済") || a.status.contains("回答済") {
            continue;
        }
        let course_name = luna_courses
            .iter()
            .find(|c| c.luna_id == a.luna_id)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        items.push(json!({
            "type": a.activity_type,
            "course": course_name,
            "luna_id": a.luna_id,
            "title": a.title,
            "deadline": a.period,
            "status": a.status,
        }));
    }
    if items.len() > LIST_CAP {
        items.truncate(LIST_CAP);
    }
    Ok(json!({ "todos": items }))
}

pub async fn list_luna_announcements(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let db = app.state::<Database>().scope();
    let acts = db.get_all_luna_activities().unwrap_or_default();
    let luna_courses = db.get_luna_courses().unwrap_or_default();
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .min(LIST_CAP as u64) as usize;
    let course_filter = sanitize_text_arg(args, "keyword", 80).map(|s| normalize_text(&s));

    let mut items: Vec<Value> = Vec::new();
    for a in acts.iter() {
        if a.activity_type != "announcement" {
            continue;
        }
        let course_name = luna_courses
            .iter()
            .find(|c| c.luna_id == a.luna_id)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        if let Some(filter) = course_filter.as_deref() {
            if !filter.is_empty() && !normalize_text(&course_name).contains(filter) {
                continue;
            }
        }
        items.push(json!({
            "course": course_name,
            "title": a.title,
            "period": a.period,
            "status": a.status,
            "luna_id": a.luna_id,
        }));
        if items.len() >= limit {
            break;
        }
    }
    Ok(json!({ "announcements": items }))
}

pub async fn get_notification_detail(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let title =
        sanitize_text_arg(args, "title", 200).ok_or_else(|| "title が空です".to_string())?;
    let needle = normalize_text(&title);
    let db = app.state::<Database>().scope();

    // 1) KWIC portal items — call live detail fetch with stored data attrs.
    if let Ok(Some((json_str, _))) = db.get_data_cache("kwic_home") {
        if let Ok(home) = serde_json::from_str::<crate::kwic_commands::KwicPortalHome>(&json_str) {
            for section in &home.sections {
                if section.title == "メインリンク" || section.title == "注目コンテンツ"
                {
                    continue;
                }
                if let Some(item) = section.items.iter().find(|i| {
                    let n = normalize_text(&i.title);
                    !n.is_empty() && (n == needle || n.contains(&needle) || needle.contains(&n))
                }) {
                    if item.id.is_empty() {
                        return Err(format!(
                            "「{}」はKWICポータルに見つかりましたが詳細IDが欠落しています",
                            item.title
                        ));
                    }
                    let detail = crate::kwic_commands::kwic_fetch_detail_internal(
                        app,
                        &item.id,
                        &item.information_type,
                        &item.person_category_cd,
                        &item.category_cd,
                    )
                    .await?;
                    let attachments: Vec<Value> = detail
                        .attachments
                        .iter()
                        .take(10)
                        .map(|a| json!({ "name": a.name, "url": a.url }))
                        .collect();
                    return Ok(json!({
                        "source": "KWIC",
                        "category": section.title,
                        "title": detail.title,
                        "date": detail.date,
                        "sender": detail.sender,
                        "body_html": truncate_chars(&detail.body_html, 8_000),
                        "attachments": attachments,
                    }));
                }
            }
        }
    }

    // 2) KGC notifications — fetch detail via stored URL.
    if let Ok(Some((json_str, _))) = db.get_data_cache("notifications") {
        if let Ok(data) = serde_json::from_str::<crate::parser::NotificationsData>(&json_str) {
            if let Some(entry) = data.entries.iter().find(|e| {
                let n = normalize_text(&e.title);
                !n.is_empty() && (n == needle || n.contains(&needle) || needle.contains(&n))
            }) {
                if entry.url.is_empty() {
                    return Ok(json!({
                        "source": "KGC",
                        "category": entry.category,
                        "title": entry.title,
                        "date": entry.date,
                        "body": "(KGC側はリストのみで、本文ページのリンクが取得できません)",
                        "attachments": [],
                    }));
                }
                let path = if entry.url.starts_with('/') {
                    entry.url.clone()
                } else if entry.url.starts_with("http") {
                    return Err("KGC外部リンクは未対応です".into());
                } else {
                    format!("/uniasv2/{}", entry.url)
                };
                let detail =
                    crate::commands::fetch_notification_detail_internal(app, &path).await?;
                let attachments: Vec<Value> = detail
                    .attachments
                    .iter()
                    .take(10)
                    .map(|a| json!({ "name": a.name, "url": a.url }))
                    .collect();
                return Ok(json!({
                    "source": "KGC",
                    "category": if detail.category.is_empty() { entry.category.clone() } else { detail.category.clone() },
                    "title": if detail.title.is_empty() { entry.title.clone() } else { detail.title },
                    "date": if detail.date.is_empty() { entry.date.clone() } else { detail.date },
                    "sender": detail.sender,
                    "body": truncate_chars(&detail.body, 8_000),
                    "attachments": attachments,
                }));
            }
        }
    }

    // 3) Luna updates — short message updates, body comes from list itself.
    if let Ok(Some((json_str, _))) = db.get_data_cache("luna_updates") {
        if let Ok(arr) = serde_json::from_str::<Vec<Value>>(&json_str) {
            if let Some(item) = arr.iter().find(|e| {
                let content = e.get("content").and_then(|x| x.as_str()).unwrap_or("");
                let n = normalize_text(content);
                !n.is_empty() && (n == needle || n.contains(&needle) || needle.contains(&n))
            }) {
                let content = item.get("content").and_then(|x| x.as_str()).unwrap_or("");
                let date = item.get("date").and_then(|x| x.as_str()).unwrap_or("");
                return Ok(json!({
                    "source": "Luna",
                    "title": content,
                    "date": date,
                    "body": content,
                    "note": "Luna更新はタイトル＝本文の短い通知のみ取得できます",
                    "attachments": [],
                }));
            }
        }
    }

    Err(format!(
        "「{}」に一致するお知らせがキャッシュ内に見つかりません。先にlist_recent_notificationsで一覧を取得してください",
        title
    ))
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max).collect();
    format!("{}…", truncated)
}

pub async fn list_recent_notifications(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let db = app.state::<Database>().scope();
    let keyword = args
        .get("keyword")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    // With a keyword this behaves as a search across all cached notifications;
    // without one it lists the most recent few.
    let limit = match keyword {
        Some(_) => LIST_CAP,
        None => (args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize).min(LIST_CAP),
    };
    let items = collect_notifications(&db, keyword, limit);
    let mut out = json!({ "notifications": items });
    if let Some(kw) = keyword {
        out["keyword"] = json!(kw);
    }
    Ok(out)
}

fn collect_notifications(db: &Database, keyword: Option<&str>, limit: usize) -> Vec<Value> {
    let mut out: Vec<(i64, Value)> = Vec::new();
    let kw_norm = keyword.map(normalize_text).filter(|s| !s.is_empty());
    let matches_kw = |hay: &str| -> bool {
        match kw_norm.as_deref() {
            Some(needle) => normalize_text(hay).contains(needle),
            None => true,
        }
    };

    // KGC notifications from data_cache["notifications"]
    if let Ok(Some((json_str, _))) = db.get_data_cache("notifications") {
        if let Ok(v) = serde_json::from_str::<Value>(&json_str) {
            if let Some(entries) = v.get("entries").and_then(|x| x.as_array()) {
                for e in entries {
                    let title = e.get("title").and_then(|x| x.as_str()).unwrap_or("");
                    let category = e.get("category").and_then(|x| x.as_str()).unwrap_or("");
                    if !matches_kw(&format!("{} {}", title, category)) {
                        continue;
                    }
                    let date_str = e.get("date").and_then(|x| x.as_str()).unwrap_or("");
                    let sortkey = date_score(date_str);
                    out.push((
                        sortkey,
                        json!({
                            "source": "KGC",
                            "identifier": e.get("id").and_then(|x| x.as_str()).unwrap_or(""),
                            "title": title,
                            "date": date_str,
                            "category": e.get("category").and_then(|x| x.as_str()).unwrap_or(""),
                        }),
                    ));
                }
            }
        }
    }
    // Luna updates
    if let Ok(Some((json_str, _))) = db.get_data_cache("luna_updates") {
        if let Ok(arr) = serde_json::from_str::<Vec<Value>>(&json_str) {
            for e in arr.iter() {
                let content = e.get("content").and_then(|x| x.as_str()).unwrap_or("");
                if !matches_kw(content) {
                    continue;
                }
                let date_str = e.get("date").and_then(|x| x.as_str()).unwrap_or("");
                let sortkey = date_score(date_str);
                out.push((
                    sortkey,
                    json!({
                        "source": "Luna",
                        "title": content,
                        "date": date_str,
                    }),
                ));
            }
        }
    }
    // KWIC portal home
    if let Ok(Some((json_str, _))) = db.get_data_cache("kwic_home") {
        if let Ok(v) = serde_json::from_str::<Value>(&json_str) {
            if let Some(sections) = v.get("sections").and_then(|x| x.as_array()) {
                for sec in sections {
                    let sec_title = sec.get("title").and_then(|x| x.as_str()).unwrap_or("");
                    if sec_title == "メインリンク" || sec_title == "注目コンテンツ" {
                        continue;
                    }
                    if let Some(items) = sec.get("items").and_then(|x| x.as_array()) {
                        for it in items {
                            let title = it.get("title").and_then(|x| x.as_str()).unwrap_or("");
                            if !matches_kw(&format!("{} {}", title, sec_title)) {
                                continue;
                            }
                            let date_str = it.get("date").and_then(|x| x.as_str()).unwrap_or("");
                            let sortkey = date_score(date_str);
                            out.push((
                                sortkey,
                                json!({
                                    "source": "KWIC",
                                    "identifier": it.get("id").and_then(|x| x.as_str()).unwrap_or(""),
                                    "title": title,
                                    "date": date_str,
                                    "category": sec_title,
                                }),
                            ));
                        }
                    }
                }
            }
        }
    }
    // Sort descending by date
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.truncate(limit);
    out.into_iter().map(|(_, v)| v).collect()
}

/// Rough date scoring: YYYY-MM-DD → YYYYMMDD.  Falls back to 0 when unparseable.
fn date_score(s: &str) -> i64 {
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    digits
        .chars()
        .take(8)
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

// ── Course detail ──

pub async fn get_course_detail(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let code = args
        .get("kgc_code")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if code.is_empty() {
        return Err("kgc_code が空です".into());
    }
    let db = app.state::<Database>().scope();
    let detail = db.get_kgc_course_detail(&code)?;
    let plans = db
        .get_all_session_plans()
        .unwrap_or_default()
        .into_iter()
        .find(|(k, _)| k == &code)
        .map(|(_, v)| v)
        .unwrap_or_default();
    if detail.is_none() && plans.is_empty() {
        return Err(format!("{} の詳細が見つかりません", code));
    }
    let detail_json = detail.map(|d| {
        json!({
            "delivery_mode": d.delivery_mode,
            "fields": d.fields.iter().take(12).collect::<Vec<_>>(),
        })
    });
    let plan_summary: Vec<_> = plans
        .iter()
        .take(15)
        .map(|p| {
            json!({
                "session": p.session_num,
                "topic": p.topic,
                "th_header": p.th_header,
            })
        })
        .collect();
    Ok(json!({
        "kgc_code": code,
        "detail": detail_json,
        "session_plan": plan_summary,
    }))
}
