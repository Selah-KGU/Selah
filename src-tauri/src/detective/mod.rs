//! Detective game backend — the FLOW layer (Tauri commands + context build).
//!
//! Submodules: `config` (tuning), `types` (data), `sources` (DB/source assembly),
//! `prompts` (AI prompts), `validate` (draft validation), `generate` (AI pipeline).
//! Command bodies live in `context`, `campaign_flow`, `chapters`, and `saves`.
use crate::db::Database;
#[allow(unused_imports)]
use serde::Deserialize;
#[allow(unused_imports)]
use std::collections::{HashMap, HashSet};

mod campaign_flow;
mod chapters;
mod config;
mod context;
mod generate;
mod prompts;
mod saves;
mod sources;
mod types;
mod validate;

pub(crate) use config::*;
pub(crate) use generate::*;
pub(crate) use prompts::*;
pub(crate) use sources::*;
pub(crate) use types::*;
pub(crate) use validate::*;

#[tauri::command]
pub fn detective_get_context(db: tauri::State<'_, Database>) -> Result<DetectiveContext, String> {
    context::build_context(&db)
}

#[tauri::command]
pub async fn detective_generate_campaign(
    db: tauri::State<'_, Database>,
    course_key: String,
    force: Option<bool>,
) -> Result<DetectiveCampaign, String> {
    campaign_flow::detective_generate_campaign(db, course_key, force).await
}

#[tauri::command]
pub fn detective_get_chapters(
    db: tauri::State<'_, Database>,
    course_key: String,
) -> Result<Vec<DetectiveChapterInfo>, String> {
    chapters::detective_get_chapters(db, course_key)
}

#[tauri::command]
pub async fn detective_generate_chapter(
    db: tauri::State<'_, Database>,
    course_key: String,
    live_id: String,
    force: Option<bool>,
) -> Result<DetectiveCase, String> {
    chapters::detective_generate_chapter(db, course_key, live_id, force).await
}

#[tauri::command]
pub fn detective_save_doubts(
    db: tauri::State<'_, Database>,
    doubts: Vec<DetectiveDoubt>,
) -> Result<(), String> {
    saves::detective_save_doubts(db, doubts)
}

#[tauri::command]
pub fn detective_save_included_courses(
    db: tauri::State<'_, Database>,
    included: Vec<String>,
) -> Result<(), String> {
    saves::detective_save_included_courses(db, included)
}

#[tauri::command]
pub fn detective_save_case_result(
    db: tauri::State<'_, Database>,
    result: DetectiveCaseResult,
) -> Result<Vec<DetectiveCaseResult>, String> {
    saves::detective_save_case_result(db, result)
}

#[tauri::command]
pub fn detective_save_memory_outcome(
    db: tauri::State<'_, Database>,
    busted_topics: Vec<String>,
    missed_topics: Vec<String>,
    course_name: String,
    evidence_titles: Vec<String>,
) -> Result<(), String> {
    saves::detective_save_memory_outcome(
        db,
        busted_topics,
        missed_topics,
        course_name,
        evidence_titles,
    )
}

#[tauri::command]
pub async fn detective_finalize_finale(
    db: tauri::State<'_, Database>,
    course_key: String,
) -> Result<DetectiveCampaign, String> {
    campaign_flow::detective_finalize_finale(db, course_key).await
}
