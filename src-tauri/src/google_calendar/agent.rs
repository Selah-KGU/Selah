use super::config::save_sync_state;
use super::sync::{DATE_RE, TIME_RE};
use super::{AgentEventMeta, GCAL_API_BASE};

impl super::GoogleCalendarClient {
    /// Create a single free-form event on the "Selah 時間割" calendar.
    /// `date` must be YYYY-MM-DD, `start_time` / `end_time` must be HH:MM.
    /// Returns a human-readable confirmation string.
    pub async fn create_single_event(
        &mut self,
        title: &str,
        date: &str,
        start_time: &str,
        end_time: &str,
        location: Option<&str>,
        description: Option<&str>,
    ) -> Result<String, String> {
        if !self.is_authenticated() {
            return Err(
                "Google Calendarにログインしていません。設定画面から連携してください。".into(),
            );
        }
        // Basic format validation to prevent injection into the API call.
        if !DATE_RE.is_match(date) {
            return Err(format!(
                "日付フォーマットが不正です (期待: YYYY-MM-DD): {}",
                date
            ));
        }
        if !TIME_RE.is_match(start_time) || !TIME_RE.is_match(end_time) {
            return Err("時刻フォーマットが不正です (期待: HH:MM)".into());
        }
        let cal_id = self.ensure_calendar().await?;
        let start_dt = format!("{}T{}:00", date, start_time);
        let end_dt = format!("{}T{}:00", date, end_time);
        let mut body = serde_json::json!({
            "summary": title,
            "start": { "dateTime": start_dt, "timeZone": "Asia/Tokyo" },
            "end":   { "dateTime": end_dt,   "timeZone": "Asia/Tokyo" },
        });
        if let Some(loc) = location {
            body["location"] = serde_json::Value::String(loc.to_string());
        }
        if let Some(desc) = description {
            body["description"] = serde_json::Value::String(desc.to_string());
        }
        let event_id = self.create_event(&cal_id, &body).await?;
        // Persist locally so we can list / edit / delete later.
        self.sync_state.agent_event_map.insert(
            event_id,
            AgentEventMeta {
                title: title.to_string(),
                date: date.to_string(),
                start_time: start_time.to_string(),
                end_time: end_time.to_string(),
                location: location.map(|s| s.to_string()),
                description: description.map(|s| s.to_string()),
            },
        );
        save_sync_state(&self.sync_state)?;
        Ok(format!(
            "「{}」を {} {} – {} にGoogle Calendarへ登録しました。",
            title, date, start_time, end_time
        ))
    }

    /// List all agent-created events (newest date first).
    pub fn list_agent_events(&self) -> Vec<(String, AgentEventMeta)> {
        let mut items: Vec<(String, AgentEventMeta)> = self
            .sync_state
            .agent_event_map
            .iter()
            .map(|(id, meta)| (id.clone(), meta.clone()))
            .collect();
        // Sort descending by date then start_time.
        items.sort_by(|a, b| {
            b.1.date
                .cmp(&a.1.date)
                .then(b.1.start_time.cmp(&a.1.start_time))
        });
        items
    }

