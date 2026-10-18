use super::*;

use crate::db::SnapshotState;
use crate::LunaState;
use tauri::State;

struct WidgetSnapshotGuard<'a>(&'a crate::db::Database);

impl Drop for WidgetSnapshotGuard<'_> {
    fn drop(&mut self) {
        crate::widget_bridge::publish(self.0);
    }
}

/// Refresh the primary Luna timetable and activity data. KGC is never contacted
/// here; its optional supplement runs separately after the primary is published.
#[tauri::command]
pub async fn sync_schedule_data(
    app: tauri::AppHandle,
    luna_state: State<'_, LunaState>,
    db: crate::db::AccountDb,
) -> Result<ScheduleResponse, String> {
    let _widget_snapshot = WidgetSnapshotGuard(db.inner());
    let response = refresh_primary_schedule(&luna_state, &db).await?;
    super::enrich::spawn_luna_counts(&app, &db, false);
    spawn_kgc_supplement(&app, &db, true);
    Ok(response)
}

async fn refresh_primary_schedule(
    luna_state: &LunaState,
    db: &crate::db::Database,
) -> Result<ScheduleResponse, String> {
    let luna = luna_state.session();
    if !luna.has_credentials() {
        return Err(crate::luna_client::LUNA_AUTH_REQUIRED_MSG.into());
    }
    refresh_primary_schedule_with_http(luna.http(), db, crate::config::LUNA_BASE).await
}

async fn refresh_primary_schedule_with_http(
    http: &reqwest::Client,
    db: &crate::db::Database,
    base_url: &str,
) -> Result<ScheduleResponse, String> {
    let previous = db.get_snapshot_state()?.unwrap_or_default();
    // KGC is an optional source. Retire warnings persisted by older versions.
    let _ = db.delete_data_cache("schedule_kgc_warning");

    let (mut current_week_label, mut next_week_label) = primary_week_labels(
        &previous.current_week_label,
        &previous.next_week_label,
        chrono::Local::now().date_naive(),
    );

    let scope = crate::academic_period::visible_weeks(
        &current_week_label,
        &next_week_label,
        &previous.luna_year,
        &previous.luna_term,
        chrono::Local::now().date_naive(),
    );
    current_week_label = scope.current.clone();
    next_week_label = scope.next.clone();
    let (display_year, display_term) = if scope.from_calendar {
        (scope.year.clone(), scope.term.clone())
    } else {
        (String::new(), String::new())
    };

    let luna = sync_luna_timetable(http, db, &previous, base_url).await?;

    let (save_year, save_term) = if scope.from_calendar {
        (display_year.clone(), display_term.clone())
    } else {
        (luna.year.clone(), luna.term.clone())
    };
    let snap = SnapshotState {
        current_week_label: current_week_label.clone(),
        next_week_label: next_week_label.clone(),
        luna_year: save_year,
        luna_term: save_term,
        luna_communities: luna.communities.clone(),
        luna_year_options: luna.year_opts.clone(),
        luna_term_options: luna.term_opts.clone(),
        updated_at: 0,
    };
    db.save_snapshot_state(&snap)?;
    // Use the persisted timestamp, shared by later secondary cache pushes.
    super::snapshot::build_schedule_snapshot_from_db(db)
}

/// Luna has a weekly timetable but no KGC week labels. Derive both weeks from
/// the calendar, reusing a cached label only when it describes that same week.
pub(super) fn primary_week_labels(
    current: &str,
    next: &str,
    today: chrono::NaiveDate,
) -> (String, String) {
    use chrono::{Datelike, Duration};
    let monday = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
    let label = |start: chrono::NaiveDate| {
        for cached in [current, next] {
            if crate::academic_period::week_label_range(cached)
                .is_some_and(|(cached_start, _)| cached_start == start)
            {
                return cached.to_string();
            }
        }
        format!(
            "{}～{}",
            start.format("%Y年%m月%d日"),
            (start + Duration::days(5)).format("%Y年%m月%d日")
        )
    };
    (label(monday), label(monday + Duration::days(7)))
}

#[cfg(test)]
#[path = "sync/tests.rs"]
mod tests;
