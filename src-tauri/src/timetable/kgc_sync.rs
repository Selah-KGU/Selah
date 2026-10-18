//! Optional KGC enrichment. The primary Luna refresh never awaits this task.
use super::*;
use crate::db::Database;
use crate::{client, config, parser, KgcState, LunaState};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;

static RUNNING: AtomicBool = AtomicBool::new(false);

pub(crate) fn spawn_kgc_supplement(app: &tauri::AppHandle, db: &Database, force: bool) {
    let lease =
        crate::session_coordinator::SESSIONS.lease(crate::session_coordinator::Service::Kgc);
    // Never probe, log in, or wait for KGC just to supplement Luna's timetable.
    if !supplement_due(db, force)
        || !lease.is_verified()
        || !lease.has_credentials()
        || RUNNING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
    {
        return;
    }
    let app = app.clone();
    let db = db.clone();
    let account = crate::db::capture_account();
    tauri::async_runtime::spawn(async move {
        struct RunningGuard;
        impl Drop for RunningGuard {
            fn drop(&mut self) {
                RUNNING.store(false, Ordering::SeqCst);
            }
        }
        let _running = RunningGuard;
        let result = crate::db::account_work(account, refresh(&app, &db)).await;
        if let Err(error) = result {
            log::info!("KGC timetable supplement skipped: {error}");
        }
    });
}

fn supplement_due(db: &Database, force: bool) -> bool {
    let Some(snapshot) = db.get_snapshot_state().ok().flatten() else {
        return false;
    };
    let weeks = format!(
        "{}|{}",
        snapshot.current_week_label, snapshot.next_week_label
    );
    force
        || db.cache_payload("schedule_kgc_supplement").as_deref() != Some(weeks.as_str())
        || db
            .cache_updated_at("schedule_kgc_supplement")
            .is_none_or(|updated| crate::db::epoch_secs().saturating_sub(updated) >= 1800)
}

async fn refresh(app: &tauri::AppHandle, db: &Database) -> Result<(), String> {
    let Some(kgc) = app.try_state::<KgcState>() else {
        return Ok(());
    };
    let Ok(gate) = kgc.gate.try_lock() else {
        return Ok(());
    };
    let lease = kgc.session();
    if !lease.is_verified() || !lease.has_credentials() {
        return Ok(());
    }
    let Some(snapshot) = db.get_snapshot_state()? else {
        return Ok(());
    };
    let url = format!(
        "{}/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014",
        config::KG_COURSE_BASE
    );
    let result = async {
        let html = client::fetch_page_with(lease.http(), &url).await?;
        let current = parser::parse_timetable(&html);
        if current.entries.is_empty() && current.week_label.is_empty() {
            return Err("KGC returned no timetable".to_string());
        }
        let mut changed = save_matching_week(db, &current, &snapshot)?;
        match fetch_next_week_kgc(lease.http(), &current).await {
            Ok(Some(next)) => changed |= save_matching_week(db, &next, &snapshot)?,
            Ok(None) => {}
            Err(error) => {
                clear_kgc_if_expired(&lease, &error).await;
                log::info!("KGC next-week supplement skipped: {error}");
            }
        }
        Ok(changed)
    }
    .await;
    drop(gate);
    let changed = match result {
        Ok(changed) => changed,
        Err(error) => {
            clear_kgc_if_expired(&lease, &error).await;
            return Err(error);
        }
    };
    if changed {
        db.save_data_cache(
            "schedule_kgc_supplement",
            &format!(
                "{}|{}",
                snapshot.current_week_label, snapshot.next_week_label
            ),
        )?;
        publish(app, db)?;
        if kgc.session().is_verified() {
            if let Some(luna) = app.try_state::<LunaState>() {
                if let Err(error) = enrich_schedule_inner(&kgc, &luna, db).await {
                    log::info!("KGC syllabus supplement skipped: {error}");
                }
                publish(app, db)?;
            }
        }
    }
    Ok(())
}

