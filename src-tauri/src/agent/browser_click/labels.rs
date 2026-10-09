use super::super::{contains_any, extract_browser_click_text, normalize_planner_text};
use crate::agent_tools;

// Largest lookback used by click confirmations and numbered selections.
pub(in crate::agent) const BROWSER_CLICK_HISTORY_ROWS: usize = 12;

pub(in crate::agent) fn requested_click_labels(norm: &str) -> Option<Vec<String>> {
    if contains_any(
        norm,
        &[
            "回首页",
            "回到首页",
            "返回首页",
            "去首页",
            "进入首页",
            "回主页",
            "回到主页",
            "返回主页",
            "トップページ",
            "ホーム",
            "homepage",
            "gohome",
        ],
    ) {
        let mut labels: Vec<String> = [
            "home",
            "ホーム",
            "トップ",
            "トップページ",
            "首页",
            "主页",
            "top",
            "logo",
            "ロゴ",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        labels.sort();
        labels.dedup();
        return Some(labels);
    }

    extract_browser_click_text(norm).and_then(|text| normalized_click_labels(&text))
}

pub(in crate::agent) fn browser_click_labels_for_turn(
    history: &[crate::db::AgentMessageRow],
    user_text: &str,
) -> Vec<String> {
    let norm = normalize_planner_text(user_text);
    if let Some(labels) = requested_click_labels(&norm) {
        return labels;
    }
    if let Some(index) = selection_index_from_norm(&norm) {
        if let Some(labels) = recent_numbered_click_labels(history, index, &norm) {
            return labels;
        }
    }
    if !is_short_click_confirmation(&norm) {
        return Vec::new();
    }
    let recent = history
        .iter()
        .rev()
        .take(6)
        .map(|row| row.content.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let recent_norm = normalize_planner_text(&recent);
    if contains_any(
        &recent_norm,
        &[
            "回首页",
            "回到首页",
            "返回首页",
            "回主页",
            "回到主页",
            "返回主页",
            "主页",
            "首页",
            "ホーム",
            "トップページ",
            "homepage",
            "logo",
            "ロゴ",
        ],
    ) {
        return requested_click_labels("回到主页").unwrap_or_default();
    }
    if let Some(labels) = recent_explicit_click_labels(history, &norm) {
        return labels;
    }
    Vec::new()
}

fn recent_explicit_click_labels(
    history: &[crate::db::AgentMessageRow],
    current_norm: &str,
) -> Option<Vec<String>> {
    for row in history.iter().rev().take(10) {
        let text = row.content.trim();
        if text.is_empty() || normalize_planner_text(text) == current_norm {
            continue;
        }
        if let Some(labels) = click_labels_from_recent_text(text) {
            return Some(labels);
        }
    }
    None
}

fn selection_index_from_norm(norm: &str) -> Option<usize> {
    match norm {
        "1" | "１" | "第1" | "第一" | "第一个" | "第一個" | "选1" | "選1" | "选择1" | "選擇1"
        | "一番目" | "1番目" => Some(1),
        "2" | "２" | "第2" | "第二" | "第二个" | "第二個" | "选2" | "選2" | "选择2" | "選擇2"
        | "二番目" | "2番目" => Some(2),
        "3" | "３" | "第3" | "第三" | "第三个" | "第三個" | "选3" | "選3" | "选择3" | "選擇3"
        | "三番目" | "3番目" => Some(3),
        _ => None,
    }
}

fn recent_numbered_click_labels(
    history: &[crate::db::AgentMessageRow],
    index: usize,
    current_norm: &str,
) -> Option<Vec<String>> {
    for row in history.iter().rev().take(BROWSER_CLICK_HISTORY_ROWS) {
        if row.role != "assistant" {
            continue;
        }
        let text = row.content.trim();
        if text.is_empty() || normalize_planner_text(text) == current_norm {
            continue;
        }
        if let Some(labels) = numbered_click_labels_from_text(text, index) {
            return Some(labels);
        }
    }
    None
}

fn numbered_click_labels_from_text(text: &str, index: usize) -> Option<Vec<String>> {
    for line in text.lines() {
        if numbered_line_index(line) != Some(index) {
            continue;
        }
        let labels = extract_click_label_candidates(line);
        if !labels.is_empty() {
            return Some(labels);
        }
    }
    None
}

fn numbered_line_index(line: &str) -> Option<usize> {
    let trimmed = line
        .trim_start()
        .trim_start_matches(['*', '-', '・', '•'])
        .trim_start();
    let mut chars = trimmed.chars();
    let first = chars.next()?;
    let value = match first {
        '1' | '１' => 1,
        '2' | '２' => 2,
        '3' | '３' => 3,
        _ => return None,
    };
    let next = chars.next().unwrap_or(' ');
    if matches!(next, '.' | '．' | ')' | '）' | '、' | ':' | '：') || next.is_whitespace() {
        Some(value)
    } else {
        None
    }
}

fn click_labels_from_recent_text(text: &str) -> Option<Vec<String>> {
    let mut candidates = Vec::new();
    for line in text.lines().rev() {
        let line_norm = normalize_planner_text(line);
        if !contains_any(
            &line_norm,
            &[
                "点击",
                "點擊",
                "点",
                "クリック",
                "押して",
                "标签",
                "タブ",
                "ボタン",
                "リンク",
                "上方",
                "导航",
                "ナビ",
            ],
        ) {
            continue;
        }
        candidates.extend(extract_click_label_candidates(line));
        if !candidates.is_empty() {
            break;
        }
    }
    if candidates.is_empty() {
        candidates.extend(extract_click_label_candidates(text));
    }
    candidates.dedup();
    if candidates.is_empty() {
        None
    } else {
        Some(candidates)
    }
}

fn extract_click_label_candidates(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for value in extract_all_between_any(
        text,
        &[
            ("“", "”"),
            ("\"", "\""),
            ("「", "」"),
            ("『", "』"),
            ("`", "`"),
            ("**", "**"),
        ],
    ) {
        out.extend(normalized_click_labels(&value).unwrap_or_default());
    }
    for value in extract_markdown_link_labels(text) {
        out.extend(normalized_click_labels(&value).unwrap_or_default());
    }
    out.sort();
    out.dedup();
    out
}

fn normalized_click_labels(text: &str) -> Option<Vec<String>> {
    let trimmed = text.trim();
    if !is_meaningful_click_label(trimmed) {
        return None;
    }
    let mut labels = Vec::new();
    push_click_label_variant(&mut labels, trimmed);
    for part in extract_all_between_any(trimmed, &[("（", "）"), ("(", ")")]) {
        push_click_label_variant(&mut labels, &part);
    }
    let before_paren = trimmed.split(['（', '(']).next().unwrap_or(trimmed).trim();
    push_click_label_variant(&mut labels, before_paren);

    labels.sort();
    labels.dedup();
    if labels.is_empty() {
        None
    } else {
        Some(labels)
    }
}

pub(in crate::agent) fn normalize_click_match_text(s: &str) -> String {
    normalize_planner_text(s)
        .chars()
        .map(agent_tools::normalize_cjk_char)
        .collect()
}

fn push_click_label_variant(out: &mut Vec<String>, value: &str) {
    let Some(cleaned) = strip_click_label_generic_words(value) else {
        return;
    };
    let normalized = normalize_planner_text(&cleaned);
    if !normalized.is_empty() {
        out.push(normalized);
    }
}

fn strip_click_label_generic_words(text: &str) -> Option<String> {
    let mut value = text
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '「' | '」' | '『' | '』'))
        .to_string();
    for suffix in [
        "というボタン",
        "这个按钮",
        "這個按鈕",
        "的按钮",
        "的按鈕",
        "ボタン",
        "按钮",
        "按鈕",
        "button",
        "リンク",
        "链接",
        "連結",
        "link",
        "タブ",
        "标签",
        "頁籤",
        "选项卡",
        "選項卡",
        "菜单",
        "メニュー",
        "入口",
        "选项",
        "選項",
    ] {
        loop {
            let trimmed = value.trim();
            if !trimmed.ends_with(suffix) {
                break;
            }
            value = trimmed[..trimmed.len().saturating_sub(suffix.len())]
                .trim()
                .to_string();
        }
    }
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

fn is_meaningful_click_label(text: &str) -> bool {
    let trimmed = text.trim();
    let count = trimmed.chars().count();
    if !(2..=80).contains(&count) {
        return false;
    }
    let norm = normalize_planner_text(trimmed);
    if norm.is_empty() || norm.starts_with("http") {
        return false;
    }
    if contains_any(
        &norm,
        &["标签", "頁籤", "选项卡", "タブ", "tab", "导航", "菜单"],
    ) && contains_any(&norm, &["全部", "看看", "看", "all", "一覧"])
    {
        return false;
    }
    !matches!(
        norm.as_str(),
        "标签"
            | "全部"
            | "这里"
            | "這里"
            | "この"
            | "これ"
            | "这个"
            | "這個"
            | "哪一个具体部分"
            | "哪个"
            | "哪個"
            | "点击"
            | "點擊"
            | "click"
            | "button"
    )
}

fn extract_all_between_any(s: &str, pairs: &[(&str, &str)]) -> Vec<String> {
    let mut out = Vec::new();
    for (open, close) in pairs {
        let mut rest = s;
        while let Some((_, tail)) = rest.split_once(open) {
            let Some((inside, next)) = tail.split_once(close) else {
                break;
            };
            let inside = inside.trim();
            if !inside.is_empty() {
                out.push(inside.to_string());
            }
            rest = next;
        }
    }
    out
}

fn extract_markdown_link_labels(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some((_, tail)) = rest.split_once('[') {
        let Some((label, after_label)) = tail.split_once("](") else {
            break;
        };
        let Some((_, next)) = after_label.split_once(')') else {
            break;
        };
        let label = label.trim();
        if !label.is_empty() {
            out.push(label.to_string());
        }
        rest = next;
    }
    out
}

fn is_short_click_confirmation(norm: &str) -> bool {
    matches!(
        norm,
        "点" | "点啊"
            | "点击"
            | "點"
            | "點啊"
            | "点吧"
            | "好"
            | "好的"
            | "执行"
            | "去点"
            | "点logo"
            | "重试"
            | "再试"
            | "再试一次"
            | "重新试"
            | "重新点击"
            | "retry"
            | "tryagain"
            | "click"
            | "doit"
            | "yes"
            | "ok"
            | "押して"
            | "クリック"
    )
}
