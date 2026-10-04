use chrono::{Datelike, Local, NaiveDate, TimeZone, Timelike, Weekday};

use crate::config::PERIOD_TIMES;
use crate::db::Database;

use super::model::{ClassSource, TodoSource, WidgetClass, WidgetSnapshot, WidgetTodo};

pub(super) fn snapshot_from_db(
    db: &Database,
    now: chrono::DateTime<Local>,
) -> Result<WidgetSnapshot, String> {
    let snap = db.get_snapshot_state()?.unwrap_or_default();
    let scope = crate::academic_period::visible_weeks(
        &snap.current_week_label,
        &snap.next_week_label,
        &snap.luna_year,
        &snap.luna_term,
        now.date_naive(),
    );
    let raw = db.build_raw_data(&scope.current, &scope.next, snap.luna_communities)?;
    let mut classes = classes_from_sources(
        &raw.kgc_entries_current
            .iter()
            .map(|row| ClassSource {
                day: row.day,
                period: row.period,
                name: row.name.clone(),
                room: row.room.clone(),
                cancelled: row.is_cancelled,
            })
            .collect::<Vec<_>>(),
    );
    if classes.is_empty() {
        classes = classes_from_sources(
            &raw.luna_courses
                .iter()
                .map(|row| ClassSource {
                    day: row.day,
                    period: row.period,
                    name: row.name.clone(),
                    room: String::new(),
                    cancelled: false,
                })
                .collect::<Vec<_>>(),
        );
    }
    let todos = todos_from_cache(db, now)?;
    Ok(assemble_snapshot(now, classes, todos))
}

fn todos_from_cache(
    db: &Database,
    now: chrono::DateTime<Local>,
) -> Result<Vec<WidgetTodo>, String> {
    let Some((json, _)) = db.get_data_cache("luna_todo")? else {
        return Ok(Vec::new());
    };
    let items: Vec<crate::luna_parser::LunaTodoItem> =
        serde_json::from_str(&json).unwrap_or_default();
    Ok(todos_from_sources(
        now,
        &items
            .into_iter()
            .map(|item| TodoSource {
                title: item.content_name,
                course: item.course_name,
                status: item.status,
                deadline: item.deadline,
            })
            .collect::<Vec<_>>(),
    ))
}

pub(crate) fn classes_from_sources(sources: &[ClassSource]) -> Vec<WidgetClass> {
    let mut classes = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for source in sources {
        if source.cancelled || source.day < 1 || source.day > 6 || source.period < 1 {
            continue;
        }
        let name = source.name.trim();
        if name.is_empty() {
            continue;
        }
        let key = (source.day, source.period, name.to_string());
        if !seen.insert(key) {
            continue;
        }
        let (start, end) = period_minutes(source.period);
        classes.push(WidgetClass {
            day: source.day,
            period: source.period,
            name: name.to_string(),
            room: source.room.trim().to_string(),
            start_minutes: start,
            end_minutes: end,
        });
    }
    classes.sort_by_key(|item| (item.day, item.period, item.name.clone()));
    classes
}

pub(crate) fn todos_from_sources(
    now: chrono::DateTime<Local>,
    sources: &[TodoSource],
) -> Vec<WidgetTodo> {
    let mut todos = Vec::new();
    for source in sources {
        if is_completed_status(&source.status) {
            continue;
        }
        let Some(due_unix) = parse_deadline(&source.deadline, now) else {
            continue;
        };
        if due_unix < now.timestamp() {
            continue;
        }
        let title = todo_title(&source.title, &source.course);
        if title.is_empty() {
            continue;
        }
        todos.push(WidgetTodo {
            title,
            course: source.course.trim().to_string(),
            due_unix,
        });
    }
    todos.sort_by_key(|item| item.due_unix);
    todos
}

pub(crate) fn assemble_snapshot(
    now: chrono::DateTime<Local>,
    classes: Vec<WidgetClass>,
    todos: Vec<WidgetTodo>,
) -> WidgetSnapshot {
    let today = weekday_number(now.date_naive());
    let now_minutes = now.hour() as i32 * 60 + now.minute() as i32;
    let remaining = classes
        .iter()
        .filter(|item| item.day == today && item.end_minutes > now_minutes)
        .count();
    let summary = if classes.iter().any(|item| item.day == today) {
        if remaining == 0 {
            "今日の授業はすべて終了".to_string()
        } else {
            format!("今日はあと{remaining}コマ")
        }
    } else if let Some(summary) = next_class(&classes, today).and_then(next_class_summary) {
        summary
    } else if classes.is_empty() && todos.is_empty() {
        "Selah を開くと表示されます".to_string()
    } else {
        "今日は授業がありません".to_string()
    };
    WidgetSnapshot {
        summary,
        classes,
        todos,
    }
}

