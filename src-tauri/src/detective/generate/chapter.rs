use super::super::*;

/// Build an empty case shell. The AI is responsible for filling every
/// content-bearing field including the evidence cards themselves.
pub(crate) fn build_case(course: &DetectiveCourse) -> DetectiveCase {
    DetectiveCase {
        id: format!("detective:{}:{}", course.key, crate::db::epoch_secs()),
        course_key: course.key.clone(),
        course_name: course.name.clone(),
        title: String::new(),
        case_type: String::new(),
        difficulty: 0,
        briefing: String::new(),
        evidence: Vec::new(),
        final_question: String::new(),
        testimony: Vec::new(),
        scenario: String::new(),
        witness_name: String::new(),
        witness_role: String::new(),
        generation_mode: "ai".to_string(),
        generation_note: String::new(),
        session_num: 0,
        knowledge_points: Vec::new(),
        coverage: Vec::new(),
        acts: Vec::new(),
        case_logic: CaseLogic::default(),
        meta_beat: String::new(),
    }
}

/// Extract a short, meaningful content snippet from an evidence excerpt.
/// Skips metadata lines and bullet markers; returns 18-70 char chunks.
pub(crate) fn content_snippet(excerpt: &str) -> Option<String> {
    let cleaned = excerpt
        .lines()
        .map(|line| {
            line.trim()
                .trim_start_matches('#')
                .trim()
                .trim_start_matches('-')
                .trim_start_matches('*')
                .trim()
                .trim_start_matches('・')
                .trim()
        })
        .filter(|line| !line.is_empty());

    // Try sentence-level splits first.
    for line in cleaned.clone() {
        for sentence in line.split(['。', '．', '！', '？']) {
            let s = sentence.trim();
            let len = s.chars().count();
            if (18..=70).contains(&len) && !looks_like_metadata(s) {
                return Some(s.to_string());
            }
        }
    }
    // Fall back to first line of decent length.
    for line in cleaned {
        let len = line.chars().count();
        if (12..=70).contains(&len) && !looks_like_metadata(line) {
            return Some(line.to_string());
        }
    }
    None
}

pub(crate) fn looks_like_metadata(text: &str) -> bool {
    matches!(
        text,
        "区間ごとの要約" | "全文転写" | "授業コード" | "教員" | "教室" | "時間帯"
    ) || text.starts_with("http")
        || text.starts_with("授業コード:")
        || text.starts_with("教員:")
        || text.starts_with("教室:")
        || text.starts_with("開始:")
        || text.starts_with("終了:")
}

/// Run one AI turn that must return a single JSON object, and extract it.
/// Shared by the three chapter-generation passes.
pub(crate) async fn detective_ai_json(
    provider: &crate::agent_provider::AgentProvider,
    max_tokens: u32,
    system: &str,
    user: String,
    temperature: f32,
    think_budget_pct: u32,
    tag: &str,
) -> Result<String, String> {
    let messages = vec![
        crate::ai::ChatMessage {
            role: "system".to_string(),
            content: system.to_string(),
            images: Vec::new(),
        },
        crate::ai::ChatMessage {
            role: "user".to_string(),
            content: user,
            images: Vec::new(),
        },
    ];
    let raw = provider
        .plan(messages, max_tokens, temperature, "", think_budget_pct, tag)
        .await
        .map_err(|e| format!("AI call failed: {e}"))?;
    eprintln!("[detective] {tag} raw BEGIN ===\n{}\n=== END", raw);
    extract_json_object(&raw).ok_or_else(|| {
        let preview = truncate_chars(raw.trim(), 300);
        if raw.trim().is_empty() {
            "AI returned an empty response (the model may have refused or been blocked)."
                .to_string()
        } else {
            format!("AI did not return JSON. The response begins with: \"{preview}\"")
        }
    })
}

