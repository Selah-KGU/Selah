use super::super::{contains_any, normalize_planner_text, Value};
use super::BrowserClickCandidate;

pub(in crate::agent) fn click_candidate_priority(
    item: &BrowserClickCandidate,
    page: &Value,
) -> i64 {
    let mut score = 0_i64;
    if item.center_y <= 220 {
        score += 1_000;
    }
    if same_host(
        page.get("url").and_then(|v| v.as_str()).unwrap_or(""),
        &item.url,
    )
    .unwrap_or(false)
    {
        score += 180;
    }
    let url = item.url.to_ascii_lowercase();
    if url.ends_with(".pdf") || url.contains(".pdf?") {
        score -= 800;
    }
    if contains_any(
        &normalize_planner_text(&item.label),
        &["申込", "予約", "login", "ログイン"],
    ) {
        score -= 500;
    }
    score - item.center_y.max(0)
}

pub(in crate::agent) fn browser_observation_candidates(page: &Value) -> Vec<BrowserClickCandidate> {
    let mut out = Vec::new();
    if let Some(links) = page.get("links").and_then(|v| v.as_array()) {
        for link in links {
            if let Some(candidate) = click_candidate_from_value(link, &["text", "url"]) {
                out.push(candidate);
            }
        }
    }
    let elements = page.get("interactive_elements").unwrap_or(&Value::Null);
    if let Some(buttons) = elements.get("buttons").and_then(|v| v.as_array()) {
        for button in buttons {
            if let Some(candidate) = click_candidate_from_value(button, &["text", "type"]) {
                out.push(candidate);
            }
        }
    }
    if let Some(inputs) = elements.get("inputs").and_then(|v| v.as_array()) {
        for input in inputs {
            if let Some(candidate) =
                click_candidate_from_value(input, &["label", "name", "placeholder", "value"])
            {
                out.push(candidate);
            }
        }
    }
    out
}

pub(in crate::agent) fn labels_indicate_home(labels: &[String]) -> bool {
    labels.iter().any(|label| {
        matches!(
            label.as_str(),
            "home"
                | "ホーム"
                | "トップ"
                | "トップページ"
                | "首页"
                | "主页"
                | "top"
                | "logo"
                | "ロゴ"
        )
    })
}

pub(in crate::agent) fn top_left_click_candidate(page: &Value) -> Option<BrowserClickCandidate> {
    let viewport_width = page
        .get("viewport")
        .and_then(|v| v.get("width"))
        .and_then(|v| v.as_i64())
        .unwrap_or(1200)
        .max(1);
    let max_x = ((viewport_width as f64) * 0.42).round() as i64;
    browser_observation_candidates(page)
        .into_iter()
        .filter(|item| item.center_x >= 0 && item.center_y >= 0)
        .filter(|item| item.center_x <= max_x && item.center_y <= 180)
        .min_by_key(|item| item.center_y * 10_000 + item.center_x)
}

pub(in crate::agent) fn wants_to_browse_visible_tabs(norm: &str) -> bool {
    contains_any(
        norm,
        &[
            "标签",
            "頁籤",
            "选项卡",
            "タブ",
            "tab",
            "导航",
            "ナビ",
            "菜单",
            "メニュー",
        ],
    ) && contains_any(
        norm,
        &[
            "点击",
            "点",
            "看看",
            "看",
            "全部",
            "全て",
            "すべて",
            "all",
            "一覧",
            "打开",
            "開く",
        ],
    )
}

pub(in crate::agent) fn top_navigation_click_candidate(
    page: &Value,
) -> Option<BrowserClickCandidate> {
    let viewport_width = page
        .get("viewport")
        .and_then(|v| v.get("width"))
        .and_then(|v| v.as_i64())
        .unwrap_or(1200)
        .max(1);
    browser_observation_candidates(page)
        .into_iter()
        .filter(|item| item.center_x >= 0 && item.center_y >= 0)
        .filter(|item| item.center_x <= viewport_width && item.center_y <= 220)
        .filter(|item| is_safe_navigation_candidate(item, page))
        .min_by_key(|item| item.center_y * 10_000 + item.center_x)
}

fn is_safe_navigation_candidate(item: &BrowserClickCandidate, page: &Value) -> bool {
    let label = normalize_planner_text(&item.label);
    if label.is_empty()
        || label.chars().count() > 40
        || labels_indicate_home(std::slice::from_ref(&label))
        || label.starts_with("home")
        || label.starts_with("トップ")
        || contains_any(
            &label,
            &[
                "問い合わせ",
                "お問い合わせ",
                "contact",
                "login",
                "ログイン",
                "予約",
                "申込",
                "申し込み",
                "apply",
                "submit",
            ],
        )
    {
        return false;
    }
    if item.url.is_empty() {
        return true;
    }
    let url_lower = item.url.to_ascii_lowercase();
    if !(url_lower.starts_with("http://") || url_lower.starts_with("https://")) {
        return false;
    }
    let page_url = page.get("url").and_then(|v| v.as_str()).unwrap_or("");
    same_host(page_url, &item.url).unwrap_or(true)
}

fn same_host(a: &str, b: &str) -> Option<bool> {
    let a = url::Url::parse(a).ok()?;
    let b = url::Url::parse(b).ok()?;
    Some(a.host_str() == b.host_str())
}

fn click_candidate_from_value(item: &Value, label_keys: &[&str]) -> Option<BrowserClickCandidate> {
    let rect = item.get("rect")?;
    let center_x = rect
        .get("centerX")
        .or_else(|| rect.get("center_x"))?
        .as_i64()?;
    let center_y = rect
        .get("centerY")
        .or_else(|| rect.get("center_y"))?
        .as_i64()?;
    let label = label_keys
        .iter()
        .filter_map(|key| item.get(*key).and_then(|v| v.as_str()))
        .filter(|s| !s.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if label.trim().is_empty() {
        return None;
    }
    Some(BrowserClickCandidate {
        label,
        url: item
            .get("url")
            .or_else(|| item.get("href"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        center_x,
        center_y,
    })
}
