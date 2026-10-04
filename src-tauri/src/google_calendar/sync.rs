use std::sync::LazyLock;

use super::config::save_sync_state;
use super::{CalendarSyncEntry, SyncState, CALENDAR_SUMMARY, GCAL_API_BASE};

/// Parse week_label like "2026/03/30(月)～2026/04/05(日)" to get Monday's date
static WEEK_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(\d{4})/(\d{2})/(\d{2})").expect("valid hardcoded regex"));
pub(super) static DATE_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^\d{4}-\d{2}-\d{2}$").expect("valid hardcoded regex"));
pub(super) static TIME_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^\d{2}:\d{2}$").expect("valid hardcoded regex"));

fn parse_week_start(week_label: &str) -> Result<chrono::NaiveDate, String> {
    let re = &*WEEK_RE;
    let caps = re
        .captures(week_label)
        .ok_or_else(|| format!("週ラベルを解析できません: {}", week_label))?;
    let y: i32 = caps[1].parse().map_err(|e| format!("year: {}", e))?;
    let m: u32 = caps[2].parse().map_err(|e| format!("month: {}", e))?;
    let d: u32 = caps[3].parse().map_err(|e| format!("day: {}", e))?;
    chrono::NaiveDate::from_ymd_opt(y, m, d).ok_or_else(|| format!("無効な日付: {}/{}/{}", y, m, d))
}

fn day_offset(day: &str) -> i64 {
    match day {
        "月" => 0,
        "火" => 1,
        "水" => 2,
        "木" => 3,
        "金" => 4,
        "土" => 5,
        _ => 0,
    }
}

impl super::GoogleCalendarClient {
    /// Find or create the "Selah 時間割" calendar
    pub(super) async fn ensure_calendar(&mut self) -> Result<String, String> {
        if !self.sync_state.calendar_id.is_empty() {
            let token = self.ensure_token().await?;
            let resp = self
                .http
                .get(format!(
                    "{}/calendars/{}",
                    GCAL_API_BASE,
                    urlencoding::encode(&self.sync_state.calendar_id)
                ))
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|e| format!("カレンダー確認失敗: {}", e))?;
            if resp.status().is_success() {
                return Ok(self.sync_state.calendar_id.clone());
            }
            self.sync_state.calendar_id.clear();
            self.sync_state.event_map.clear();
        }

