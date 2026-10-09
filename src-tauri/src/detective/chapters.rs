use super::config::chapter_plan;
use super::context::build_context;
use super::generate::{
    build_case, build_chapter_input, ensure_campaign, ensure_knowledge_points,
    generate_case_with_ai,
};
use super::sources::{
    load_align, load_campaign, load_case_results, load_chapter_case, load_memory,
    offline_session_nums, planned_sessions, save_align, save_campaign, save_chapter_case,
    truncate_chars,
};
use super::types::{CanonHook, DetectiveCase, DetectiveChapterInfo, DetectiveLiveRecord};
use crate::db::Database;
use std::collections::HashSet;

/// List a course's chapters — one per Live note, oldest lecture first. Each
/// chapter reports whether it has been generated and/or played. New lectures
/// surface here automatically as fresh (ungenerated) chapters.
pub(super) fn detective_get_chapters(
    db: crate::db::AccountDb,
    course_key: String,
) -> Result<Vec<DetectiveChapterInfo>, String> {
    let context = build_context(&db)?;
    let course = context
        .courses
        .iter()
        .find(|course| course.key == course_key)
        .ok_or_else(|| "選んだ科目にはライブメモが見つかりませんでした。".to_string())?;
    let results = load_case_results(&db);
    let align = load_align(&db, &course_key);

    let mut records: Vec<&DetectiveLiveRecord> = course.live_records.iter().collect();
    records.sort_by(|a, b| a.downloaded_at.cmp(&b.downloaded_at)); // download order

    // Build one row per captured Live note, numbered by its content-aligned
    // 第N回 (not by capture order — robust to missing/online lectures).
    let mut rows: Vec<DetectiveChapterInfo> = Vec::new();
    for record in &records {
        let case_id = format!("detective:chapter:{course_key}:{}", record.id);
        let cached = load_chapter_case(&db, &course_key, &record.id);
        let result = results.iter().find(|r| r.case_id == case_id);
        let session_num = align.get(&record.id).copied().unwrap_or(0).max(0) as u8;
        let aligned = session_num > 0;
        let title = cached
            .as_ref()
            .map(|c| c.title.clone())
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| {
                if aligned {
                    format!("第{session_num}章")
                } else {
                    "未整理の記録".to_string()
                }
            });
        rows.push(DetectiveChapterInfo {
            live_id: record.id.clone(),
            index: session_num,
            title,
            generated: cached.is_some(),
            played: result.is_some(),
            best_confidence: result.map(|r| r.confidence).unwrap_or(0),
            played_at: result.map(|r| r.closed_at).unwrap_or(0),
            locked: false,
            aligned,
        });
    }
    // Aligned rows first (by 回), then not-yet-aligned notes (by capture order).
    rows.sort_by(|a, b| match (a.aligned, b.aligned) {
        (true, true) => a.index.cmp(&b.index),
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => std::cmp::Ordering::Equal,
    });

    // Append OFFLINE 授業計画 回 that no captured note covers, as locked 未配信
    // rows — only the future tail, so existing-but-unaligned notes aren't
    // double-counted. Online 回 are intentionally omitted (they bear no Live).
    let offline = offline_session_nums(&db, &course_key);
    if !offline.is_empty() {
        let covered: std::collections::HashSet<i32> = rows
            .iter()
            .filter(|r| r.aligned)
            .map(|r| r.index as i32)
            .collect();
        let uncovered: Vec<i32> = offline
            .iter()
            .copied()
            .filter(|n| !covered.contains(n))
            .collect();
        let show = offline.len().saturating_sub(records.len());
        let start = uncovered.len().saturating_sub(show);
        for &n in &uncovered[start..] {
            rows.push(DetectiveChapterInfo {
                live_id: String::new(),
                index: n.max(0) as u8,
                title: format!("第{n}章（未配信）"),
                generated: false,
                played: false,
                best_confidence: 0,
                played_at: 0,
                locked: true,
                aligned: true,
            });
        }
    }
    Ok(rows)
}

