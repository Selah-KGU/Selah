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
        80 | 81 | 82 => "にわか雨",
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

    let mut items: Vec<Value> = Vec::new();
    for a in &acts {
        if !matches!(a.activity_type.as_str(), "report" | "exam" | "discussion") {
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
    if title.is_empty() {
        return Err("titleを指定してください".into());
    }

    let db = app.state::<Database>();
    let acts = db.get_all_luna_activities().unwrap_or_default();
    if acts.is_empty() {
        return Err("Luna活動データがまだ同期されていません".into());
    }

    // Find best match: exact -> contains(title) -> contains(fragment).
    let needle = title.to_lowercase();
    let best = acts
        .iter()
        .find(|a| a.title == title)
        .or_else(|| {
            acts.iter()
                .find(|a| a.title.to_lowercase().contains(&needle))
        })
        .or_else(|| {
            acts.iter()
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
            let candidates: Vec<&str> = acts.iter().take(10).map(|a| a.title.as_str()).collect();
            return Err(format!(
                "「{}」に一致する活動が見つかりませんでした。候補: {}",
                title,
                candidates.join(" / ")
            ));
        }
    };

    let luna_courses = db.get_luna_courses().unwrap_or_default();
    let course_name = luna_courses
        .iter()
        .find(|c| c.luna_id == row.luna_id)
        .map(|c| c.name.clone())
        .unwrap_or_default();

    let luna_state = app.state::<crate::LunaState>();
    let http = {
        let luna = luna_state.client.lock().await;
        if !luna.authenticated {
            return Err(crate::luna_client::LUNA_AUTH_REQUIRED_MSG.into());
        }
        luna.http.clone()
    };
    let url = format!("{}{}", crate::config::LUNA_BASE, row.detail_path);
    let html = crate::client::fetch_with_redirect(
        &http,
        &url,
        crate::config::LUNA_BASE,
        crate::luna_client::LUNA_SESSION_EXPIRED_MSG,
        crate::luna_client::is_luna_session_expired,
    )
    .await
    .map_err(|e| format!("Luna取得失敗: {}", e))?;

    let detail = if row.activity_type == "announcement" {
        crate::luna_parser::parse_luna_announcement_detail(&html)
    } else {
        crate::luna_parser::parse_luna_detail_page(&html)
    };

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
