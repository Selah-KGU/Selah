//! Course/source assembly + all DB persistence (campaign, chapter, knowledge, align, memory).
use super::*;
use crate::db::Database;
use serde::de::DeserializeOwned;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Decide a course's preview `caseType` for the picker list. The AI will
/// choose the final caseType when the case is opened.
pub(crate) fn course_case_type(course: &CourseBuilder) -> &'static str {
    let has_live = !course.live_records.is_empty();
    let has_signal = !course.exam_signals.is_empty();
    let has_doubts = !course.doubts.is_empty();
    if has_signal && has_live {
        "Exam Signal Case"
    } else if has_signal {
        "Contradiction Case"
    } else if has_doubts {
        "Doubt Repair Case"
    } else if course.live_records.len() >= 2 {
        "Missing Link Case"
    } else {
        "Concept Web Case"
    }
}

pub(crate) fn ensure_course<'a>(
    courses: &'a mut HashMap<String, CourseBuilder>,
    name: &str,
) -> &'a mut CourseBuilder {
    let clean = clean_course_name(name);
    let key = normalize_course_key(&clean);
    courses.entry(key.clone()).or_insert_with(|| CourseBuilder {
        name: clean,
        key,
        ..Default::default()
    })
}

pub(crate) fn clean_course_name(name: &str) -> String {
    let simplified = crate::commands::simplify_course_name(name);
    let trimmed = simplified.trim();
    if trimmed.is_empty() {
        "Detective".to_string()
    } else {
        trimmed.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}

pub(crate) fn normalize_course_key(name: &str) -> String {
    crate::commands::simplify_course_name(name)
        .chars()
        .flat_map(|ch| ch.to_lowercase())
        .filter(|ch| ch.is_alphanumeric())
        .collect()
}

pub(crate) fn is_live_record(record: &crate::commands::DownloadRecord) -> bool {
    let source = record.source.to_ascii_lowercase();
    let filename = record.filename.to_ascii_lowercase();
    let path = record.path.to_ascii_lowercase();
    source == "live" || filename.contains("_live") || path.contains("_live")
}

pub(crate) fn infer_course_name_from_record(record: &crate::commands::DownloadRecord) -> String {
    let from_parent = Path::new(&record.path)
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|name| name.to_str())
        .map(clean_course_name);
    if let Some(parent) = from_parent.filter(|name| !name.is_empty()) {
        return parent;
    }
    record
        .filename
        .replace("_live", "")
        .trim_end_matches(".md")
        .trim_end_matches(".markdown")
        .to_string()
}

pub(crate) fn live_excerpt(path: &str) -> String {
    let path = Path::new(path);
    let Ok(metadata) = std::fs::metadata(path) else {
        return String::new();
    };
    if !metadata.is_file() || metadata.len() > LIVE_EXCERPT_MAX_BYTES {
        return String::new();
    }
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    compact_live_note_markdown(&String::from_utf8_lossy(&bytes), LIVE_EXCERPT_MAX_CHARS)
}

pub(crate) fn compact_live_note_markdown(markdown: &str, max_chars: usize) -> String {
    let head = markdown.split("\n## 全文転写").next().unwrap_or(markdown);
    let mut lines = Vec::new();
    for line in head.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let cleaned = trimmed
            .trim_start_matches('#')
            .trim()
            .trim_start_matches('-')
            .trim();
        if cleaned.is_empty()
            || cleaned == "区間ごとの要約"
            || cleaned == "全文転写"
            || matches!(
                cleaned,
                "開始:" | "終了:" | "授業コード:" | "教員:" | "教室:" | "時間帯:"
            )
        {
            continue;
        }
        lines.push(cleaned.to_string());
    }
    truncate_chars(&lines.join("\n"), max_chars)
}

pub(crate) fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max_chars).collect();
    format!("{truncated}...")
}

