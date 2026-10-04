use serde_json::json;

use super::super::{contains_any, AgentTurnContext, Plan, ToolCall};
use super::single::single_tool_plan;

pub(in crate::agent) fn is_browser_operation_intent(norm: &str) -> bool {
    contains_any(
        norm,
        &[
            "点击",
            "點擊",
            "点一下",
            "点开",
            "押して",
            "クリック",
            "click",
            "填写",
            "填",
            "fill",
            "typeinto",
            "入力",
            "入力して",
            "submit",
            "送信",
            "提出して",
            "选择",
            "選択",
            "選んで",
            "select",
            "choose",
            "保存",
            "save",
            "決定",
            "scroll",
            "スクロール",
            "拖拽",
            "拖动",
            "ドラッグ",
            "drag",
            "mouse",
            "鼠标",
            "マウス",
        ],
    )
}

pub(in crate::agent) fn attached_browser_control_plan(
    norm: &str,
    turn_context: &AgentTurnContext,
) -> Option<Plan> {
    turn_context.browser_target.as_ref()?;
    if !turn_context.browser_click_labels.is_empty() {
        return Some(single_tool_plan("computer_screenshot", json!({})));
    }

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
            "home page",
            "homepage",
            "go home",
        ],
    ) {
        return Some(single_tool_plan("computer_screenshot", json!({})));
    }

    if contains_any(norm, &["返回", "后退", "戻って", "戻る", "back"]) {
        return Some(browser_nav_then_read_plan("browser_back"));
    }
    if contains_any(norm, &["前进", "進む", "forward"]) {
        return Some(browser_nav_then_read_plan("browser_forward"));
    }
    if contains_any(norm, &["刷新页面", "重载", "リロード", "reload page"]) {
        return Some(browser_nav_then_read_plan("browser_reload_page"));
    }

    if contains_any(norm, &["往下", "下に", "scroll down", "向下"]) {
        return Some(computer_scroll_then_observe_plan(-900));
    }
    if contains_any(norm, &["往上", "上に", "scroll up", "向上"]) {
        return Some(computer_scroll_then_observe_plan(900));
    }

    if is_browser_operation_intent(norm) {
        return Some(single_tool_plan("read_browser_page", json!({})));
    }

    None
}

fn browser_nav_then_read_plan(tool: &str) -> Plan {
    Plan {
        tools: vec![
            ToolCall {
                name: tool.to_string(),
                args: json!({}),
            },
            ToolCall {
                name: "read_browser_page".into(),
                args: json!({}),
            },
        ],
        image_only: false,
    }
}

fn computer_scroll_then_observe_plan(delta_y: i64) -> Plan {
    Plan {
        tools: vec![
            ToolCall {
                name: "computer_scroll".into(),
                args: json!({ "delta_y": delta_y }),
            },
            ToolCall {
                name: "computer_screenshot".into(),
                args: json!({}),
            },
            ToolCall {
                name: "read_browser_page".into(),
                args: json!({}),
            },
        ],
        image_only: false,
    }
}

pub(in crate::agent) fn extract_browser_click_text(norm: &str) -> Option<String> {
    let quoted = extract_between_any(
        norm,
        &[("“", "”"), ("\"", "\""), ("「", "」"), ("『", "』")],
    );
    if quoted.as_deref().is_some_and(|s| !s.trim().is_empty()) {
        return quoted;
    }

    for marker in [
        "点击",
        "點擊",
        "点一下",
        "点开",
        "押して",
        "クリック",
        "click",
    ] {
        if let Some((_, tail)) = norm.split_once(marker) {
            let text = tail
                .trim_matches(|c: char| c.is_whitespace() || matches!(c, ':' | '：' | ',' | '，'))
                .split_whitespace()
                .take(4)
                .collect::<Vec<_>>()
                .join(" ");
            if (2..=80).contains(&text.chars().count()) {
                return Some(text);
            }
        }
    }

    None
}

fn extract_between_any(s: &str, pairs: &[(&str, &str)]) -> Option<String> {
    for (open, close) in pairs {
        let Some((_, tail)) = s.split_once(open) else {
            continue;
        };
        let Some((inside, _)) = tail.split_once(close) else {
            continue;
        };
        let inside = inside.trim();
        if !inside.is_empty() {
            return Some(inside.to_string());
        }
    }
    None
}
