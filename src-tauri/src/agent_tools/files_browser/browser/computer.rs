use serde_json::Value;

use super::support::insert_browser_window_snapshot;

pub async fn computer_screenshot(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let target = args.get("target").and_then(|v| v.as_str());
    crate::computer_control::screenshot(app, target).await
}

pub async fn computer_mouse_click(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let target = args.get("target").and_then(|v| v.as_str());
    let x = args.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let y = args.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let coordinate_space = args.get("coordinate_space").and_then(|v| v.as_str());
    let result = crate::computer_control::mouse_click(app, target, x, y, coordinate_space).await?;
    tokio::time::sleep(std::time::Duration::from_millis(450)).await;
    let mut out = match result {
        Value::Object(map) => map,
        other => {
            let mut map = serde_json::Map::new();
            map.insert("result".into(), other);
            map
        }
    };
    insert_browser_window_snapshot(app, &mut out);
    Ok(Value::Object(out))
}

pub async fn computer_mouse_drag(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let target = args.get("target").and_then(|v| v.as_str());
    let from_x = args.get("from_x").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let from_y = args.get("from_y").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let to_x = args.get("to_x").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let to_y = args.get("to_y").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let steps = args.get("steps").and_then(|v| v.as_u64()).unwrap_or(8);
    let coordinate_space = args.get("coordinate_space").and_then(|v| v.as_str());
    crate::computer_control::mouse_drag(
        app,
        target,
        from_x,
        from_y,
        to_x,
        to_y,
        steps,
        coordinate_space,
    )
    .await
}

pub async fn computer_scroll(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let target = args.get("target").and_then(|v| v.as_str());
    let delta_y = args
        .get("delta_y")
        .and_then(|v| v.as_i64())
        .unwrap_or(-700)
        .clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    let x = args.get("x").and_then(|v| v.as_f64());
    let y = args.get("y").and_then(|v| v.as_f64());
    let coordinate_space = args.get("coordinate_space").and_then(|v| v.as_str());
    crate::computer_control::scroll(app, target, delta_y, x, y, coordinate_space).await
}
