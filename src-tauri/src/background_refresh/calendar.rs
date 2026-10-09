//! Google Calendar auto-sync from the background refresh loop.

use crate::db::Database;

use super::*;

fn read_numeric_cache(db: &Database, key: &str) -> Option<i64> {
    db.get_data_cache(key)
        .ok()
        .flatten()
        .and_then(|(json, _)| serde_json::from_str::<i64>(&json).ok())
}

fn save_numeric_cache(db: &Database, key: &str, value: i64) {
    if let Ok(json) = serde_json::to_string(&value) {
        let _ = db.save_data_cache(key, &json);
    }
}

fn gcal_sync_interval_secs() -> i64 {
    let cfg = crate::commands::load_calendar_config();
    let hours = cfg
        .cal_sync_interval
        .clamp(GCAL_SYNC_MIN_HOURS, GCAL_SYNC_MAX_HOURS);
    let hours = if hours == 0 {
        GCAL_SYNC_DEFAULT_HOURS
    } else {
        hours
    };
    i64::from(hours) * 60 * 60
}

fn build_calendar_entries(
    entries: &[crate::db::KgcCourseRow],
) -> Vec<crate::google_calendar::CalendarSyncEntry> {
    entries
        .iter()
        .map(|entry| crate::google_calendar::CalendarSyncEntry {
            day: match entry.day {
                1 => "月",
                2 => "火",
                3 => "水",
                4 => "木",
                5 => "金",
                6 => "土",
                _ => "",
            }
            .to_string(),
            period: entry.period,
            course_name: entry.name.clone(),
            room: entry.room.clone(),
            is_cancelled: entry.is_cancelled,
        })
        .filter(|entry| !entry.day.is_empty())
        .collect()
}

fn build_sync_weeks(
    raw: &crate::db::ScheduleRawData,
) -> Vec<(String, Vec<crate::google_calendar::CalendarSyncEntry>)> {
    let candidates = [
        (&raw.current_week_label, &raw.kgc_entries_current),
        (&raw.next_week_label, &raw.kgc_entries_next),
    ];
    let mut seen = BTreeSet::new();
    let mut weeks = Vec::new();

    for (label, entries) in candidates {
        let label = label.trim();
        if label.is_empty() || entries.is_empty() || !seen.insert(label.to_string()) {
            continue;
        }
        weeks.push((label.to_string(), build_calendar_entries(entries)));
    }

    weeks
}

pub(super) async fn maybe_auto_sync_calendars(
    app: &AppHandle,
    db: &Database,
    schedule_changed: bool,
    force: bool,
) {
    let account = crate::db::capture_account();
    let cal_cfg = crate::commands::load_calendar_config();
    if !cal_cfg.gcal_auto_sync {
        return;
    }

    let last_run = read_numeric_cache(db, GCAL_AUTO_SYNC_LAST_RUN_KEY).unwrap_or(0);
    let due = epoch_secs().saturating_sub(last_run) >= gcal_sync_interval_secs();
    if !force && !schedule_changed && !due {
        return;
    }

    let Some(snapshot) = db.get_snapshot_state().ok().flatten() else {
        return;
    };

    let scope = crate::academic_period::visible_weeks(
        &snapshot.current_week_label,
        &snapshot.next_week_label,
        &snapshot.luna_year,
        &snapshot.luna_term,
        chrono::Local::now().date_naive(),
    );
    let raw = match db.build_raw_data(
        &scope.current,
        &scope.next,
        snapshot.luna_communities.clone(),
    ) {
        Ok(raw) => raw,
        Err(e) => {
            log::warn!(
                "background refresh: build raw schedule for gcal sync failed: {}",
                e
            );
            return;
        }
    };
    let weeks = build_sync_weeks(&raw);
    if weeks.is_empty() {
        return;
    }

    let gcal_state = app.state::<crate::GCalState>();
    let mut gcal = gcal_state.client.lock().await;
    if db.ensure_current_account().is_err() || !gcal.auto_sync_allowed(&account) {
        return;
    }

    for (label, entries) in weeks {
        if entries.is_empty() {
            continue;
        }
        if let Err(e) = gcal
            .sync_timetable_automatically(entries, label, &account)
            .await
        {
            log::warn!("background refresh: gcal auto-sync failed: {}", e);
            return;
        }
    }

    drop(gcal);
    save_numeric_cache(db, GCAL_AUTO_SYNC_LAST_RUN_KEY, epoch_secs());
}
