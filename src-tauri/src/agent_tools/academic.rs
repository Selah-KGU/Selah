use super::*;
use std::collections::{HashMap, HashSet};

pub(super) fn normalize_text(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && !"[]()（）【】「」『』・,，.。:：!?！？_-".contains(*c))
        .map(normalize_cjk_char)
        .collect()
}

#[derive(Default)]
struct CourseAggregate {
    display_name: String,
    normalized_name: String,
    kgc_codes: HashSet<String>,
    luna_ids: HashSet<String>,
    teachers: HashSet<String>,
    current_slots: Vec<Value>,
    next_slots: Vec<Value>,
}

fn build_course_aggregates(db: &Database) -> Result<Vec<CourseAggregate>, String> {
    let snap = db.get_snapshot_state()?.unwrap_or_default();
    let mut map: HashMap<String, CourseAggregate> = HashMap::new();

    for (week_kind, week_label) in [
        ("current", snap.current_week_label),
        ("next", snap.next_week_label),
    ] {
        if week_label.is_empty() {
            continue;
        }
        for row in db.get_kgc_courses(&week_label).unwrap_or_default() {
            let key = normalize_text(&row.name);
            if key.is_empty() {
                continue;
            }
            let entry = map.entry(key.clone()).or_insert_with(|| CourseAggregate {
                display_name: row.name.clone(),
                normalized_name: key.clone(),
                ..Default::default()
            });
            entry.kgc_codes.insert(row.kgc_code.clone());
            let slot = json!({
                "day": row.day,
                "period": row.period,
                "room": row.room,
                "kgc_code": row.kgc_code,
                "cancelled": row.is_cancelled,
                "makeup": row.is_makeup,
                "room_changed": row.is_room_changed,
            });
            if week_kind == "current" {
                entry.current_slots.push(slot);
            } else {
                entry.next_slots.push(slot);
            }
        }
    }

    for row in db.get_luna_courses().unwrap_or_default() {
        let key = normalize_text(&row.name);
        if key.is_empty() {
            continue;
        }
        let entry = map.entry(key.clone()).or_insert_with(|| CourseAggregate {
            display_name: row.name.clone(),
            normalized_name: key.clone(),
            ..Default::default()
        });
        entry.luna_ids.insert(row.luna_id);
        if !row.teacher.trim().is_empty() {
            entry.teachers.insert(row.teacher);
        }
    }

    Ok(map.into_values().collect())
}

fn bigram_similarity(a: &str, b: &str) -> f64 {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    if a_chars.len() < 2 || b_chars.len() < 2 {
        return 0.0;
    }
    let a_bigrams: HashSet<(char, char)> = a_chars.windows(2).map(|w| (w[0], w[1])).collect();
    let b_bigrams: HashSet<(char, char)> = b_chars.windows(2).map(|w| (w[0], w[1])).collect();
    let intersection = a_bigrams.intersection(&b_bigrams).count();
    if intersection == 0 {
        return 0.0;
    }
    2.0 * intersection as f64 / (a_bigrams.len() + b_bigrams.len()) as f64
}

fn score_course_match(query: &str, aggregate: &CourseAggregate) -> i32 {
    let query = normalize_text(query);
    if query.is_empty() {
        return 0;
    }
    let mut score = 0;
    if aggregate.normalized_name == query {
        score += 120;
    } else if aggregate.normalized_name.contains(&query) {
        score += 90;
    } else if query.contains(&aggregate.normalized_name) {
        score += 60;
    }
    if aggregate
        .kgc_codes
        .iter()
        .any(|code| normalize_text(code) == query)
    {
        score += 140;
    }
    if aggregate
        .kgc_codes
        .iter()
        .any(|code| normalize_text(code).contains(&query))
    {
        score += 70;
    }
    if aggregate
        .luna_ids
        .iter()
        .any(|id| normalize_text(id) == query)
    {
        score += 100;
    }
    if aggregate
        .teachers
        .iter()
        .any(|teacher| normalize_text(teacher).contains(&query))
    {
        score += 40;
    }
    // Cross-lingual fuzzy matching when exact/substring matching fails
    if score == 0 {
        let sim = bigram_similarity(&query, &aggregate.normalized_name);
        if sim >= 0.35 {
            score += (sim * 70.0) as i32;
        }
    }
    score
}

fn course_match_json(aggregate: &CourseAggregate) -> Value {
    let mut kgc_codes: Vec<_> = aggregate.kgc_codes.iter().cloned().collect();
    let mut luna_ids: Vec<_> = aggregate.luna_ids.iter().cloned().collect();
    let mut teachers: Vec<_> = aggregate.teachers.iter().cloned().collect();
    kgc_codes.sort();
    luna_ids.sort();
    teachers.sort();
    json!({
        "name": aggregate.display_name,
        "kgc_codes": kgc_codes,
        "luna_ids": luna_ids,
        "teachers": teachers,
        "current_slots": aggregate.current_slots,
        "next_slots": aggregate.next_slots,
    })
}