        let token = self.ensure_token().await?;
        let resp = self
            .http
            .get(format!("{}/users/me/calendarList", GCAL_API_BASE))
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| format!("カレンダー一覧取得失敗: {}", e))?;
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("カレンダー一覧レスポンス解析失敗: {}", e))?;
        if let Some(items) = body["items"].as_array() {
            for item in items {
                if item["summary"].as_str() == Some(CALENDAR_SUMMARY) {
                    if let Some(id) = item["id"].as_str() {
                        self.sync_state.calendar_id = id.to_string();
                        save_sync_state(&self.sync_state)?;
                        return Ok(id.to_string());
                    }
                }
            }
        }

        let token = self.ensure_token().await?;
        let resp = self
            .http
            .post(format!("{}/calendars", GCAL_API_BASE))
            .bearer_auth(&token)
            .json(&serde_json::json!({ "summary": CALENDAR_SUMMARY, "timeZone": "Asia/Tokyo" }))
            .send()
            .await
            .map_err(|e| format!("カレンダー作成失敗: {}", e))?;
        if !resp.status().is_success() {
            let err: serde_json::Value = resp.json().await.unwrap_or_default();
            return Err(format!("カレンダー作成失敗: {}", err));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("カレンダー作成レスポンス解析失敗: {}", e))?;
        let cal_id = body["id"]
            .as_str()
            .ok_or("カレンダーID取得失敗")?
            .to_string();
        self.sync_state.calendar_id = cal_id.clone();
        self.sync_state.event_map.clear();
        save_sync_state(&self.sync_state)?;
        log::info!("Created Google Calendar: {}", cal_id);
        Ok(cal_id)
    }

    /// Sync this week's timetable to Google Calendar.
    /// Parses week_label for the Monday date, creates one event per class per day.
    /// Cancelled classes are skipped. Stale events from this week are deleted.
    pub async fn sync_timetable(
        &mut self,
        entries: Vec<CalendarSyncEntry>,
        week_label: String,
    ) -> Result<String, String> {
        let monday = parse_week_start(&week_label)?;
        let cal_id = self.ensure_calendar().await?;

        // Build desired events: key = "YYYY-MM-DD-period"
        let mut desired: std::collections::HashMap<String, &CalendarSyncEntry> =
            std::collections::HashMap::new();
        for entry in &entries {
            if entry.is_cancelled {
                continue;
            }
            let date = monday + chrono::Duration::days(day_offset(&entry.day));
            let key = format!("{}-{}", date.format("%Y-%m-%d"), entry.period);
            desired.insert(key, entry);
        }

        // Keys belonging to this week (Mon..Sat)
        let week_prefixes: Vec<String> = (0..6)
            .map(|off| {
                (monday + chrono::Duration::days(off))
                    .format("%Y-%m-%d")
                    .to_string()
            })
            .collect();
        let is_this_week = |k: &str| week_prefixes.iter().any(|p| k.starts_with(p));

        // Delete stale events from this week
        let old_keys: Vec<String> = self
            .sync_state
            .event_map
            .keys()
            .filter(|k| is_this_week(k))
            .cloned()
            .collect();
        let mut deleted = 0usize;
        for key in &old_keys {
            if !desired.contains_key(key) {
                if let Some(event_id) = self.sync_state.event_map.remove(key) {
                    let _ = self.delete_event(&cal_id, &event_id).await;
                    deleted += 1;
                }
            }
        }

        // Create or update
        let mut created = 0usize;
        let mut updated = 0usize;
        for (key, entry) in &desired {
            let date_str = &key[..10];
            let times = crate::config::PERIOD_TIMES;
            let idx = (entry.period - 1).clamp(0, 6) as usize;
            let (sh, sm, eh, em) = times[idx];
            let start_dt = format!("{}T{:02}:{:02}:00", date_str, sh, sm);
            let end_dt = format!("{}T{:02}:{:02}:00", date_str, eh, em);

            let event_body = serde_json::json!({
                "summary": entry.course_name,
                "location": entry.room,
                "start": { "dateTime": start_dt, "timeZone": "Asia/Tokyo" },
                "end": { "dateTime": end_dt, "timeZone": "Asia/Tokyo" },
            });

            if let Some(existing_id) = self.sync_state.event_map.get(key).cloned() {
                match self.update_event(&cal_id, &existing_id, &event_body).await {
                    Ok(_) => {
                        updated += 1;
                    }
                    Err(_) => {
                        self.sync_state.event_map.remove(key);
                        if let Ok(id) = self.create_event(&cal_id, &event_body).await {
                            self.sync_state.event_map.insert(key.clone(), id);
                            created += 1;
                        }
                    }
                }
            } else if let Ok(id) = self.create_event(&cal_id, &event_body).await {
                self.sync_state.event_map.insert(key.clone(), id);
                created += 1;
            }
        }

        save_sync_state(&self.sync_state)?;
        let week_count = self
            .sync_state
            .event_map
            .keys()
            .filter(|k| is_this_week(k))
            .count();
        log::info!(
            "Google Calendar sync: created={}, updated={}, deleted={}",
            created,
            updated,
            deleted
        );
        Ok(format!(
            "Google Calendar: {}件同期 (新規{} / 更新{} / 削除{})",
            week_count, created, updated, deleted
        ))
    }

    pub(super) async fn create_event(
        &mut self,
        cal_id: &str,
        body: &serde_json::Value,
    ) -> Result<String, String> {
        let token = self.ensure_token().await?;
        let resp = self
            .http
            .post(format!(
                "{}/calendars/{}/events",
                GCAL_API_BASE,
                urlencoding::encode(cal_id)
            ))
            .bearer_auth(&token)
            .json(body)
            .send()
            .await
            .map_err(|e| format!("イベント作成失敗: {}", e))?;
        if !resp.status().is_success() {
            let err: serde_json::Value = resp.json().await.unwrap_or_default();
            return Err(format!("イベント作成失敗: {}", err));
        }
        let result: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("イベント作成レスポンス解析失敗: {}", e))?;
        Ok(result["id"].as_str().unwrap_or("").to_string())
    }

    pub(super) async fn update_event(
        &mut self,
        cal_id: &str,
        event_id: &str,
        body: &serde_json::Value,
    ) -> Result<(), String> {
        let token = self.ensure_token().await?;
        let resp = self
            .http
            .put(format!(
                "{}/calendars/{}/events/{}",
                GCAL_API_BASE,
                urlencoding::encode(cal_id),
                urlencoding::encode(event_id)
            ))
            .bearer_auth(&token)
            .json(body)
            .send()
            .await
            .map_err(|e| format!("イベント更新失敗: {}", e))?;
        if !resp.status().is_success() {
            let err: serde_json::Value = resp.json().await.unwrap_or_default();
            return Err(format!("イベント更新失敗: {}", err));
        }
        Ok(())
    }

    pub(super) async fn delete_event(
        &mut self,
        cal_id: &str,
        event_id: &str,
    ) -> Result<(), String> {
        let token = self.ensure_token().await?;
        let resp = self
            .http
            .delete(format!(
                "{}/calendars/{}/events/{}",
                GCAL_API_BASE,
                urlencoding::encode(cal_id),
                urlencoding::encode(event_id)
            ))
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| format!("イベント削除失敗: {}", e))?;
        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::GONE {
            let err: serde_json::Value = resp.json().await.unwrap_or_default();
            return Err(format!("イベント削除失敗: {}", err));
        }
        Ok(())
    }

    pub async fn clear_calendar(&mut self, delete_calendar: bool) -> Result<String, String> {
        let cal_id = self.sync_state.calendar_id.clone();
        if cal_id.is_empty() {
            return Ok("Google Calendarは未作成です".into());
        }
        if delete_calendar {
            let token = self.ensure_token().await?;
            let resp = self
                .http
                .delete(format!(
                    "{}/calendars/{}",
                    GCAL_API_BASE,
                    urlencoding::encode(&cal_id)
                ))
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|e| format!("カレンダー削除失敗: {}", e))?;
            if !resp.status().is_success() && resp.status() != reqwest::StatusCode::NOT_FOUND {
                let err: serde_json::Value = resp.json().await.unwrap_or_default();
                return Err(format!("カレンダー削除失敗: {}", err));
            }
            self.sync_state = SyncState::default();
            save_sync_state(&self.sync_state)?;
            Ok("Google Calendarを削除しました".into())
        } else {
            let event_ids: Vec<(String, String)> = self.sync_state.event_map.drain().collect();
            let mut deleted = 0;
            for (_, eid) in &event_ids {
                if self.delete_event(&cal_id, eid).await.is_ok() {
                    deleted += 1;
                }
            }
            save_sync_state(&self.sync_state)?;
            Ok(format!("{}件のイベントを削除しました", deleted))
        }
    }
}