fn next_class(classes: &[WidgetClass], today: i32) -> Option<&WidgetClass> {
    classes
        .iter()
        .filter(|item| (1..=6).contains(&item.day) && item.day != today)
        .min_by(|left, right| {
            class_distance(left.day, today)
                .cmp(&class_distance(right.day, today))
                .then(left.start_minutes.cmp(&right.start_minutes))
                .then(left.period.cmp(&right.period))
                .then(left.name.cmp(&right.name))
        })
}

fn class_distance(day: i32, today: i32) -> i32 {
    if day > today {
        day - today
    } else {
        day + 7 - today
    }
}

fn next_class_summary(class: &WidgetClass) -> Option<String> {
    let label = crate::config::DAY_SHORT
        .get(class.day as usize)
        .copied()
        .filter(|label| !label.is_empty())?;
    let hour = class.start_minutes.div_euclid(60);
    let minute = class.start_minutes.rem_euclid(60);
    Some(format!("次は{label}曜 {hour}:{minute:02}"))
}

fn period_minutes(period: i32) -> (i32, i32) {
    let index = (period - 1) as usize;
    let Some(&(start_h, start_m, end_h, end_m)) = PERIOD_TIMES.get(index) else {
        return (0, 0);
    };
    (
        (start_h as i32) * 60 + start_m as i32,
        (end_h as i32) * 60 + end_m as i32,
    )
}

fn weekday_number(date: NaiveDate) -> i32 {
    match date.weekday() {
        Weekday::Mon => 1,
        Weekday::Tue => 2,
        Weekday::Wed => 3,
        Weekday::Thu => 4,
        Weekday::Fri => 5,
        Weekday::Sat => 6,
        Weekday::Sun => 7,
    }
}

fn is_completed_status(status: &str) -> bool {
    let text = status.trim();
    text.contains("提出済")
        || text.contains("完了")
        || text.contains("採点済")
        || text.contains("受験済")
}

fn todo_title(content: &str, course: &str) -> String {
    let content = content.trim();
    let course = course.trim();
    let generic = matches!(
        content,
        "" | "課題"
            | "レポート"
            | "テスト"
            | "小テスト"
            | "掲示板"
            | "アンケート"
            | "出席"
            | "その他"
            | "未設定"
    );
    if !generic {
        return content.to_string();
    }
    if !course.is_empty() && !content.is_empty() {
        return format!("{course} {content}");
    }
    if !course.is_empty() {
        return course.to_string();
    }
    content.to_string()
}

pub(crate) fn parse_deadline(text: &str, now: chrono::DateTime<Local>) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    let date = find_date(&chars)?;
    let time = find_time(&chars);
    let (hour, minute, second) = time.unwrap_or((23, 59, 59));
    let local = Local
        .with_ymd_and_hms(date.0, date.1, date.2, hour, minute, second)
        .single()?;
    let _ = now;
    Some(local.timestamp())
}

fn find_date(chars: &[char]) -> Option<(i32, u32, u32)> {
    let mut index = 0;
    while index + 8 <= chars.len() {
        if let Some(parsed) = date_at(chars, index) {
            return Some(parsed);
        }
        index += 1;
    }
    None
}

fn date_at(chars: &[char], index: usize) -> Option<(i32, u32, u32)> {
    let year = four_digits(chars, index)?;
    let separator = *chars.get(index + 4)?;
    if separator != '/' && separator != '-' {
        return None;
    }
    let (month, month_len) = number_at(chars, index + 5, 2)?;
    let after_month = index + 5 + month_len;
    let month_separator = *chars.get(after_month)?;
    if month_separator != '/' && month_separator != '-' && month_separator != '月' {
        return None;
    }
    let (day, _) = number_at(chars, after_month + 1, 2)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year, month, day))
}

fn four_digits(chars: &[char], index: usize) -> Option<i32> {
    let slice = chars.get(index..index + 4)?;
    if !slice.iter().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let text: String = slice.iter().collect();
    text.parse().ok()
}

fn number_at(chars: &[char], index: usize, max_digits: usize) -> Option<(u32, usize)> {
    let mut digits = String::new();
    for ch in chars.get(index..)?.iter().take(max_digits) {
        if !ch.is_ascii_digit() {
            break;
        }
        digits.push(*ch);
    }
    if digits.is_empty() {
        return None;
    }
    Some((digits.parse().ok()?, digits.len()))
}

fn find_time(chars: &[char]) -> Option<(u32, u32, u32)> {
    let text: String = chars.iter().collect();
    let mut rest = text.as_str();
    while let Some(index) = rest.find(':') {
        let before = &rest[..index];
        let hour_digits: String = before
            .chars()
            .rev()
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        let after = &rest[index + 1..];
        if let (Some((hour, _)), Some((minute, _))) =
            (take_number(&hour_digits, 2), take_number(after, 2))
        {
            if hour <= 23 && minute <= 59 {
                return Some((hour, minute, 0));
            }
        }
        rest = &rest[index + 1..];
    }
    None
}

fn take_number(text: &str, max_digits: usize) -> Option<(u32, usize)> {
    let digits: String = text
        .chars()
        .take(max_digits)
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return None;
    }
    Some((digits.parse().ok()?, digits.len()))
}
