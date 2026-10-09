use super::*;

use super::ai_analysis::load_ai_cache_with_snapshot;
use crate::db::{AiScheduleResult, Database, ScheduleRawData};
use crate::luna_parser;
use serde::Serialize;
use tauri::Manager;

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
/// SQLite reads run on the blocking pool so a large snapshot cannot stall async commands.
#[tauri::command]
pub async fn get_schedule_snapshot<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<tauri::ipc::Response, String> {
    let db = app.state::<Database>().scope();
    crate::background_ipc::respond(
        "時間割スナップショットの読み込みに失敗しました",
        "時間割応答の変換に失敗しました",
        move || build_schedule_snapshot_from_db(&db),
    )
    .await
}

pub(crate) fn build_schedule_snapshot<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<ScheduleResponse, String> {
    build_schedule_snapshot_from_db(&app.state::<Database>().scope())
}

pub(crate) fn build_schedule_snapshot_from_db(db: &Database) -> Result<ScheduleResponse, String> {
    let saved_snapshot = db.get_snapshot_state()?;
    let has_saved_snapshot = saved_snapshot.is_some();
    let snap = saved_snapshot.unwrap_or_default();
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
    let (ai_result, ai_stale) =
        load_ai_cache_with_snapshot(&db, has_saved_snapshot.then_some(&snap))?;
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
