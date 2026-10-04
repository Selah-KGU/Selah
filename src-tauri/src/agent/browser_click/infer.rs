use super::super::{
    is_browser_operation_intent, json, normalize_planner_text, AgentTurnContext, Value,
};
use super::candidates::{
    browser_observation_candidates, click_candidate_priority, labels_indicate_home,
    top_left_click_candidate, top_navigation_click_candidate, wants_to_browse_visible_tabs,
};
use super::labels::{normalize_click_match_text, requested_click_labels};

pub(in crate::agent) fn local_browser_action_answer(
    user_text: &str,
    tool_results: &[(String, Value)],
    turn_context: &AgentTurnContext,
) -> Option<String> {
    if turn_context.browser_target.is_none()
        || !is_browser_operation_intent(&normalize_planner_text(user_text))
    {
        return None;
    }

    let (_, result) = tool_results.iter().rev().find(|(name, result)| {
        matches!(
            name.as_str(),
            "computer_mouse_click" | "browser_mouse_click" | "browser_click"
        ) && result.get("error").is_none()
    })?;
    let answer = result
        .get("current_url")
        .and_then(|v| v.as_str())
        .filter(|url| !url.trim().is_empty())
        .map(|url| format!("已点击。当前页面：{url}"))
        .unwrap_or_else(|| "已点击。".to_string());
    Some(answer)
}

pub(in crate::agent) fn infer_mouse_click_from_observation(
    user_text: &str,
    page: &Value,
    turn_context: &AgentTurnContext,
) -> Option<Value> {
    let norm = normalize_planner_text(user_text);
    let labels = if turn_context.browser_click_labels.is_empty() {
        requested_click_labels(&norm)?
    } else {
        turn_context.browser_click_labels.clone()
    };
    let matched = browser_observation_candidates(page)
        .into_iter()
        .filter(|item| {
            let hay = normalize_click_match_text(&item.label);
            labels
                .iter()
                .map(|label| normalize_click_match_text(label))
                .any(|label| !label.is_empty() && (hay.contains(&label) || label.contains(&hay)))
        })
        .max_by_key(|item| click_candidate_priority(item, page));
    if let Some(item) = matched {
        return Some(json!({
            "x": item.center_x,
            "y": item.center_y,
            "coordinate_space": "webview",
        }));
    }
    if labels_indicate_home(&labels) {
        if let Some(item) = top_left_click_candidate(page) {
            return Some(json!({
                "x": item.center_x,
                "y": item.center_y,
                "coordinate_space": "webview",
            }));
        }
    }
    None
}

pub(in crate::agent) fn infer_tab_browse_click_from_observation(
    user_text: &str,
    page: &Value,
) -> Option<Value> {
    let norm = normalize_planner_text(user_text);
    if !wants_to_browse_visible_tabs(&norm) {
        return None;
    }
    top_navigation_click_candidate(page).map(|item| {
        json!({
            "x": item.center_x,
            "y": item.center_y,
            "coordinate_space": "webview",
        })
    })
}

pub(in crate::agent) fn infer_mouse_click_from_screenshot(
    user_text: &str,
    screenshot: &Value,
    turn_context: &AgentTurnContext,
) -> Option<Value> {
    let norm = normalize_planner_text(user_text);
    let labels = if turn_context.browser_click_labels.is_empty() {
        requested_click_labels(&norm)?
    } else {
        turn_context.browser_click_labels.clone()
    };
    if !labels_indicate_home(&labels) {
        return None;
    }

    let width = screenshot
        .get("screen_rect")
        .and_then(|v| v.get("width"))
        .and_then(|v| v.as_f64())
        .unwrap_or(1200.0)
        .max(1.0);
    let height = screenshot
        .get("screen_rect")
        .and_then(|v| v.get("height"))
        .and_then(|v| v.as_f64())
        .unwrap_or(800.0)
        .max(1.0);
    let x = (width * 0.12).clamp(48.0, 160.0).min(width - 1.0);
    let y = (height * 0.08).clamp(32.0, 92.0).min(height - 1.0);
    Some(json!({
        "x": x.round() as i64,
        "y": y.round() as i64,
        "coordinate_space": "screenshot",
    }))
}
