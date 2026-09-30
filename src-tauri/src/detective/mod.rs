//! Detective game backend — the FLOW layer (Tauri commands + context build).
//!
//! Submodules: `config` (tuning), `types` (data), `sources` (DB/source assembly),
//! `prompts` (AI prompts), `validate` (draft validation), `generate` (AI pipeline).
use crate::db::Database;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

mod config;
mod generate;
mod prompts;
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
    build_context(&db)
}

/// Generate (or fetch the cached) campaign bible for a single course. With
/// `force: true` the world is rebuilt from scratch (e.g. to backfill the
/// meta-arc / finale onto an older campaign) while carrying over the player's
/// meta_progress and played chapters; reveals are re-unlocked to match.
#[tauri::command]
pub async fn detective_generate_campaign(
    db: tauri::State<'_, Database>,
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

/// List a course's chapters — one per Live note, oldest lecture first. Each
/// chapter reports whether it has been generated and/or played. New lectures
/// surface here automatically as fresh (ungenerated) chapters.
#[tauri::command]
pub fn detective_get_chapters(
    db: tauri::State<'_, Database>,
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
#[tauri::command]
pub async fn detective_generate_chapter(
    db: tauri::State<'_, Database>,
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

#[tauri::command]
pub fn detective_save_doubts(
    db: tauri::State<'_, Database>,
    doubts: Vec<DetectiveDoubt>,
) -> Result<(), String> {
    let json = serde_json::to_string(&doubts).map_err(|e| format!("Detective doubts JSON: {e}"))?;
    db.save_data_cache(DETECTIVE_DOUBTS_KEY, &json)
}

#[tauri::command]
pub fn detective_save_included_courses(
    db: tauri::State<'_, Database>,
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

#[tauri::command]
pub fn detective_save_case_result(
    db: tauri::State<'_, Database>,
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
#[tauri::command]
pub fn detective_save_memory_outcome(
    db: tauri::State<'_, Database>,
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

fn build_context(db: &Database) -> Result<DetectiveContext, String> {
    let doubts = load_doubts(db);
    let results = load_case_results(db);
    let mut courses: HashMap<String, CourseBuilder> = HashMap::new();

    let mut schedule_items = Vec::new();
    let snap = db.get_snapshot_state().ok().flatten();
    let scope = snap.as_ref().map(|snap| {
        crate::academic_period::visible_weeks(
            &snap.current_week_label,
            &snap.next_week_label,
            &snap.luna_year,
            &snap.luna_term,
            chrono::Local::now().date_naive(),
        )
    });
    if let (Some(scope), Ok(Some((result, _)))) = (scope.as_ref(), db.get_ai_schedule_cache()) {
        if crate::academic_period::week_belongs_to_visible_label(
            &scope.current,
            &result.current_week_label,
        ) {
            schedule_items.extend(result.current_week);
        }
        if crate::academic_period::week_belongs_to_visible_label(
            &scope.next,
            &result.next_week_label,
        ) {
            schedule_items.extend(result.next_week);
        }
    }
    for item in schedule_items {
        if item.course_name.trim().is_empty() {
            continue;
        }
        let course = ensure_course(&mut courses, &item.course_name);
        course.schedule_items.push(item);
    }

    if let Some(scope) = scope.as_ref() {
        if let Some(snap) = snap {
            let mut communities = snap.luna_communities;
            communities.retain(|community| {
                crate::db::luna_course_matches_snapshot(
                    &community.idnumber,
                    &scope.year,
                    &scope.term,
                )
            });
            if let Ok(raw) = db.build_raw_data(&scope.current, &scope.next, communities) {
                for row in raw
                    .kgc_entries_current
                    .iter()
                    .chain(raw.kgc_entries_next.iter())
                {
                    if !row.name.trim().is_empty() {
                        ensure_course(&mut courses, &row.name);
                    }
                }
                for row in raw.luna_courses.iter() {
                    if !row.name.trim().is_empty() {
                        ensure_course(&mut courses, &row.name);
                    }
                }
            }
        }
    }

    let mut records = crate::commands::list_downloads();
    if !records.iter().any(is_live_record) {
        records = crate::commands::scan_download_dir();
    }
    for record in records
        .into_iter()
        .filter(|record| record.file_exists && is_live_record(record))
    {
        let course_name = if record.course_name.trim().is_empty() {
            infer_course_name_from_record(&record)
        } else {
            record.course_name.clone()
        };
        let course = ensure_course(&mut courses, &course_name);
        course.latest_at = course.latest_at.max(record.downloaded_at);
        course.live_records.push(DetectiveLiveRecord {
            id: record.id,
            filename: record.filename,
            path: record.path.clone(),
            course_name: course.name.clone(),
            downloaded_at: record.downloaded_at,
            excerpt: live_excerpt(&record.path),
        });
    }

    let signals = collect_exam_signals(db);
    for signal in signals {
        if let Some(key) = match_signal_course_key(&signal, &courses) {
            if let Some(course) = courses.get_mut(&key) {
                course.latest_at = course.latest_at.max(date_score(&signal.date));
                course.exam_signals.push(signal);
            }
        } else if !signal.course_info.trim().is_empty() {
            let course = ensure_course(&mut courses, &signal.course_info);
            course.exam_signals.push(signal);
        }
    }

    for doubt in doubts {
        let course = ensure_course(&mut courses, &doubt.course_name);
        course.doubts.push(doubt);
    }

    for result in results.iter().cloned() {
        let course = ensure_course(&mut courses, &result.course_name);
        course.latest_at = course.latest_at.max(result.closed_at);
        course.recent_results.push(result);
    }

    let mut out: Vec<DetectiveCourse> = courses
        .into_values()
        .filter(|course| {
            !course.live_records.is_empty()
                || !course.exam_signals.is_empty()
                || !course.doubts.is_empty()
                || !course.recent_results.is_empty()
        })
        .map(|mut course| {
            course
                .live_records
                .sort_by(|a, b| b.downloaded_at.cmp(&a.downloaded_at));
            course.exam_signals = dedupe_signals(course.exam_signals);
            course
                .recent_results
                .sort_by(|a, b| b.closed_at.cmp(&a.closed_at));
            course.recent_results.truncate(5);
            let case_type = course_case_type(&course).to_string();
            let readiness = readiness_text(&course);
            DetectiveCourse {
                name: course.name,
                key: course.key,
                live_records: course.live_records,
                exam_signals: course.exam_signals,
                schedule_items: course.schedule_items,
                latest_at: course.latest_at,
                doubts: course.doubts,
                recent_results: course.recent_results,
                case_type,
                readiness,
            }
        })
        .collect();

    out.sort_by(|a, b| course_score(b).total_cmp(&course_score(a)));

    let review_queue = build_review_queue(&out);

    let included_course_keys = load_included_courses(db);
    let memory = load_memory(db);

    // Surface any campaign bibles already cached for the visible courses.
    let campaigns: Vec<DetectiveCampaign> = out
        .iter()
        .filter_map(|course| load_campaign(db, &course.key))
        .collect();

    Ok(DetectiveContext {
        courses: out,
        review_queue,
        recent_results: results.into_iter().take(12).collect(),
        generated_at: crate::db::epoch_secs(),
        included_course_keys,
        memory,
        campaigns,
    })
}

/// Rewrite a completed campaign's finale to pay off the REAL accumulated canon
/// (chapter beats, established facts, the staged 暗线 reveals) rather than the
/// static guess written at bible time. Only fires once a campaign hits 100%;
/// the frontend calls this when a chapter clear pushes progress to completion.
#[tauri::command]
pub async fn detective_finalize_finale(
    db: tauri::State<'_, Database>,
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
