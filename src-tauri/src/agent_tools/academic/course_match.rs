use super::*;
use std::collections::{HashMap, HashSet};

pub fn normalize_text(s: &str) -> String {
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
    let scope = crate::academic_period::visible_weeks(
        &snap.current_week_label,
        &snap.next_week_label,
        &snap.luna_year,
        &snap.luna_term,
        chrono::Local::now().date_naive(),
    );
    let mut map: HashMap<String, CourseAggregate> = HashMap::new();

    for (week_kind, week_label) in [("current", scope.current), ("next", scope.next)] {
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

pub async fn search_courses(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
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

pub async fn get_course_context(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
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
