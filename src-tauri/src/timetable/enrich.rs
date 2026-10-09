use super::*;

use crate::client;
use crate::config;
use crate::db::{Database, KgcCourseDetailRow, LunaActivityRow, LunaCountsRow, SessionPlanRow};
use crate::luna_client;
use crate::luna_parser;
use crate::parser;
use crate::{KgcState, LunaState};
use futures_util::stream::{self, StreamExt};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::State;

/// Guard to prevent concurrent enrichment runs (Struts token conflicts).
static ENRICHMENT_RUNNING: AtomicBool = AtomicBool::new(false);

/// Background enrichment: fetch KGC syllabus pages for session plans + Luna counts.
/// The Struts gate is taken per course inside the syllabus fetch, not for the
/// whole run, so Luna counts and foreground KGC commands are not blocked on it.
#[tauri::command]
pub async fn enrich_schedule(
    state: State<'_, KgcState>,
    luna_state: State<'_, LunaState>,
    db: crate::db::AccountDb,
) -> Result<(), String> {
    if ENRICHMENT_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        log::info!("enrich_schedule: skipped (already running)");
        return Ok(());
    }
    struct RunningGuard;
    impl Drop for RunningGuard {
        fn drop(&mut self) {
            ENRICHMENT_RUNNING.store(false, Ordering::SeqCst);
        }
    }
    let _guard = RunningGuard;
    enrich_schedule_inner(&state, &luna_state, &db).await
}

const LUNA_COUNT_CONCURRENCY: usize = 3;

struct FetchedLunaCourse {
    luna_id: String,
    activities: Vec<LunaActivityRow>,
    counts: LunaCountsRow,
}

fn parse_luna_course_bundle(
    luna_id: &str,
    course_html: &str,
    contents_html: Option<&str>,
) -> FetchedLunaCourse {
    let course_data = luna_parser::parse_luna_course_contents(course_html, luna_id);
    let new_announcements = course_data
        .announcements
        .iter()
        .filter(|a| a.is_new)
        .count() as i32;
    let announcement_count = course_data.announcements.len() as i32;
    let mut activities: Vec<LunaActivityRow> = Vec::new();
    for ann in &course_data.announcements {
        activities.push(LunaActivityRow {
            luna_id: luna_id.to_string(),
            activity_type: "announcement".into(),
            title: ann.title.clone(),
            period: format!("{} ~ {}", ann.start_date, ann.end_date),
            status: if ann.is_new {
                "new".into()
            } else {
                "read".into()
            },
            detail_path: format!(
                "/lms/coursetop/information/listdetail?idnumber={}&informationId={}",
                luna_id, ann.info_id
            ),
        });
    }
    let (reports, exams, discussions) = if let Some(html) = contents_html {
        let (materials, reps, exs, discs, _surveys) = luna_parser::parse_luna_contents_page(html);
        for m in &materials {
            activities.push(LunaActivityRow {
                luna_id: luna_id.to_string(),
                activity_type: "material".into(),
                title: m.title.clone(),
                period: m.period.clone(),
                status: m.status.clone(),
                detail_path: m.url.clone(),
            });
        }
        for r in &reps {
            activities.push(LunaActivityRow {
                luna_id: luna_id.to_string(),
                activity_type: "report".into(),
                title: r.title.clone(),
                period: r.period.clone(),
                status: r.status.clone(),
                detail_path: r.url.clone(),
            });
        }
        for e in &exs {
            activities.push(LunaActivityRow {
                luna_id: luna_id.to_string(),
                activity_type: "exam".into(),
                title: e.title.clone(),
                period: e.period.clone(),
                status: e.status.clone(),
                detail_path: e.url.clone(),
            });
        }
        for d in &discs {
            activities.push(LunaActivityRow {
                luna_id: luna_id.to_string(),
                activity_type: "discussion".into(),
                title: d.title.clone(),
                period: d.period.clone(),
                status: d.status.clone(),
                detail_path: d.url.clone(),
            });
        }
        let pending_reports = reps.iter().filter(|r| r.status.contains("未提出")).count() as i32;
        let pending_exams = exs
            .iter()
            .filter(|e| e.status.contains("未回答") || e.status.contains("未受験"))
            .count() as i32;
        (pending_reports, pending_exams, discs.len() as i32)
    } else {
        (0, 0, 0)
    };
    FetchedLunaCourse {
        luna_id: luna_id.to_string(),
        activities,
        counts: LunaCountsRow {
            announcements: announcement_count,
            new_announcements,
            reports,
            exams,
            discussions,
        },
    }
}

