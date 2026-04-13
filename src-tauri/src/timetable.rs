// timetable.rs — AI-driven schedule: fetch KGC + Luna raw data, enrich, then AI analysis.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::State;

use crate::ai;
use crate::client;
use crate::commands;
use crate::config;
use crate::db::{
    AiScheduleItem, AiScheduleResult, Database, KgcCourseDetailRow,
    LunaActivityRow, LunaCountsRow, ScheduleRawData, SessionPlanRow,
    SnapshotState,
};
use crate::db::epoch_secs;
use crate::luna_client;
use crate::luna_parser;
use crate::parser;
use crate::AppState;

const AI_CACHE_MAX_AGE: i64 = 12 * 3600; // 12 hours

/// Guard to prevent concurrent enrichment runs (Struts token conflicts).
static ENRICHMENT_RUNNING: AtomicBool = AtomicBool::new(false);

/// Char-boundary-safe string preview for logging/error messages.
fn safe_preview(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// KGC day letter -> integer (1=Mon .. 6=Sat)
fn day_str_to_int(d: &str) -> i32 {
    match d {
        "月" => 1, "火" => 2, "水" => 3, "木" => 4, "金" => 5, "土" => 6,
        _ => 0,
    }
}

fn day_int_to_str(d: i32) -> &'static str {
    if (1..=6).contains(&d) { config::DAY_SHORT[d as usize] } else { "?" }
}

/// Response type: raw data + optional cached AI result.
#[derive(Debug, Clone, Serialize)]
pub struct ScheduleResponse {
    pub raw: ScheduleRawData,
    pub ai_result: Option<AiScheduleResult>,
    pub ai_stale: bool,
    pub snapshot_updated_at: i64,
    pub luna_communities: Vec<luna_parser::LunaCommunity>,
    pub luna_year_options: Vec<luna_parser::SelectOption>,
    pub luna_term_options: Vec<luna_parser::SelectOption>,
    pub luna_year: String,
    pub luna_term: String,
}

// ── Commands ──

/// Load schedule from DB snapshot only (no network). Fast, used on page mount.
#[tauri::command]
pub async fn get_schedule_snapshot(
    db: State<'_, Database>,
) -> Result<ScheduleResponse, String> {
    let snap = db.get_snapshot_state()?.unwrap_or_default();
    let raw = db.build_raw_data(
        &snap.current_week_label,
        &snap.next_week_label,
        snap.luna_communities.clone(),
    )?;
    let (ai_result, ai_stale) = load_ai_cache(&db)?;
    Ok(ScheduleResponse {
        raw,
        ai_result,
        ai_stale,
        snapshot_updated_at: snap.updated_at,
        luna_communities: snap.luna_communities,
        luna_year_options: snap.luna_year_options,
        luna_term_options: snap.luna_term_options,
        luna_year: snap.luna_year,
        luna_term: snap.luna_term,
    })
}

/// Serial data sync: KGC current → KGC next → Luna → enrichment → persist all.
/// User-triggered from timetable page. Avoids parallel requests that break login state.
#[tauri::command]
pub async fn sync_schedule_data(
    state: State<'_, AppState>,
    db: State<'_, Database>,
) -> Result<ScheduleResponse, String> {
    // Serialize all KGC requests — Struts 1 stores one token per session; any
    // concurrent KGC page load (background polling) invalidates pending tokens.
    let _kgc_gate = state.kgc_gate.lock().await;

    // ── Step 1: KGC current week (serial) ──
    let kgc_http = {
        let client = state.client.lock().await;
        if !client.is_authenticated() {
            return Err(config::KGC_AUTH_REQUIRED_MSG.into());
        }
        client.http.clone()
    };

    let kgc_url = format!(
        "{}/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014",
        config::KG_COURSE_BASE
    );
    let kgc_html = client::fetch_page_with(&kgc_http, &kgc_url).await?;
    let kgc_data = parser::parse_timetable(&kgc_html);

    let current_week_label = kgc_data.week_label.clone();
    log::info!(
        "sync_schedule_data: parsed KGC: {} entries, week_label='{}'",
        kgc_data.entries.len(), current_week_label
    );

    // Guard: empty KGC page — return DB snapshot as-is
    if kgc_data.entries.is_empty() && current_week_label.is_empty() {
        log::warn!("sync_schedule_data: KGC returned empty page");
        return get_schedule_snapshot(db).await;
    }

    // Store KGC current-week entries
    for entry in &kgc_data.entries {
        let day_int = day_str_to_int(&entry.day);
        if day_int == 0 { continue; }
        db.upsert_kgc_course(
            &entry.course_code, &entry.course_name, day_int, entry.period,
            &entry.room, &entry.detail_path,
            entry.is_cancelled, entry.is_makeup, entry.is_room_changed,
            &current_week_label,
        )?;
    }

    // ── Step 2: KGC next week (serial, reuses same HTTP client) ──
    let next_week_label = fetch_next_week_kgc(&kgc_http, &kgc_data, &db).await?;
    log::info!("sync_schedule_data: next_week_label='{}'", next_week_label);

    // ── Step 3: Luna timetable (serial, after KGC) ──
    let (communities, year_opts, term_opts, year, term) = {
        let luna_http = {
            let luna = state.luna.lock().await;
            if luna.authenticated { Some(luna.http.clone()) } else { None }
        };
        if let Some(http) = luna_http {
            let url = format!("{}/lms/timetable", config::LUNA_BASE);
            match client::fetch_with_redirect(
                &http, &url, config::LUNA_BASE,
                luna_client::LUNA_SESSION_EXPIRED_MSG,
                luna_client::is_luna_session_expired,
            ).await {
                Ok(html) => {
                    let l = luna_parser::parse_luna_timetable(&html);
                    log::info!("sync_schedule_data: Luna: {} courses, {} communities",
                        l.courses.len(), l.communities.len());
                    for course in &l.courses {
                        db.upsert_luna_course(
                            &course.idnumber, &course.name, &course.teacher,
                            course.day as i32, course.period as i32,
                        )?;
                    }
                    (l.communities, l.year_options, l.term_options, l.year, l.term)
                }
                Err(e) => {
                    log::warn!("sync_schedule_data: Luna fetch failed: {}", e);
                    (Vec::new(), Vec::new(), Vec::new(), String::new(), String::new())
                }
            }
        } else {
            log::info!("sync_schedule_data: Luna not authenticated");
            (Vec::new(), Vec::new(), Vec::new(), String::new(), String::new())
        }
    };

    // ── Step 4: Persist snapshot state ──
    let snap = SnapshotState {
        current_week_label: current_week_label.clone(),
        next_week_label: next_week_label.clone(),
        luna_year: year,
        luna_term: term,
        luna_communities: communities.clone(),
        luna_year_options: year_opts,
        luna_term_options: term_opts,
        updated_at: 0, // filled by save_snapshot_state
    };
    db.save_snapshot_state(&snap)?;

    // ── Step 5: Enrichment (serial — KGC syllabus details, then Luna counts) ──
    if let Err(e) = enrich_schedule_inner(&state, &db).await {
        log::warn!("sync_schedule_data: enrichment failed: {}", e);
    }

    // ── Step 6: Build final response from DB ──
    let raw = db.build_raw_data(&current_week_label, &next_week_label, communities)?;
    log::info!(
        "sync_schedule_data: done — kgc_current={}, kgc_next={}, luna={}, plans={}, counts={}",
        raw.kgc_entries_current.len(), raw.kgc_entries_next.len(),
        raw.luna_courses.len(), raw.session_plans.len(), raw.luna_counts.len()
    );
    let (ai_result, ai_stale) = load_ai_cache(&db)?;

    Ok(ScheduleResponse {
        raw,
        ai_result,
        ai_stale,
        snapshot_updated_at: epoch_secs(),
        luna_communities: snap.luna_communities,
        luna_year_options: snap.luna_year_options,
        luna_term_options: snap.luna_term_options,
        luna_year: snap.luna_year,
        luna_term: snap.luna_term,
    })
}