pub(crate) fn collect_exam_signals(db: &Database) -> Vec<DetectiveSignal> {
    let mut out = Vec::new();

    if let Some(notifications) =
        load_cache_json::<crate::parser::NotificationsData>(db, "notifications")
    {
        for notif in notifications.entries {
            if !is_exam_signal(&format!("{} {}", notif.title, notif.category)) {
                continue;
            }
            out.push(DetectiveSignal {
                id: format!("kgc:{}:{}", notif.id, notif.date),
                source_id: notif.id,
                title: notif.title,
                date: notif.date,
                category: notif.category,
                source: "kgc".to_string(),
                course_info: String::new(),
                source_url: notif.url,
                information_type: String::new(),
                person_category_cd: String::new(),
                category_cd: String::new(),
            });
        }
    }

    if let Some(luna_updates) =
        load_cache_json::<Vec<crate::luna_parser::LunaNotification>>(db, "luna_updates")
    {
        for notif in luna_updates {
            let text = format!("{} {} {}", notif.content, notif.module, notif.course_info);
            if !is_exam_signal(&text) {
                continue;
            }
            let source_id = if notif.idnumber.is_empty() {
                notif.url.clone()
            } else {
                notif.idnumber.clone()
            };
            out.push(DetectiveSignal {
                id: format!(
                    "luna:{}:{}",
                    if notif.url.is_empty() {
                        &notif.idnumber
                    } else {
                        &notif.url
                    },
                    notif.date
                ),
                source_id,
                title: notif.content,
                date: notif.date,
                category: if notif.module.is_empty() {
                    notif.course_info.clone()
                } else {
                    notif.module
                },
                source: "luna".to_string(),
                course_info: notif.course_info,
                source_url: notif.url,
                information_type: String::new(),
                person_category_cd: String::new(),
                category_cd: String::new(),
            });
        }
    }

    if let Some(home) = load_cache_json::<crate::kwic_commands::KwicPortalHome>(db, "kwic_home") {
        for section in home.sections {
            if section.title == "メインリンク" || section.title == "注目コンテンツ" {
                continue;
            }
            for item in section.items {
                let text = format!("{} {} {}", item.title, item.category, section.title);
                if !is_exam_signal(&text) {
                    continue;
                }
                out.push(DetectiveSignal {
                    id: format!("kwic:{}:{}", item.id, item.date),
                    source_id: item.id,
                    title: item.title,
                    date: item.date,
                    category: if item.category.is_empty() {
                        section.title.clone()
                    } else {
                        item.category
                    },
                    source: "kwic".to_string(),
                    course_info: String::new(),
                    source_url: item.url,
                    information_type: item.information_type,
                    person_category_cd: item.person_category_cd,
                    category_cd: item.category_cd,
                });
            }
        }
    }

    if let Ok(Some((result, _))) = db.get_ai_schedule_cache() {
        let snap = db.get_snapshot_state().ok().flatten().unwrap_or_default();
        let scope = crate::academic_period::visible_weeks(
            &snap.current_week_label,
            &snap.next_week_label,
            &snap.luna_year,
            &snap.luna_term,
            chrono::Local::now().date_naive(),
        );
        let mut weeks = Vec::new();
        if crate::academic_period::week_belongs_to_visible_label(
            &scope.current,
            &result.current_week_label,
        ) {
            weeks.extend(result.current_week);
        }
        if crate::academic_period::week_belongs_to_visible_label(
            &scope.next,
            &result.next_week_label,
        ) {
            weeks.extend(result.next_week);
        }
        for item in weeks {
            for signal in item
                .exams
                .iter()
                .chain(item.notifications.iter().filter(|s| is_exam_signal(s)))
            {
                out.push(DetectiveSignal {
                    id: format!("schedule:{}:{}", item.course_name, signal),
                    source_id: item.course_name.clone(),
                    title: signal.clone(),
                    date: String::new(),
                    category: "時間割".to_string(),
                    source: "schedule".to_string(),
                    course_info: item.course_name.clone(),
                    source_url: String::new(),
                    information_type: String::new(),
                    person_category_cd: String::new(),
                    category_cd: String::new(),
                });
            }
        }
    }

    dedupe_signals(out)
}

