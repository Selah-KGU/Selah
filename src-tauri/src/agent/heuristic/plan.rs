use serde_json::json;

use super::super::{
    contains_any, extract_kgc_code, normalize_planner_text, recent_downloaded_file_path,
    should_skip_tools, tomorrow_week_offset, Plan,
};
use super::browser::is_browser_operation_intent;
use super::rules::HEURISTIC_RULES;
use super::single::single_tool_plan;

pub(in crate::agent) fn heuristic_plan(
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
) -> Option<Plan> {
    if should_skip_tools(history, user_text) {
        return Some(Plan::default());
    }

    let norm = normalize_planner_text(user_text);

    if is_browser_operation_intent(&norm) {
        return None;
    }

    if has_multiple_tool_domains(&norm) {
        return None;
    }

    if let Some(plan) = campus_browser_plan(&norm) {
        return Some(plan);
    }

    if let Some(path) = recent_downloaded_file_path(history) {
        if contains_any(
            &norm,
            &[
                "看看",
                "看一下",
                "看看内容",
                "内容",
                "总结",
                "總結",
                "summary",
                "要点",
                "重點",
                "写了什么",
                "寫了什麼",
                "说了什么",
                "說了什麼",
                "読んで",
                "読んでみて",
                "見て",
                "中身",
                "内容みて",
                "何が書いてある",
                "ppt",
                "pdf",
                "doc",
                "docx",
            ],
        ) {
            return Some(single_tool_plan(
                "read_downloaded_file",
                json!({ "path": path }),
            ));
        }
        if contains_any(&norm, &["打开", "打開", "開いて", "open"]) {
            return Some(single_tool_plan(
                "open_downloaded_file",
                json!({ "path": path }),
            ));
        }
    }

    // Table-driven matching.
    for rule in HEURISTIC_RULES {
        if !contains_any(&norm, rule.keywords) {
            continue;
        }
        if !rule.requires.is_empty() && !contains_any(&norm, rule.requires) {
            continue;
        }
        return Some(single_tool_plan(rule.tool, (rule.args)()));
    }

    if contains_any(
        &norm,
        &[
            "重新连接",
            "重新連接",
            "再接続",
            "reconnect",
            "retry",
            "重新试试",
            "重新試試",
        ],
    ) && !contains_any(
        &norm,
        &[
            "課題",
            "レポート",
            "mail",
            "メール",
            "通知",
            "授業",
            "课程",
            "course",
            "资料",
            "資料",
        ],
    ) {
        return Some(single_tool_plan("refresh_data", json!({})));
    }

    // "明日" / "明天" / "tomorrow" — needs dynamic offset based on day of week.
    if contains_any(&norm, &["明日", "明天", "tomorrow"]) {
        return Some(single_tool_plan(
            "list_week_classes",
            json!({ "offset": tomorrow_week_offset() }),
        ));
    }

    // KGC code extraction (structural, not keyword-based).
    if let Some(code) = extract_kgc_code(user_text) {
        if contains_any(
            &norm,
            &[
                "授業計画",
                "教材",
                "教科書",
                "詳細",
                "syllabus",
                "detail",
                "textbook",
            ],
        ) {
            return Some(single_tool_plan(
                "get_course_detail",
                json!({ "kgc_code": code }),
            ));
        }
    }

    None // Fall through to model inference.
}

fn has_multiple_tool_domains(norm: &str) -> bool {
    const DOMAINS: &[&[&str]] = &[
        &["メール", "mail", "邮件", "郵件"],
        &[
            "課題",
            "レポート",
            "todo",
            "task",
            "作业",
            "作業",
            "締切",
            "deadline",
        ],
        &["授業", "時間割", "schedule", "class", "课程", "上课"],
        &["成績", "grade", "成绩", "単位", "credit"],
        &["お知らせ", "通知", "notification"],
        &["ファイル", "資料", "添付", "file", "attachment", "文件"],
        &["天気", "weather", "天气"],
        &["カレンダー", "calendar", "日历", "日程"],
        &["luna", "ルナ"],
        &["kwic"],
        &["kgcourse", "kgc"],
    ];
    DOMAINS
        .iter()
        .filter(|markers| contains_any(norm, markers))
        .take(2)
        .count()
        >= 2
}

fn campus_browser_plan(norm: &str) -> Option<Plan> {
    if is_browser_operation_intent(norm) {
        return None;
    }

    // Distinguish "open Luna (the site root)" from "see the Luna *detail / content
    // / materials*" — the latter is about the page the user is already looking at,
    // not the portal root. When the request names specific content, fall through
    // to the model so it reads the current page (read_browser_page) or uses a
    // detail tool, instead of hard-jumping to the site root.
    if contains_any(
        norm,
        &[
            "详情",
            "詳細",
            "詳细",
            "detail",
            "内容",
            "中身",
            "なかみ",
            "教材",
            "资料",
            "資料",
        ],
    ) {
        return None;
    }

    if !contains_any(
        norm,
        &[
            "打开",
            "打開",
            "看看",
            "看一下",
            "浏览",
            "開いて",
            "開く",
            "見て",
            "open",
            "browser",
        ],
    ) {
        return None;
    }

    if contains_any(norm, &["luna", "ルナ"]) {
        return Some(single_tool_plan(
            "open_browser_url",
            json!({ "url": crate::config::LUNA_BASE }),
        ));
    }
    if contains_any(norm, &["kwic"]) {
        return Some(single_tool_plan(
            "open_browser_url",
            json!({ "url": crate::config::KWIC_BASE }),
        ));
    }
    if contains_any(norm, &["kgcourse", "kgc"]) {
        return Some(single_tool_plan(
            "open_browser_url",
            json!({ "url": crate::config::KG_COURSE_BASE }),
        ));
    }

    None
}