/// Generate a Detective chapter via a three-pass AI pipeline (outline → draft →
/// editor). There is no silent fallback to a worse case: the outline + draft
/// passes are required and surface `Err` on failure; the editor pass only ever
/// *improves* the draft and never regresses it.
///   Pass A — outline: the 推理 spine (truth/culprit/motive/red herrings/
///     deduction chain) + act plan + 暗线 beat. Logic only, no prose.
///   Pass B — draft: the full chapter written to conform to the outline.
///   Pass C — editor: a consistency critique that may return a repaired draft.
/// The hard structural + content + coverage gate (`apply_ai_case_draft`) runs
/// on whichever draft we keep.
pub(crate) async fn generate_case_with_ai(
    case: DetectiveCase,
    input: Vec<EvidenceInputEntry>,
    memory: DetectiveMemory,
    campaign: Option<DetectiveCampaign>,
    syllabus: Vec<PlannedSession>,
    knowledge: Vec<KnowledgePoint>,
    plan: ChapterPlan,
) -> Result<DetectiveCase, String> {
    let cfg = crate::ai::load_ai_config();
    eprintln!(
        "[detective] generate_case_with_ai (3-pass): course={} ai_enabled={} input_sources={}",
        case.course_name,
        cfg.ai_enabled,
        input.len()
    );

    if !cfg.ai_enabled {
        return Err(
            "AI is disabled. Detective cases require an AI provider — enable AI in Selah settings."
                .to_string(),
        );
    }
    if input.is_empty() {
        return Err(
            "No Live notes or exam signals are available for this course. Capture a Live session or wait for notifications, then try again.".to_string(),
        );
    }

    let provider = crate::agent_provider::AgentProvider::resolve().map_err(|e| {
        eprintln!("[detective] provider resolve FAILED: {}", e);
        format!("AI provider unavailable: {e}")
    })?;

    // Scale the chapter's structure targets to how much testable content this
    // Live note actually yielded — thin notes get a tighter (but real) chapter.
    let targets = gen_targets(knowledge.len());
    eprintln!(
        "[detective] targets: {} knowledge pts → acts {}–{}, coverage {}, lies {}",
        knowledge.len(),
        targets.acts_min,
        targets.acts_max,
        targets.coverage_min,
        targets.lies_min
    );

    eprintln!(
        "[detective] plan: arc_focus={:?}/{} archetype={}",
        plan.arc_focus, plan.arc_total, plan.archetype
    );

    // ── Pass A — outline (推理 & 暗线 skeleton) ─────────────────────────────
    let outline_user = detective_outline_user_prompt(
        &case,
        &input,
        &memory,
        campaign.as_ref(),
        &syllabus,
        &knowledge,
        &targets,
        &plan,
    );
    eprintln!("[detective] PASS A (outline) dispatching…");
    let outline_json = detective_ai_json(
        &provider,
        cfg.max_tokens,
        detective_outline_system_prompt(),
        outline_user,
        0.4,
        25,
        &format!("detective-outline:{}", case.id),
    )
    .await
    .map_err(|e| format!("案件の推理プロットの生成に失敗しました（Pass A）: {e}"))?;
    let outline: CaseOutlineDraft = serde_json::from_str(&outline_json)
        .map_err(|e| format!("Outline JSON parse failed (Pass A): {e}"))?;

    // ── Pass B — draft (full chapter prose conforming to the outline) ──────
    let draft_user = format!(
        "{base}\n\n═══ 承認済みプロット（このスケルトンに厳密に従う） ═══\n{outline}\n\n上のプロットが合意済みの推理スパイン＋幕構成です。本章を執筆する際は必ず: (1) 幕の数と種類が `actPlan` と一致する。(2) 各 testimony 幕で仕込む唯一の嘘は、その幕の `lieAbout` が指す“既に教えた事実”を歪める。(3) `coveragePlan` の知識点 id をすべて被覆する。(4) `caseLogic`（実際に書いた内容に合わせて微調整可）と `metaBeat` をトップレベルで返す。(5) CAMPAIGN WORLD・世界の正典・投下済みの伏線とすべて整合させる。",
        base = detective_ai_user_prompt(&case, &input, &memory, campaign.as_ref(), &syllabus, &knowledge, &targets, &plan),
        outline = outline_json,
    );
    eprintln!(
        "[detective] PASS B (draft) dispatching ({} chars)…",
        draft_user.chars().count()
    );
    let draft_json = detective_ai_json(
        &provider,
        cfg.max_tokens,
        detective_ai_system_prompt(),
        draft_user,
        0.2,
        20,
        &format!("detective:{}", case.id),
    )
    .await
    .map_err(|e| format!("案件の本文生成に失敗しました（Pass B）: {e}"))?;

    // Parse + apply + hard-validate a draft JSON onto a fresh case shell.
    let parse_apply = |json: &str, base: DetectiveCase| -> Result<DetectiveCase, String> {
        let d: DetectiveAiCaseDraft =
            serde_json::from_str(json).map_err(|e| format!("draft JSON parse failed: {e}"))?;
        apply_ai_case_draft(base, d, &input, &knowledge, &targets)
    };
    let applied = parse_apply(&draft_json, case.clone())?;
    eprintln!(
        "[detective] PASS B accepted: acts={} evidence={} testimony={}",
        applied.acts.len(),
        applied.evidence.len(),
        applied.testimony.len()
    );

    // ── Pass C — editor critique / repair (best-effort, never regresses) ───
    let mut final_case = applied;
    match detective_editor_pass(
        &provider,
        &cfg,
        &draft_json,
        &outline_json,
        campaign.as_ref(),
        &knowledge,
        plan.arc_focus,
        &case.id,
    )
    .await
    {
        Ok(Some(patched_json)) => match parse_apply(&patched_json, case.clone()) {
            Ok(better) => {
                eprintln!("[detective] PASS C: editor repair applied + revalidated");
                final_case = better;
            }
            Err(e) => eprintln!("[detective] PASS C: repair rejected ({e}); keeping Pass B draft"),
        },
        Ok(None) => eprintln!("[detective] PASS C: editor reports no changes needed"),
        Err(e) => eprintln!("[detective] PASS C skipped (non-fatal): {e}"),
    }

    // Backfill the logic spine / 暗线 beat / session number from the outline if
    // the draft didn't echo them.
    if final_case.case_logic.truth.trim().is_empty() {
        if let Some(logic) = outline.case_logic {
            final_case.case_logic = resolve_case_logic(logic);
        }
    }
    if final_case.meta_beat.trim().is_empty() {
        if let Some(beat) = clean_ai_text(outline.meta_beat, 300)
            .filter(|t| !looks_like_metadata_leak(t) && !looks_like_platitude(t))
        {
            final_case.meta_beat = beat;
        }
    }
    if final_case.session_num == 0 {
        final_case.session_num = outline.session_num.unwrap_or(0).clamp(0, 99) as u8;
    }

    eprintln!(
        "[detective] case ACCEPTED: acts={} evidence={} culprit={:?} chain={}",
        final_case.acts.len(),
        final_case.evidence.len(),
        final_case.case_logic.culprit,
        final_case.case_logic.deduction_chain.len()
    );
    Ok(final_case)
}