async fn fetch_luna_course_snapshot(
    http: reqwest::Client,
    luna_id: String,
) -> Result<FetchedLunaCourse, String> {
    let course_url = format!("{}/lms/course?idnumber={}", config::LUNA_BASE, luna_id);
    let contents_url = format!("{}/lms/contents?idnumber={}", config::LUNA_BASE, luna_id);
    let course_html = client::fetch_with_redirect(
        &http,
        &course_url,
        config::LUNA_BASE,
        luna_client::LUNA_SESSION_EXPIRED_MSG,
        luna_client::is_luna_session_expired,
    )
    .await?;
    let contents_html = match client::fetch_with_redirect(
        &http,
        &contents_url,
        config::LUNA_BASE,
        luna_client::LUNA_SESSION_EXPIRED_MSG,
        luna_client::is_luna_session_expired,
    )
    .await
    {
        Ok(html) => Some(html),
        Err(e) => {
            if is_luna_auth_error(&e) {
                return Err(e);
            }
            log::warn!(
                "refresh_luna_counts: contents failed for {}: {}",
                luna_id,
                e
            );
            None
        }
    };
    let luna_id_for_parse = luna_id.clone();
    tokio::task::spawn_blocking(move || {
        parse_luna_course_bundle(&luna_id_for_parse, &course_html, contents_html.as_deref())
    })
    .await
    .map_err(|err| format!("luna parse task failed: {err}"))
}

/// Standalone Luna activity counts refresh (no KGC gate needed).
/// Only fetches counts for courses whose cached data is older than the DB threshold (3h).
#[tauri::command]
pub async fn refresh_luna_counts(
    state: State<'_, LunaState>,
    db: crate::db::AccountDb,
) -> Result<i32, String> {
    refresh_luna_counts_internal(&state, &db, false).await
}

/// Same as refresh_luna_counts but bypasses the 3-hour freshness threshold.
/// Used when the caller (e.g. agent) explicitly wants fresh data.
pub async fn refresh_luna_counts_internal(
    state: &LunaState,
    db: &Database,
    force: bool,
) -> Result<i32, String> {
    let luna_targets = if force {
        let courses = db.get_luna_courses().unwrap_or_default();
        let mut ids: Vec<String> = courses.into_iter().map(|c| c.luna_id).collect();
        ids.sort();
        ids.dedup();
        ids
    } else {
        db.luna_ids_needing_counts()?
    };
    if luna_targets.is_empty() {
        log::info!("refresh_luna_counts: all counts are fresh, skipping");
        return Ok(0);
    }

    let luna_http = {
        let luna = state.session();
        if !luna.has_credentials() {
            return Err("Luna not authenticated".into());
        }
        luna.http().clone()
    };

    log::info!(
        "refresh_luna_counts: {} courses need updates",
        luna_targets.len()
    );
    let target_count = luna_targets.len();
    let mut updated = 0i32;
    let mut in_flight = stream::iter(luna_targets.into_iter().filter(|id| !id.is_empty()).map(
        |luna_id| {
            let http = luna_http.clone();
            async move { fetch_luna_course_snapshot(http, luna_id).await }
        },
    ))
    .buffer_unordered(LUNA_COUNT_CONCURRENCY);

    while let Some(result) = in_flight.next().await {
        match result {
            Ok(item) => {
                if let Err(e) = db.replace_luna_activities(&item.luna_id, &item.activities) {
                    log::warn!(
                        "refresh_luna_counts: failed to save activities for {}: {}",
                        item.luna_id,
                        e
                    );
                }
                if let Err(e) = db.upsert_luna_counts(&item.luna_id, &item.counts) {
                    log::warn!(
                        "refresh_luna_counts: failed to save counts for {}: {}",
                        item.luna_id,
                        e
                    );
                }
                updated += 1;
            }
            Err(e) => {
                log::warn!("refresh_luna_counts: {}", e);
                if is_luna_auth_error(&e) {
                    break;
                }
            }
        }
    }

    log::info!(
        "refresh_luna_counts: updated {}/{} courses",
        updated,
        target_count
    );
    Ok(updated)
}

