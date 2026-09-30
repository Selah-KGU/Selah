// timetable.rs — AI-driven schedule: fetch KGC + Luna raw data, enrich, then AI analysis.

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::State;

use crate::client;
use crate::commands;
use crate::config;
use crate::db::epoch_secs;
use crate::db::{
    AiScheduleResult, Database, KgcCourseDetailRow, LunaActivityRow, LunaCountsRow,
    ScheduleRawData, SessionPlanRow, SnapshotState,
};
use crate::luna_client;
use crate::luna_parser;
use crate::parser;
use crate::{KgcState, LunaState};

#[path = "timetable/ai_analysis.rs"]
mod ai_analysis;

use self::ai_analysis::load_ai_cache;
pub use ai_analysis::*;

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
        "月" => 1,
        "火" => 2,
        "水" => 3,
        "木" => 4,
        "金" => 5,
        "土" => 6,
        _ => 0,
    }
}

fn day_int_to_str(d: i32) -> &'static str {
    if (1..=6).contains(&d) {
        config::DAY_SHORT[d as usize]
    } else {
        "?"
    }
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
    #[serde(default)]
    pub kgc_warning: String,
}

// ── Commands ──

/// Load schedule from DB snapshot only (no network). Fast, used on page mount.
#[tauri::command]
pub async fn get_schedule_snapshot(db: State<'_, Database>) -> Result<ScheduleResponse, String> {
    let snap = db.get_snapshot_state()?.unwrap_or_default();
    let scope = crate::academic_period::visible_weeks(
        &snap.current_week_label,
        &snap.next_week_label,
        &snap.luna_year,
        &snap.luna_term,
        chrono::Local::now().date_naive(),
    );
    let mut communities = snap.luna_communities.clone();
    retain_current_communities(&mut communities, &scope.year, &scope.term);
    let raw = db.build_raw_data(&scope.current, &scope.next, communities.clone())?;
    let (ai_result, ai_stale) = load_ai_cache(&db)?;
    let mut kgc_warning = load_kgc_warning(&db);
    if scope.hid_current {
        kgc_warning = STALE_SEMESTER_KGC_WARNING.to_string();
    }
    Ok(ScheduleResponse {
        raw,
        ai_result,
        ai_stale,
        snapshot_updated_at: snap.updated_at,
        luna_communities: communities,
        luna_year_options: snap.luna_year_options,
        luna_term_options: snap.luna_term_options,
        luna_year: scope.year,
        luna_term: scope.term,
        kgc_warning,
    })
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

const STALE_SEMESTER_KGC_WARNING: &str =
    "前学期の時間割は表示していません。KGC から今学期の時間割を取得できていません。";

fn load_kgc_warning(db: &Database) -> String {
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

struct LunaSnapshotFields {
    communities: Vec<luna_parser::LunaCommunity>,
    year_opts: Vec<luna_parser::SelectOption>,
    term_opts: Vec<luna_parser::SelectOption>,
    year: String,
    term: String,
    replaced: bool,
    metadata_changed: bool,
}

fn luna_fields_from_snapshot(snapshot: &SnapshotState) -> LunaSnapshotFields {
    LunaSnapshotFields {
        communities: snapshot.luna_communities.clone(),
        year_opts: snapshot.luna_year_options.clone(),
        term_opts: snapshot.luna_term_options.clone(),
        year: snapshot.luna_year.clone(),
        term: snapshot.luna_term.clone(),
        replaced: false,
        metadata_changed: false,
    }
}

async fn sync_luna_timetable(
    luna_state: &LunaState,
    db: &Database,
    previous: &SnapshotState,
) -> Result<LunaSnapshotFields, String> {
    let http = {
        let luna = luna_state.client.lock().await;
        if luna.authenticated {
            Some(luna.http.clone())
        } else {
            None
        }
    };
    let Some(http) = http else {
        log::info!("sync_schedule_data: Luna not authenticated; keeping stored timetable");
        return Ok(luna_fields_from_snapshot(previous));
    };

    let target = target_luna_period(previous);
    let parsed = match fetch_current_luna_timetable(&http, target.as_ref()).await {
        Ok(parsed) => parsed,
        Err(error) if is_luna_auth_error(&error) => return Err(error),
        Err(error) => {
            log::warn!("sync_schedule_data: Luna fetch skipped persist: {}", error);
            return Ok(luna_fields_from_snapshot(previous));
        }
    };

    if !courses_fit_target(&parsed.courses, target.as_ref()) {
        log::warn!(
            "sync_schedule_data: Luna courses do not match target term {}/{}; keeping stored rows",
            parsed.year,
            parsed.term
        );
        return Ok(luna_fields_from_snapshot(previous));
    }

    let replaced = if parsed.courses.is_empty() {
        log::warn!("sync_schedule_data: Luna timetable had no courses; keeping stored rows");
        false
    } else {
        db.replace_luna_courses(&parsed.courses)?;
        log::info!(
            "sync_schedule_data: Luna: {} courses, {} communities, term={}/{}",
            parsed.courses.len(),
            parsed.communities.len(),
            parsed.year,
            parsed.term
        );
        true
    };

    let year = if parsed.year.is_empty() {
        previous.luna_year.clone()
    } else {
        parsed.year
    };
    let term = if parsed.term.is_empty() {
        previous.luna_term.clone()
    } else {
        parsed.term
    };
    Ok(LunaSnapshotFields {
        communities: if parsed.communities.is_empty() {
            previous.luna_communities.clone()
        } else {
            parsed.communities
        },
        year_opts: if parsed.year_options.is_empty() {
            previous.luna_year_options.clone()
        } else {
            parsed.year_options
        },
        term_opts: if parsed.term_options.is_empty() {
            previous.luna_term_options.clone()
        } else {
            parsed.term_options
        },
        year,
        term,
        replaced,
        metadata_changed: true,
    })
}

async fn fetch_current_luna_timetable(
    http: &reqwest::Client,
    target: Option<&(String, String)>,
) -> Result<luna_parser::LunaTimetable, String> {
    let url = format!("{}/lms/timetable", config::LUNA_BASE);
    let html = client::fetch_with_redirect(
        http,
        &url,
        config::LUNA_BASE,
        luna_client::LUNA_SESSION_EXPIRED_MSG,
        luna_client::is_luna_session_expired,
    )
    .await?;
    let parsed = luna_parser::parse_luna_timetable(&html);
    if luna_timetable_matches(&parsed, target) {
        return Ok(parsed);
    }
    let Some((year, term)) = target else {
        return Ok(parsed);
    };
    let Some(request) = luna_parser::build_timetable_switch(&html, year, term) else {
        log::warn!(
            "sync_schedule_data: Luna page is {}/{}, cannot switch to {}/{}",
            parsed.year,
            parsed.term,
            year,
            term
        );
        return Err("luna-term-mismatch".into());
    };
    log::info!(
        "sync_schedule_data: Luna page is {}/{}, switching to {}/{}",
        parsed.year,
        parsed.term,
        year,
        term
    );
    let switched_html = submit_luna_timetable_switch(http, &request).await?;
    let switched = luna_parser::parse_luna_timetable(&switched_html);
    if luna_timetable_matches(&switched, target) {
        return Ok(switched);
    }
    log::warn!(
        "sync_schedule_data: Luna switch stayed on {}/{}",
        switched.year,
        switched.term
    );
    Err("luna-term-mismatch".into())
}

async fn submit_luna_timetable_switch(
    http: &reqwest::Client,
    request: &luna_parser::LunaTimetableSwitch,
) -> Result<String, String> {
    let url = resolve_luna_action(&request.action)?;
    let referer = format!("{}/lms/timetable", config::LUNA_BASE);
    if request.method.eq_ignore_ascii_case("get") {
        let builder = http
            .get(&url)
            .query(&request.fields)
            .header("Referer", &referer);
        return client::send_and_follow_redirect(
            http,
            builder,
            config::LUNA_BASE,
            luna_client::LUNA_SESSION_EXPIRED_MSG,
            luna_client::is_luna_session_expired,
        )
        .await;
    }
    client::post_form_with_redirect(
        http,
        &url,
        config::LUNA_BASE,
        luna_client::LUNA_SESSION_EXPIRED_MSG,
        luna_client::is_luna_session_expired,
        request.fields.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        &[("Referer", referer.as_str()), ("Origin", config::LUNA_BASE)],
    )
    .await
}

fn resolve_luna_action(action: &str) -> Result<String, String> {
    let action = action.trim();
    if action.is_empty() || action == "#" || action.starts_with("javascript:") {
        return Ok(format!("{}/lms/timetable", config::LUNA_BASE));
    }
    if action.starts_with("http://") || action.starts_with("https://") {
        if action.starts_with(config::LUNA_BASE) {
            return Ok(action.to_string());
        }
        return Err("Luna timetable form action is outside Luna".into());
    }
    if let Some(path) = action.strip_prefix('/') {
        return Ok(format!("{}/{}", config::LUNA_BASE, path));
    }
    Ok(format!(
        "{}/{}",
        config::LUNA_BASE,
        action.trim_start_matches("./")
    ))
}

fn target_luna_period(snapshot: &SnapshotState) -> Option<(String, String)> {
    let today = chrono::Local::now().date_naive();
    if let Some(period) = crate::academic_period::calendar_academic_period(today) {
        return Some((period.year, period.term));
    }
    if !snapshot.luna_year.is_empty() && !snapshot.luna_term.is_empty() {
        return Some((snapshot.luna_year.clone(), snapshot.luna_term.clone()));
    }
    None
}

fn retain_current_communities(
    communities: &mut Vec<luna_parser::LunaCommunity>,
    year: &str,
    term: &str,
) {
    communities.retain(|community| {
        crate::db::luna_course_matches_snapshot(&community.idnumber, year, term)
    });
}

fn luna_timetable_matches(
    parsed: &luna_parser::LunaTimetable,
    target: Option<&(String, String)>,
) -> bool {
    let Some((year, term)) = target else {
        return true;
    };
    let year_ok = parsed.year.is_empty() || &parsed.year == year;
    parsed.term == *term && year_ok
}

fn courses_fit_target(
    courses: &[luna_parser::LunaCourse],
    target: Option<&(String, String)>,
) -> bool {
    let Some((year, term)) = target else {
        return true;
    };
    courses
        .iter()
        .all(|course| crate::db::luna_course_matches_snapshot(&course.idnumber, year, term))
}

fn is_luna_auth_error(error: &str) -> bool {
    error == luna_client::LUNA_SESSION_EXPIRED_MSG || error.contains("Lunaセッションが期限切れ")
}

async fn clear_kgc_if_expired(kgc: &KgcState, error: &str) {
    if error == client::SESSION_EXPIRED_MSG {
        kgc.client.lock().await.clear_session();
        log::info!("KGC session expired during schedule sync; stopped background KGC requests");
    }
}

/// Fetch next week's KGC data by navigating the Struts form.
async fn fetch_next_week_kgc(
    kgc_http: &reqwest::Client,
    current_data: &parser::TimetableData,
    db: &Database,
) -> Result<String, String> {
    if current_data.form_fields.is_empty() {
        return Ok(String::new());
    }

    let fresh_url = format!(
        "{}/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014",
        config::KG_COURSE_BASE
    );
    let fresh_html = client::fetch_page_with(kgc_http, &fresh_url).await?;
    let fresh_data = parser::parse_timetable(&fresh_html);

    let mut params: Vec<(String, String)> = fresh_data.form_fields.into_iter().collect();
    params.push(("ENext.x".into(), "1".into()));
    params.push(("ENext.y".into(), "1".into()));

    let post_url = format!(
        "{}/uniasv2/ARF010PCT01EventAction.do",
        config::KG_COURSE_BASE
    );
    let html = client::post_form_with_redirect(
        kgc_http,
        &post_url,
        config::KG_COURSE_BASE,
        client::SESSION_EXPIRED_MSG,
        client::is_session_expired_body,
        params.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        &[
            (
                "Referer",
                &format!("{}/uniasv2/ARF010.do", config::KG_COURSE_BASE),
            ),
            ("Origin", config::KG_COURSE_BASE),
        ],
    )
    .await?;

    let next_data = parser::parse_timetable(&html);
    let next_week_label = next_data.week_label.clone();
    log::info!(
        "fetch_next_week_kgc: next page: {} entries, week_label='{}'",
        next_data.entries.len(),
        next_week_label
    );

    if next_data.entries.is_empty() && next_week_label.is_empty() {
        return Ok(String::new());
    }

    for entry in &next_data.entries {
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
            &next_week_label,
        )?;
    }

    Ok(next_week_label)
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
                    log::warn!(
                        "batch_fetch_syllabi: {} GET search page failed: {}",
                        code,
                        e
                    );
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
                http,
                "/uniasv2/AGA030PSC01EventAction.do",
                &search_params,
            )
            .await
            {
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
            let target = match parsed
                .entries
                .iter()
                .find(|e| e.class_code == code.as_str())
            {
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
                let pos = parsed
                    .entries
                    .iter()
                    .position(|e| e.class_code == code.as_str())
                    .unwrap_or(0);
                log::warn!(
                    "batch_fetch_syllabi: {} ereferIndex is empty, using positional fallback: {}",
                    code,
                    pos
                );
                pos.to_string()
            } else {
                refer_index.clone()
            };

            log::info!(
                "batch_fetch_syllabi: {} ereferIndex={}",
                code,
                effective_refer_index
            );

            // Navigate to syllabus detail page.
            // Extract inputs ONLY from the results list form (AGA030PLS01Form),
            // not from the search form which shares the page and has conflicting params.
            let mut form_params =
                commands::extract_named_form_inputs(&results_html, "AGA030PLS01Form");

            // Log diagnostic info about extracted params
            let token_count = form_params
                .iter()
                .filter(|(k, _)| k == "org.apache.struts.taglib.html.TOKEN")
                .count();
            log::info!(
                "batch_fetch_syllabi: {} extracted {} form params (AGA030PLS01Form), {} Struts tokens",
                code, form_params.len(), token_count
            );

            // With targeted form extraction, token dedup is unnecessary
            // (only one form's token is extracted).

            form_params.retain(|(k, _)| {
                !k.starts_with("ESearch")
                    && !k.starts_with("ENarrowSearch")
                    && !k.starts_with("EBack")
                    && !k.starts_with("ENext")
                    && !k.starts_with("EPrev")
                    && !k.starts_with("ERefer")
                    && !k.starts_with("ERegister")
                    && !k.starts_with("EPageSet")
                    && k != "hdnEsearch"
            });
            form_params.retain(|(k, _)| k != "ereferIndex");
            form_params.push(("ereferIndex".into(), effective_refer_index.clone()));
            form_params.push(("ERefer.x".into(), "10".into()));
            form_params.push(("ERefer.y".into(), "10".into()));

            // Dump params for debugging
            log::debug!(
                "batch_fetch_syllabi: {} POST params: {:?}",
                code,
                form_params
                    .iter()
                    .map(|(k, v)| {
                        if v.len() > 60 {
                            format!("{}={}...", k, &v[..60])
                        } else {
                            format!("{}={}", k, v)
                        }
                    })
                    .collect::<Vec<_>>()
            );

            match commands::kgc_post(http, "/uniasv2/AGA030PLS01EventAction.do", &form_params).await
            {
                Ok(detail_html) => {
                    if !detail_html.contains("AGA030PVI01Form") {
                        log::warn!(
                            "batch_fetch_syllabi: {} detail POST did not reach detail page (term {}), {} bytes",
                            code, term_code, detail_html.len()
                        );
                        #[cfg(debug_assertions)]
                        {
                            if crate::should_dump_debug_html() {
                                let _ = std::fs::write(
                                    std::env::temp_dir()
                                        .join(format!("kwic_detail_fail_{}.html", code)),
                                    &detail_html,
                                );
                            }
                        }
                        continue;
                    }
                    log::info!(
                        "batch_fetch_syllabi: {} -> {} bytes (term {})",
                        code,
                        detail_html.len(),
                        term_code
                    );

                    results.push((code.clone(), Ok(detail_html)));
                    found = true;
                    break;
                }
                Err(e) => {
                    log::warn!(
                        "batch_fetch_syllabi: {} POST detail failed (term {}): {}",
                        code,
                        term_code,
                        e
                    );
                    continue;
                }
            }
        }

        if !found && !results.iter().any(|(c, _)| c == code) {
            results.push((
                code.clone(),
                Err(format!("科目コード {} が見つかりません", code)),
            ));
        }

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    results
}