pub(crate) fn load_cache_json<T: DeserializeOwned>(db: &Database, key: &str) -> Option<T> {
    let (json, _) = db.get_data_cache(key).ok().flatten()?;
    serde_json::from_str(&json).ok()
}

pub(crate) fn load_doubts(db: &Database) -> Vec<DetectiveDoubt> {
    load_cache_json(db, DETECTIVE_DOUBTS_KEY).unwrap_or_default()
}

pub(crate) fn load_memory(db: &Database) -> DetectiveMemory {
    load_cache_json(db, DETECTIVE_MEMORY_KEY).unwrap_or_default()
}

pub(crate) fn save_memory(db: &Database, memory: &DetectiveMemory) {
    if let Ok(json) = serde_json::to_string(memory) {
        let _ = db.save_data_cache(DETECTIVE_MEMORY_KEY, &json);
    }
}

pub(crate) fn campaign_key(course_key: &str) -> String {
    format!("{DETECTIVE_CAMPAIGN_PREFIX}{course_key}")
}

pub(crate) fn load_campaign(db: &Database, course_key: &str) -> Option<DetectiveCampaign> {
    load_cache_json(db, &campaign_key(course_key))
}

pub(crate) fn save_campaign(db: &Database, campaign: &DetectiveCampaign) {
    if let Ok(json) = serde_json::to_string(campaign) {
        let _ = db.save_data_cache(&campaign_key(&campaign.course_key), &json);
    }
}

pub(crate) fn chapter_case_key(course_key: &str, live_id: &str) -> String {
    format!("{DETECTIVE_CHAPTER_PREFIX}{course_key}:{live_id}")
}

pub(crate) fn load_chapter_case(
    db: &Database,
    course_key: &str,
    live_id: &str,
) -> Option<DetectiveCase> {
    load_cache_json(db, &chapter_case_key(course_key, live_id))
}

pub(crate) fn save_chapter_case(
    db: &Database,
    course_key: &str,
    live_id: &str,
    case: &DetectiveCase,
) {
    if let Ok(json) = serde_json::to_string(case) {
        let _ = db.save_data_cache(&chapter_case_key(course_key, live_id), &json);
    }
}

pub(crate) fn knowledge_key(course_key: &str, live_id: &str) -> String {
    format!("{DETECTIVE_KNOWLEDGE_PREFIX}{course_key}:{live_id}")
}

pub(crate) fn load_knowledge_points(
    db: &Database,
    course_key: &str,
    live_id: &str,
) -> Option<Vec<KnowledgePoint>> {
    load_cache_json(db, &knowledge_key(course_key, live_id))
}

pub(crate) fn save_knowledge_points(
    db: &Database,
    course_key: &str,
    live_id: &str,
    pts: &[KnowledgePoint],
) {
    if let Ok(json) = serde_json::to_string(pts) {
        let _ = db.save_data_cache(&knowledge_key(course_key, live_id), &json);
    }
}

/// One planned lecture from the 授業計画.
pub(crate) struct PlannedSession {
    pub(crate) num: i32,
    pub(crate) topic: String,
    /// True for online / on-demand 回 that never produce a Live recording.
    pub(crate) online: bool,
}

/// Classify a 授業計画 delivery_mode string. Anything that clearly says online /
/// on-demand / remote counts as online (no Live); everything else (対面,
/// ハイブリッド, blank, …) is treated as offline / Live-bearing.
pub(crate) fn is_online_mode(mode: &str) -> bool {
    const JP: &[&str] = &["オンデマンド", "オンライン", "遠隔", "非対面", "配信"];
    const EN: &[&str] = &["on-demand", "ondemand", "online", "remote"];
    let lower = mode.to_lowercase();
    JP.iter().any(|k| mode.contains(k)) || EN.iter().any(|k| lower.contains(k))
}