    /// Read the actual upcoming events on the app's own "Selah 時間割" calendar
    /// (the true Google state, including timetable-synced entries — not just the
    /// locally-tracked agent event map).
    pub async fn list_upcoming_events(
        &mut self,
        days_ahead: i64,
        max_results: u32,
    ) -> Result<Vec<serde_json::Value>, String> {
        if !self.is_authenticated() {
            return Err("Google Calendarにログインしていません。".into());
        }
        let cal_id = self.ensure_calendar().await?;
        let token = self.ensure_token().await?;
        let now = chrono::Utc::now();
        let time_min = now.to_rfc3339();
        let time_max = (now + chrono::Duration::days(days_ahead.clamp(1, 90))).to_rfc3339();
        let max_results = max_results.clamp(1, 100).to_string();
        let resp = self
            .http
            .send(
                self.http
                    .client
                    .get(format!(
                        "{}/calendars/{}/events",
                        GCAL_API_BASE,
                        urlencoding::encode(&cal_id)
                    ))
                    .bearer_auth(&token)
                    .query(&[
                        ("timeMin", time_min.as_str()),
                        ("timeMax", time_max.as_str()),
                        ("singleEvents", "true"),
                        ("orderBy", "startTime"),
                        ("maxResults", max_results.as_str()),
                    ]),
            )
            .await
            .map_err(|e| format!("予定取得失敗: {}", e))?;
        if !resp.status().is_success() {
            let err: serde_json::Value = resp.json().unwrap_or_default();
            return Err(format!("予定取得失敗: {}", err));
        }
        let body: serde_json::Value = resp
            .json()
            .map_err(|e| format!("予定レスポンス解析失敗: {}", e))?;
        let events = body["items"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|it| {
                        let start = it.get("start");
                        let end = it.get("end");
                        // Timed events carry "dateTime"; all-day events carry "date".
                        let pick = |slot: Option<&serde_json::Value>| {
                            slot.and_then(|s| s.get("dateTime").or_else(|| s.get("date")))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string()
                        };
                        let all_day = start.and_then(|s| s.get("date")).is_some();
                        serde_json::json!({
                            "title": it.get("summary").and_then(|v| v.as_str()).unwrap_or("(無題)"),
                            "start": pick(start),
                            "end": pick(end),
                            "all_day": all_day,
                            "location": it.get("location").and_then(|v| v.as_str()),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(events)
    }

    /// Delete an agent-created event by its Google event ID.
    pub async fn delete_agent_event(&mut self, event_id: &str) -> Result<String, String> {
        if !self.is_authenticated() {
            return Err("Google Calendarにログインしていません。".into());
        }
        let meta = self
            .sync_state
            .agent_event_map
            .get(event_id)
            .cloned()
            .ok_or_else(|| format!("イベントID '{}' は見つかりません", event_id))?;
        let cal_id = self.sync_state.calendar_id.clone();
        if cal_id.is_empty() {
            return Err("カレンダーが作成されていません".into());
        }
        // Best-effort API delete; if already gone on Google side, still remove locally.
        let _ = self.delete_event(&cal_id, event_id).await;
        self.sync_state.agent_event_map.remove(event_id);
        save_sync_state(&self.sync_state)?;
        Ok(format!(
            "「{}」({} {}) を削除しました。",
            meta.title, meta.date, meta.start_time
        ))
    }

    /// Update an agent-created event. Only fields provided (Some) are changed.
    // Each parameter represents one independently-optional field on the calendar
    // event payload; bundling them into a struct just to satisfy clippy would
    // add ceremony without clarity.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_agent_event(
        &mut self,
        event_id: &str,
        title: Option<&str>,
        date: Option<&str>,
        start_time: Option<&str>,
        end_time: Option<&str>,
        location: Option<Option<&str>>,
        description: Option<Option<&str>>,
    ) -> Result<String, String> {
        if !self.is_authenticated() {
            return Err("Google Calendarにログインしていません。".into());
        }
        let meta = self
            .sync_state
            .agent_event_map
            .get(event_id)
            .cloned()
            .ok_or_else(|| format!("イベントID '{}' は見つかりません", event_id))?;
        let cal_id = self.sync_state.calendar_id.clone();
        if cal_id.is_empty() {
            return Err("カレンダーが作成されていません".into());
        }

        let new_title = title.unwrap_or(&meta.title);
        let new_date = date.unwrap_or(&meta.date);
        let new_start = start_time.unwrap_or(&meta.start_time);
        let new_end = end_time.unwrap_or(&meta.end_time);
        if !DATE_RE.is_match(new_date) {
            return Err(format!("日付フォーマットが不正です: {}", new_date));
        }
        if !TIME_RE.is_match(new_start) || !TIME_RE.is_match(new_end) {
            return Err("時刻フォーマットが不正です (HH:MM)".into());
        }
        let new_location: Option<String> = match location {
            Some(Some(v)) => Some(v.to_string()),
            Some(None) => None, // explicitly cleared
            None => meta.location.clone(),
        };
        let new_description: Option<String> = match description {
            Some(Some(v)) => Some(v.to_string()),
            Some(None) => None,
            None => meta.description.clone(),
        };

        let start_dt = format!("{}T{}:00", new_date, new_start);
        let end_dt = format!("{}T{}:00", new_date, new_end);
        let mut body = serde_json::json!({
            "summary": new_title,
            "start": { "dateTime": start_dt, "timeZone": "Asia/Tokyo" },
            "end":   { "dateTime": end_dt,   "timeZone": "Asia/Tokyo" },
        });
        if let Some(ref loc) = new_location {
            body["location"] = serde_json::Value::String(loc.clone());
        }
        if let Some(ref desc) = new_description {
            body["description"] = serde_json::Value::String(desc.clone());
        }
        self.update_event(&cal_id, event_id, &body).await?;

        // Update local metadata.
        let updated_meta = AgentEventMeta {
            title: new_title.to_string(),
            date: new_date.to_string(),
            start_time: new_start.to_string(),
            end_time: new_end.to_string(),
            location: new_location,
            description: new_description,
        };
        self.sync_state
            .agent_event_map
            .insert(event_id.to_string(), updated_meta);
        save_sync_state(&self.sync_state)?;
        Ok(format!(
            "「{}」を {} {} – {} に更新しました。",
            new_title, new_date, new_start, new_end
        ))
    }
}
