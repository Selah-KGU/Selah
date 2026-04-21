use super::*;

pub(super) async fn list_recent_mail(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .min(LIST_CAP as u64) as u32;
    let msgs = crate::mail_commands::fetch_inbox_internal(app, limit, 0).await?;
    let items: Vec<Value> = msgs
        .iter()
        .map(|m| {
            json!({
                "id": m.id,
                "subject": m.subject.clone().unwrap_or_default(),
                "from": m.from.as_ref().map(|a| json!({
                    "name": a.email_address.name.clone().unwrap_or_default(),
                    "address": a.email_address.address.clone().unwrap_or_default(),
                })),
                "received": m.received_date_time.clone().unwrap_or_default(),
                "is_read": m.is_read.unwrap_or(false),
                "preview": m.body_preview.clone().unwrap_or_default(),
            })
        })
        .collect();
    Ok(json!({ "mails": items }))
}

pub(super) async fn read_mail(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let id = args
        .get("message_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if id.is_empty() {
        return Err("message_id が空です".into());
    }
    let detail = crate::mail_commands::fetch_message_internal(app, &id).await?;
    let body_text = detail
        .body
        .as_ref()
        .map(|b| {
            let content = b.content.clone().unwrap_or_default();
            if b.content_type.as_deref() == Some("html") {
                strip_html(&content)
            } else {
                content
            }
        })
        .unwrap_or_default();
    let truncated = truncate_bytes(&body_text, MAIL_BODY_CAP);
    Ok(json!({
        "id": detail.id,
        "subject": detail.subject.unwrap_or_default(),
        "from": detail.from.as_ref().map(|a| json!({
            "name": a.email_address.name.clone().unwrap_or_default(),
            "address": a.email_address.address.clone().unwrap_or_default(),
        })),
        "received": detail.received_date_time.unwrap_or_default(),
        "body": truncated,
    }))
}

pub(super) async fn get_student_profile(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    let (json_str, _) = db
        .get_data_cache("student_profile")?
        .ok_or_else(|| "学生プロフィールがまだ取得されていません".to_string())?;
    let profile: crate::parser::StudentInfo =
        serde_json::from_str(&json_str).map_err(|e| format!("JSON解析失敗: {}", e))?;
    Ok(json!({
        "student_id": profile.student_id,
        "name": profile.name,
        "name_en": profile.name_en,
        "student_type": profile.student_type,
        "affiliation_type": profile.affiliation_type,
        "status": profile.status,
        "class": profile.class,
        "faculty": profile.faculty,
        "department": profile.department,
        "major": profile.major,
        "address": profile.address,
    }))
}

pub(super) async fn get_mail_profile(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    let (json_str, _) = db
        .get_data_cache("mail_profile")?
        .ok_or_else(|| "メールプロフィールがまだ取得されていません".to_string())?;
    let profile: crate::mail::MailProfile =
        serde_json::from_str(&json_str).map_err(|e| format!("JSON解析失敗: {}", e))?;
    Ok(json!({
        "display_name": profile.display_name.unwrap_or_default(),
        "mail": profile.mail.unwrap_or_default(),
        "user_principal_name": profile.user_principal_name.unwrap_or_default(),
    }))
}

pub(super) async fn list_syllabus_favorites(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let db = app.state::<Database>();
    let (json_str, _) = db
        .get_data_cache("syllabus_favorites")?
        .ok_or_else(|| "お気に入りシラバスがまだ取得されていません".to_string())?;
    let result: crate::syllabus::SyllabusSearchResult =
        serde_json::from_str(&json_str).map_err(|e| format!("JSON解析失敗: {}", e))?;
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .min(LIST_CAP as u64) as usize;
    let keyword = sanitize_text_arg(args, "keyword", 80).unwrap_or_default();
    let keyword_norm = normalize_text(&keyword);
    let mut items: Vec<Value> = result
        .entries
        .into_iter()
        .filter(|entry| {
            if keyword_norm.is_empty() {
                return true;
            }
            let hay = normalize_text(&format!(
                "{} {} {} {}",
                entry.class_code, entry.course_title, entry.instructor, entry.term
            ));
            hay.contains(&keyword_norm)
        })
        .take(limit)
        .map(|entry| {
            json!({
                "class_code": entry.class_code,
                "course_title": entry.course_title,
                "instructor": entry.instructor,
                "term": entry.term,
                "day_period": entry.day_period,
                "campus": entry.campus,
                "credits": entry.credits,
                "bookmarked": entry.bookmarked,
            })
        })
        .collect();
    if items.len() > limit {
        items.truncate(limit);
    }
    Ok(json!({
        "keyword": keyword,
        "favorites": items,
    }))
}

fn strip_html(s: &str) -> String {
    // Minimal tag strip + whitespace squash. The agent only needs readable text.
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out.replace('\u{00a0}', " ");
    let mut collapsed = String::with_capacity(out.len());
    let mut prev_ws = false;
    for ch in out.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                collapsed.push(' ');
            }
            prev_ws = true;
        } else {
            collapsed.push(ch);
            prev_ws = false;
        }
    }
    collapsed.trim().to_string()
}

fn truncate_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = max;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…<truncated>", &s[..cut])
}
