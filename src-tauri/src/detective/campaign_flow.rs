use super::context::build_context;
use super::generate::{
    build_evidence_input, detective_ai_json, ensure_campaign, generate_campaign_bible,
};
use super::prompts::{finale_system_prompt, finale_user_prompt};
use super::sources::{chapter_case_key, load_campaign, save_campaign};
use super::types::DetectiveCampaign;
use serde::Deserialize;

/// Generate (or fetch the cached) campaign bible for a single course. With
/// `force: true` the world is rebuilt from scratch (e.g. to backfill the
/// meta-arc / finale onto an older campaign) while carrying over the player's
/// meta_progress and played chapters; reveals are re-unlocked to match.
pub(super) async fn detective_generate_campaign(
    db: crate::db::AccountDb,
    course_key: String,
    force: Option<bool>,
) -> Result<DetectiveCampaign, String> {
    let context = build_context(&db)?;
    let course = context
        .courses
        .iter()
        .find(|course| course.key == course_key)
        .ok_or_else(|| "選んだ科目にはライブメモ／通知が見つかりませんでした。".to_string())?;

    if !force.unwrap_or(false) {
        return ensure_campaign(&db, course).await;
    }

    let prev = load_campaign(&db, &course.key);
    let input = build_evidence_input(course);
    let mut fresh = generate_campaign_bible(course.key.clone(), course.name.clone(), input).await?;
    if let Some(prev) = prev {
        fresh.meta_progress = prev.meta_progress;
        fresh.chapters = prev.chapters;
        fresh.created_at = prev.created_at;
        for rev in fresh.meta_arc.iter_mut() {
            if rev.threshold <= fresh.meta_progress {
                rev.unlocked = true;
            }
        }
    }
    save_campaign(&db, &fresh);
    // The world changed — drop cached chapter cases so they regenerate inside
    // the new world on next play. Progress/clear status survive (they live in
    // case results + the campaign, keyed by the stable case_id, not the case).
    for record in &course.live_records {
        let _ = db.delete_data_cache(&chapter_case_key(&course.key, &record.id));
    }
    Ok(fresh)
}

/// Rewrite a completed campaign's finale to pay off the REAL accumulated canon
/// (chapter beats, established facts, the staged 暗线 reveals) rather than the
/// static guess written at bible time. Only fires once a campaign hits 100%;
/// the frontend calls this when a chapter clear pushes progress to completion.
pub(super) async fn detective_finalize_finale(
    db: crate::db::AccountDb,
    course_key: String,
) -> Result<DetectiveCampaign, String> {
    let Some(mut campaign) = load_campaign(&db, &course_key) else {
        return Err("この科目の世界観がまだありません。".to_string());
    };
    if campaign.meta_progress < 100 {
        return Ok(campaign);
    }
    let cfg = crate::ai::load_ai_config();
    if !cfg.ai_enabled {
        return Ok(campaign); // keep the static finale when AI is off
    }
    let provider = crate::agent_provider::AgentProvider::resolve()
        .map_err(|e| format!("AI provider unavailable: {e}"))?;
    let user = finale_user_prompt(&campaign);
    let json = detective_ai_json(
        &provider,
        cfg.max_tokens,
        finale_system_prompt(),
        user,
        0.5,
        20,
        &format!("detective-finale:{course_key}"),
    )
    .await?;
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FinaleDraft {
        finale: Option<String>,
        reveals: Option<Vec<RevealRefine>>,
    }
    #[derive(Deserialize)]
    struct RevealRefine {
        stage: Option<u8>,
        reveal: Option<String>,
    }
    let draft: FinaleDraft =
        serde_json::from_str(&json).map_err(|e| format!("Finale JSON parse failed: {e}"))?;
    let finale = draft.finale.unwrap_or_default().trim().to_string();
    if finale.is_empty() {
        return Ok(campaign);
    }
    campaign.finale = finale;
    // Rewrite each stage's player-facing reveal to match the canon that actually
    // accumulated, so the staged 暗线 reveals cohere with the real story.
    for refine in draft.reveals.unwrap_or_default() {
        let (Some(stage), Some(text)) = (refine.stage, refine.reveal) else {
            continue;
        };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        if let Some(rev) = campaign.meta_arc.iter_mut().find(|r| r.stage == stage) {
            rev.reveal = text.chars().take(280).collect();
        }
    }
    campaign.updated_at = crate::db::epoch_secs();
    save_campaign(&db, &campaign);
    eprintln!("[detective] finale + reveals finalized for {course_key}");
    Ok(campaign)
}