/// The course's 授業計画 sessions (content-ordered), or empty if none parsed.
pub(crate) fn planned_sessions(db: &Database, course_key: &str) -> Vec<PlannedSession> {
    db.get_planned_sessions_by_name()
        .ok()
        .into_iter()
        .flatten()
        .find(|(name, _)| normalize_course_key(name) == course_key)
        .map(|(_, rows)| {
            rows.into_iter()
                .map(|(num, topic, mode)| PlannedSession {
                    num,
                    topic,
                    online: is_online_mode(&mode),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Offline (Live-bearing) session numbers — the chapters that can actually be
/// played. The finale is the highest of these.
pub(crate) fn offline_session_nums(db: &Database, course_key: &str) -> Vec<i32> {
    let mut nums: Vec<i32> = planned_sessions(db, course_key)
        .into_iter()
        .filter(|s| !s.online)
        .map(|s| s.num)
        .collect();
    nums.sort_unstable();
    nums.dedup();
    nums
}

pub(crate) fn align_key(course_key: &str) -> String {
    format!("{DETECTIVE_ALIGN_PREFIX}{course_key}")
}

/// Map of Live note id → matched 第N回 (content alignment).
pub(crate) fn load_align(
    db: &Database,
    course_key: &str,
) -> std::collections::HashMap<String, i32> {
    load_cache_json(db, &align_key(course_key)).unwrap_or_default()
}

pub(crate) fn save_align(
    db: &Database,
    course_key: &str,
    map: &std::collections::HashMap<String, i32>,
) {
    if let Ok(json) = serde_json::to_string(map) {
        let _ = db.save_data_cache(&align_key(course_key), &json);
    }
}

/// Campaign completion %, based on how many OFFLINE 授業計画 回 have an aligned,
/// cleared Live note. 100% ⇒ the final offline 回 is done ⇒ finale. Returns
/// `None` when the course has no usable 授業計画 (caller falls back).
pub(crate) fn campaign_progress_pct(db: &Database, course_key: &str) -> Option<u8> {
    let offline = offline_session_nums(db, course_key);
    if offline.is_empty() {
        return None;
    }
    let align = load_align(db, course_key);
    // No alignment data yet (older chapters, or alignment failed) — let the
    // caller fall back rather than freezing progress at 0%.
    if align.is_empty() {
        return None;
    }
    let offline_set: std::collections::HashSet<i32> = offline.iter().copied().collect();
    let prefix = format!("detective:chapter:{course_key}:");
    let mut cleared: std::collections::HashSet<i32> = std::collections::HashSet::new();
    for r in load_case_results(db) {
        if let Some(live_id) = r.case_id.strip_prefix(&prefix) {
            if let Some(&num) = align.get(live_id) {
                if offline_set.contains(&num) {
                    cleared.insert(num);
                }
            }
        }
    }
    let pct = ((cleared.len().min(offline.len()) as f32 / offline.len() as f32) * 100.0).round();
    Some((pct as u8).min(100))
}

pub(crate) fn load_included_courses(db: &Database) -> Vec<String> {
    load_cache_json(db, DETECTIVE_INCLUDED_KEY).unwrap_or_default()
}

pub(crate) fn load_case_results(db: &Database) -> Vec<DetectiveCaseResult> {
    let mut results: Vec<DetectiveCaseResult> =
        load_cache_json(db, DETECTIVE_RESULTS_KEY).unwrap_or_default();
    results.sort_by(|a, b| b.closed_at.cmp(&a.closed_at));
    results
}

pub(crate) fn build_review_queue(courses: &[DetectiveCourse]) -> Vec<DetectiveReviewItem> {
    let now_ms = crate::db::epoch_secs() * 1000;
    let mut out = Vec::new();

    for course in courses {
        for doubt in course.doubts.iter().take(3) {
            let overdue = doubt.due_at > 0 && doubt.due_at <= now_ms;
            out.push(DetectiveReviewItem {
                id: format!("doubt:{}", doubt.id),
                course_key: course.key.clone(),
                course_name: course.name.clone(),
                reason: if overdue {
                    format!(
                        "Unresolved doubt is due: {}",
                        truncate_chars(&doubt.note, 90)
                    )
                } else {
                    format!("Unresolved doubt: {}", truncate_chars(&doubt.note, 90))
                },
                priority: if overdue { 5 } else { 4 },
                due_at: doubt.due_at,
            });
        }

        if let Some(result) = course
            .recent_results
            .iter()
            .find(|item| item.confidence <= 2)
        {
            out.push(DetectiveReviewItem {
                id: format!("confidence:{}", result.id),
                course_key: course.key.clone(),
                course_name: course.name.clone(),
                reason: format!(
                    "Last closed case had low confidence ({}/5): {}",
                    result.confidence,
                    truncate_chars(&result.case_title, 80)
                ),
                priority: 3,
                due_at: result.closed_at + 24 * 60 * 60 * 1000,
            });
        }

        if course.live_records.is_empty() && !course.exam_signals.is_empty() {
            out.push(DetectiveReviewItem {
                id: format!("source-gap:{}", course.key),
                course_key: course.key.clone(),
                course_name: course.name.clone(),
                reason: "Exam signal exists, but Detective has no Live note evidence yet."
                    .to_string(),
                priority: 2,
                due_at: 0,
            });
        }
    }

    out.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| nonzero_or_max(a.due_at).cmp(&nonzero_or_max(b.due_at)))
    });
    out.truncate(8);
    out
}

pub(crate) fn nonzero_or_max(value: i64) -> i64 {
    if value > 0 {
        value
    } else {
        i64::MAX
    }
}

pub(crate) fn is_exam_signal(text: &str) -> bool {
    let lower = text.to_lowercase();
    EXAM_KEYWORDS
        .iter()
        .any(|keyword| lower.contains(&keyword.to_lowercase()))
}

pub(crate) fn match_signal_course_key(
    signal: &DetectiveSignal,
    courses: &HashMap<String, CourseBuilder>,
) -> Option<String> {
    if !signal.course_info.trim().is_empty() {
        let key = normalize_course_key(&signal.course_info);
        if courses.contains_key(&key) {
            return Some(key);
        }
    }
    let hay = normalize_course_key(&format!(
        "{} {} {}",
        signal.title, signal.category, signal.course_info
    ));
    let mut best: Option<String> = None;
    let mut best_len = 0usize;
    for key in courses.keys() {
        if key.len() < 3 {
            continue;
        }
        if (hay.contains(key) || key.contains(&hay)) && key.len() > best_len {
            best = Some(key.clone());
            best_len = key.len();
        }
    }
    best
}

pub(crate) fn date_score(date: &str) -> i64 {
    date.chars()
        .filter(|c| c.is_ascii_digit())
        .take(8)
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

pub(crate) fn dedupe_signals(signals: Vec<DetectiveSignal>) -> Vec<DetectiveSignal> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for signal in signals {
        let key = format!(
            "{}:{}:{}:{}",
            signal.source, signal.title, signal.date, signal.category
        );
        if seen.insert(key) {
            out.push(signal);
        }
    }
    out
}

pub(crate) fn readiness_text(course: &CourseBuilder) -> String {
    match (
        course.live_records.is_empty(),
        course.exam_signals.is_empty(),
        course.doubts.is_empty(),
    ) {
        (false, false, _) => "Live notes and exam signals ready".to_string(),
        (false, true, false) => "Live-note investigation with unresolved doubts".to_string(),
        (false, true, true) => "Live-note investigation ready".to_string(),
        (true, false, _) => "Exam-signal investigation ready".to_string(),
        (true, true, false) => "Unresolved doubt case ready".to_string(),
        _ => "Waiting for course evidence".to_string(),
    }
}

pub(crate) fn course_score(course: &DetectiveCourse) -> f64 {
    course.exam_signals.len() as f64 * 8.0
        + course.doubts.len() as f64 * 5.0
        + course.live_records.len() as f64 * 2.0
        + course.latest_at as f64 / 1_000_000_000_000.0
}