fn publish(app: &tauri::AppHandle, db: &Database) -> Result<(), String> {
    db.ensure_current_account()?;
    crate::background_refresh::emit_cache_updates(app, vec!["schedule_data".into()]);
    crate::widget_bridge::publish(db);
    Ok(())
}

fn save_matching_week(
    db: &Database,
    data: &parser::TimetableData,
    snapshot: &crate::db::SnapshotState,
) -> Result<bool, String> {
    let Some((start, _)) = crate::academic_period::week_label_range(&data.week_label) else {
        return Ok(false);
    };
    // Store secondary rows under Luna's existing week label. A stale KGC page
    // cannot change Luna's displayed week or its last successful refresh time.
    let label = [&snapshot.current_week_label, &snapshot.next_week_label]
        .into_iter()
        .find(|label| {
            crate::academic_period::week_label_range(label)
                .is_some_and(|(primary_start, _)| primary_start == start)
        });
    let Some(label) = label else {
        return Ok(false);
    };
    let entries: Vec<_> = data
        .entries
        .iter()
        .filter_map(|entry| {
            let day = day_str_to_int(&entry.day);
            (day > 0 && (1..=7).contains(&entry.period)).then(|| crate::db::KgcCourseRow {
                id: 0,
                kgc_code: entry.course_code.clone(),
                name: entry.course_name.clone(),
                day,
                period: entry.period,
                room: entry.room.clone(),
                detail_path: entry.detail_path.clone(),
                is_cancelled: entry.is_cancelled,
                is_makeup: entry.is_makeup,
                is_room_changed: entry.is_room_changed,
                week_label: label.to_string(),
            })
        })
        .collect();
    db.replace_kgc_week(label, &entries)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secondary_rows_use_primary_weeks_without_overwriting_primary_metadata() {
        let directory =
            std::env::temp_dir().join(format!("selah-secondary-{}", uuid::Uuid::new_v4()));
        let db = Database::open(&directory).unwrap();
        db.save_snapshot_state(&crate::db::SnapshotState {
            current_week_label: "2026年10月19日～2026年10月24日".into(),
            next_week_label: "2026年10月26日～2026年10月31日".into(),
            luna_year: "2026".into(),
            luna_term: "03".into(),
            ..Default::default()
        })
        .unwrap();
        let primary = db.get_snapshot_state().unwrap().unwrap();
        assert!(supplement_due(&db, false));
        let mut data = parser::parse_timetable("");
        data.week_label = "2026/10/19～2026/10/24".into();
        data.entries = vec![parser::TimetableEntry {
            day: "月".into(),
            period: 1,
            course_name: "Supplement".into(),
            room: "A101".into(),
            course_code: "code".into(),
            is_cancelled: false,
            is_makeup: false,
            is_room_changed: false,
            detail_path: String::new(),
        }];
        assert!(save_matching_week(&db, &data, &primary).unwrap());
        assert_eq!(
            db.get_kgc_courses(&primary.current_week_label).unwrap()[0].room,
            "A101"
        );
        db.save_data_cache(
            "schedule_kgc_supplement",
            &format!("{}|{}", primary.current_week_label, primary.next_week_label),
        )
        .unwrap();
        assert!(!supplement_due(&db, false));
        assert!(supplement_due(&db, true));
        data.entries.clear();
        assert!(save_matching_week(&db, &data, &primary).unwrap());
        assert!(db
            .get_kgc_courses(&primary.current_week_label)
            .unwrap()
            .is_empty());
        data.week_label = "2026/09/07～2026/09/12".into();
        assert!(!save_matching_week(&db, &data, &primary).unwrap());
        assert!(db.get_kgc_courses(&data.week_label).unwrap().is_empty());
        assert_eq!(
            serde_json::to_value(db.get_snapshot_state().unwrap().unwrap()).unwrap(),
            serde_json::to_value(primary).unwrap()
        );
        drop(db);
        let _ = std::fs::remove_dir_all(directory);
    }
}
