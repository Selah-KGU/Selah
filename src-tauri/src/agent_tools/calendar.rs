use super::*;

/// Create a single Google Calendar event from explicit fields.
pub(super) async fn create_google_calendar_event(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or("titleは必須です")?;
    let date = args
        .get("date")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or("dateは必須です (YYYY-MM-DD)")?;
    let start_time = args
        .get("start_time")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or("start_timeは必須です (HH:MM)")?;
    let end_time = args
        .get("end_time")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or("end_timeは必須です (HH:MM)")?;
    let location = args
        .get("location")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let description = args
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());

    let gcal_state = app.state::<crate::GCalState>();
    let mut gcal = gcal_state.client.lock().await;

    let message = gcal
        .create_single_event(title, date, start_time, end_time, location, description)
        .await?;

    Ok(json!({ "message": message }))
}

/// List all agent-created Google Calendar events (newest first).
pub(super) async fn list_google_calendar_events(app: &tauri::AppHandle) -> Result<Value, String> {
    let gcal_state = app.state::<crate::GCalState>();
    let gcal = gcal_state.client.lock().await;
    let events: Vec<Value> = gcal
        .list_agent_events()
        .into_iter()
        .map(|(event_id, meta)| {
            let mut obj = serde_json::json!({
                "event_id": event_id,
                "title": meta.title,
                "date": meta.date,
                "start_time": meta.start_time,
                "end_time": meta.end_time,
            });
            if let Some(loc) = meta.location {
                obj["location"] = Value::String(loc);
            }
            if let Some(desc) = meta.description {
                obj["description"] = Value::String(desc);
            }
            obj
        })
        .collect();
    Ok(json!({ "events": events }))
}

/// Delete an agent-created Google Calendar event by event_id.
pub(super) async fn delete_google_calendar_event(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let event_id = args
        .get("event_id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or("event_idは必須です")?;

    let gcal_state = app.state::<crate::GCalState>();
    let mut gcal = gcal_state.client.lock().await;
    let message = gcal.delete_agent_event(event_id).await?;
    Ok(json!({ "message": message }))
}

/// Update fields of an agent-created Google Calendar event.
pub(super) async fn update_google_calendar_event(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let event_id = args
        .get("event_id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or("event_idは必須です")?;

    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let date = args
        .get("date")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let start_time = args
        .get("start_time")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let end_time = args
        .get("end_time")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());

    // For location / description: None = keep existing, Some(None) = clear
    let location: Option<Option<&str>> = if args.get("location").is_some() {
        Some(
            args.get("location")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty()),
        )
    } else {
        None
    };
    let description: Option<Option<&str>> = if args.get("description").is_some() {
        Some(
            args.get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty()),
        )
    } else {
        None
    };

    let gcal_state = app.state::<crate::GCalState>();
    let mut gcal = gcal_state.client.lock().await;
    let message = gcal
        .update_agent_event(event_id, title, date, start_time, end_time, location, description)
        .await?;
    Ok(json!({ "message": message }))
}