pub(super) async fn enrich_schedule_inner(
    kgc: &KgcState,
    luna: &LunaState,
    db: &Database,
) -> Result<(), String> {
    // Session plans from KGC syllabus pages (not timetable detail pages)
    let plan_targets = db.kgc_codes_needing_plans()?;
    log::info!("enrich_schedule: {} courses need plans", plan_targets.len());
    if !plan_targets.is_empty() && kgc.session().has_credentials() {
        let batch_results = batch_fetch_syllabi(kgc, &plan_targets).await;
        for (kgc_code, result) in batch_results {
            match result {
                Ok(detail_html) => {
                    let kgc_code_for_log = kgc_code.clone();
                    let parsed_bundle = tokio::task::spawn_blocking(move || {
                        let parsed = parser::parse_session_plans(&detail_html);
                        let detail = parser::parse_course_detail(&detail_html);
                        let delivery_mode = parser::detect_delivery_mode_from_detail(&detail_html);
                        let textbooks = parser::parse_textbooks(&detail_html);
                        (parsed, detail, delivery_mode, textbooks, detail_html.len())
                    })
                    .await;
                    let (parsed, detail, delivery_mode, textbooks, html_len) = match parsed_bundle {
                        Ok(bundle) => bundle,
                        Err(err) => {
                            log::warn!(
                                "enrich_schedule: {} parse task failed: {}",
                                kgc_code_for_log,
                                err
                            );
                            continue;
                        }
                    };
                    log::info!(
                        "enrich_schedule: {} parsed {} plans from syllabus ({} bytes)",
                        kgc_code,
                        parsed.len(),
                        html_len
                    );
                    if parsed.is_empty() {
                        log::warn!("enrich_schedule: {} - syllabus fetched but 0 plans parsed (no 第N回 rows?)",
                            kgc_code);
                    } else {
                        for p in parsed.iter().take(3) {
                            log::debug!(
                                "  plan #{}: header={:?}, dm={:?}, topic={:.60}",
                                p.session_num,
                                p.th_header,
                                p.delivery_mode,
                                p.topic
                            );
                        }
                        let rows: Vec<SessionPlanRow> = parsed
                            .iter()
                            .map(|p| SessionPlanRow {
                                session_num: p.session_num,
                                th_header: p.th_header.clone(),
                                topic: p.topic.clone(),
                                delivery_mode: p.delivery_mode.clone(),
                                study_outside: p.study_outside.clone(),
                            })
                            .collect();
                        if let Err(e) = db.upsert_session_plans(&kgc_code, &rows) {
                            log::warn!("Failed to save plans for {}: {}", kgc_code, e);
                        }
                    }

                    let detail_row = KgcCourseDetailRow {
                        kgc_code: kgc_code.clone(),
                        fields: detail.fields,
                        delivery_mode,
                        textbooks,
                    };
                    if let Err(e) = db.upsert_kgc_course_detail(&detail_row) {
                        log::warn!("Failed to save detail for {}: {}", kgc_code, e);
                    }
                }
                Err(e) => log::warn!("enrich_schedule: {} syllabus fetch failed: {}", kgc_code, e),
            }
        }
    }

    // Luna counts do not touch the KGC token. Shared with the standalone command
    // so a schedule sync and a later counts refresh cannot scan the same pages twice
    // in one process without the 3-hour freshness check seeing the first write.
    if let Err(e) = refresh_luna_counts_internal(luna, db, false).await {
        log::warn!("enrich_schedule: luna counts failed: {}", e);
    }

    Ok(())
}