pub(super) async fn search_courses(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let query = sanitize_text_arg(args, "query", 80).ok_or_else(|| "query が空です".to_string())?;
    let db = app.state::<Database>();
    let mut matches: Vec<(i32, CourseAggregate)> = build_course_aggregates(&db)?
        .into_iter()
        .map(|aggregate| (score_course_match(&query, &aggregate), aggregate))
        .filter(|(score, _)| *score > 0)
        .collect();
    matches.sort_by(|a, b| b.0.cmp(&a.0));
    let items: Vec<Value> = matches
        .into_iter()
        .take(5)
        .map(|(_, aggregate)| course_match_json(&aggregate))
        .collect();
    Ok(json!({ "query": query, "matches": items }))
}

pub(super) async fn get_course_context(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let query = sanitize_text_arg(args, "query", 80).ok_or_else(|| "query が空です".to_string())?;
    let db = app.state::<Database>();
    let mut matches: Vec<(i32, CourseAggregate)> = build_course_aggregates(&db)?
        .into_iter()
        .map(|aggregate| (score_course_match(&query, &aggregate), aggregate))
        .filter(|(score, _)| *score > 0)
        .collect();
    matches.sort_by(|a, b| b.0.cmp(&a.0));
    let Some((_, best)) = matches.first() else {
        return Err(format!("{} に一致する科目が見つかりません", query));
    };

    let all_plans = db.get_all_session_plans().unwrap_or_default();
    let all_counts = db.get_all_luna_counts().unwrap_or_default();
    let all_activities = db.get_all_luna_activities().unwrap_or_default();
    let first_kgc_code = best.kgc_codes.iter().next().cloned().unwrap_or_default();
    let detail = if first_kgc_code.is_empty() {
        None
    } else {
        db.get_kgc_course_detail(&first_kgc_code)?
    };
    let session_plan = if first_kgc_code.is_empty() {
        Vec::new()
    } else {
        all_plans
            .into_iter()
            .find(|(kgc_code, _)| kgc_code == &first_kgc_code)
            .map(|(_, plans)| {
                plans
                    .into_iter()
                    .take(15)
                    .map(|p| {
                        json!({
                            "session": p.session_num,
                            "topic": p.topic,
                            "delivery_mode": p.delivery_mode,
                            "study_outside": p.study_outside,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };

    let mut luna_counts = Vec::new();
    for luna_id in &best.luna_ids {
        if let Some((_, counts)) = all_counts.iter().find(|(id, _)| id == luna_id) {
            luna_counts.push(json!({
                "luna_id": luna_id,
                "announcements": counts.announcements,
                "new_announcements": counts.new_announcements,
                "reports": counts.reports,
                "exams": counts.exams,
                "discussions": counts.discussions,
            }));
        }
    }
    let activities: Vec<Value> = all_activities
        .into_iter()
        .filter(|activity| best.luna_ids.contains(&activity.luna_id))
        .take(20)
        .map(|activity| {
            json!({
                "luna_id": activity.luna_id,
                "type": activity.activity_type,
                "title": activity.title,
                "period": activity.period,
                "status": activity.status,
            })
        })
        .collect();

    let mut online_tools = Vec::new();
    let mut materials = Vec::new();
    for luna_id in &best.luna_ids {
        let cache_key = format!("luna_course:{}", luna_id);
        let Some((json_str, _)) = db.get_data_cache(&cache_key)? else {
            continue;
        };
        let Ok(contents) =
            serde_json::from_str::<crate::luna_parser::LunaCourseContents>(&json_str)
        else {
            continue;
        };
        online_tools.extend(contents.online_tools.into_iter().take(10).map(|tool| {
            json!({
                "name": tool.name,
                "url": tool.url,
                "icon": tool.icon,
            })
        }));
        materials.extend(contents.materials.into_iter().take(10).map(|item| {
            json!({
                "title": item.title,
                "url": item.url,
                "period": item.period,
                "status": item.status,
                "item_type": item.item_type,
                "files": item.files.into_iter().take(10).map(|f| json!({
                    "display_name": f.display_name,
                    "file_name": f.file_name,
                    "link_type": f.link_type,
                })).collect::<Vec<_>>(),
            })
        }));
    }
    if online_tools.len() > 10 {
        online_tools.truncate(10);
    }
    if materials.len() > 10 {
        materials.truncate(10);
    }

    let top_matches: Vec<Value> = matches
        .iter()
        .take(3)
        .map(|(_, aggregate)| course_match_json(aggregate))
        .collect();
    Ok(json!({
        "query": query,
        "ambiguous": top_matches.len() > 1,
        "matches": top_matches,
        "course": {
            "name": best.display_name,
            "kgc_codes": best.kgc_codes.iter().cloned().collect::<Vec<_>>(),
            "luna_ids": best.luna_ids.iter().cloned().collect::<Vec<_>>(),
            "teachers": best.teachers.iter().cloned().collect::<Vec<_>>(),
            "current_slots": best.current_slots,
            "next_slots": best.next_slots,
            "online_tools": online_tools,
            "materials": materials,
            "detail": detail.as_ref().map(|d| json!({
                "delivery_mode": d.delivery_mode,
                "fields": d.fields.iter().take(12).collect::<Vec<_>>(),
                "textbooks": d.textbooks.iter().take(10).collect::<Vec<_>>(),
            })),
            "session_plan": session_plan,
            "luna_counts": luna_counts,
            "activities": activities,
        }
    }))
}

// ── Schedule tools ──

pub(super) async fn list_today_classes(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    let (week, dow) = current_week_and_dow(&db)?;
    let classes = collect_classes(&db, &week, Some(dow))?;
    Ok(json!({
        "day_of_week": dow_label(dow),
        "week_label": week,
        "classes": classes,
    }))
}

pub(super) async fn list_week_classes(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let db = app.state::<Database>();
    let offset = args.get("offset").and_then(|v| v.as_i64()).unwrap_or(0);
    let snap = db
        .get_snapshot_state()?
        .ok_or_else(|| "時間割データがありません".to_string())?;
    let week_label = if offset == 1 {
        snap.next_week_label.clone()
    } else {
        snap.current_week_label.clone()
    };
    if week_label.is_empty() {
        return Err("週ラベルが未設定です".into());
    }
    let classes = collect_classes(&db, &week_label, None)?;
    Ok(json!({
        "week_label": week_label,
        "offset": offset,
        "classes": classes,
    }))
}

fn current_week_and_dow(db: &Database) -> Result<(String, i32), String> {
    let snap = db
        .get_snapshot_state()?
        .ok_or_else(|| "時間割データがありません".to_string())?;
    let week = if snap.current_week_label.is_empty() {
        return Err("今週ラベルが未設定です".into());
    } else {
        snap.current_week_label
    };
    use chrono::Datelike;
    let dow = chrono::Local::now().weekday().number_from_monday() as i32; // 1=Mon..7=Sun
    Ok((week, dow))
}

fn dow_label(dow: i32) -> &'static str {
    match dow {
        1 => "月曜日",
        2 => "火曜日",
        3 => "水曜日",
        4 => "木曜日",
        5 => "金曜日",
        6 => "土曜日",
        7 => "日曜日",
        _ => "?",
    }
}

fn collect_classes(
    db: &Database,
    week_label: &str,
    filter_day: Option<i32>,
) -> Result<Vec<Value>, String> {
    let kgc = db.get_kgc_courses(week_label).unwrap_or_default();
    let luna = db.get_luna_courses().unwrap_or_default();
    let mut out: Vec<Value> = Vec::new();

    for c in kgc.iter() {
        if let Some(d) = filter_day {
            if c.day != d {
                continue;
            }
        }
        out.push(json!({
            "source": "kgc",
            "day": c.day,
            "period": c.period,
            "name": c.name,
            "room": c.room,
            "kgc_code": c.kgc_code,
            "cancelled": c.is_cancelled,
            "makeup": c.is_makeup,
            "room_changed": c.is_room_changed,
        }));
    }
    for c in luna.iter() {
        if let Some(d) = filter_day {
            if c.day != d {
                continue;
            }
        }
        out.push(json!({
            "source": "luna",
            "day": c.day,
            "period": c.period,
            "name": c.name,
            "teacher": c.teacher,
            "luna_id": c.luna_id,
        }));
    }
    // Sort by day then period
    out.sort_by_key(|v| {
        (
            v.get("day").and_then(|x| x.as_i64()).unwrap_or(0),
            v.get("period").and_then(|x| x.as_i64()).unwrap_or(0),
        )
    });
    if out.len() > LIST_CAP {
        out.truncate(LIST_CAP);
    }
    Ok(out)
}

// ── Luna activity tools ──

pub(super) async fn list_luna_todos(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
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

pub(super) async fn list_luna_announcements(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let db = app.state::<Database>();
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

pub(super) async fn get_notification_detail(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let title =
        sanitize_text_arg(args, "title", 200).ok_or_else(|| "title が空です".to_string())?;
    let needle = normalize_text(&title);
    let db = app.state::<Database>();

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
        "「{}」に一致するお知らせがキャッシュ内に見つかりません。先にlist_recent_notificationsかsearch_notificationsで一覧を取得してください",
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

pub(super) async fn list_recent_notifications(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let db = app.state::<Database>();
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
    let limit = limit.min(LIST_CAP);
    let items = collect_notifications(&db, None, limit);
    Ok(json!({ "notifications": items }))
}

pub(super) async fn search_notifications(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let keyword = args
        .get("keyword")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if keyword.is_empty() {
        return Err("keyword が空です".into());
    }
    let db = app.state::<Database>();
    let items = collect_notifications(&db, Some(&keyword), LIST_CAP);
    Ok(json!({ "keyword": keyword, "notifications": items }))
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

pub(super) async fn get_course_detail(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let code = args
        .get("kgc_code")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if code.is_empty() {
        return Err("kgc_code が空です".into());
    }
    let db = app.state::<Database>();
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
