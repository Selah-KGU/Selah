//! Read-only tool implementations for the Selah agent.
//!
//! Each tool takes a JSON-encoded argument object (often empty `{}`) and
//! returns a JSON value.  Tools are intentionally few and semantically
//! narrow so a 2B model can reliably pick among them.

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use tauri::Manager;

use crate::db::Database;

/// Maximum number of list items returned by any single tool.
const LIST_CAP: usize = 15;
/// Mail body truncation threshold (bytes).
const MAIL_BODY_CAP: usize = 4096;

// ─────────────────────── Tool Spec & Arg Schema ───────────────────────

/// Describes how to sanitize tool arguments before dispatch.
#[derive(Clone, Copy)]
enum ArgSchema {
    /// No arguments — always returns `{}`.
    Empty,
    /// Single integer arg with key, clamped to 0..=max.
    Int { key: &'static str, max: i64 },
    /// Single text arg with key, max_len.
    Text { key: &'static str, max_len: usize },
    /// Course code arg (alphanumeric, uppercased).
    CourseCode { key: &'static str },
    /// limit + optional keyword.
    LimitKeyword,
    /// Custom sanitizer (message_id with validation).
    MailMessageId,
}

struct ToolSpec {
    name: &'static str,
    category: &'static str,
    signature: &'static str,
    purpose: &'static str,
    schema: ArgSchema,
}

const TOOL_SPECS: &[ToolSpec] = &[
    ToolSpec { name: "list_today_classes", category: "授業・時間割", signature: "list_today_classes()", purpose: "今日の授業一覧", schema: ArgSchema::Empty },
    ToolSpec { name: "list_week_classes", category: "授業・時間割", signature: "list_week_classes(offset: 0|1)", purpose: "今週または来週の時間割", schema: ArgSchema::Int { key: "offset", max: 1 } },
    ToolSpec { name: "search_courses", category: "授業・時間割", signature: "search_courses(query: string)", purpose: "科目名・科目コード・教員名から候補を探す", schema: ArgSchema::Text { key: "query", max_len: 80 } },
    ToolSpec { name: "get_course_context", category: "授業・時間割", signature: "get_course_context(query: string)", purpose: "科目の時間割・授業計画・教材・Luna活動をまとめて取得", schema: ArgSchema::Text { key: "query", max_len: 80 } },
    ToolSpec { name: "get_course_detail", category: "授業・時間割", signature: "get_course_detail(kgc_code: string)", purpose: "KGC科目コード指定で詳細・授業計画を取得", schema: ArgSchema::CourseCode { key: "kgc_code" } },
    ToolSpec { name: "get_cancellations", category: "授業・時間割", signature: "get_cancellations()", purpose: "休講情報一覧", schema: ArgSchema::Empty },
    ToolSpec { name: "get_makeup_classes", category: "授業・時間割", signature: "get_makeup_classes()", purpose: "補講情報一覧", schema: ArgSchema::Empty },
    ToolSpec { name: "get_room_changes", category: "授業・時間割", signature: "get_room_changes()", purpose: "教室変更情報一覧", schema: ArgSchema::Empty },
    ToolSpec { name: "get_exam_timetable", category: "授業・時間割", signature: "get_exam_timetable()", purpose: "試験時間割", schema: ArgSchema::Empty },
    ToolSpec { name: "list_luna_todos", category: "課題・成績・履修", signature: "list_luna_todos()", purpose: "Luna の未提出レポート・テスト一覧", schema: ArgSchema::Empty },
    ToolSpec { name: "get_grades", category: "課題・成績・履修", signature: "get_grades()", purpose: "成績・単位取得状況", schema: ArgSchema::Empty },
    ToolSpec { name: "get_registration", category: "課題・成績・履修", signature: "get_registration()", purpose: "履修登録科目一覧・単位集計", schema: ArgSchema::Empty },
    ToolSpec { name: "list_syllabus_favorites", category: "課題・成績・履修", signature: "list_syllabus_favorites(limit?: number, keyword?: string)", purpose: "お気に入りシラバス一覧", schema: ArgSchema::LimitKeyword },
    ToolSpec { name: "list_recent_notifications", category: "お知らせ・メール", signature: "list_recent_notifications(limit?: number)", purpose: "最新のお知らせ", schema: ArgSchema::LimitKeyword },
    ToolSpec { name: "search_notifications", category: "お知らせ・メール", signature: "search_notifications(keyword: string)", purpose: "お知らせをキーワード検索", schema: ArgSchema::Text { key: "keyword", max_len: 80 } },
    ToolSpec { name: "list_recent_mail", category: "お知らせ・メール", signature: "list_recent_mail(limit?: number)", purpose: "受信メール一覧", schema: ArgSchema::LimitKeyword },
    ToolSpec { name: "read_mail", category: "お知らせ・メール", signature: "read_mail(message_id: string)", purpose: "メール本文", schema: ArgSchema::MailMessageId },
    ToolSpec { name: "get_mail_profile", category: "お知らせ・メール", signature: "get_mail_profile()", purpose: "メールアカウント情報", schema: ArgSchema::Empty },
    ToolSpec { name: "get_student_profile", category: "学生情報・その他", signature: "get_student_profile()", purpose: "学籍番号・氏名・学部学科など", schema: ArgSchema::Empty },
    ToolSpec { name: "get_weather", category: "学生情報・その他", signature: "get_weather()", purpose: "今日と明日の天気(西宮キャンパス)", schema: ArgSchema::Empty },
    ToolSpec { name: "get_weekly_summary", category: "学生情報・その他", signature: "get_weekly_summary()", purpose: "AI生成済みの週間サマリー・来週の準備事項", schema: ArgSchema::Empty },
    ToolSpec { name: "get_todo_guide", category: "課題・成績・履修", signature: "get_todo_guide()", purpose: "AI生成のタスクガイド・学習ヒント・3日間の計画", schema: ArgSchema::Empty },
    ToolSpec { name: "get_upcoming_deadlines", category: "課題・成績・履修", signature: "get_upcoming_deadlines()", purpose: "全科目の締め切り間近のレポート・テスト(着手状況付き)", schema: ArgSchema::Empty },
    ToolSpec { name: "get_luna_activity_detail", category: "課題・成績・履修", signature: "get_luna_activity_detail(title: string)", purpose: "タイトルでレポート/テスト/掲示/お知らせの本文・提出要件・添付を取得", schema: ArgSchema::Text { key: "title", max_len: 120 } },
    ToolSpec { name: "refresh_data", category: "学生情報・その他", signature: "refresh_data()", purpose: "Lunaの課題・お知らせ・提出状況を強制的に最新化(数秒かかる)", schema: ArgSchema::Empty },
];

/// Check if a tool name is in the registry.
pub fn is_known_tool(name: &str) -> bool {
    TOOL_SPECS.iter().any(|s| s.name == name)
}

/// Dispatch a single tool call.  Returns a JSON value even on failure so the
/// agent can still surface the error to the user.
pub async fn dispatch(app: &tauri::AppHandle, name: &str, args: &Value) -> Value {
    if !is_known_tool(name) {
        return json!({ "error": format!("unknown tool: {}", name) });
    }
    let result: Result<Value, String> = match name {
        "list_today_classes" => list_today_classes(app).await,
        "list_week_classes" => list_week_classes(app, args).await,
        "search_courses" => search_courses(app, args).await,
        "get_course_context" => get_course_context(app, args).await,
        "list_luna_todos" => list_luna_todos(app).await,
        "list_recent_notifications" => list_recent_notifications(app, args).await,
        "search_notifications" => search_notifications(app, args).await,
        "get_course_detail" => get_course_detail(app, args).await,
        "list_recent_mail" => list_recent_mail(app, args).await,
        "read_mail" => read_mail(app, args).await,
        "get_student_profile" => get_student_profile(app).await,
        "get_mail_profile" => get_mail_profile(app).await,
        "list_syllabus_favorites" => list_syllabus_favorites(app, args).await,
        "get_grades" => get_grades(app).await,
        "get_cancellations" => get_cancellations(app).await,
        "get_makeup_classes" => get_makeup_classes(app).await,
        "get_room_changes" => get_room_changes(app).await,
        "get_registration" => get_registration(app).await,
        "get_exam_timetable" => get_exam_timetable(app).await,
        "get_weather" => get_weather(app).await,
        "get_weekly_summary" => get_weekly_summary(app).await,
        "get_todo_guide" => get_todo_guide(app).await,
        "get_upcoming_deadlines" => get_upcoming_deadlines(app).await,
        "get_luna_activity_detail" => get_luna_activity_detail(app, args).await,
        "refresh_data" => refresh_data(app).await,
        // is_known_tool guard above ensures we never reach here
        _ => unreachable!(),
    };
    match result {
        Ok(v) => v,
        Err(e) => json!({ "error": e }),
    }
}

/// Static description given to the model during the planning phase.
pub fn tool_catalog_prompt() -> String {
    let mut out = String::new();
    let mut current_category = "";
    for spec in TOOL_SPECS {
        if spec.category != current_category {
            if !out.is_empty() {
                out.push('\n');
            }
            current_category = spec.category;
            out.push_str(&format!("【{}】\n", spec.category));
        }
        out.push_str(&format!("- {}: {}\n", spec.signature, spec.purpose));
    }
    out.trim_end().to_string()
}

pub fn sanitize_tool_args(name: &str, args: &Value) -> Option<Value> {
    let spec = TOOL_SPECS.iter().find(|s| s.name == name)?;
    sanitize_by_schema(spec.schema, args)
}

fn sanitize_by_schema(schema: ArgSchema, args: &Value) -> Option<Value> {
    match schema {
        ArgSchema::Empty => Some(json!({})),
        ArgSchema::Int { key, max } => {
            let val = args.get(key).and_then(|v| v.as_i64()).unwrap_or(0).clamp(0, max);
            Some(json!({ key: val }))
        }
        ArgSchema::Text { key, max_len } => {
            sanitize_text_arg(args, key, max_len).map(|v| json!({ key: v }))
        }
        ArgSchema::CourseCode { key } => {
            sanitize_course_code(args, key).map(|v| json!({ key: v }))
        }
        ArgSchema::LimitKeyword => {
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10).min(LIST_CAP as u64);
            let keyword = sanitize_text_arg(args, "keyword", 80);
            let mut out = json!({ "limit": limit });
            if let Some(keyword) = keyword {
                out["keyword"] = Value::String(keyword);
            }
            Some(out)
        }
        ArgSchema::MailMessageId => {
            sanitize_text_arg(args, "message_id", 200).and_then(|message_id| {
                crate::mail::validate_message_id(&message_id).ok()?;
                Some(json!({ "message_id": message_id }))
            })
        }
    }
}

fn sanitize_text_arg(args: &Value, key: &str, max_len: usize) -> Option<String> {
    let value = args.get(key).and_then(|v| v.as_str())?.trim();
    if value.is_empty() {
        return None;
    }
    let mut out = value.chars().take(max_len).collect::<String>();
    out = out.replace('\n', " ").replace('\r', " ");
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.is_empty() { None } else { Some(out) }
}

fn sanitize_course_code(args: &Value, key: &str) -> Option<String> {
    let raw = sanitize_text_arg(args, key, 32)?;
    let code = raw.to_uppercase();
    if code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        Some(code)
    } else {
        None
    }
}

/// Map simplified Chinese characters to their Japanese kanji equivalents
/// so cross-lingual course searches work.
fn normalize_cjk_char(c: char) -> char {
    match c {
        '际' => '際', '关' => '関', '历' => '歴', '础' => '礎', '现' => '現',
        '经' => '経', '济' => '済', '统' => '統', '计' => '計', '术' => '術',
        '语' => '語', '论' => '論', '电' => '電', '机' => '機', '业' => '業',
        '环' => '環', '药' => '薬', '设' => '設', '构' => '構', '门' => '門',
        '发' => '発', '报' => '報', '导' => '導', '义' => '義', '种' => '種',
        '类' => '類', '图' => '図', '馆' => '館', '问' => '問', '题' => '題',
        '对' => '対', '乐' => '楽', '书' => '書', '习' => '習', '练' => '練',
        '传' => '伝', '识' => '識', '认' => '認', '讲' => '講', '谈' => '談',
        '词' => '詞', '读' => '読', '记' => '記', '证' => '証', '评' => '評',
        '试' => '試', '验' => '験', '实' => '実', '达' => '達', '远' => '遠',
        '运' => '運', '进' => '進', '选' => '選', '过' => '過', '专' => '専',
        '组' => '組', '绍' => '紹', '细' => '細', '约' => '約', '线' => '線',
        '确' => '確', '长' => '長', '广' => '広', '应' => '応', '贸' => '貿',
        '资' => '資', '连' => '連', '层' => '層', '积' => '積', '质' => '質',
        '单' => '単', '变' => '変', '观' => '観', '规' => '規', '视' => '視',
        '战' => '戦', '动' => '動', '产' => '産', '营' => '営', '织' => '織',
        '举' => '挙', '兴' => '興', '项' => '項', '归' => '帰', '满' => '満',
        '难' => '難',
        _ => c,
    }
}

fn normalize_text(s: &str) -> String {
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

    for (week_kind, week_label) in [("current", snap.current_week_label), ("next", snap.next_week_label)] {
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
    if aggregate.kgc_codes.iter().any(|code| normalize_text(code) == query) {
        score += 140;
    }
    if aggregate.kgc_codes.iter().any(|code| normalize_text(code).contains(&query)) {
        score += 70;
    }
    if aggregate.luna_ids.iter().any(|id| normalize_text(id) == query) {
        score += 100;
    }
    if aggregate.teachers.iter().any(|teacher| normalize_text(teacher).contains(&query)) {
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

async fn search_courses(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let query = sanitize_text_arg(args, "query", 80).ok_or_else(|| "query が空です".to_string())?;
    let db = app.state::<Database>();
    let mut matches: Vec<(i32, CourseAggregate)> = build_course_aggregates(&db)?
        .into_iter()
        .map(|aggregate| (score_course_match(&query, &aggregate), aggregate))
        .filter(|(score, _)| *score > 0)
        .collect();
    matches.sort_by(|a, b| b.0.cmp(&a.0));
    let items: Vec<Value> = matches.into_iter().take(5).map(|(_, aggregate)| course_match_json(&aggregate)).collect();
    Ok(json!({ "query": query, "matches": items }))
}

async fn get_course_context(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
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
        all_plans.into_iter()
            .find(|(kgc_code, _)| kgc_code == &first_kgc_code)
            .map(|(_, plans)| {
                plans.into_iter().take(15).map(|p| json!({
                    "session": p.session_num,
                    "topic": p.topic,
                    "delivery_mode": p.delivery_mode,
                    "study_outside": p.study_outside,
                })).collect::<Vec<_>>()
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
    let activities: Vec<Value> = all_activities.into_iter()
        .filter(|activity| best.luna_ids.contains(&activity.luna_id))
        .take(20)
        .map(|activity| json!({
            "luna_id": activity.luna_id,
            "type": activity.activity_type,
            "title": activity.title,
            "period": activity.period,
            "status": activity.status,
        }))
        .collect();

    let top_matches: Vec<Value> = matches.iter().take(3).map(|(_, aggregate)| course_match_json(aggregate)).collect();
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

async fn list_today_classes(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    let (week, dow) = current_week_and_dow(&db)?;
    let classes = collect_classes(&db, &week, Some(dow))?;
    Ok(json!({
        "day_of_week": dow_label(dow),
        "week_label": week,
        "classes": classes,
    }))
}

async fn list_week_classes(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let db = app.state::<Database>();
    let offset = args.get("offset").and_then(|v| v.as_i64()).unwrap_or(0);
    let snap = db.get_snapshot_state()?.ok_or_else(|| "時間割データがありません".to_string())?;
    let week_label = if offset == 1 { snap.next_week_label.clone() } else { snap.current_week_label.clone() };
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
    let snap = db.get_snapshot_state()?.ok_or_else(|| "時間割データがありません".to_string())?;
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

fn collect_classes(db: &Database, week_label: &str, filter_day: Option<i32>) -> Result<Vec<Value>, String> {
    let kgc = db.get_kgc_courses(week_label).unwrap_or_default();
    let luna = db.get_luna_courses().unwrap_or_default();
    let mut out: Vec<Value> = Vec::new();

    for c in kgc.iter() {
        if let Some(d) = filter_day { if c.day != d { continue; } }
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
        if let Some(d) = filter_day { if c.day != d as i32 { continue; } }
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
    out.sort_by_key(|v| (
        v.get("day").and_then(|x| x.as_i64()).unwrap_or(0),
        v.get("period").and_then(|x| x.as_i64()).unwrap_or(0),
    ));
    if out.len() > LIST_CAP { out.truncate(LIST_CAP); }
    Ok(out)
}

// ── Luna activity tools ──

async fn list_luna_todos(app: &tauri::AppHandle) -> Result<Value, String> {
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
        let course_name = luna_courses.iter().find(|c| c.luna_id == a.luna_id)
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
    if items.len() > LIST_CAP { items.truncate(LIST_CAP); }
    Ok(json!({ "todos": items }))
}

async fn list_recent_notifications(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let db = app.state::<Database>();
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
    let limit = limit.min(LIST_CAP);
    let items = collect_notifications(&db, None, limit);
    Ok(json!({ "notifications": items }))
}

async fn search_notifications(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let keyword = args.get("keyword").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if keyword.is_empty() {
        return Err("keyword が空です".into());
    }
    let db = app.state::<Database>();
    let items = collect_notifications(&db, Some(&keyword), LIST_CAP);
    Ok(json!({ "keyword": keyword, "notifications": items }))
}

fn collect_notifications(db: &Database, keyword: Option<&str>, limit: usize) -> Vec<Value> {
    let mut out: Vec<(i64, Value)> = Vec::new();

    // KGC notifications from data_cache["notifications"]
    if let Ok(Some((json_str, _))) = db.get_data_cache("notifications") {
        if let Ok(v) = serde_json::from_str::<Value>(&json_str) {
            if let Some(entries) = v.get("entries").and_then(|x| x.as_array()) {
                for e in entries {
                    let title = e.get("title").and_then(|x| x.as_str()).unwrap_or("");
                    if let Some(kw) = keyword { if !title.contains(kw) { continue; } }
                    let date_str = e.get("date").and_then(|x| x.as_str()).unwrap_or("");
                    let sortkey = date_score(date_str);
                    out.push((sortkey, json!({
                        "source": "KGC",
                        "title": title,
                        "date": date_str,
                        "category": e.get("category").and_then(|x| x.as_str()).unwrap_or(""),
                    })));
                }
            }
        }
    }
    // Luna updates
    if let Ok(Some((json_str, _))) = db.get_data_cache("luna_updates") {
        if let Ok(arr) = serde_json::from_str::<Vec<Value>>(&json_str) {
            for e in arr.iter() {
                let content = e.get("content").and_then(|x| x.as_str()).unwrap_or("");
                if let Some(kw) = keyword { if !content.contains(kw) { continue; } }
                let date_str = e.get("date").and_then(|x| x.as_str()).unwrap_or("");
                let sortkey = date_score(date_str);
                out.push((sortkey, json!({
                    "source": "Luna",
                    "title": content,
                    "date": date_str,
                })));
            }
        }
    }
    // KWIC portal home
    if let Ok(Some((json_str, _))) = db.get_data_cache("kwic_home") {
        if let Ok(v) = serde_json::from_str::<Value>(&json_str) {
            if let Some(sections) = v.get("sections").and_then(|x| x.as_array()) {
                for sec in sections {
                    let sec_title = sec.get("title").and_then(|x| x.as_str()).unwrap_or("");
                    if sec_title == "メインリンク" || sec_title == "注目コンテンツ" { continue; }
                    if let Some(items) = sec.get("items").and_then(|x| x.as_array()) {
                        for it in items {
                            let title = it.get("title").and_then(|x| x.as_str()).unwrap_or("");
                            if let Some(kw) = keyword { if !title.contains(kw) { continue; } }
                            let date_str = it.get("date").and_then(|x| x.as_str()).unwrap_or("");
                            let sortkey = date_score(date_str);
                            out.push((sortkey, json!({
                                "source": "KWIC",
                                "title": title,
                                "date": date_str,
                                "category": sec_title,
                            })));
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
    digits.chars().take(8).collect::<String>().parse().unwrap_or(0)
}

// ── Course detail ──

async fn get_course_detail(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let code = args.get("kgc_code").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if code.is_empty() { return Err("kgc_code が空です".into()); }
    let db = app.state::<Database>();
    let detail = db.get_kgc_course_detail(&code)?;
    let plans = db.get_all_session_plans().unwrap_or_default()
        .into_iter().find(|(k, _)| k == &code).map(|(_, v)| v).unwrap_or_default();
    if detail.is_none() && plans.is_empty() {
        return Err(format!("{} の詳細が見つかりません", code));
    }
    let detail_json = detail.map(|d| json!({
        "delivery_mode": d.delivery_mode,
        "fields": d.fields.iter().take(12).collect::<Vec<_>>(),
    }));
    let plan_summary: Vec<_> = plans.iter().take(15).map(|p| json!({
        "session": p.session_num,
        "topic": p.topic,
        "th_header": p.th_header,
    })).collect();
    Ok(json!({
        "kgc_code": code,
        "detail": detail_json,
        "session_plan": plan_summary,
    }))
}

// ── Mail tools ──

async fn list_recent_mail(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10).min(LIST_CAP as u64) as u32;
    let msgs = crate::mail_commands::fetch_inbox_internal(app, limit, 0).await?;
    let items: Vec<Value> = msgs.iter().map(|m| json!({
        "id": m.id,
        "subject": m.subject.clone().unwrap_or_default(),
        "from": m.from.as_ref().map(|a| json!({
            "name": a.email_address.name.clone().unwrap_or_default(),
            "address": a.email_address.address.clone().unwrap_or_default(),
        })),
        "received": m.received_date_time.clone().unwrap_or_default(),
        "is_read": m.is_read.unwrap_or(false),
        "preview": m.body_preview.clone().unwrap_or_default(),
    })).collect();
    Ok(json!({ "mails": items }))
}

async fn read_mail(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let id = args.get("message_id").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if id.is_empty() { return Err("message_id が空です".into()); }
    let detail = crate::mail_commands::fetch_message_internal(app, &id).await?;
    let body_text = detail.body.as_ref().map(|b| {
        let content = b.content.clone().unwrap_or_default();
        if b.content_type.as_deref() == Some("html") {
            strip_html(&content)
        } else {
            content
        }
    }).unwrap_or_default();
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

async fn get_student_profile(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    let (json_str, _) = db.get_data_cache("student_profile")?
        .ok_or_else(|| "学生プロフィールがまだ取得されていません".to_string())?;
    let profile: crate::parser::StudentInfo = serde_json::from_str(&json_str)
        .map_err(|e| format!("JSON解析失敗: {}", e))?;
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

async fn get_mail_profile(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    let (json_str, _) = db.get_data_cache("mail_profile")?
        .ok_or_else(|| "メールプロフィールがまだ取得されていません".to_string())?;
    let profile: crate::mail::MailProfile = serde_json::from_str(&json_str)
        .map_err(|e| format!("JSON解析失敗: {}", e))?;
    Ok(json!({
        "display_name": profile.display_name.unwrap_or_default(),
        "mail": profile.mail.unwrap_or_default(),
        "user_principal_name": profile.user_principal_name.unwrap_or_default(),
    }))
}

async fn list_syllabus_favorites(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let db = app.state::<Database>();
    let (json_str, _) = db.get_data_cache("syllabus_favorites")?
        .ok_or_else(|| "お気に入りシラバスがまだ取得されていません".to_string())?;
    let result: crate::syllabus::SyllabusSearchResult = serde_json::from_str(&json_str)
        .map_err(|e| format!("JSON解析失敗: {}", e))?;
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10).min(LIST_CAP as u64) as usize;
    let keyword = sanitize_text_arg(args, "keyword", 80).unwrap_or_default();
    let keyword_norm = normalize_text(&keyword);
    let mut items: Vec<Value> = result.entries.into_iter()
        .filter(|entry| {
            if keyword_norm.is_empty() {
                return true;
            }
            let hay = normalize_text(&format!("{} {} {} {}", entry.class_code, entry.course_title, entry.instructor, entry.term));
            hay.contains(&keyword_norm)
        })
        .take(limit)
        .map(|entry| json!({
            "class_code": entry.class_code,
            "course_title": entry.course_title,
            "instructor": entry.instructor,
            "term": entry.term,
            "day_period": entry.day_period,
            "campus": entry.campus,
            "credits": entry.credits,
            "bookmarked": entry.bookmarked,
        }))
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
    // Minimal tag strip + whitespace squash.  The agent only needs readable text.
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
            if !prev_ws { collapsed.push(' '); }
            prev_ws = true;
        } else {
            collapsed.push(ch);
            prev_ws = false;
        }
    }
    collapsed.trim().to_string()
}

fn truncate_bytes(s: &str, max: usize) -> String {
    if s.len() <= max { return s.to_string(); }
    let mut cut = max;
    while cut > 0 && !s.is_char_boundary(cut) { cut -= 1; }
    format!("{}…<truncated>", &s[..cut])
}

// ── Grades ──

async fn get_grades(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    let (json_str, _) = db.get_data_cache("grades")?
        .ok_or_else(|| "成績データがまだ取得されていません".to_string())?;
    let v: Value = serde_json::from_str(&json_str).map_err(|e| format!("JSON解析失敗: {}", e))?;
    let curriculum = v.get("curriculum").and_then(|x| x.as_array());
    let items: Vec<Value> = curriculum.map(|arr| {
        arr.iter().map(|c| json!({
            "category": c.get("category").and_then(|x| x.as_str()).unwrap_or(""),
            "required": c.get("required_credits").and_then(|x| x.as_str()).unwrap_or(""),
            "earned": c.get("earned_credits").and_then(|x| x.as_str()).unwrap_or(""),
            "enrolled": c.get("enrolled_credits").and_then(|x| x.as_str()).unwrap_or(""),
            "deficit": c.get("is_deficit").and_then(|x| x.as_bool()).unwrap_or(false),
        })).collect()
    }).unwrap_or_default();
    Ok(json!({ "curriculum": items }))
}

// ── Cancellations ──

// ── Generic data-cache helper ──

/// Read entries from a cached JSON object, project specified string fields, and
/// wrap in a result key.  Covers cancellations, makeup, room changes, exams.
fn read_cache_entries(
    db: &Database,
    cache_key: &str,
    result_key: &str,
    error_hint: &str,
    fields: &[&str],
) -> Result<Value, String> {
    let (json_str, _) = db.get_data_cache(cache_key)?
        .ok_or_else(|| format!("{}がまだ取得されていません", error_hint))?;
    let v: Value = serde_json::from_str(&json_str).map_err(|e| format!("JSON解析失敗: {}", e))?;
    let items: Vec<Value> = v.get("entries").and_then(|x| x.as_array()).map(|arr| {
        arr.iter().take(LIST_CAP).map(|entry| {
            let mut obj = serde_json::Map::with_capacity(fields.len());
            for &field in fields {
                let val = entry.get(field).cloned().unwrap_or(Value::String(String::new()));
                obj.insert(field.to_string(), val);
            }
            Value::Object(obj)
        }).collect()
    }).unwrap_or_default();
    Ok(json!({ result_key: items }))
}

async fn get_cancellations(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    read_cache_entries(&db, "cancellations", "cancellations", "休講データ",
        &["date", "period", "course_name", "instructor", "room", "comment"])
}

async fn get_makeup_classes(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    read_cache_entries(&db, "makeup", "makeup_classes", "補講データ",
        &["date", "period", "course_name", "instructor", "room", "comment"])
}

async fn get_room_changes(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    read_cache_entries(&db, "rooms", "room_changes", "教室変更データ",
        &["date", "course_name", "room", "instructor", "schedule", "comment"])
}

// ── Registration ──

async fn get_registration(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    let (json_str, _) = db.get_data_cache("registration")?
        .ok_or_else(|| "履修データがまだ取得されていません".to_string())?;
    let v: Value = serde_json::from_str(&json_str).map_err(|e| format!("JSON解析失敗: {}", e))?;
    let credit_summary = v.get("credit_summary").and_then(|x| x.as_array()).map(|arr| {
        arr.iter().map(|s| json!({
            "semester": s.get("semester").and_then(|x| x.as_str()).unwrap_or(""),
            "enrolled": s.get("enrolled").and_then(|x| x.as_str()).unwrap_or(""),
            "limit": s.get("limit").and_then(|x| x.as_str()).unwrap_or(""),
        })).collect::<Vec<Value>>()
    }).unwrap_or_default();
    let courses = v.get("courses").and_then(|x| x.as_array()).map(|arr| {
        arr.iter().map(|c| json!({
            "day": c.get("day").and_then(|x| x.as_str()).unwrap_or(""),
            "period": c.get("period").and_then(|x| x.as_str()).unwrap_or(""),
            "course_name": c.get("course_name").and_then(|x| x.as_str()).unwrap_or(""),
            "instructor": c.get("instructor").and_then(|x| x.as_str()).unwrap_or(""),
            "credits": c.get("credits").and_then(|x| x.as_str()).unwrap_or(""),
            "room": c.get("room").and_then(|x| x.as_str()).unwrap_or(""),
            "status": c.get("status").and_then(|x| x.as_str()).unwrap_or(""),
        })).collect::<Vec<Value>>()
    }).unwrap_or_default();
    Ok(json!({
        "year_semester": v.get("year_semester").and_then(|x| x.as_str()).unwrap_or(""),
        "credit_summary": credit_summary,
        "courses": courses,
    }))
}

// ── Exam timetable ──

async fn get_exam_timetable(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    read_cache_entries(&db, "exam_timetable", "exams", "試験時間割",
        &["day", "period", "course_name", "room"])
}

// ── Weather ──

async fn get_weather(_app: &tauri::AppHandle) -> Result<Value, String> {
    let data: crate::commands::WeatherData =
        crate::commands::fetch_weather().await?;
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

// ── Weekly summary (from AI schedule cache) ──

async fn get_weekly_summary(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    let (cache, _ts) = db.get_ai_schedule_cache()?
        .ok_or_else(|| "週間サマリーがまだ生成されていません".to_string())?;
    Ok(json!({
        "current_week": cache.current_week_label,
        "next_week": cache.next_week_label,
        "weekly_summary": cache.weekly_summary,
        "cross_week_insights": cache.cross_week_insights,
    }))
}

// ── Todo guide (AI-generated study plan) ──

async fn get_todo_guide(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    let (json_str, ts) = db.get_data_cache("ai_todo_analysis")?
        .ok_or_else(|| "課題ガイドがまだ生成されていません。ホーム画面で課題一覧を取得してください。".to_string())?;
    let v: Value = serde_json::from_str(&json_str).map_err(|e| format!("JSON解析失敗: {}", e))?;
    let age_hours = (chrono::Utc::now().timestamp() - ts) / 3600;
    Ok(json!({
        "generated_hours_ago": age_hours,
        "task_guides": v.get("task_guides"),
        "daily_plan": v.get("daily_plan"),
        "priority_summary": v.get("priority_summary"),
    }))
}

// ── Upcoming deadlines (cross-course, with urgency) ──

async fn get_upcoming_deadlines(app: &tauri::AppHandle) -> Result<Value, String> {
    let db = app.state::<Database>();
    let acts = db.get_all_luna_activities().unwrap_or_default();
    let luna_courses = db.get_luna_courses().unwrap_or_default();
    let now = chrono::Local::now();

    let mut items: Vec<Value> = Vec::new();
    for a in acts.iter() {
        if !matches!(a.activity_type.as_str(), "report" | "exam" | "discussion") {
            continue;
        }
        let course_name = luna_courses.iter().find(|c| c.luna_id == a.luna_id)
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
    // Sort by urgency: overdue > critical > soon > normal > submitted
    items.sort_by_key(|v| {
        let u = v.get("urgency").and_then(|x| x.as_str()).unwrap_or("normal");
        let sub = v.get("submitted").and_then(|x| x.as_bool()).unwrap_or(false);
        match (sub, u) {
            (true, _) => 4,
            (_, "overdue") => 0,
            (_, "critical") => 1,
            (_, "soon") => 2,
            _ => 3,
        }
    });
    if items.len() > LIST_CAP { items.truncate(LIST_CAP); }
    Ok(json!({ "deadlines": items }))
}

// ── Force refresh of Luna activities/counts (fills in stale detail_paths) ──

async fn refresh_data(app: &tauri::AppHandle) -> Result<Value, String> {
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

// ── Luna activity detail (on-demand fetch of body/attachments/requirements) ──

async fn get_luna_activity_detail(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("").trim();
    if title.is_empty() {
        return Err("titleを指定してください".into());
    }

    let db = app.state::<Database>();
    let acts = db.get_all_luna_activities().unwrap_or_default();
    if acts.is_empty() {
        return Err("Luna活動データがまだ同期されていません".into());
    }

    // Find best match: exact → contains(title) → contains(fragment).
    let needle = title.to_lowercase();
    let best = acts.iter().find(|a| a.title == title)
        .or_else(|| acts.iter().find(|a| a.title.to_lowercase().contains(&needle)))
        .or_else(|| acts.iter().find(|a| needle.contains(&a.title.to_lowercase()) && !a.title.is_empty()));

    let row = match best {
        Some(r) if !r.detail_path.is_empty() => r,
        Some(_) => return Err(format!("「{}」には詳細ページのパスが記録されていません。時間割を再同期してください。", title)),
        None => {
            let candidates: Vec<&str> = acts.iter().take(10).map(|a| a.title.as_str()).collect();
            return Err(format!("「{}」に一致する活動が見つかりませんでした。候補: {}", title, candidates.join(" / ")));
        }
    };

    // Course name lookup.
    let luna_courses = db.get_luna_courses().unwrap_or_default();
    let course_name = luna_courses.iter().find(|c| c.luna_id == row.luna_id)
        .map(|c| c.name.clone())
        .unwrap_or_default();

    // Fetch HTML via authenticated Luna client.
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
        &http, &url, crate::config::LUNA_BASE,
        crate::luna_client::LUNA_SESSION_EXPIRED_MSG,
        crate::luna_client::is_luna_session_expired,
    ).await.map_err(|e| format!("Luna取得失敗: {}", e))?;

    // Pick parser by activity_type.
    let detail = if row.activity_type == "announcement" {
        crate::luna_parser::parse_luna_announcement_detail(&html)
    } else {
        crate::luna_parser::parse_luna_detail_page(&html)
    };

    // Serialize sections with per-section body truncation.
    const SECTION_CAP: usize = 1200;
    let sections: Vec<Value> = detail.sections.iter().map(|s| {
        let mut body = s.body.clone();
        if body.len() > SECTION_CAP {
            let mut cut = SECTION_CAP;
            while cut > 0 && !body.is_char_boundary(cut) { cut -= 1; }
            body.truncate(cut);
            body.push_str("...<truncated>");
        }
        json!({ "heading": s.heading, "body": body })
    }).collect();

    let attachments: Vec<Value> = detail.attachments.iter().take(10)
        .map(|a| json!({ "name": a.name, "type": a.link_type }))
        .collect();

    let meta: Vec<Value> = detail.meta.iter().take(10)
        .map(|(k, v)| json!({ "label": k, "value": v }))
        .collect();

    Ok(json!({
        "matched_title": row.title,
        "activity_type": row.activity_type,
        "course": course_name,
        "period": row.period,
        "status": row.status,
        "detail_title": detail.title,
        "meta": meta,
        "sections": sections,
        "attachments": attachments,
    }))
}

fn deadline_urgency(period_str: &str, now: &chrono::DateTime<chrono::Local>) -> &'static str {
    // Try parsing common deadline formats: "YYYY/MM/DD HH:MM", "YYYY-MM-DD HH:MM"
    let cleaned = period_str.replace('/', "-");
    let deadline = chrono::NaiveDateTime::parse_from_str(&cleaned, "%Y-%m-%d %H:%M")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(&cleaned, "%Y-%m-%d"));
    match deadline {
        Ok(dt) => {
            let local_dt = dt.and_local_timezone(chrono::Local).single();
            match local_dt {
                Some(d) => {
                    let hours = (d - *now).num_hours();
                    if hours < 0 { "overdue" }
                    else if hours < 24 { "critical" }
                    else if hours < 72 { "soon" }
                    else { "normal" }
                }
                None => "normal",
            }
        }
        Err(_) => "normal",
    }
}
