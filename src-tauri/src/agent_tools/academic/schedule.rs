use super::*;

// ── Schedule tools ──

pub async fn list_today_classes(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>().scope();
    let (week, dow) = current_week_and_dow(&db)?;
    let classes = collect_classes(&db, &week, Some(dow))?;
    Ok(json!({
        "day_of_week": dow_label(dow),
        "week_label": week,
        "classes": classes,
    }))
}

pub async fn list_week_classes(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let db = app.state::<Database>().scope();
    let offset = args.get("offset").and_then(|v| v.as_i64()).unwrap_or(0);
    let snap = db
        .get_snapshot_state()?
        .ok_or_else(|| "時間割データがありません".to_string())?;
    let scope = crate::academic_period::visible_weeks(
        &snap.current_week_label,
        &snap.next_week_label,
        &snap.luna_year,
        &snap.luna_term,
        chrono::Local::now().date_naive(),
    );
    let week_label = if offset == 1 {
        scope.next
    } else {
        scope.current
    };
    if week_label.is_empty() {
        return Ok(json!({
            "week_label": "",
            "offset": offset,
            "classes": [],
        }));
    }
    let classes = collect_classes(&db, &week_label, None)?;
    Ok(json!({
        "week_label": week_label,
        "offset": offset,
        "classes": classes,
    }))
}

fn current_week_and_dow(db: &Database) -> Result<(String, i32), String> {
    let snap = db.get_snapshot_state()?.unwrap_or_default();
    let scope = crate::academic_period::visible_weeks(
        &snap.current_week_label,
        &snap.next_week_label,
        &snap.luna_year,
        &snap.luna_term,
        chrono::Local::now().date_naive(),
    );
    use chrono::Datelike;
    let dow = chrono::Local::now().weekday().number_from_monday() as i32; // 1=Mon..7=Sun
    Ok((scope.current, dow))
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
