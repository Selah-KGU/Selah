use super::*;

pub(super) async fn get_weather(_app: &tauri::AppHandle) -> Result<Value, String> {
    let data: crate::commands::WeatherData = crate::commands::fetch_weather().await?;
    let desc = wmo_description(data.weather_code);
    let mut out = json!({
        "location": "西宮上ケ原キャンパス",
        "current": {
            "temperature_c": data.temperature,
            "weather": desc,
            "humidity_pct": data.humidity,
            "wind_kmh": data.wind_speed,
        },
    });
    if let Some(t) = data.tomorrow {
        out["tomorrow"] = json!({
            "weather": wmo_description(t.weather_code),
            "temp_max_c": t.temp_max,
            "temp_min_c": t.temp_min,
        });
    }
    Ok(out)
}

fn wmo_description(code: i32) -> &'static str {
    match code {
        0 => "快晴",
        1 => "晴れ",
        2 => "晴れ時々曇り",
        3 => "曇り",
        45 | 48 => "霧",
        51 | 53 | 55 => "霧雨",
        61 | 63 | 65 => "雨",
        66 | 67 => "凍雨",
        71 | 73 | 75 => "雪",
        77 => "霰",
        80..=82 => "にわか雨",
        85 | 86 => "にわか雪",
        95 => "雷雨",
        96 | 99 => "雷雨(雹)",
        _ => "不明",
    }
}

pub(super) async fn get_weekly_summary(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    let (cache, _ts) = db
        .get_ai_schedule_cache()?
        .ok_or_else(|| "週間サマリーがまだ生成されていません".to_string())?;
    let snap = db.get_snapshot_state()?.unwrap_or_default();
    let scope = crate::academic_period::visible_weeks(
        &snap.current_week_label,
        &snap.next_week_label,
        &snap.luna_year,
        &snap.luna_term,
        chrono::Local::now().date_naive(),
    );
    if !crate::academic_period::week_belongs_to_visible_label(
        &scope.current,
        &cache.current_week_label,
    ) {
        return Err("今学期の週間サマリーはまだありません".into());
    }
    Ok(json!({
        "current_week": cache.current_week_label,
        "next_week": cache.next_week_label,
        "weekly_summary": cache.weekly_summary,
        "cross_week_insights": cache.cross_week_insights,
    }))
}

pub(super) async fn get_todo_guide(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    let (json_str, ts) = db.get_data_cache("ai_todo_analysis")?.ok_or_else(|| {
        "課題ガイドがまだ生成されていません。ホーム画面で課題一覧を取得してください。".to_string()
    })?;
    let v: Value = serde_json::from_str(&json_str).map_err(|e| format!("JSON解析失敗: {}", e))?;
    let age_hours = (chrono::Utc::now().timestamp() - ts) / 3600;
    Ok(json!({
        "generated_hours_ago": age_hours,
        "task_guides": v.get("task_guides"),
        "daily_plan": v.get("daily_plan"),
        "priority_summary": v.get("priority_summary"),
    }))
}

pub(super) async fn get_upcoming_deadlines(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    let acts = db.get_all_luna_activities().unwrap_or_default();
    let luna_courses = db.get_luna_courses().unwrap_or_default();
    let now = chrono::Local::now();
    let snap = db.get_snapshot_state().ok().flatten().unwrap_or_default();
    let scope = crate::academic_period::visible_weeks(
        &snap.current_week_label,
        &snap.next_week_label,
        &snap.luna_year,
        &snap.luna_term,
        now.date_naive(),
    );

    let mut items: Vec<Value> = Vec::new();
    for a in &acts {
        if !matches!(a.activity_type.as_str(), "report" | "exam" | "discussion") {
            continue;
        }
        if !crate::db::luna_course_matches_snapshot(&a.luna_id, &scope.year, &scope.term) {
            continue;
        }
        let course_name = luna_courses
            .iter()
            .find(|c| c.luna_id == a.luna_id)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        let submitted = a.status.contains("提出済") || a.status.contains("回答済");
        let urgency = deadline_urgency(&a.period, &now);
        items.push(json!({
            "type": a.activity_type,
            "course": course_name,
            "title": a.title,
            "deadline": a.period,
            "status": a.status,
            "submitted": submitted,
            "urgency": urgency,
        }));
    }
    items.sort_by_key(|v| {
        let u = v
            .get("urgency")
            .and_then(|x| x.as_str())
            .unwrap_or("normal");
        let sub = v
            .get("submitted")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        match (sub, u) {
            (true, _) => 4,
            (_, "overdue") => 0,
            (_, "critical") => 1,
            (_, "soon") => 2,
            _ => 3,
        }
    });
    if items.len() > LIST_CAP {
        items.truncate(LIST_CAP);
    }
    Ok(json!({ "deadlines": items }))
}

