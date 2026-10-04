use super::*;

use crate::client;
use crate::config;
use crate::db::epoch_secs;
use crate::db::{Database, SnapshotState};
use crate::parser;
use crate::{KgcState, LunaState};
use tauri::State;

struct WidgetSnapshotGuard<'a>(&'a crate::db::Database);

impl Drop for WidgetSnapshotGuard<'_> {
    fn drop(&mut self) {
        crate::widget_bridge::publish(self.0);
    }
}

/// Serial data sync: KGC current, KGC next, Luna, then enrichment.
/// KGC being down or logged out must not fail the command. Luna still refreshes,
/// and a login-required error is not returned unless Luna itself needs recovery.
#[tauri::command]
pub async fn sync_schedule_data(
    kgc: State<'_, KgcState>,
    luna_state: State<'_, LunaState>,
    db: State<'_, Database>,
) -> Result<ScheduleResponse, String> {
    let _widget_snapshot = WidgetSnapshotGuard(db.inner());
    // Logged-out KGC skips the Struts gate so Luna can refresh immediately.
    let previous = db.get_snapshot_state()?.unwrap_or_default();

    let mut kgc_warning = String::new();
    let mut current_week_label = previous.current_week_label.clone();
    let mut next_week_label = previous.next_week_label.clone();
    let mut kgc_fetched = false;

    let kgc_authenticated = kgc.client.lock().await.is_authenticated();
    if !kgc_authenticated {
        log::warn!("sync_schedule_data: KGC not authenticated; continuing with Luna");
        kgc_warning = KGC_UNAVAILABLE_WARNING.to_string();
    } else {
        // Struts 1 keeps one token per session, so KGC fetches stay serial.
        let _kgc_gate = kgc.gate.lock().await;
        let http = {
            let client = kgc.client.lock().await;
            if client.is_authenticated() {
                Some(client.http.clone())
            } else {
                None
            }
        };
        if let Some(http) = http.as_ref() {
            let kgc_url = format!(
                "{}/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014",
                config::KG_COURSE_BASE
            );
            match client::fetch_page_with(http, &kgc_url).await {
                Ok(html) => {
                    let kgc_data = parser::parse_timetable(&html);
                    let label = kgc_data.week_label.clone();
                    log::info!(
                        "sync_schedule_data: parsed KGC: {} entries, week_label='{}'",
                        kgc_data.entries.len(),
                        label
                    );
                    if kgc_data.entries.is_empty() && label.is_empty() {
                        log::warn!("sync_schedule_data: KGC returned empty page");
                        kgc_warning = KGC_UNAVAILABLE_WARNING.to_string();
                    } else {
                        for entry in &kgc_data.entries {
                            let day_int = day_str_to_int(&entry.day);
                            if day_int == 0 {
                                continue;
                            }
                            db.upsert_kgc_course(
                                &entry.course_code,
                                &entry.course_name,
                                day_int,
                                entry.period,
                                &entry.room,
                                &entry.detail_path,
                                entry.is_cancelled,
                                entry.is_makeup,
                                entry.is_room_changed,
                                &label,
                            )?;
                        }
                        current_week_label = label;
                        kgc_fetched = true;
                        match fetch_next_week_kgc(http, &kgc_data, &db).await {
                            Ok(next_label) => {
                                if !next_label.is_empty() {
                                    next_week_label = next_label;
                                }
                            }
                            Err(error) => {
                                clear_kgc_if_expired(&kgc, &error).await;
                                log::warn!("sync_schedule_data: next week failed: {}", error);
                                kgc_warning = KGC_NEXT_WEEK_WARNING.to_string();
                            }
                        }
                        log::info!("sync_schedule_data: next_week_label='{}'", next_week_label);
                    }
                }
                Err(error) => {
                    clear_kgc_if_expired(&kgc, &error).await;
                    log::warn!("sync_schedule_data: KGC current week failed: {}", error);
                    kgc_warning = KGC_UNAVAILABLE_WARNING.to_string();
                }
            }
        } else {
            log::warn!("sync_schedule_data: KGC logged out before fetch; continuing with Luna");
            kgc_warning = KGC_UNAVAILABLE_WARNING.to_string();
        }
    }

    let scope = crate::academic_period::visible_weeks(
        &current_week_label,
        &next_week_label,
        &previous.luna_year,
        &previous.luna_term,
        chrono::Local::now().date_naive(),
    );
    if scope.hid_current {
        kgc_warning = STALE_SEMESTER_KGC_WARNING.to_string();
    }
    current_week_label = scope.current.clone();
    next_week_label = scope.next.clone();
    let (display_year, display_term) = if scope.from_calendar {
        (scope.year.clone(), scope.term.clone())
    } else {
        (String::new(), String::new())
    };

    let luna = match sync_luna_timetable(&luna_state, &db, &previous).await {
        Ok(fields) => fields,
        Err(error) => {
            store_kgc_warning(&db, &kgc_warning);
            let semester_rolled = scope.from_calendar
                && (scope.year != previous.luna_year || scope.term != previous.luna_term);
            if kgc_fetched || scope.hid_current || semester_rolled {
                let (year, term) = if scope.from_calendar {
                    (display_year.clone(), display_term.clone())
                } else {
                    (previous.luna_year.clone(), previous.luna_term.clone())
                };
                let snap = SnapshotState {
                    current_week_label: current_week_label.clone(),
                    next_week_label: next_week_label.clone(),
                    luna_year: year,
                    luna_term: term,
                    luna_communities: previous.luna_communities.clone(),
                    luna_year_options: previous.luna_year_options.clone(),
                    luna_term_options: previous.luna_term_options.clone(),
                    updated_at: 0,
                };
                db.save_snapshot_state(&snap)?;
            }
            return Err(error);
        }
    };

    let (save_year, save_term) = if scope.from_calendar {
        (display_year.clone(), display_term.clone())
    } else {
        (luna.year.clone(), luna.term.clone())
    };
    let labels_changed = current_week_label != previous.current_week_label
        || next_week_label != previous.next_week_label;
    let term_changed = save_year != previous.luna_year || save_term != previous.luna_term;
    let should_save =
        kgc_fetched || luna.replaced || luna.metadata_changed || labels_changed || term_changed;
    if should_save {
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
    }
    store_kgc_warning(&db, &kgc_warning);

    if kgc_fetched {
        if let Err(e) = enrich_schedule_inner(&kgc, &luna_state, &db).await {
            clear_kgc_if_expired(&kgc, &e).await;
            log::warn!("sync_schedule_data: enrichment failed: {}", e);
        }
    }

    let response_year = if scope.from_calendar {
        display_year
    } else {
        luna.year.clone()
    };
    let response_term = if scope.from_calendar {
        display_term
    } else {
        luna.term.clone()
    };
    let mut communities = luna.communities.clone();
    retain_current_communities(&mut communities, &response_year, &response_term);
    let raw = db.build_raw_data(&current_week_label, &next_week_label, communities.clone())?;
    let semester_changed =
        response_year != previous.luna_year || response_term != previous.luna_term;
    if raw.kgc_entries_current.is_empty()
        && raw.kgc_entries_next.is_empty()
        && raw.luna_courses.is_empty()
        && current_week_label.is_empty()
        && !semester_changed
    {
        return Err("時間割を取得できませんでした。Luna の接続を確認してください。".into());
    }
    if semester_changed && raw.luna_courses.is_empty() && current_week_label.is_empty() {
        kgc_warning = STALE_SEMESTER_KGC_WARNING.to_string();
        store_kgc_warning(&db, &kgc_warning);
    }
    log::info!(
        "sync_schedule_data: done — kgc_current={}, kgc_next={}, luna={}, plans={}, counts={}",
        raw.kgc_entries_current.len(),
        raw.kgc_entries_next.len(),
        raw.luna_courses.len(),
        raw.session_plans.len(),
        raw.luna_counts.len()
    );
    let (ai_result, ai_stale) = load_ai_cache(&db)?;
    Ok(ScheduleResponse {
        raw,
        ai_result,
        ai_stale,
        snapshot_updated_at: if should_save {
            epoch_secs()
        } else {
            previous.updated_at
        },
        luna_communities: communities,
        luna_year_options: luna.year_opts,
        luna_term_options: luna.term_opts,
        luna_year: response_year,
        luna_term: response_term,
        kgc_warning,
    })
}

const SCHEDULE_KGC_WARNING_KEY: &str = "schedule_kgc_warning";
const KGC_UNAVAILABLE_WARNING: &str =
    "KGC に接続できません。Luna と保存済みの時間割を表示しています。";
const KGC_NEXT_WEEK_WARNING: &str =
    "KGC の来週の時間割を取得できませんでした。保存済みのデータを表示しています。";

pub(super) const STALE_SEMESTER_KGC_WARNING: &str =
    "前学期の時間割は表示していません。KGC から今学期の時間割を取得できていません。";

pub(super) fn load_kgc_warning(db: &Database) -> String {
    db.get_data_cache(SCHEDULE_KGC_WARNING_KEY)
        .ok()
        .flatten()
        .map(|(text, _)| text)
        .unwrap_or_default()
}

fn store_kgc_warning(db: &Database, warning: &str) {
    if warning.is_empty() {
        let _ = db.delete_data_cache(SCHEDULE_KGC_WARNING_KEY);
    } else {
        let _ = db.save_data_cache(SCHEDULE_KGC_WARNING_KEY, warning);
    }
}