/// Pass C: ask an editor model to critique the draft for logical consistency,
/// motive traceability, fair-play clueing, 暗线-stage fit, and canon consistency.
/// Returns `Ok(Some(json))` with a fully-repaired draft when it found problems,
/// `Ok(None)` when the draft is already clean, or `Err` on AI failure.
pub(crate) async fn detective_editor_pass(
    provider: &crate::agent_provider::AgentProvider,
    cfg: &crate::ai::AiConfig,
    draft_json: &str,
    outline_json: &str,
    campaign: Option<&DetectiveCampaign>,
    knowledge: &[KnowledgePoint],
    arc_focus: Option<u8>,
    case_id: &str,
) -> Result<Option<String>, String> {
    let user =
        detective_editor_user_prompt(draft_json, outline_json, campaign, knowledge, arc_focus);
    let json = detective_ai_json(
        provider,
        cfg.max_tokens,
        detective_editor_system_prompt(),
        user,
        0.1,
        15,
        &format!("detective-editor:{case_id}"),
    )
    .await?;
    #[derive(Deserialize)]
    struct Env {
        ok: Option<bool>,
    }
    if let Ok(env) = serde_json::from_str::<Env>(&json) {
        if env.ok == Some(true) {
            return Ok(None);
        }
    }
    Ok(Some(json))
}