pub(super) async fn get_today_brief(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    let now = chrono::Local::now();
    use chrono::Datelike;
    let dow = now.weekday().number_from_monday() as i32;
    let dow_label = match dow {
        1 => "月曜日",
        2 => "火曜日",
        3 => "水曜日",
        4 => "木曜日",
        5 => "金曜日",
        6 => "土曜日",
        7 => "日曜日",
        _ => "?",
    };

    // Today's classes (best-effort: snapshot may be missing).
    let schedule_scope = db.get_snapshot_state().ok().flatten().map(|snap| {
        crate::academic_period::visible_weeks(
            &snap.current_week_label,
            &snap.next_week_label,
            &snap.luna_year,
            &snap.luna_term,
            now.date_naive(),
        )
    });
    let classes: Vec<Value> = match schedule_scope.as_ref() {
        Some(scope) => {
            let kgc = db.get_kgc_courses(&scope.current).unwrap_or_default();
            let luna = db.get_luna_courses().unwrap_or_default();
            let mut out: Vec<Value> = Vec::new();
            for c in kgc.iter().filter(|c| c.day == dow) {
                out.push(json!({
                    "source": "kgc",
                    "period": c.period,
                    "name": c.name,
                    "room": c.room,
                    "cancelled": c.is_cancelled,
                    "makeup": c.is_makeup,
                    "room_changed": c.is_room_changed,
                }));
            }
            for c in luna.iter().filter(|c| c.day == dow) {
                out.push(json!({
                    "source": "luna",
                    "period": c.period,
                    "name": c.name,
                    "teacher": c.teacher,
                }));
            }
            out.sort_by_key(|v| v.get("period").and_then(|x| x.as_i64()).unwrap_or(0));
            out
        }
        _ => Vec::new(),
    };

    // Most-urgent unsubmitted deadlines (max 5).
    let acts = db.get_all_luna_activities().unwrap_or_default();
    let luna_courses = db.get_luna_courses().unwrap_or_default();
    let mut deadlines: Vec<Value> = Vec::new();
    for a in &acts {
        if !matches!(a.activity_type.as_str(), "report" | "exam" | "discussion") {
            continue;
        }
        if a.status.contains("提出済") || a.status.contains("回答済") {
            continue;
        }
        if let Some(scope) = schedule_scope.as_ref() {
            if !crate::db::luna_course_matches_snapshot(&a.luna_id, &scope.year, &scope.term) {
                continue;
            }
        }
        let urgency = deadline_urgency(&a.period, &now);
        if !matches!(urgency, "overdue" | "critical" | "soon") {
            continue;
        }
        let course = luna_courses
            .iter()
            .find(|c| c.luna_id == a.luna_id)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        deadlines.push(json!({
            "type": a.activity_type,
            "course": course,
            "title": a.title,
            "deadline": a.period,
            "urgency": urgency,
        }));
    }
    deadlines.sort_by_key(|v| {
        match v
            .get("urgency")
            .and_then(|u| u.as_str())
            .unwrap_or("normal")
        {
            "overdue" => 0,
            "critical" => 1,
            "soon" => 2,
            _ => 3,
        }
    });
    deadlines.truncate(5);

    // Weather (best-effort).
    let weather = match crate::commands::fetch_weather().await {
        Ok(data) => Some(json!({
            "temperature_c": data.temperature,
            "humidity_pct": data.humidity,
            "wind_kmh": data.wind_speed,
            "weather_code": data.weather_code,
        })),
        Err(_) => None,
    };

    Ok(json!({
        "date": now.format("%Y-%m-%d").to_string(),
        "day_of_week": dow_label,
        "classes": classes,
        "urgent_deadlines": deadlines,
        "weather": weather,
    }))
}