/// Generate (or fetch the cached) case for one chapter = one Live note. The
/// case is set inside the course's campaign world. Cached after first build so
/// replays are instant; pass `force: true` to regenerate from scratch.
pub(super) async fn detective_generate_chapter(
    db: crate::db::AccountDb,
    course_key: String,
    live_id: String,
    force: Option<bool>,
) -> Result<DetectiveCase, String> {
    if !force.unwrap_or(false) {
        if let Some(cached) = load_chapter_case(&db, &course_key, &live_id) {
            return Ok(cached);
        }
    }
    let context = build_context(&db)?;
    let course = context
        .courses
        .iter()
        .find(|course| course.key == course_key)
        .ok_or_else(|| "選んだ科目にはライブメモが見つかりませんでした。".to_string())?;
    let input = build_chapter_input(course, &live_id)
        .ok_or_else(|| "そのライブメモが見つかりませんでした。".to_string())?;

    let mut case = build_case(course);
    case.id = format!("detective:chapter:{course_key}:{live_id}");
    case.course_key = course_key.clone();

    let memory = load_memory(&db);
    let campaign = ensure_campaign(&db, course).await.ok();
    // Derive this chapter's authoring plan (which 暗线 stage to advance + which
    // dramatic archetype) from its position in the season, NOT play order.
    let plan = chapter_plan(course, &live_id, campaign.as_ref());
    let syllabus = planned_sessions(&db, &course_key);
    // Pre-extract a knowledge-point checklist from this Live note (cached per
    // live_id) so chapter generation is driven by — and validated against —
    // the actual concepts the lecture covered. The Live note's own structure
    // (箇条書き / 「今日のポイント」 / 「まとめ」) is the primary signal; no
    // separate topic hint needed here.
    let knowledge = ensure_knowledge_points(&db, course, &live_id, "").await?;
    let case =
        generate_case_with_ai(case, input, memory, campaign, syllabus, knowledge, plan).await?;
    save_chapter_case(&db, &course_key, &live_id, &case);
    // Persist the content-derived 回 alignment so the chapter list can number
    // chapters by their真の第N回 (robust to missing/online lectures).
    if case.session_num > 0 {
        let mut align = load_align(&db, &course_key);
        align.insert(live_id.clone(), case.session_num as i32);
        save_align(&db, &course_key, &align);
    }
    // Fold this chapter's 暗线 beat + established truth into the campaign canon
    // so later (independently generated) chapters share one coherent world.
    record_chapter_canon(&db, &course_key, &case);
    Ok(case)
}

/// Fold a freshly generated chapter's 暗线 beat + truth into the campaign canon
/// (weak-continuity: chapters are independent, but read a shared, deduped,
/// bounded canon so they stay mutually consistent). No-op until the chapter
/// generator actually emits `meta_beat` / `case_logic` (Phase 2).
fn record_chapter_canon(db: &Database, course_key: &str, case: &DetectiveCase) {
    let Some(mut campaign) = load_campaign(db, course_key) else {
        return;
    };
    let mut changed = false;

    let beat = case.meta_beat.trim();
    if !beat.is_empty()
        && !campaign
            .canon
            .dropped_hooks
            .iter()
            .any(|h| h.chapter_id == case.id || h.hook == beat)
    {
        campaign.canon.dropped_hooks.push(CanonHook {
            stage: 0,
            hook: truncate_chars(beat, 200),
            chapter_id: case.id.clone(),
        });
        changed = true;
    }

    let truth = case.case_logic.truth.trim();
    if !truth.is_empty() {
        let fact = truncate_chars(truth, 200);
        if !campaign.canon.facts.iter().any(|f| f == &fact) {
            campaign.canon.facts.push(fact);
            changed = true;
        }
    }

    // Log recurring-cast appearances: any testimony witness whose name matches a
    // bible cast member becomes a continuity entry for later chapters.
    let cast_names: Vec<String> = campaign
        .cast
        .iter()
        .map(|m| m.name.trim().to_string())
        .collect();
    let chapter_label = if case.title.trim().is_empty() {
        "ある章".to_string()
    } else {
        truncate_chars(case.title.trim(), 40)
    };
    let mut seen_here: HashSet<String> = HashSet::new();
    for act in &case.acts {
        let w = act.witness_name.trim();
        if w.is_empty() || !seen_here.insert(w.to_string()) {
            continue;
        }
        if cast_names.iter().any(|n| !n.is_empty() && n == w) {
            let entry = format!("{w}: 「{chapter_label}」に登場");
            if !campaign.canon.cast_log.iter().any(|e| e == &entry) {
                campaign.canon.cast_log.push(entry);
                changed = true;
            }
        }
    }

    // Bound growth (keep most recent).
    if campaign.canon.facts.len() > 40 {
        let cut = campaign.canon.facts.len() - 40;
        campaign.canon.facts.drain(0..cut);
        changed = true;
    }
    if campaign.canon.dropped_hooks.len() > 30 {
        let cut = campaign.canon.dropped_hooks.len() - 30;
        campaign.canon.dropped_hooks.drain(0..cut);
        changed = true;
    }
    if campaign.canon.cast_log.len() > 30 {
        let cut = campaign.canon.cast_log.len() - 30;
        campaign.canon.cast_log.drain(0..cut);
        changed = true;
    }

    if changed {
        campaign.updated_at = crate::db::epoch_secs();
        save_campaign(db, &campaign);
    }
}
