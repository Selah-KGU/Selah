use super::config::{DETECTIVE_DOUBTS_KEY, DETECTIVE_INCLUDED_KEY, DETECTIVE_RESULTS_KEY};
use super::context::build_context;
use super::sources::{
    campaign_progress_pct, load_campaign, load_case_results, load_memory, save_campaign,
    save_memory, truncate_chars,
};
use super::types::{CampaignChapter, DetectiveCaseResult, DetectiveDoubt, MemoryItem};

pub(super) fn detective_save_doubts(
    db: crate::db::AccountDb,
    doubts: Vec<DetectiveDoubt>,
) -> Result<(), String> {
    let json = serde_json::to_string(&doubts).map_err(|e| format!("Detective doubts JSON: {e}"))?;
    db.save_data_cache(DETECTIVE_DOUBTS_KEY, &json)
}

pub(super) fn detective_save_included_courses(
    db: crate::db::AccountDb,
    included: Vec<String>,
) -> Result<(), String> {
    let mut clean: Vec<String> = included
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    clean.sort();
    clean.dedup();
    let json =
        serde_json::to_string(&clean).map_err(|e| format!("Detective included JSON: {e}"))?;
    db.save_data_cache(DETECTIVE_INCLUDED_KEY, &json)
}

pub(super) fn detective_save_case_result(
    db: crate::db::AccountDb,
    result: DetectiveCaseResult,
) -> Result<Vec<DetectiveCaseResult>, String> {
    let anchor_key = result
        .course_key
        .split(',')
        .next()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(|s| s.to_string());

    // Persist the result FIRST so progress recompute below counts this chapter.
    let mut results = load_case_results(&db);
    results.retain(|item| item.id != result.id);
    let result_for_campaign = result.clone();
    results.insert(0, result);
    results.sort_by(|a, b| b.closed_at.cmp(&a.closed_at));
    results.truncate(80);
    let json =
        serde_json::to_string(&results).map_err(|e| format!("Detective case result JSON: {e}"))?;
    db.save_data_cache(DETECTIVE_RESULTS_KEY, &json)?;

    // Advance the campaign meta-plot. Progress = fraction of the course's
    // OFFLINE 授業計画 回 (the ones that actually produce a Live note) that have
    // an aligned, cleared chapter — so 100% fires only when the final offline
    // 回 is done. Online/on-demand 回 are excluded. Dedup chapters by case_id.
    if let Some(anchor_key) = anchor_key {
        if let Some(mut campaign) = load_campaign(&db, &anchor_key) {
            let is_new = !campaign
                .chapters
                .iter()
                .any(|ch| ch.id == result_for_campaign.case_id);
            if is_new {
                let title = if result_for_campaign.case_title.trim().is_empty() {
                    format!("第{}章", campaign.chapters.len() + 1)
                } else {
                    result_for_campaign.case_title.trim().to_string()
                };
                campaign.chapters.push(CampaignChapter {
                    id: result_for_campaign.case_id.clone(),
                    title,
                    summary: truncate_chars(result_for_campaign.deduction.trim(), 160),
                    played_at: result_for_campaign.closed_at,
                });
            }
            // Recompute against the 授業計画 (offline 回). Fall back to a coarse
            // chapters-cleared / captured-Live ratio when no syllabus exists.
            campaign.meta_progress = campaign_progress_pct(&db, &anchor_key)
                .or_else(|| {
                    build_context(&db).ok().and_then(|ctx| {
                        ctx.courses
                            .iter()
                            .find(|c| c.key == anchor_key)
                            .map(|c| c.live_records.len())
                            .filter(|n| *n > 0)
                            .map(|total| {
                                let cleared = campaign.chapters.len().min(total);
                                ((cleared as f32 / total as f32) * 100.0).round() as u8
                            })
                    })
                })
                .unwrap_or_else(|| campaign.meta_progress.saturating_add(12))
                .min(100);
            // Unlock any reveal whose threshold the player has now reached.
            for rev in campaign.meta_arc.iter_mut() {
                if !rev.unlocked && rev.threshold <= campaign.meta_progress {
                    rev.unlocked = true;
                }
            }
            campaign.updated_at = crate::db::epoch_secs();
            save_campaign(&db, &campaign);
        }
    }

    Ok(results)
}

/// Record per-session outcomes that drive cross-session continuity. The
/// frontend calls this once the player closes a session (win or loss).
pub(super) fn detective_save_memory_outcome(
    db: crate::db::AccountDb,
    busted_topics: Vec<String>,
    missed_topics: Vec<String>,
    course_name: String,
    evidence_titles: Vec<String>,
) -> Result<(), String> {
    let mut memory = load_memory(&db);
    let now = crate::db::epoch_secs();
    let course = course_name.trim().to_string();

    for topic in busted_topics {
        let t = topic.trim();
        if t.is_empty() {
            continue;
        }
        // Drop from mistakes if it was there (player has now resolved it).
        memory.mistakes.retain(|m| m.topic != t);
        memory.mastered.push(MemoryItem {
            topic: t.to_string(),
            course_name: course.clone(),
            at: now,
        });
    }
    for topic in missed_topics {
        let t = topic.trim();
        if t.is_empty() {
            continue;
        }
        // Remove any older mastered claim — clearly not mastered now.
        memory.mastered.retain(|m| m.topic != t);
        memory.mistakes.push(MemoryItem {
            topic: t.to_string(),
            course_name: course.clone(),
            at: now,
        });
    }
    for title in evidence_titles {
        let t = title.trim();
        if !t.is_empty() {
            memory.recent_evidence_titles.push(t.to_string());
        }
    }

    // Sliding windows: keep recency, drop ancient items.
    if memory.mastered.len() > 40 {
        let cut = memory.mastered.len() - 40;
        memory.mastered.drain(0..cut);
    }
    if memory.mistakes.len() > 24 {
        let cut = memory.mistakes.len() - 24;
        memory.mistakes.drain(0..cut);
    }
    if memory.recent_evidence_titles.len() > 60 {
        let cut = memory.recent_evidence_titles.len() - 60;
        memory.recent_evidence_titles.drain(0..cut);
    }
    memory.updated_at = now;
    save_memory(&db, &memory);
    Ok(())
}