pub(super) async fn refresh_data(app: &tauri::AppHandle) -> Result<Value, String> {
    let started = std::time::Instant::now();
    let luna_state = app.state::<crate::LunaState>();
    let db = app.state::<Database>();
    let updated = crate::timetable::refresh_luna_counts_internal(&luna_state, &db, true)
        .await
        .map_err(|e| format!("データ更新失敗: {}", e))?;
    Ok(json!({
        "scope": "luna_activities",
        "courses_refreshed": updated,
        "elapsed_ms": started.elapsed().as_millis() as u64,
    }))
}

pub(super) async fn get_luna_activity_detail(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let luna_id_arg = args
        .get("luna_id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim());

    let db = app.state::<Database>();
    let acts = db.get_all_luna_activities().unwrap_or_default();
    if acts.is_empty() {
        return Err("Luna活動データがまだ同期されていません".into());
    }

    // Filter acts by luna_id if it's explicitly specified in arguments (helps matching identical titles across courses)
    let filtered_acts: Vec<_> = if let Some(lid) = luna_id_arg {
        if !lid.is_empty() {
            acts.into_iter().filter(|a| a.luna_id == lid).collect()
        } else {
            acts
        }
    } else {
        acts
    };

    if filtered_acts.is_empty() {
        return Err("指定されたluna_idに一致するLuna活動データが存在しません".into());
    }

    // We can also have an optional activity_type arg if specified
    let activity_type_arg = args
        .get("activity_type")
        .and_then(|v| v.as_str())
        .map(|s| s.trim());

    let final_acts: Vec<_> = if let Some(atype) = activity_type_arg {
        if !atype.is_empty() {
            filtered_acts
                .into_iter()
                .filter(|a| a.activity_type == atype)
                .collect()
        } else {
            filtered_acts
        }
    } else {
        filtered_acts
    };

    if final_acts.is_empty() {
        return Err("指定されたactivity_typeに一致するLuna活動データが存在しません".into());
    }

    if title.is_empty() {
        // If title is empty but we have unique luna_id + activity_type, we can fall back to the first one
        if final_acts.len() == 1 {
            let row = &final_acts[0];
            if row.detail_path.is_empty() {
                return Err("詳細ページのパスが記録されていません".into());
            }
            return fetch_and_parse_detail(app, row, db).await;
        }
        return Err("titleを指定してください".into());
    }

    // Find best match: exact -> contains(title) -> contains(fragment).
    let needle = title.to_lowercase();
    let best = final_acts
        .iter()
        .find(|a| a.title == title)
        .or_else(|| {
            final_acts
                .iter()
                .find(|a| a.title.to_lowercase().contains(&needle))
        })
        .or_else(|| {
            final_acts
                .iter()
                .find(|a| needle.contains(&a.title.to_lowercase()) && !a.title.is_empty())
        });

    let row = match best {
        Some(r) if !r.detail_path.is_empty() => r,
        Some(_) => {
            return Err(format!(
                "「{}」には詳細ページのパスが記録されていません。時間割を再同期してください。",
                title
            ));
        }
        None => {
            let candidates: Vec<&str> = final_acts
                .iter()
                .take(10)
                .map(|a| a.title.as_str())
                .collect();
            return Err(format!(
                "「{}」に一致する活動が見つかりませんでした。候補: {}",
                title,
                candidates.join(" / ")
            ));
        }
    };

    fetch_and_parse_detail(app, row, db).await
}

async fn fetch_and_parse_detail(
    app: &tauri::AppHandle,
    row: &crate::db::LunaActivityRow,
    db: tauri::State<'_, Database>,
) -> Result<Value, String> {
    let luna_courses = db.get_luna_courses().unwrap_or_default();
    let course_name = luna_courses
        .iter()
        .find(|c| c.luna_id == row.luna_id)
        .map(|c| c.name.clone())
        .unwrap_or_default();

    let (html, cache_age) =
        super::files_browser::fetch_luna_detail_html_with_age(app, &row.detail_path).await?;
    let url = format!("{}{}", crate::config::LUNA_BASE, row.detail_path);

    let parse_detail = |h: &str| -> crate::luna_parser::LunaDetailPage {
        if row.activity_type == "announcement" {
            crate::luna_parser::parse_luna_announcement_detail(h)
        } else {
            crate::luna_parser::parse_luna_detail_page(h)
        }
    };

    let mut detail = parse_detail(&html);

    // Only force a fresh fetch when the cache is not brand-new (> 60 s old).
    // Pages that were just refreshed and have no attachments are trusted as-is.
    if detail.attachments.is_empty() && cache_age > 60 {
        log::debug!(
            "[get_luna_activity_detail] no attachments in cache (age={}s) for '{}', forcing fresh fetch",
            cache_age,
            row.detail_path
        );
        let cache_key = format!("luna_detail_html:{}", row.detail_path);
        let _ = db.delete_data_cache(&cache_key);
        match super::files_browser::fetch_luna_detail_html_cached(app, &row.detail_path).await {
            Ok(fresh_html) => detail = parse_detail(&fresh_html),
            Err(e) => log::warn!("Fresh fetch failed for '{}': {}", row.detail_path, e),
        }
    }

    const SECTION_CAP: usize = 1200;
    let sections: Vec<Value> = detail
        .sections
        .iter()
        .map(|s| {
            let mut body = s.body.clone();
            if body.len() > SECTION_CAP {
                let mut cut = SECTION_CAP;
                while cut > 0 && !body.is_char_boundary(cut) {
                    cut -= 1;
                }
                body.truncate(cut);
                body.push_str("...<truncated>");
            }
            json!({ "heading": s.heading, "body": body })
        })
        .collect();

    let attachments: Vec<Value> = detail
        .attachments
        .iter()
        .take(10)
        .map(|a| {
            json!({
                "name": a.name,
                "type": a.link_type,
                "url": a.url,
                "object_name": a.object_name,
                "download_action": a.download_action,
                "download_params": a.download_params,
            })
        })
        .collect();

    let meta: Vec<Value> = detail
        .meta
        .iter()
        .take(10)
        .map(|(k, v)| json!({ "label": k, "value": v }))
        .collect();

    Ok(json!({
        "matched_title": row.title,
        "activity_type": row.activity_type,
        "source": {
            "service": "luna",
            "luna_id": row.luna_id,
            "detail_path": row.detail_path,
            "detail_url": url,
        },
        "course": course_name,
        "period": row.period,
        "status": row.status,
        "detail_title": detail.title,
        "detail_course_name": detail.course_name,
        "meta": meta,
        "sections": sections,
        "attachments": attachments,
    }))
}

fn deadline_urgency(period_str: &str, now: &chrono::DateTime<chrono::Local>) -> &'static str {
    let cleaned = period_str.replace('/', "-");
    let deadline = chrono::NaiveDateTime::parse_from_str(&cleaned, "%Y-%m-%d %H:%M")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(&cleaned, "%Y-%m-%d"));
    match deadline {
        Ok(dt) => {
            let local_dt = dt.and_local_timezone(chrono::Local).single();
            match local_dt {
                Some(d) => {
                    let hours = (d - *now).num_hours();
                    if hours < 0 {
                        "overdue"
                    } else if hours < 24 {
                        "critical"
                    } else if hours < 72 {
                        "soon"
                    } else {
                        "normal"
                    }
                }
                None => "normal",
            }
        }
        Err(_) => "normal",
    }
}
