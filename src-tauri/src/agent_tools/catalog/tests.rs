use super::super::sanitize_tool_args;
use super::*;
use serde_json::json;
use std::collections::BTreeSet;

#[test]
fn tool_aliases_point_to_registered_tools() {
    let registered = TOOL_SPECS
        .iter()
        .map(|spec| spec.name)
        .collect::<BTreeSet<_>>();
    let mut aliases = BTreeSet::new();
    for (alias, target) in TOOL_ALIASES {
        assert!(aliases.insert(*alias), "duplicate alias: {alias}");
        assert!(
            registered.contains(target),
            "alias {alias} points to missing tool {target}"
        );
        assert_eq!(canonical_tool_name(alias), Some(*target));
    }
}

#[test]
fn tool_catalog_prompt_is_cached() {
    let first = tool_catalog_prompt().as_ptr();
    let second = tool_catalog_prompt().as_ptr();
    assert_eq!(first, second);
    assert!(tool_catalog_prompt().contains("open_browser_url(url: string)"));
    assert!(tool_catalog_prompt().contains("open_copilot_page(page:"));
}

#[test]
fn copilot_page_args_are_structured_and_whitelisted() {
    assert_eq!(
        sanitize_tool_args(
            "open_copilot_page",
            &json!({"page":"files","course_name":"政治学"})
        ),
        Some(json!({"page":"files","context":"政治学"}))
    );
    assert_eq!(
        sanitize_tool_args("open_copilot_page", &json!({"page":"luna"})),
        Some(json!({"page":"luna"}))
    );
    assert_eq!(
        sanitize_tool_args(
            "open_copilot_page",
            &json!({"page":"luna_activity","context":"第7回課題","luna_id":"LUNA-42"})
        ),
        Some(json!({"page":"luna_activity","context":"第7回課題","luna_id":"LUNA-42"}))
    );
    assert!(sanitize_tool_args("open_copilot_page", &json!({"page":"luna_activity"})).is_none());
    assert_eq!(
        sanitize_tool_args(
            "open_copilot_page",
            &json!({"page":"luna_course","course":"政治学","luna_id":"LUNA-7"})
        ),
        Some(json!({"page":"luna_course","context":"政治学","luna_id":"LUNA-7"}))
    );
    assert!(sanitize_tool_args("open_copilot_page", &json!({"page":"luna_course"})).is_none());
    assert_eq!(
        sanitize_tool_args(
            "open_copilot_page",
            &json!({"page":"kwic_notification","context":"履修登録のお知らせ","id":"kwic-7"})
        ),
        Some(
            json!({"page":"kwic_notification","context":"履修登録のお知らせ","identifier":"kwic-7"})
        )
    );
    assert_eq!(
        sanitize_tool_args(
            "open_copilot_page",
            &json!({"page":"kgc_notification","identifier":"kgc-9"})
        ),
        Some(json!({"page":"kgc_notification","identifier":"kgc-9"}))
    );
    assert_eq!(
        sanitize_tool_args("open_copilot_page", &json!({"page":"kwic_cabinet"})),
        Some(json!({"page":"kwic_cabinet"}))
    );
    assert!(sanitize_tool_args("open_copilot_page", &json!({"page":"kgc_notification"})).is_none());
    assert!(sanitize_tool_args("open_copilot_page", &json!({"page":"made-up-page"})).is_none());
}