/// Syllabus search URL — enters the syllabus system through SSO.
const SYLLABUS_SSO_URL: &str =
    "/uniasv2/UnSSOLoginControl2?REQ_LOGIN_NO=2&REQ_ACTION_DO=/AGA030.do&REQ_PRFR_MNU_ID=MNUIDSTD0103011";

/// Batch-fetch syllabus detail pages for multiple class codes.
///
/// Enters the syllabus system ONCE via SSO, then searches each code sequentially.
/// For each code we try spring term first (02), then fall (03), then year-long (01).
/// Returns `(class_code, Ok(detail_html))` or `(class_code, Err(reason))`.
async fn batch_fetch_syllabi(
    http: &reqwest::Client,
    codes: &[String],
) -> Vec<(String, Result<String, String>)> {
    let mut results = Vec::new();
    let terms = ["02", "03", "01", "04", "05"];

    for code in codes {
        let mut found = false;
        for term_code in &terms {
            // Get a fresh search form (each search POST consumes the Struts token)
            let search_html = match commands::kgc_get(http, SYLLABUS_SSO_URL).await {
                Ok(h) => h,
                Err(e) => {
                    log::warn!("batch_fetch_syllabi: {} GET search page failed: {}", code, e);
                    break; // session broken, skip remaining terms for this code
                }
            };
            let token = match commands::extract_struts_token(&search_html) {
                Ok(t) => t,
                Err(_) => continue, // try next term
            };
            let year = commands::extract_year_from_search_page(&search_html)
                .unwrap_or_else(|| "2026".into());

            // POST search for this class_code + term
            let search_params = vec![
                ("org.apache.struts.taglib.html.TOKEN".into(), token),
                ("selTypeCalLsnOpcFcy".into(), "0".into()),
                ("txtLsnOpcFcy".into(), year.clone()),
                ("selTypeCalLsnEndFcy".into(), "0".into()),
                ("txtLsnEndFcy".into(), year),
                ("selTacTrmCd".into(), term_code.to_string()),
                ("selOpcCmpsCd".into(), String::new()),
                ("selLsnMngPostCd".into(), String::new()),
                ("txtLsnCd_01".into(), code.to_string()),
                ("txtLsnCd_02".into(), String::new()),
                ("selTmtxCd".into(), String::new()),
                ("txtSlbSrchKwd".into(), String::new()),
                ("selVolCd1".into(), String::new()),
                ("txtTchKnjfn_01".into(), String::new()),
                ("txtTchKnafn_01".into(), String::new()),
                ("txtCbbTchRnmAlpfn_01".into(), String::new()),
                ("hdnClassisyUser".into(), "S".into()),
                ("hdnEsearch".into(), "true".into()),
                ("hdnPhfyPrcFlg".into(), String::new()),
                ("ESearch".into(), "検索/Search".into()),
                ("hdnLoginUrl".into(), String::new()),
            ];

            let results_html = match commands::kgc_post(
                http, "/uniasv2/AGA030PSC01EventAction.do", &search_params,
            ).await {
                Ok(h) => h,
                Err(e) => {
                    log::warn!("batch_fetch_syllabi: {} POST search failed: {}", code, e);
                    continue;
                }
            };

            if !results_html.contains("結果一覧画面") {
                continue; // no results for this term
            }

            let parsed = match crate::syllabus::parse_search_results_public(&results_html) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let target = match parsed.entries.iter().find(|e| e.class_code == code.as_str()) {
                Some(t) => t,
                None => continue,
            };
            let refer_index = target.refer_index.clone();
            log::info!(
                "batch_fetch_syllabi: {} found in term {}, refer_index='{}', total_results={}, course_title='{}'",
                code, term_code, refer_index, parsed.entries.len(), target.course_title
            );

            // Guard: empty refer_index means the hidden input wasn't in the row HTML
            // (likely set by JavaScript onclick). Use positional index as fallback.
            let effective_refer_index = if refer_index.is_empty() {
                let pos = parsed.entries.iter()
                    .position(|e| e.class_code == code.as_str())
                    .unwrap_or(0);
                log::warn!(
                    "batch_fetch_syllabi: {} ereferIndex is empty, using positional fallback: {}",
                    code, pos
                );
                pos.to_string()
            } else {
                refer_index.clone()
            };

            log::info!("batch_fetch_syllabi: {} ereferIndex={}", code, effective_refer_index);

            // Navigate to syllabus detail page.
            // Extract inputs ONLY from the results list form (AGA030PLS01Form),
            // not from the search form which shares the page and has conflicting params.
            let mut form_params = commands::extract_named_form_inputs(&results_html, "AGA030PLS01Form");

            // Log diagnostic info about extracted params
            let token_count = form_params.iter()
                .filter(|(k, _)| k == "org.apache.struts.taglib.html.TOKEN")
                .count();
            log::info!(
                "batch_fetch_syllabi: {} extracted {} form params (AGA030PLS01Form), {} Struts tokens",
                code, form_params.len(), token_count
            );

            // With targeted form extraction, token dedup is unnecessary
            // (only one form's token is extracted).

            form_params.retain(|(k, _)| {
                !k.starts_with("ESearch") && !k.starts_with("ENarrowSearch")
                && !k.starts_with("EBack") && !k.starts_with("ENext")
                && !k.starts_with("EPrev") && !k.starts_with("ERefer")
                && !k.starts_with("ERegister") && !k.starts_with("EPageSet")
                && k != "hdnEsearch"
            });
            form_params.retain(|(k, _)| k != "ereferIndex");
            form_params.push(("ereferIndex".into(), effective_refer_index.clone()));
            form_params.push(("ERefer.x".into(), "10".into()));
            form_params.push(("ERefer.y".into(), "10".into()));

            // Dump params for debugging
            log::debug!("batch_fetch_syllabi: {} POST params: {:?}",
                code,
                form_params.iter().map(|(k, v)| {
                    if v.len() > 60 { format!("{}={}...", k, &v[..60]) }
                    else { format!("{}={}", k, v) }
                }).collect::<Vec<_>>()
            );

            match commands::kgc_post(http, "/uniasv2/AGA030PLS01EventAction.do", &form_params).await {
                Ok(detail_html) => {
                    if !detail_html.contains("AGA030PVI01Form") {
                        log::warn!(
                            "batch_fetch_syllabi: {} detail POST did not reach detail page (term {}), {} bytes",
                            code, term_code, detail_html.len()
                        );
                        #[cfg(debug_assertions)]
                        { let _ = std::fs::write(
                            std::env::temp_dir().join(format!("kwic_detail_fail_{}.html", code)),
                            &detail_html
                        ); }
                        continue;
                    }
                    log::info!("batch_fetch_syllabi: {} -> {} bytes (term {})",
                        code, detail_html.len(), term_code);

                    results.push((code.clone(), Ok(detail_html)));
                    found = true;
                    break;
                }
                Err(e) => {
                    log::warn!("batch_fetch_syllabi: {} POST detail failed (term {}): {}", code, term_code, e);
                    continue;
                }
            }
        }

        if !found && !results.iter().any(|(c, _)| c == code) {
            results.push((code.clone(), Err(format!("科目コード {} が見つかりません", code))));
        }

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    results
}

/// Background enrichment: fetch KGC syllabus pages for session plans + Luna counts.
#[tauri::command]
pub async fn enrich_schedule(
    state: State<'_, AppState>,
    db: State<'_, Database>,
) -> Result<(), String> {
    // Prevent concurrent runs — Struts tokens conflict when two enrichments hit KGC simultaneously
    if ENRICHMENT_RUNNING.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        log::info!("enrich_schedule: skipped (already running)");
        return Ok(());
    }
    let _kgc_gate = state.kgc_gate.lock().await;
    let result = enrich_schedule_inner(&state, &db).await;
    ENRICHMENT_RUNNING.store(false, Ordering::SeqCst);
    result
}