/// Background enrichment: fetch KGC syllabus pages for session plans + Luna counts.
#[tauri::command]
pub async fn enrich_schedule(
    state: State<'_, KgcState>,
    luna_state: State<'_, LunaState>,
    db: State<'_, Database>,
) -> Result<(), String> {
    // Prevent concurrent runs — Struts tokens conflict when two enrichments hit KGC simultaneously
    if ENRICHMENT_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        log::info!("enrich_schedule: skipped (already running)");
        return Ok(());
    }
    let _kgc_gate = state.gate.lock().await;
    let result = enrich_schedule_inner(&state, &luna_state, &db).await;
    ENRICHMENT_RUNNING.store(false, Ordering::SeqCst);
    result
}

/// Standalone Luna activity counts refresh (no KGC gate needed).
/// Only fetches counts for courses whose cached data is older than the DB threshold (3h).
#[tauri::command]
pub async fn refresh_luna_counts(
    state: State<'_, LunaState>,
    db: State<'_, Database>,
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
        let luna = state.client.lock().await;
        if !luna.authenticated {
            return Err("Luna not authenticated".into());
        }
        luna.http.clone()
    };

    log::info!(
        "refresh_luna_counts: {} courses need updates",
        luna_targets.len()
    );
    let mut updated = 0i32;

    for luna_id in &luna_targets {
        if luna_id.is_empty() {
            continue;
        }
        let course_url = format!("{}/lms/course?idnumber={}", config::LUNA_BASE, luna_id);
        let contents_url = format!("{}/lms/contents?idnumber={}", config::LUNA_BASE, luna_id);

        let course_html = match client::fetch_with_redirect(
            &luna_http,
            &course_url,
            config::LUNA_BASE,
            luna_client::LUNA_SESSION_EXPIRED_MSG,
            luna_client::is_luna_session_expired,
        )
        .await
        {
            Ok(h) => h,
            Err(e) => {
                log::warn!(
                    "refresh_luna_counts: course page failed for {}: {}",
                    luna_id,
                    e
                );
                continue;
            }
        };

        let course_data = luna_parser::parse_luna_course_contents(&course_html, luna_id);
        let new_announcements = course_data
            .announcements
            .iter()
            .filter(|a| a.is_new)
            .count() as i32;
        let announcement_count = course_data.announcements.len() as i32;

        let mut activities: Vec<LunaActivityRow> = Vec::new();
        for ann in &course_data.announcements {
            activities.push(LunaActivityRow {
                luna_id: luna_id.clone(),
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

        let (reports, exams, discussions) = match client::fetch_with_redirect(
            &luna_http,
            &contents_url,
            config::LUNA_BASE,
            luna_client::LUNA_SESSION_EXPIRED_MSG,
            luna_client::is_luna_session_expired,
        )
        .await
        {
            Ok(html) => {
                let (materials, reps, exs, discs, _surveys) =
                    luna_parser::parse_luna_contents_page(&html);
                for m in &materials {
                    activities.push(LunaActivityRow {
                        luna_id: luna_id.clone(),
                        activity_type: "material".into(),
                        title: m.title.clone(),
                        period: m.period.clone(),
                        status: m.status.clone(),
                        detail_path: m.url.clone(),
                    });
                }
                for r in &reps {
                    activities.push(LunaActivityRow {
                        luna_id: luna_id.clone(),
                        activity_type: "report".into(),
                        title: r.title.clone(),
                        period: r.period.clone(),
                        status: r.status.clone(),
                        detail_path: r.url.clone(),
                    });
                }
                for e in &exs {
                    activities.push(LunaActivityRow {
                        luna_id: luna_id.clone(),
                        activity_type: "exam".into(),
                        title: e.title.clone(),
                        period: e.period.clone(),
                        status: e.status.clone(),
                        detail_path: e.url.clone(),
                    });
                }
                for d in &discs {
                    activities.push(LunaActivityRow {
                        luna_id: luna_id.clone(),
                        activity_type: "discussion".into(),
                        title: d.title.clone(),
                        period: d.period.clone(),
                        status: d.status.clone(),
                        detail_path: d.url.clone(),
                    });
                }

                let pending_reports =
                    reps.iter().filter(|r| r.status.contains("未提出")).count() as i32;
                let pending_exams = exs
                    .iter()
                    .filter(|e| e.status.contains("未回答") || e.status.contains("未受験"))
                    .count() as i32;
                (pending_reports, pending_exams, discs.len() as i32)
            }
            Err(e) => {
                log::warn!(
                    "refresh_luna_counts: contents failed for {}: {}",
                    luna_id,
                    e
                );
                (0, 0, 0)
            }
        };

        if let Err(e) = db.replace_luna_activities(luna_id, &activities) {
            log::warn!(
                "refresh_luna_counts: failed to save activities for {}: {}",
                luna_id,
                e
            );
        }

        let counts = LunaCountsRow {
            announcements: announcement_count,
            new_announcements,
            reports,
            exams,
            discussions,
        };
        if let Err(e) = db.upsert_luna_counts(luna_id, &counts) {
            log::warn!(
                "refresh_luna_counts: failed to save counts for {}: {}",
                luna_id,
                e
            );
        }

        updated += 1;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    log::info!(
        "refresh_luna_counts: updated {}/{} courses",
        updated,
        luna_targets.len()
    );
    Ok(updated)
}

async fn enrich_schedule_inner(
    kgc: &KgcState,
    luna: &LunaState,
    db: &Database,
) -> Result<(), String> {
    // Session plans from KGC syllabus pages (not timetable detail pages)
    let plan_targets = db.kgc_codes_needing_plans()?;
    log::info!("enrich_schedule: {} courses need plans", plan_targets.len());
    if !plan_targets.is_empty() {
        let kgc_http = {
            let client = kgc.client.lock().await;
            if !client.is_authenticated() {
                return Ok(());
            }
            client.http.clone()
        };

        let batch_results = batch_fetch_syllabi(&kgc_http, &plan_targets).await;
        for (kgc_code, result) in batch_results {
            match result {
                Ok(detail_html) => {
                    // Parse session plans from the real syllabus page
                    let parsed = parser::parse_session_plans(&detail_html);
                    log::info!(
                        "enrich_schedule: {} parsed {} plans from syllabus ({} bytes)",
                        kgc_code,
                        parsed.len(),
                        detail_html.len()
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

                    // Parse full course detail fields + delivery mode from syllabus
                    let detail = parser::parse_course_detail(&detail_html);
                    let delivery_mode = parser::detect_delivery_mode_from_detail(&detail_html);
                    let textbooks = parser::parse_textbooks(&detail_html);
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

    // Luna activity counts
    let luna_targets = db.luna_ids_needing_counts()?;
    if !luna_targets.is_empty() {
        let luna_http = {
            let luna = luna.client.lock().await;
            if !luna.authenticated {
                return Ok(());
            }
            luna.http.clone()
        };

        for luna_id in luna_targets {
            if luna_id.is_empty() {
                continue;
            }
            let course_url = format!("{}/lms/course?idnumber={}", config::LUNA_BASE, luna_id);
            let contents_url = format!("{}/lms/contents?idnumber={}", config::LUNA_BASE, luna_id);

            let course_html = match client::fetch_with_redirect(
                &luna_http,
                &course_url,
                config::LUNA_BASE,
                luna_client::LUNA_SESSION_EXPIRED_MSG,
                luna_client::is_luna_session_expired,
            )
            .await
            {
                Ok(h) => h,
                Err(e) => {
                    log::warn!("Luna course page failed for {}: {}", luna_id, e);
                    continue;
                }
            };

            let course_data = luna_parser::parse_luna_course_contents(&course_html, &luna_id);
            let new_announcements = course_data
                .announcements
                .iter()
                .filter(|a| a.is_new)
                .count() as i32;
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

            let (reports, exams, discussions) = match client::fetch_with_redirect(
                &luna_http,
                &contents_url,
                config::LUNA_BASE,
                luna_client::LUNA_SESSION_EXPIRED_MSG,
                luna_client::is_luna_session_expired,
            )
            .await
            {
                Ok(html) => {
                    let (materials, reps, exs, discs, _surveys) =
                        luna_parser::parse_luna_contents_page(&html);

                    // Store material items
                    for m in &materials {
                        activities.push(LunaActivityRow {
                            luna_id: luna_id.clone(),
                            activity_type: "material".into(),
                            title: m.title.clone(),
                            period: m.period.clone(),
                            status: m.status.clone(),
                            detail_path: m.url.clone(),
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
                            detail_path: r.url.clone(),
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
                            detail_path: e.url.clone(),
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
                            detail_path: d.url.clone(),
                        });
                    }

                    let pending_reports =
                        reps.iter().filter(|r| r.status.contains("未提出")).count() as i32;
                    let pending_exams = exs
                        .iter()
                        .filter(|e| e.status.contains("未回答") || e.status.contains("未受験"))
                        .count() as i32;
                    (pending_reports, pending_exams, discs.len() as i32)
                }
                Err(e) => {
                    log::warn!("Luna contents failed for {}: {}", luna_id, e);
                    (0, 0, 0)
                }
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
