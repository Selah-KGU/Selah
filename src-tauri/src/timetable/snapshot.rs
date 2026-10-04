use super::*;

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
pub async fn get_schedule_snapshot(app: tauri::AppHandle) -> Result<ScheduleResponse, String> {
    tokio::task::spawn_blocking(move || build_schedule_snapshot(&app))
        .await
        .map_err(|err| format!("時間割スナップショットの読み込みに失敗しました: {err}"))?
}

pub(crate) fn build_schedule_snapshot(app: &tauri::AppHandle) -> Result<ScheduleResponse, String> {
    let db = app.state::<Database>();
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