/// Standalone Luna activity counts refresh (no KGC gate needed).
/// Only fetches counts for courses whose cached data is older than the DB threshold (3h).
#[tauri::command]
pub async fn refresh_luna_counts(
    state: State<'_, AppState>,
    db: State<'_, Database>,
) -> Result<i32, String> {
    let luna_targets = db.luna_ids_needing_counts()?;
    if luna_targets.is_empty() {
        log::info!("refresh_luna_counts: all counts are fresh, skipping");
        return Ok(0);
    }

    let luna_http = {
        let luna = state.luna.lock().await;
        if !luna.authenticated {
            return Err("Luna not authenticated".into());
        }
        luna.http.clone()
    };

    log::info!("refresh_luna_counts: {} courses need updates", luna_targets.len());
    let mut updated = 0i32;

    for luna_id in &luna_targets {
        if luna_id.is_empty() { continue; }
        let course_url = format!("{}/lms/course?idnumber={}", config::LUNA_BASE, luna_id);
        let contents_url = format!("{}/lms/contents?idnumber={}", config::LUNA_BASE, luna_id);

        let course_html = match client::fetch_with_redirect(
            &luna_http, &course_url, config::LUNA_BASE,
            luna_client::LUNA_SESSION_EXPIRED_MSG,
            luna_client::is_luna_session_expired,
        ).await {
            Ok(h) => h,
            Err(e) => { log::warn!("refresh_luna_counts: course page failed for {}: {}", luna_id, e); continue; }
        };

        let course_data = luna_parser::parse_luna_course_contents(&course_html, luna_id);
        let new_announcements = course_data.announcements.iter().filter(|a| a.is_new).count() as i32;
        let announcement_count = course_data.announcements.len() as i32;

        let mut activities: Vec<LunaActivityRow> = Vec::new();
        for ann in &course_data.announcements {
            activities.push(LunaActivityRow {
                luna_id: luna_id.clone(),
                activity_type: "announcement".into(),
                title: ann.title.clone(),
                period: format!("{} ~ {}", ann.start_date, ann.end_date),
                status: if ann.is_new { "new".into() } else { "read".into() },
            });
        }

        let (reports, exams, discussions) = match client::fetch_with_redirect(
            &luna_http, &contents_url, config::LUNA_BASE,
            luna_client::LUNA_SESSION_EXPIRED_MSG,
            luna_client::is_luna_session_expired,
        ).await {
            Ok(html) => {
                let (materials, reps, exs, discs) = luna_parser::parse_luna_contents_page(&html);
                for m in &materials {
                    activities.push(LunaActivityRow {
                        luna_id: luna_id.clone(),
                        activity_type: "material".into(),
                        title: m.title.clone(),
                        period: m.period.clone(),
                        status: m.status.clone(),
                    });
                }
                for r in &reps {
                    activities.push(LunaActivityRow {
                        luna_id: luna_id.clone(),
                        activity_type: "report".into(),
                        title: r.title.clone(),
                        period: r.period.clone(),
                        status: r.status.clone(),
                    });
                }
                for e in &exs {
                    activities.push(LunaActivityRow {
                        luna_id: luna_id.clone(),
                        activity_type: "exam".into(),
                        title: e.title.clone(),
                        period: e.period.clone(),
                        status: e.status.clone(),
                    });
                }
                for d in &discs {
                    activities.push(LunaActivityRow {
                        luna_id: luna_id.clone(),
                        activity_type: "discussion".into(),
                        title: d.title.clone(),
                        period: d.period.clone(),
                        status: d.status.clone(),
                    });
                }

                let pending_reports = reps.iter().filter(|r| r.status.contains("未提出")).count() as i32;
                let pending_exams = exs.iter().filter(|e| {
                    e.status.contains("未回答") || e.status.contains("未受験")
                }).count() as i32;
                (pending_reports, pending_exams, discs.len() as i32)
            }
            Err(e) => { log::warn!("refresh_luna_counts: contents failed for {}: {}", luna_id, e); (0, 0, 0) }
        };

        if let Err(e) = db.replace_luna_activities(luna_id, &activities) {
            log::warn!("refresh_luna_counts: failed to save activities for {}: {}", luna_id, e);
        }

        let counts = LunaCountsRow {
            announcements: announcement_count,
            new_announcements,
            reports,
            exams,
            discussions,
        };
        if let Err(e) = db.upsert_luna_counts(luna_id, &counts) {
            log::warn!("refresh_luna_counts: failed to save counts for {}: {}", luna_id, e);
        }

        updated += 1;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    log::info!("refresh_luna_counts: updated {}/{} courses", updated, luna_targets.len());
    Ok(updated)
}

async fn enrich_schedule_inner(
    state: &AppState,
    db: &Database,
) -> Result<(), String> {
    // Session plans from KGC syllabus pages (not timetable detail pages)
    let plan_targets = db.kgc_codes_needing_plans()?;
    log::info!("enrich_schedule: {} courses need plans", plan_targets.len());
    if !plan_targets.is_empty() {
        let kgc_http = {
            let client = state.client.lock().await;
            if !client.is_authenticated() { return Ok(()); }
            client.http.clone()
        };

        let batch_results = batch_fetch_syllabi(&kgc_http, &plan_targets).await;
        for (kgc_code, result) in batch_results {
            match result {
                Ok(detail_html) => {
                    // Parse session plans from the real syllabus page
                    let parsed = parser::parse_session_plans(&detail_html);
                    log::info!("enrich_schedule: {} parsed {} plans from syllabus ({} bytes)",
                        kgc_code, parsed.len(), detail_html.len());
                    if parsed.is_empty() {
                        log::warn!("enrich_schedule: {} - syllabus fetched but 0 plans parsed (no 第N回 rows?)",
                            kgc_code);
                    } else {
                        for p in parsed.iter().take(3) {
                            log::debug!("  plan #{}: header={:?}, dm={:?}, topic={:.60}",
                                p.session_num, p.th_header, p.delivery_mode, p.topic);
                        }
                        let rows: Vec<SessionPlanRow> = parsed.iter().map(|p| SessionPlanRow {
                            session_num: p.session_num,
                            th_header: p.th_header.clone(),
                            topic: p.topic.clone(),
                            delivery_mode: p.delivery_mode.clone(),
                            study_outside: p.study_outside.clone(),
                        }).collect();
                        if let Err(e) = db.upsert_session_plans(&kgc_code, &rows) {
                            log::warn!("Failed to save plans for {}: {}", kgc_code, e);
                        }
                    }

                    // Parse full course detail fields + delivery mode from syllabus
                    let detail = parser::parse_course_detail(&detail_html);
                    let delivery_mode = parser::detect_delivery_mode_from_detail(&detail_html);
                    let detail_row = KgcCourseDetailRow {
                        kgc_code: kgc_code.clone(),
                        fields: detail.fields,
                        delivery_mode,
                    };
                    if let Err(e) = db.upsert_kgc_course_detail(&detail_row) {
                        log::warn!("Failed to save detail for {}: {}", kgc_code, e);
                    }
                }
                Err(e) => log::warn!("enrich_schedule: {} syllabus fetch failed: {}", kgc_code, e),
            }
        }
    }

    // Luna activity counts
    let luna_targets = db.luna_ids_needing_counts()?;
    if !luna_targets.is_empty() {
        let luna_http = {
            let luna = state.luna.lock().await;
            if !luna.authenticated { return Ok(()); }
            luna.http.clone()
        };

        for luna_id in luna_targets {
            if luna_id.is_empty() { continue; }
            let course_url = format!("{}/lms/course?idnumber={}", config::LUNA_BASE, luna_id);
            let contents_url = format!("{}/lms/contents?idnumber={}", config::LUNA_BASE, luna_id);

            let course_html = match client::fetch_with_redirect(
                &luna_http, &course_url, config::LUNA_BASE,
                luna_client::LUNA_SESSION_EXPIRED_MSG,
                luna_client::is_luna_session_expired,
            ).await {
                Ok(h) => h,
                Err(e) => { log::warn!("Luna course page failed for {}: {}", luna_id, e); continue; }
            };

            let course_data = luna_parser::parse_luna_course_contents(&course_html, &luna_id);
            let new_announcements = course_data.announcements.iter().filter(|a| a.is_new).count() as i32;
            let announcement_count = course_data.announcements.len() as i32;

            // Collect detailed activity items for AI prompt
            let mut activities: Vec<LunaActivityRow> = Vec::new();

            // Announcements
            for ann in &course_data.announcements {
                activities.push(LunaActivityRow {
                    luna_id: luna_id.clone(),
                    activity_type: "announcement".into(),
                    title: ann.title.clone(),
                    period: format!("{} ~ {}", ann.start_date, ann.end_date),
                    status: if ann.is_new { "new".into() } else { "read".into() },
                });
            }

            let (reports, exams, discussions) = match client::fetch_with_redirect(
                &luna_http, &contents_url, config::LUNA_BASE,
                luna_client::LUNA_SESSION_EXPIRED_MSG,
                luna_client::is_luna_session_expired,
            ).await {
                Ok(html) => {
                    let (materials, reps, exs, discs) = luna_parser::parse_luna_contents_page(&html);

                    // Store material items
                    for m in &materials {
                        activities.push(LunaActivityRow {
                            luna_id: luna_id.clone(),
                            activity_type: "material".into(),
                            title: m.title.clone(),
                            period: m.period.clone(),
                            status: m.status.clone(),
                        });
                    }

                    // Store detailed report items
                    for r in &reps {
                        activities.push(LunaActivityRow {
                            luna_id: luna_id.clone(),
                            activity_type: "report".into(),
                            title: r.title.clone(),
                            period: r.period.clone(),
                            status: r.status.clone(),
                        });
                    }

                    // Store detailed exam items
                    for e in &exs {
                        activities.push(LunaActivityRow {
                            luna_id: luna_id.clone(),
                            activity_type: "exam".into(),
                            title: e.title.clone(),
                            period: e.period.clone(),
                            status: e.status.clone(),
                        });
                    }

                    // Store detailed discussion items
                    for d in &discs {
                        activities.push(LunaActivityRow {
                            luna_id: luna_id.clone(),
                            activity_type: "discussion".into(),
                            title: d.title.clone(),
                            period: d.period.clone(),
                            status: d.status.clone(),
                        });
                    }

                    let pending_reports = reps.iter().filter(|r| r.status.contains("未提出")).count() as i32;
                    let pending_exams = exs.iter().filter(|e| {
                        e.status.contains("未回答") || e.status.contains("未受験")
                    }).count() as i32;
                    (pending_reports, pending_exams, discs.len() as i32)
                }
                Err(e) => { log::warn!("Luna contents failed for {}: {}", luna_id, e); (0, 0, 0) }
            };

            // Save detailed activities
            if let Err(e) = db.replace_luna_activities(&luna_id, &activities) {
                log::warn!("Failed to save luna activities for {}: {}", luna_id, e);
            }

            let counts = LunaCountsRow {
                announcements: announcement_count,
                new_announcements,
                reports,
                exams,
                discussions,
            };
            if let Err(e) = db.upsert_luna_counts(&luna_id, &counts) {
                log::warn!("Failed to save luna counts for {}: {}", luna_id, e);
            }

            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    Ok(())
}

/// AI analysis: generate structured two-week schedule from raw DB data.
#[tauri::command]
pub async fn ai_generate_schedule(
    db: State<'_, Database>,
    current_week_label: String,
    next_week_label: String,
    force: bool,
) -> Result<AiScheduleResult, String> {
    // Check cache first
    if !force {
        if let Some((cached, _)) = load_ai_cache_inner(&db)? {
            if cached.current_week_label == current_week_label {
                return Ok(cached);
            }
        }
    }

    let raw = db.build_raw_data(&current_week_label, &next_week_label, Vec::new())?;
    let config = ai::load_ai_config();

    if config.api_key.is_empty() {
        return Err("APIキーが設定されていません。設定画面でAPIキーを入力してください。".into());
    }

    let prompt = build_ai_schedule_prompt(&raw, &config);
    let lang_hint = match config.reply_language.as_str() {
        "zh" => "\n\n重要: 所有文本字段用中文（简体字）写。科目名・日付保持原数据不变。",
        "en" => "\n\nIMPORTANT: Write all text fields in English. Keep course names and dates as-is.",
        "ko" => "\n\n중요: 모든 텍스트 필드를 한국어로 작성. 과목명・날짜는 원본 그대로.",
        _ => "",
    };
    log::info!("ai_generate_schedule: calling AI with {} chars prompt", prompt.len());
    log::debug!("ai_generate_schedule: full prompt:\n{}", prompt);
    let sys = if lang_hint.is_empty() {
        SCHEDULE_SYSTEM_PROMPT.to_string()
    } else {
        format!("{}{}", SCHEDULE_SYSTEM_PROMPT, lang_hint)
    };
    let messages = vec![
        ai::ChatMessage { role: "system".into(), content: sys },
        ai::ChatMessage { role: "user".into(), content: prompt },
    ];

    let response = ai::chat_completion_public(&config, messages).await?;
    log::info!("ai_generate_schedule: got response ({} chars)", response.len());
    log::debug!("ai_generate_schedule: response preview: {}", safe_preview(&response, 500));
    let result = parse_ai_schedule_response(&response, &current_week_label, &next_week_label)?;
    log::info!(
        "ai_generate_schedule: parsed OK — current_week={} items, next_week={} items",
        result.current_week.len(), result.next_week.len()
    );

    db.save_ai_schedule_cache(&result)?;
    Ok(result)
}

// ── Todo AI Analysis ──

const TODO_AI_CACHE_KEY: &str = "ai_todo_analysis";
const TODO_AI_CACHE_MAX_AGE: i64 = 6 * 3600; // 6 hours

const TODO_SYSTEM_PROMPT: &str = r#"あなたは関西学院大学の学生専属の学習コンサルタントAIです。
学生が今抱えている未提出課題・テスト・ディスカッション等のタスクと、それに紐づくコースの授業計画（シラバス）・教材・過去の授業内容を受け取り、**本当に役立つ具体的な学習支援**を行ってください。

## あなたの役割
1. **課題の本質を理解する**: 授業計画・シラバスから、その課題が「何を求めているか」「どの授業回の内容に対応するか」を特定し、学生に伝える
2. **必要な知識を整理する**: その課題に取り組むために必要な前提知識・概念・理論を、授業内容から推測して簡潔にまとめる
3. **具体的な行動手順を示す**: 「調べましょう」「頑張りましょう」のような曖昧な助言ではなく、「第N回の〇〇の概念を復習→△△の観点でアウトラインを作成→□□に注意して執筆」のように、実際に手を動かせるステップを示す
4. **時間配分を最適化する**: 3日間の計画で、各タスクの所要時間・優先度・授業スケジュール（空きコマ）を考慮した現実的な作業スケジュールを組む

## 出力JSON形式（他のテキストは一切不要）
{
  "task_guides": [
    {
      "task_name": "課題名（提出物のタイトルをそのまま使用）",
      "course_name": "科目名",
      "deadline": "YYYY/MM/DD HH:MM",
      "urgency": "overdue|critical|soon|normal",
      "background": "この課題の文脈説明。「第N回で扱った〇〇（具体的な概念名）に関連する課題です。△△の理論/手法/知識が前提となります。教材『□□』の内容を参照すると理解が深まります。」のように、授業計画・教材タイトル・シラバスの内容を根拠にした具体的な記述を2-4文で書く。",
      "study_hints": [
        "第5回の講義スライドでXXの定義を確認する",
        "XXとYYの関係を表にまとめる",
        "序論でXXを定義し、本論でYYの事例を3つ挙げて分析する"
      ],
      "estimated_minutes": 120
    }
  ],
  "daily_plan": [
    {
      "label": "今日（M/D）|明日（M/D）|明後日（M/D）",
      "tasks": ["（task_guidesのtask_nameと完全一致）（N分・緊急）", "（同上）（N分）"],
      "free_hours": 4.0
    }
  ],
  "advice": "句点区切りの3〜5文。作業負荷の全体像と戦略的なアドバイス。"
}

## urgency判定基準
- overdue: 締切が既に過ぎている
- critical: 締切まで24時間以内
- soon: 締切まで3日以内
- normal: それ以外

## 品質基準
- backgroundは**必ず授業計画・教材・シラバスの具体的な情報を引用**すること。汎用的な文章は禁止
- study_hintsは**その課題固有の手順**を書くこと。どの課題にも使い回せる汎用ステップは禁止
  - 良い例: 「第3回で扱ったマーケティングミックス（4P）のフレームワークを使って分析する」
  - 悪い例: 「関連資料を調べて要点をまとめる」
- 課題タイプ別の重点:
  - レポート/課題 → テーマの特定・必要な理論の整理・構成案・執筆・推敲の手順
  - テスト/小テスト → 出題範囲の特定・重要概念リストアップ・理解度チェック方法
  - ディスカッション → 議題の背景理解・多角的な視点の整理・投稿文の構成
- study_hintsの各項目に「ステップN:」「第N步:」「まず」「次に」などの序数・接続詞を付けない（UIが自動で番号を付与する）
- daily_planのlabelには日付を含める（例: 「今日（4/12）」）
- daily_plan.tasksの各文字列は「{task_guidesのtask_nameと完全一致}（N分）」の形式にする。task_nameと一致しないと詳細が表示されないため厳守
- free_hoursは時間割の授業時間を除いた学習可能時間（9:00-22:00の範囲で概算）
- 期限切れタスクは最優先で今日の計画に入れる
- estimated_minutesは課題の複雑さと授業レベルを考慮した現実的な見積もり
- adviceは「。」区切りで循環表示されるため、各文が独立して意味をなすようにする
- 回答はJSONのみ。マークダウンのコードブロック(```)は使わない
- 回答は指定された言語で書くこと"#;

#[tauri::command]
pub async fn ai_analyze_todo(
    db: State<'_, Database>,
    force: bool,
) -> Result<serde_json::Value, String> {
    // Check cache first
    if !force {
        if let Ok(Some((json, ts))) = db.get_data_cache(TODO_AI_CACHE_KEY) {
            let now = crate::db::epoch_secs();
            if now - ts < TODO_AI_CACHE_MAX_AGE {
                if let Ok(cached) = serde_json::from_str::<serde_json::Value>(&json) {
                    log::info!("ai_analyze_todo: returning cached result (age={}s)", now - ts);
                    return Ok(cached);
                }
            }
        }
    }

    let config = ai::load_ai_config();
    if config.api_key.is_empty() {
        return Err("APIキーが設定されていません。設定画面でAPIキーを入力してください。".into());
    }

    // Gather todo items from cache
    let todo_items: Vec<crate::luna_parser::LunaTodoItem> = db
        .get_data_cache("luna_todo")
        .ok()
        .flatten()
        .and_then(|(json, _)| serde_json::from_str(&json).ok())
        .unwrap_or_default();

    if todo_items.is_empty() {
        return Err("TODO項目がありません。先にTODOリストを読み込んでください。".into());
    }

    // Gather enrichment data from DB
    let snap = db.get_snapshot_state()?.unwrap_or_default();
    let raw = db.build_raw_data(
        &snap.current_week_label,
        &snap.next_week_label,
        Vec::new(),
    )?;

    let prompt = build_todo_ai_prompt(&todo_items, &raw, &config);
    log::info!("ai_analyze_todo: calling AI with {} chars prompt, {} todo items", prompt.len(), todo_items.len());
    log::debug!("ai_analyze_todo: full prompt:\n{}", prompt);

    let lang_hint = match config.reply_language.as_str() {
        "zh" => "\n\n重要: background, study_hints, advice, daily_plan.label, daily_plan.tasks 等所有文本用中文（简体字）写。task_name・course_name・deadline保持原数据不变。",
        "en" => "\n\nIMPORTANT: Write background, study_hints, advice, daily_plan.label, daily_plan.tasks in English. Keep task_name, course_name, deadline as-is from source data.",
        "ko" => "\n\n중요: background, study_hints, advice, daily_plan.label, daily_plan.tasks 등 모든 텍스트를 한국어로 작성. task_name・course_name・deadline은 원본 데이터 그대로.",
        _ => "",
    };
    let sys = if lang_hint.is_empty() {
        TODO_SYSTEM_PROMPT.to_string()
    } else {
        format!("{}{}", TODO_SYSTEM_PROMPT, lang_hint)
    };
    let messages = vec![
        ai::ChatMessage { role: "system".into(), content: sys },
        ai::ChatMessage { role: "user".into(), content: prompt },
    ];

    let response = ai::chat_completion_public(&config, messages).await?;
    log::info!("ai_analyze_todo: got response ({} chars)", response.len());

    let json_str = extract_json_from_response(&response);
    let result: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| format!("AI応答のJSON解析に失敗: {} — 応答: {}", e, safe_preview(&response, 200)))?;

    // Cache the result
    let cache_json = serde_json::to_string(&result).unwrap_or_default();
    let _ = db.save_data_cache(TODO_AI_CACHE_KEY, &cache_json);

    Ok(result)
}

fn build_todo_ai_prompt(
    todos: &[crate::luna_parser::LunaTodoItem],
    raw: &ScheduleRawData,
    config: &crate::ai::AiConfig,
) -> String {
    let mut text = String::new();

    // Today's date and day of week
    let today = chrono::Local::now();
    let today_date = today.date_naive();
    text.push_str(&format!("## 今日: {} ({})\n", today.format("%Y年%m月%d日"), today.format("%A")));

    // Semester week info
    let mut current_week: i32 = 4; // default fallback
    if !config.spring_start.is_empty() {
        if let Ok(spring) = chrono::NaiveDate::parse_from_str(&config.spring_start, "%Y-%m-%d") {
            let days_since = (today_date - spring).num_days();
            if days_since >= 0 && days_since < 150 {
                let week = (days_since / 7 + 1) as i32;
                current_week = week;
                text.push_str(&format!("春学期 第{}週目（全15週）\n", week));
            }
        }
    }
    if !config.fall_start.is_empty() {
        if let Ok(fall) = chrono::NaiveDate::parse_from_str(&config.fall_start, "%Y-%m-%d") {
            let days_since = (today_date - fall).num_days();
            if days_since >= 0 && days_since < 150 {
                let week = (days_since / 7 + 1) as i32;
                current_week = week;
                text.push_str(&format!("秋学期 第{}週目（全15週）\n", week));
            }
        }
    }

    // ── Pending TODO items with full detail ──
    text.push_str("\n## 未提出タスク一覧\n");
    let pending: Vec<&crate::luna_parser::LunaTodoItem> = todos.iter()
        .filter(|t| !t.status.contains("提出済"))
        .collect();

    for item in &pending {
        // Calculate urgency for context
        let urgency_hint = if !item.deadline.is_empty() {
            if let Ok(dl) = chrono::NaiveDateTime::parse_from_str(&item.deadline, "%Y-%m-%d %H:%M") {
                let diff = dl.signed_duration_since(today.naive_local());
                let hours = diff.num_hours();
                if hours < 0 { "【期限超過】" }
                else if hours < 24 { "【24h以内】" }
                else if hours < 72 { "【3日以内】" }
                else { "" }
            } else { "" }
        } else { "" };

        text.push_str(&format!(
            "- {}{} [{}] | 科目: {} | 締切: {} | 状態: {}\n",
            urgency_hint,
            item.content_name, item.content_type, item.course_name,
            if item.deadline.is_empty() { "未設定" } else { &item.deadline },
            item.status,
        ));
        if !item.feedback.is_empty() {
            text.push_str(&format!("  教員フィードバック: {}\n", item.feedback));
        }
    }

    // ── This week timetable (for daily_plan scheduling) ──
    if !raw.kgc_entries_current.is_empty() {
        text.push_str(&format!("\n## 今週の時間割 ({})\n", raw.current_week_label));
        for e in &raw.kgc_entries_current {
            let status = if e.is_cancelled { " [休講]" } else if e.is_makeup { " [補講]" } else { "" };
            text.push_str(&format!(
                "- {}曜{}限: {}{}\n", day_int_to_str(e.day), e.period, e.name, status
            ));
        }
    }

    // ── Luna activity details for EVERY pending course ──
    let pending_course_names: std::collections::HashSet<&str> = pending.iter()
        .map(|t| t.course_name.as_str())
        .collect();

    if !raw.luna_activities.is_empty() {
        let luna_id_to_name: std::collections::HashMap<&str, &str> = raw.luna_courses.iter()
            .map(|c| (c.luna_id.as_str(), c.name.as_str()))
            .collect();

        let mut grouped: std::collections::HashMap<&str, Vec<&crate::db::LunaActivityRow>> = Default::default();
        for a in &raw.luna_activities {
            grouped.entry(a.luna_id.as_str()).or_default().push(a);
        }

        text.push_str("\n## コース別の活動詳細（教材・課題・テスト・ディスカッション）\n");
        for (id, items) in &grouped {
            let name = luna_id_to_name.get(id).unwrap_or(id);
            if !pending_course_names.contains(*name) { continue; }

            text.push_str(&format!("### {}\n", name));

            // Separate by type for clarity
            let materials: Vec<_> = items.iter().filter(|a| a.activity_type == "material").collect();
            let reports: Vec<_> = items.iter().filter(|a| a.activity_type == "report").collect();
            let exams: Vec<_> = items.iter().filter(|a| a.activity_type == "exam").collect();
            let discussions: Vec<_> = items.iter().filter(|a| a.activity_type == "discussion").collect();


            if !materials.is_empty() {
                text.push_str("  教材:\n");
                for a in &materials {
                    text.push_str(&format!("    - {}", a.title));
                    if !a.period.is_empty() { text.push_str(&format!(" ({})", a.period)); }
                    text.push('\n');
                }
            }
            if !reports.is_empty() {
                text.push_str("  課題:\n");
                for a in &reports {
                    text.push_str(&format!("    - {}", a.title));
                    if !a.period.is_empty() { text.push_str(&format!(" (期限: {})", a.period)); }
                    if !a.status.is_empty() { text.push_str(&format!(" [{}]", a.status)); }
                    text.push('\n');
                }
            }
            if !exams.is_empty() {
                text.push_str("  テスト:\n");
                for a in &exams {
                    text.push_str(&format!("    - {}", a.title));
                    if !a.period.is_empty() { text.push_str(&format!(" (期間: {})", a.period)); }
                    if !a.status.is_empty() { text.push_str(&format!(" [{}]", a.status)); }
                    text.push('\n');
                }
            }
            if !discussions.is_empty() {
                text.push_str("  ディスカッション:\n");
                for a in &discussions {
                    text.push_str(&format!("    - {}", a.title));
                    if !a.period.is_empty() { text.push_str(&format!(" (期間: {})", a.period)); }
                    if !a.status.is_empty() { text.push_str(&format!(" [{}]", a.status)); }
                    text.push('\n');
                }
            }
        }
    }

    // ── Session plans (授業計画) — show more sessions for full context ──
    if !raw.session_plans.is_empty() {
        let code_to_name: std::collections::HashMap<&str, &str> = raw.kgc_entries_current.iter()
            .chain(raw.kgc_entries_next.iter())
            .map(|e| (e.kgc_code.as_str(), e.name.as_str()))
            .collect();

        let mut any_plan = false;
        for (code, plans) in &raw.session_plans {
            let cname = code_to_name.get(code.as_str()).copied().unwrap_or("");
            if !pending_course_names.contains(cname) { continue; }
            if !any_plan {
                text.push_str("\n## 関連コースの授業計画\n");
                any_plan = true;
            }
            text.push_str(&format!("### {} [{}]\n", cname, code));
            for p in plans {
                // Show all sessions up to current + 3 for full context
                if p.session_num <= current_week + 3 {
                    let marker = if p.session_num == current_week { " ← 今週" }
                        else if p.session_num == current_week - 1 { " ← 先週" }
                        else { "" };
                    let mut line = format!("  第{}回:", p.session_num);
                    if !p.topic.is_empty() { line.push_str(&format!(" {}", p.topic)); }
                    if !p.delivery_mode.is_empty() { line.push_str(&format!(" [{}]", p.delivery_mode)); }
                    if !p.study_outside.is_empty() { line.push_str(&format!(" | 予復習: {}", p.study_outside)); }
                    line.push_str(marker);
                    line.push('\n');
                    text.push_str(&line);
                }
            }
        }
    }

    // ── KGC course details (syllabus: grading, textbooks, objectives, etc.) ──
    if !raw.kgc_course_details.is_empty() {
        let code_to_name: std::collections::HashMap<&str, &str> = raw.kgc_entries_current.iter()
            .chain(raw.kgc_entries_next.iter())
            .map(|e| (e.kgc_code.as_str(), e.name.as_str()))
            .collect();

        let mut any_detail = false;
        for detail in &raw.kgc_course_details {
            let cname = code_to_name.get(detail.kgc_code.as_str()).copied().unwrap_or("");
            if !pending_course_names.contains(cname) { continue; }
            if detail.fields.is_empty() { continue; }
            if !any_detail {
                text.push_str("\n## 関連コースのシラバス詳細\n");
                any_detail = true;
            }
            text.push_str(&format!("### {} [{}]\n", cname, detail.kgc_code));
            if !detail.delivery_mode.is_empty() {
                text.push_str(&format!("  授業形態: {}\n", detail.delivery_mode));
            }
            for (label, value) in &detail.fields {
                if !value.is_empty() {
                    text.push_str(&format!("  {}: {}\n", label, value));
                }
            }
        }
    }

    text
}

// ── Internal helpers ──

/// Fetch next week's KGC data by navigating the Struts form.
async fn fetch_next_week_kgc(
    kgc_http: &reqwest::Client,
    current_data: &parser::TimetableData,
    db: &Database,
) -> Result<String, String> {
    if current_data.form_fields.is_empty() {
        return Ok(String::new());
    }

    // Fresh GET to get valid Struts token
    let fresh_url = format!(
        "{}/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014",
        config::KG_COURSE_BASE
    );
    let fresh_html = client::fetch_page_with(kgc_http, &fresh_url).await?;
    let fresh_data = parser::parse_timetable(&fresh_html);

    let mut params: Vec<(String, String)> = fresh_data.form_fields.into_iter().collect();
    params.push(("ENext.x".into(), "1".into()));
    params.push(("ENext.y".into(), "1".into()));

    let post_url = format!("{}/uniasv2/ARF010PCT01EventAction.do", config::KG_COURSE_BASE);
    let html = client::post_form_with_redirect(
        kgc_http, &post_url, config::KG_COURSE_BASE,
        client::SESSION_EXPIRED_MSG, client::is_session_expired_body,
        params.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        &[
            ("Referer", &format!("{}/uniasv2/ARF010.do", config::KG_COURSE_BASE)),
            ("Origin", config::KG_COURSE_BASE),
        ],
    ).await?;

    let next_data = parser::parse_timetable(&html);
    let next_week_label = next_data.week_label.clone();
    log::info!(
        "fetch_next_week_kgc: next page: {} entries, week_label='{}'",
        next_data.entries.len(), next_week_label
    );

    if next_data.entries.is_empty() && next_week_label.is_empty() {
        return Ok(String::new());
    }

    for entry in &next_data.entries {
        let day_int = day_str_to_int(&entry.day);
        if day_int == 0 { continue; }
        db.upsert_kgc_course(
            &entry.course_code, &entry.course_name, day_int, entry.period,
            &entry.room, &entry.detail_path,
            entry.is_cancelled, entry.is_makeup, entry.is_room_changed,
            &next_week_label,
        )?;
    }

    Ok(next_week_label)
}

fn load_ai_cache(db: &Database) -> Result<(Option<AiScheduleResult>, bool), String> {
    load_ai_cache_inner(db).map(|opt| {
        match opt {
            Some((result, ts)) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                let stale = now - ts > AI_CACHE_MAX_AGE;
                (Some(result), stale)
            }
            None => (None, true),
        }
    })
}

fn load_ai_cache_inner(db: &Database) -> Result<Option<(AiScheduleResult, i64)>, String> {
    db.get_ai_schedule_cache()
}

// ── AI prompt construction ──

const SCHEDULE_SYSTEM_PROMPT: &str = r#"あなたは関西学院大学の学生向けスケジュール分析AIです。
提供された時間割データ（KGC + Luna）を分析し、構造化されたJSON形式で2週間分の日程表を生成してください。

出力は必ず以下のJSON形式で返してください（他のテキストは不要）:
{
  "current_week": [
    {
      "day": 1,
      "period": 1,
      "course_name": "科目名",
      "delivery_mode": "対面/オンライン/同時双方向/オンデマンド",
      "room": "教室",
      "teacher": "教員名",
      "session_topic": "第N回: 今回の授業内容",
      "is_cancelled": false,
      "notifications": ["新しいお知らせの概要"],
      "assignments": ["未提出課題: 課題名 (締切: YYYY-MM-DD)"],
      "exams": ["テスト名 (期間: YYYY-MM-DD ~ YYYY-MM-DD, 状態: 未回答)"]
    }
  ],
  "next_week": [...],
  "weekly_summary": "今週の提案1。今週の提案2。今週の提案3",
  "cross_week_insights": "来週に向けた提案1。来週に向けた提案2"
}

表示上の注意:
- weekly_summaryとcross_week_insightsは画面上部で句単位で循環表示される。「。」で区切る。
- これらは単なる要約ではなく、学生への具体的なアドバイスや行動提案として書くこと。
  良い例: 「木曜の○○は課題締切前日、早めに着手しましょう」「来週○曜は対面とオンデマンドが混在、時間管理に注意」
  悪い例: 「今週は5コマある」「来週も通常通り」（情報の羅列や当たり前の内容は避ける）
- weekly_summaryは4〜6文程度。休講・締切間近・テスト・形態変更・予習が必要な科目など、実際に行動すべき内容を具体的に。
- cross_week_insightsは2〜4文程度。来週の対策に絞る。
- session_topicはセル内に表示されるため簡潔に（例: 「第5回: データ構造の基礎」）。
- notifications/assignments/examsの各項目も1行でセル内に表示されるため、要点のみ記載。

ルール:
- day: 1=月曜 2=火曜 3=水曜 4=木曜 5=金曜 6=土曜
- period: 1-7限
- 今週/来週ラベル（例: "2026年04月07日～2026年04月11日"）は該当週の月曜～金曜の日付範囲を示す。授業計画のtopicにある日付がこの範囲に含まれるかで今週/来週の対応回を判定する

## Step 1: 授業計画から日付とdelivery_modeを読み取る

各科目の授業計画にはheader（セッションヘッダー）、column（表の中間列データ）、topic（授業内容）が分離して提供される。

**headerから読み取れる情報:**
- 週タイプ: 【スタートアップウィーク】【コアウィークス①】【フレックスアワーズ①】等
- 授業形態: 「・オンライン授業（オンデマンド型）・」「対面授業／」「同時双方向型」等
- 授業時間: 《60分》《90分》等

**columnから読み取れる情報:**
- 授業形態の短縮表記: 「対面」「オンデマンド」「オンライン」等（表の独立した列データ）

**topicから読み取れる情報:**
- 日付: 「4/16」「5/14」等
- 授業内容の説明文

**delivery_modeの判定順序:**
1. headerに「オンデマンド」「同時双方向」「オンライン」「対面授業」がある → そのまま使う
2. columnが存在する → columnの値を使う（「対面」「オンデマンド」等、表の独立した列なので信頼性が高い）
3. headerにもcolumnにも情報がない → KGC課程詳細の授業形態を参照
4. どこにも明示的な情報がない → 「対面」と推定する（大学の授業は対面がデフォルト）
- **「要確認」はdelivery_modeには使わない**。必ず上記の優先順位で判定すること。
- **topicの授業内容説明文の中に含まれる「対面」「対面授業XX回中」等の単語は出席ルール説明であり、delivery_modeではない**

## Step 2: 授業回数の特定

### 2a. 日付で確定できる回を先に埋める
topicに日付がある回を、今週・来週のカレンダーと照合して確定する。

### 2b. 日付のない回は前後関係から推論する
- 2つの日付付き回の間に日付のない回がある場合:
  例: 第2回=4/16、第4回=4/23 → 第3回は4/16～4/22の期間（フレックスアワーズ等のオンデマンド回の可能性が高い）
  → この回は対面で出席する必要がないので、出力のis_cancelledをfalseにしつつ、session_topicに「(オンデマンド視聴期間)」と補足
- 毎週ある科目で1週間に2コマ消化されるパターン（例: 9日→16日に2回分進む）場合も同様に推論

### 2c. 全回に日付がなく、学期開始日が提供されている場合
ユーザデータに「現在は春/秋学期 第N週目」と記載されていればその週数を使う。
記載がなければ学期開始日と曜日から逆算して「第N週目」を計算し、対応する回を推定する。
ただし祝日・長期休暇のスキップがあるため、推定であることをsession_topicに「(推定)」と付記する。

### 2d. 確定できない場合
学期開始日からの週数推定を優先し、session_topicに「(推定)」と付記する。
推定すらできない場合のみ「(要確認)」を使う。ただし空白や省略はしない——必ず最も可能性の高い回を記載する。

## Step 3: データ統合
- KGCデータとLunaデータを照合し、同じコマの情報を統合（科目名/曜日/時限で照合）
- Luna活動詳細の個別アイテム（課題名、締切、状態）をassignments/exams/notificationsに反映
- 休講の授業は is_cancelled: true
- 通知・課題・テストが無い場合は空配列
- weekly_summaryは「今週やるべきこと・気をつけること」を具体的に提案
- cross_week_insightsは「来週に向けて今週中にやっておくべきこと」を提案
- 回答はJSONのみ、マークダウンのコードブロックは不要
- 回答は指定された言語で書くこと"#;

fn build_ai_schedule_prompt(raw: &ScheduleRawData, config: &crate::ai::AiConfig) -> String {
    let mut text = String::new();

    // Today's date — critical for AI to determine current session number
    let today = chrono::Local::now();
    let today_str = today.format("%Y-%m-%d (%A)").to_string();
    text.push_str(&format!("## 今日の日付: {}\n", today_str));

    // Semester start dates + current week number
    let current_date = today.date_naive();
    {
        let mut semester_lines: Vec<String> = Vec::new();
        if !config.spring_start.is_empty() {
            if let Ok(spring) = chrono::NaiveDate::parse_from_str(&config.spring_start, "%Y-%m-%d") {
                semester_lines.push(format!("- 春学期開始: {}", config.spring_start));
                let days_since = (current_date - spring).num_days();
                if days_since >= 0 && days_since < 150 {
                    semester_lines.push(format!("- ★ 現在は春学期 第{}週目", days_since / 7 + 1));
                }
            }
        }
        if !config.fall_start.is_empty() {
            if let Ok(fall) = chrono::NaiveDate::parse_from_str(&config.fall_start, "%Y-%m-%d") {
                semester_lines.push(format!("- 秋学期開始: {}", config.fall_start));
                let days_since = (current_date - fall).num_days();
                if days_since >= 0 && days_since < 150 {
                    semester_lines.push(format!("- ★ 現在は秋学期 第{}週目", days_since / 7 + 1));
                }
            }
        }
        if !semester_lines.is_empty() {
            text.push_str("\n## 学期情報\n");
            for line in &semester_lines {
                text.push_str(line);
                text.push('\n');
            }
        }
    }

    text.push('\n');

    // ── KGC Timetable (Current Week) ──
    text.push_str(&format!("## 今週: {}\n", raw.current_week_label));
    text.push_str("### KGC時間割（今週）\n");
    for e in &raw.kgc_entries_current {
        text.push_str(&format!(
            "- {}曜{}限: {} [{}] 教室:{} 休講:{} 補講:{} 変更:{}\n",
            day_int_to_str(e.day), e.period, e.name, e.kgc_code, e.room,
            e.is_cancelled, e.is_makeup, e.is_room_changed
        ));
    }

    // ── KGC Timetable (Next Week) ──
    text.push_str(&format!("\n## 来週: {}\n", raw.next_week_label));
    text.push_str("### KGC時間割（来週）\n");
    for e in &raw.kgc_entries_next {
        text.push_str(&format!(
            "- {}曜{}限: {} [{}] 教室:{} 休講:{} 補講:{} 変更:{}\n",
            day_int_to_str(e.day), e.period, e.name, e.kgc_code, e.room,
            e.is_cancelled, e.is_makeup, e.is_room_changed
        ));
    }

    // ── Luna Courses (with name-to-id mapping) ──
    if !raw.luna_courses.is_empty() {
        text.push_str("\n### Luna登録コース\n");
        for c in &raw.luna_courses {
            text.push_str(&format!(
                "- {}曜{}限: {} [luna_id:{}] 教員:{}\n",
                day_int_to_str(c.day), c.period, c.name, c.luna_id, c.teacher
            ));
        }
    }

    // Shared kgc_code → course name mapping
    let code_to_name: std::collections::HashMap<&str, &str> = raw.kgc_entries_current.iter()
        .chain(raw.kgc_entries_next.iter())
        .map(|e| (e.kgc_code.as_str(), e.name.as_str()))
        .collect();

    // ── Session Plans (授業計画) ──
    if !raw.session_plans.is_empty() {
        text.push_str("\n### 授業計画\n");
        for (code, plans) in &raw.session_plans {
            let course_label = code_to_name.get(code.as_str())
                .map(|n| format!("{} [{}]", n, code))
                .unwrap_or_else(|| code.clone());
            text.push_str(&format!("#### {}\n", course_label));
            for p in plans {
                let mut line = format!("  第{}回:", p.session_num);
                if !p.th_header.is_empty() {
                    line.push_str(&format!(" [header: {}]", p.th_header));
                }
                if !p.delivery_mode.is_empty() {
                    line.push_str(&format!(" [column: {}]", p.delivery_mode));
                }
                if !p.topic.is_empty() {
                    line.push_str(&format!(" {}", p.topic));
                }
                if !p.study_outside.is_empty() {
                    line.push_str(&format!(" (予習: {})", p.study_outside));
                }
                line.push('\n');
                text.push_str(&line);
            }
        }
    }

    // ── KGC Course Details (授業形態 etc.) ──
    if !raw.kgc_course_details.is_empty() {
        text.push_str("\n### KGC課程詳細情報\n");
        // Only fields relevant to schedule analysis (delivery mode, grading, notes)
        let important_labels = [
            "授業形態", "授業方法", "授業スタイル",
            "授業の進め方", "備考", "注意事項",
        ];
        for d in &raw.kgc_course_details {
            let course_label = code_to_name.get(d.kgc_code.as_str())
                .map(|n| format!("{} [{}]", n, d.kgc_code))
                .unwrap_or_else(|| d.kgc_code.clone());
            // Filter to important fields only
            let relevant: Vec<_> = d.fields.iter()
                .filter(|(label, value)| {
                    !value.is_empty() && important_labels.iter().any(|k| label.contains(k))
                })
                .collect();
            if relevant.is_empty() && d.delivery_mode.is_empty() { continue; }
            text.push_str(&format!("#### {}\n", course_label));
            if !d.delivery_mode.is_empty() {
                text.push_str(&format!("  授業形態: {}\n", d.delivery_mode));
            }
            for (label, value) in &relevant {
                // Truncate very long values to keep prompt manageable
                let truncated = if value.chars().count() > 300 {
                    let s: String = value.chars().take(300).collect();
                    format!("{}...", s)
                } else {
                    value.clone()
                };
                text.push_str(&format!("  {}: {}\n", label, truncated));
            }
        }
    }

    // ── Luna Activity Counts (summary) ──
    if !raw.luna_counts.is_empty() {
        text.push_str("\n### Luna活動サマリー\n");
        // Build luna_id → course name mapping
        let luna_id_to_name: std::collections::HashMap<&str, &str> = raw.luna_courses.iter()
            .map(|c| (c.luna_id.as_str(), c.name.as_str()))
            .collect();
        for (id, c) in &raw.luna_counts {
            let fallback = id.as_str();
            let name = luna_id_to_name.get(id.as_str()).unwrap_or(&fallback);
            text.push_str(&format!(
                "- {} [{}]: お知らせ{}(新{}), 未提出課題{}, テスト{}, ディスカッション{}\n",
                name, id, c.announcements, c.new_announcements, c.reports, c.exams, c.discussions
            ));
        }
    }

    // ── Luna Activity Details (個別アイテム) ──
    if !raw.luna_activities.is_empty() {
        text.push_str("\n### Luna活動詳細\n");
        let luna_id_to_name: std::collections::HashMap<&str, &str> = raw.luna_courses.iter()
            .map(|c| (c.luna_id.as_str(), c.name.as_str()))
            .collect();

        // Group activities by luna_id
        let mut grouped: std::collections::HashMap<&str, Vec<&crate::db::LunaActivityRow>> = Default::default();
        for a in &raw.luna_activities {
            grouped.entry(a.luna_id.as_str()).or_default().push(a);
        }

        for (id, items) in &grouped {
            let name = luna_id_to_name.get(id).unwrap_or(id);
            text.push_str(&format!("#### {} [{}]\n", name, id));
            for a in items {
                let type_label = match a.activity_type.as_str() {
                    "announcement" => "お知らせ",
                    "report" => "課題",
                    "exam" => "テスト",
                    "discussion" => "ディスカッション",
                    "material" => "教材",
                    _ => &a.activity_type,
                };
                let mut line = format!("  [{}] {}", type_label, a.title);
                if !a.period.is_empty() {
                    line.push_str(&format!(" (期間: {})", a.period));
                }
                if !a.status.is_empty() {
                    line.push_str(&format!(" {{状態: {}}}", a.status));
                }
                line.push('\n');
                text.push_str(&line);
            }
        }
    }

    // Luna Communities omitted — not relevant for schedule analysis

    text
}

fn parse_ai_schedule_response(
    response: &str,
    current_week_label: &str,
    next_week_label: &str,
) -> Result<AiScheduleResult, String> {
    // Try to extract JSON from response (may be wrapped in markdown code blocks)
    let json_str = extract_json_from_response(response);

    #[derive(Deserialize)]
    struct AiResponse {
        current_week: Vec<AiScheduleItem>,
        next_week: Vec<AiScheduleItem>,
        weekly_summary: Option<String>,
        cross_week_insights: Option<String>,
    }

    let parsed: AiResponse = serde_json::from_str(json_str)
        .map_err(|e| format!("AI応答のJSON解析に失敗: {} — 応答: {}", e, safe_preview(response, 200)))?;

    Ok(AiScheduleResult {
        current_week_label: current_week_label.to_string(),
        next_week_label: next_week_label.to_string(),
        current_week: parsed.current_week,
        next_week: parsed.next_week,
        weekly_summary: parsed.weekly_summary.unwrap_or_default(),
        cross_week_insights: parsed.cross_week_insights.unwrap_or_default(),
    })
}

fn extract_json_from_response(text: &str) -> &str {
    // Try to find JSON within ```json ... ``` or ``` ... ``` blocks
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```") {
            return after[..end].trim();
        }
    }
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        if let Some(end) = after.find("```") {
            return after[..end].trim();
        }
    }
    // Try to find raw JSON object
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            return &text[start..=end];
        }
    }
    text.trim()
}


