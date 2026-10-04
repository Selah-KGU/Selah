use serde_json::Value;

use super::browser::{
    sanitize_browser_coord, sanitize_browser_mouse_drag_args, sanitize_browser_target_arg,
    sanitize_coordinate_space_arg,
};

pub(in crate::agent_tools) fn sanitize_computer_screenshot_args(args: &Value) -> Option<Value> {
    let target = sanitize_browser_target_arg(args);
    let mut out = serde_json::Map::new();
    if let Some(target) = target {
        out.insert("target".into(), Value::String(target));
    }
    Some(Value::Object(out))
}

pub(in crate::agent_tools) fn sanitize_computer_mouse_click_args(args: &Value) -> Option<Value> {
    let target = sanitize_browser_target_arg(args);
    let x = sanitize_browser_coord(args, "x")?;
    let y = sanitize_browser_coord(args, "y")?;
    let mut out = serde_json::Map::new();
    if let Some(target) = target {
        out.insert("target".into(), Value::String(target));
    }
    out.insert("x".into(), Value::Number(x.into()));
    out.insert("y".into(), Value::Number(y.into()));
    if let Some(space) = sanitize_coordinate_space_arg(args) {
        out.insert("coordinate_space".into(), Value::String(space));
    }
    Some(Value::Object(out))
}

pub(in crate::agent_tools) fn sanitize_computer_mouse_drag_args(args: &Value) -> Option<Value> {
    let mut out = sanitize_browser_mouse_drag_args(args)?;
    if let (Value::Object(map), Some(space)) = (&mut out, sanitize_coordinate_space_arg(args)) {
        map.insert("coordinate_space".into(), Value::String(space));
    }
    Some(out)
}

pub(in crate::agent_tools) fn sanitize_computer_scroll_args(args: &Value) -> Option<Value> {
    let target = sanitize_browser_target_arg(args);
    let delta_y = args
        .get("delta_y")
        .or_else(|| args.get("deltaY"))
        .and_then(|v| v.as_i64())
        .or_else(|| {
            args.get("direction").and_then(|v| v.as_str()).map(|dir| {
                match dir.trim().to_ascii_lowercase().as_str() {
                    "up" => 700,
                    "down" => -700,
                    _ => 0,
                }
            })
        })
        .unwrap_or(-700)
        .clamp(-5000, 5000);
    let mut out = serde_json::Map::new();
    if let Some(target) = target {
        out.insert("target".into(), Value::String(target));
    }
    out.insert("delta_y".into(), Value::Number(delta_y.into()));
    let x = args.get("x").and_then(|v| v.as_i64());
    let y = args.get("y").and_then(|v| v.as_i64());
    if let (Some(x), Some(y)) = (x, y) {
        out.insert("x".into(), Value::Number(x.clamp(-200_000, 200_000).into()));
        out.insert("y".into(), Value::Number(y.clamp(-200_000, 200_000).into()));
    }
    if let Some(space) = sanitize_coordinate_space_arg(args) {
        out.insert("coordinate_space".into(), Value::String(space));
    }
    Some(Value::Object(out))
}
