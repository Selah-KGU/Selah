use super::super::{
    truncate_chars, ChapterPlan, DetectiveCampaign, DetectiveCase, DetectiveMemory,
    EvidenceInputEntry, GenTargets, KnowledgePoint, PlannedSession,
};
use super::sections::{format_campaign_section, format_memory_section};

// ─── Pass A: outline (推理 & 暗线 skeleton) ────────────────────────────────

pub(crate) fn detective_outline_system_prompt() -> &'static str {
    "You are the story architect for a 逆転裁判-style study-mystery. Your job in THIS pass is ONLY the logical skeleton of one chapter — no prose, no dialogue. A separate writer will turn your outline into the finished script, so the skeleton must be airtight.\n\nOUTPUT FORMAT — MANDATORY: a single JSON object, nothing else. Start with `{` end with `}`. No code fences, no commentary.\n\nWHAT MAKES A PROFESSIONAL MYSTERY SKELETON:\n- A real 明线 (the chapter case): a concrete TRUTH of what happened, a responsible party (culprit — PREFER a campaign bible cast member), a MOTIVE with means + opportunity, and 2–3 fair-play RED HERRINGS (plausible wrong readings that the evidence later eliminates).\n- A DEDUCTION CHAIN: the ordered steps by which busting the planted contradictions reconstructs the truth; the final step answers the chapter's question. Each step must be logically entailed by an evidence card or a busted lie — no leaps, no clue from nowhere.\n- TEACHING-BEFORE-TESTING: every testimony act's planted lie distorts a fact that an EARLIER investigation act teaches. In `actPlan`, name that fact in `lieAbout`.\n- The 暗线 (season conspiracy) advances by EXACTLY ONE planted beat — the campaign's current stage `setup`/`misdirection`. It is seeded as a passing detail in ONE act (`seedsMeta: true`), NOT resolved here, and must NOT repeat an already-dropped hook.\n\nHARD STRUCTURE (the writer pass + validator enforce these — plan for them now):\n- 6–8 acts, ALTERNATING investigation ↔ testimony, the FIRST act investigation; at least 3 investigation and 3 testimony acts.\n- Each testimony act has exactly ONE lie. The whole chapter has at least 3 lies total.\n- The chapter must cover at least the required number of knowledge points (★ must-cover ones are non-negotiable). Map them in `coveragePlan` and per-act `knowledgeIds`.\n\nCONTENT RULE: everything ties to TESTABLE lecture knowledge from the supplied Live note. No administrative trivia, no filenames/dates/codes, no invented facts. If the note is thin, build a tighter chapter rather than padding with filler — but still respect the structure targets the user prompt gives you.\n\nOUTPUT SHAPE:\n{\n  \"sessionNum\": integer (which 授業計画 第N回 this lecture matches by content; 0 if unsure),\n  \"caseType\": one of [\"Exam Signal Case\",\"Concept Web Case\",\"Doubt Repair Case\",\"Contradiction Case\",\"Missing Link Case\"],\n  \"caseLogic\": { \"truth\": \"…\", \"culprit\": \"…\", \"motive\": \"…\", \"redHerrings\": [\"…\",\"…\"], \"deductionChain\": [\"…\",\"…\",\"最後に問いへ答える結論\"] },\n  \"metaBeat\": \"この章が暗线に与える一拍（現段階の布石を、新しい角度で）\",\n  \"actPlan\": [\n    { \"index\": 1, \"kind\": \"investigation\", \"beat\": \"この幕で何を捜査し何を教えるか(1文)\", \"knowledgeIds\": [\"k1\",\"k3\"], \"seedsMeta\": false },\n    { \"index\": 2, \"kind\": \"testimony\", \"beat\": \"誰がなぜ証言台に立つか(1文)\", \"lieAbout\": \"歪める既習事実（どの知識点/捜査結果か）\", \"knowledgeIds\": [\"k2\"], \"witnessName\": \"…\", \"witnessRole\": \"…\", \"seedsMeta\": true }\n  ],\n  \"coveragePlan\": [\"k1\",\"k2\",\"k3\", \"…★を全て含み、必要数以上\"]\n}\nKeep all human-readable text in Japanese."
}

/// Build the Pass A (outline) user prompt — the same source/knowledge/world
/// context as the draft pass, but asking only for the logical skeleton.
pub(crate) fn detective_outline_user_prompt(
    case: &DetectiveCase,
    input: &[EvidenceInputEntry],
    memory: &DetectiveMemory,
    campaign: Option<&DetectiveCampaign>,
    syllabus: &[PlannedSession],
    knowledge: &[KnowledgePoint],
    targets: &GenTargets,
    plan: &ChapterPlan,
) -> String {
    let live = input
        .iter()
        .filter(|e| e.source_type == "live")
        .map(|e| truncate_chars(e.raw_content.replace('\r', "").trim(), 6500))
        .collect::<Vec<_>>()
        .join("\n---\n");
    let live = if live.trim().is_empty() {
        "(本文未抽出)".to_string()
    } else {
        live
    };
    let signals = input
        .iter()
        .filter(|e| e.source_type == "signal")
        .map(|e| truncate_chars(e.raw_content.trim(), 200))
        .collect::<Vec<_>>()
        .join("\n");
    let signals = if signals.trim().is_empty() {
        "(none)".to_string()
    } else {
        signals
    };
    let knowledge_section = if knowledge.is_empty() {
        "(知識点リスト未取得)".to_string()
    } else {
        knowledge
            .iter()
            .map(|p| {
                let star = if p.must_cover { "★" } else { " " };
                format!(
                    "- {star} `{}` {} — {}",
                    p.id,
                    p.label,
                    truncate_chars(p.gist.trim(), 80)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let must_cover: Vec<&str> = knowledge
        .iter()
        .filter(|p| p.must_cover)
        .map(|p| p.id.as_str())
        .collect();
    let syllabus_section = if syllabus.is_empty() {
        "(授業計画は未取得。sessionNum は 0)".to_string()
    } else {
        syllabus
            .iter()
            .map(|s| {
                let mode = if s.online {
                    "（オンライン）"
                } else {
                    ""
                };
                format!(
                    "- 第{}回{}: {}",
                    s.num,
                    mode,
                    truncate_chars(s.topic.trim(), 80)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        r#"Course: {course}

═══ LIVE LECTURE NOTE (primary source — the testable content) ═══
{live}

═══ Notifications (exam-context only) ═══
{signals}

═══ 授業計画 (match this lecture to its 第N回 by content) ═══
{syllabus}

═══ KNOWLEDGE POINTS (★ = must-cover) ═══
{knowledge}
must-cover ids: {must_cover}

═══ CAMPAIGN WORLD (this chapter is a beat inside it) ═══
{campaign}

═══ MEMORY (player history — drive continuity) ═══
{memory}

═══ THIS CHAPTER'S DIRECTION ═══
事件原型のヒント（今回はこの型で組み立てる）: {archetype}
暗线の担当段階: {arc_line}

═══ TASK ═══
Design the LOGICAL SKELETON (推理プロット) of ONE chapter for this lecture, as a {acts_min}–{acts_max}-act 逆転裁判 episode shaped by the archetype hint above. Decide the 明线 (truth / culprit / motive / red herrings / deduction chain). The CULPRIT should be one of the testimony witnesses (prefer a campaign cast member), and the deduction chain's steps should reference the evidence ids / testimony ids they rely on. Lay out the act plan (alternating investigation/testimony, first act investigation, ≥3 of each, exactly one lie per testimony act, ≥3 lies total), and plan knowledge-point coverage (every ★ + at least {coverage_min} total). Advance the 暗线 by the assigned stage's beat ONLY (do not jump ahead or resolve it), seeded in one act (`seedsMeta:true`) and not repeating an already-dropped hook. Output ONLY the JSON object specified in the system message."#,
        course = case.course_name,
        live = live,
        signals = signals,
        syllabus = syllabus_section,
        knowledge = knowledge_section,
        must_cover = if must_cover.is_empty() {
            "(なし)".to_string()
        } else {
            must_cover.join(", ")
        },
        campaign = format_campaign_section(campaign, plan.arc_focus),
        memory = format_memory_section(memory),
        archetype = plan.archetype,
        arc_line = match plan.arc_focus {
            Some(n) => format!("第{n}/{}段階（この段階の布石だけを進める）", plan.arc_total),
            None => "(暗线未設定)".to_string(),
        },
        acts_min = targets.acts_min,
        acts_max = targets.acts_max,
        coverage_min = targets.coverage_min,
    )
}

// ─── Pass C: editor critique / repair ──────────────────────────────────────

pub(crate) fn detective_editor_system_prompt() -> &'static str {
    "You are a senior script editor for a 逆転裁判-style study-mystery. You receive ONE chapter draft as JSON and audit it against a professional checklist. Your standard is high — a produced episode, not a rough cut.\n\nCHECKLIST:\n1. Logical consistency — the planted lie in each testimony act genuinely CONTRADICTS its `keyEvidenceId` card (revealed in an earlier investigation act). No lie keyed to a not-yet-shown card. The `caseLogic.deductionChain` actually follows from the evidence + busted lies and ends by answering `finalQuestion`.\n2. Motive traceability — every witness who lies has a plausible, inferable motive (cover an ally, protect reputation, conceal involvement, protect a payoff), consistent with the campaign cast's 背景/動機/利害 when a bible character is reused. `caseLogic.culprit` + `motive` are concrete.\n3. Fair play — red herrings are plausible but eliminable from the evidence; nothing is a cheat or a leap.\n4. 明线/暗线 — the chapter case resolves fully here; the 暗线 advances by exactly ONE seeded beat (`seedsMeta:true` on one act), consistent with the campaign's current stage and NOT repeating an already-dropped hook, NOT contradicting the world canon.\n5. Craft — scenes have subtext and voice, dialogue reveals under pressure rather than narrating, each act ends on a hook. press responses teach (for true statements) / deflect without confessing (for the lie).\n6. Content rule — everything is testable lecture knowledge; no admin trivia, filenames, dates-as-codes, invented facts; every ★ knowledge point is still covered.\n\nOUTPUT — MANDATORY, a single JSON object, nothing else:\n- If the draft already passes every check, output exactly: {\"ok\": true}\n- Otherwise output the FULL corrected chapter in the SAME JSON shape as the draft (all fields: caseType, difficulty, sessionNum, briefing, scenario, finalQuestion, acts[…], coverage[…], caseLogic{…}, metaBeat). Fix only what fails the checklist; preserve everything that already works, keep the same act count/kinds and the same knowledge coverage, and NEVER turn a correct testable fact into a wrong one. Keep all human-readable text in Japanese."
}

/// Build the Pass C (editor) user prompt: the draft JSON plus the consistency
/// anchors (must-cover points, campaign canon) the editor must respect.
pub(crate) fn detective_editor_user_prompt(
    draft_json: &str,
    outline_json: &str,
    campaign: Option<&DetectiveCampaign>,
    knowledge: &[KnowledgePoint],
    arc_focus: Option<u8>,
) -> String {
    let must_cover: Vec<&str> = knowledge
        .iter()
        .filter(|p| p.must_cover)
        .map(|p| p.label.as_str())
        .collect();
    format!(
        r#"═══ CAMPAIGN WORLD / CANON (the chapter must stay consistent with this) ═══
{campaign}

═══ APPROVED OUTLINE (the draft must stay faithful to this — same culprit / motive / per-act lie targets / structure / 暗线 stage) ═══
{outline}

═══ MUST-COVER knowledge points (all must remain covered) ═══
{must_cover}

═══ CHAPTER DRAFT (audit + repair this) ═══
{draft}

Run the checklist from the system message, and additionally verify the draft is FAITHFUL to the approved outline above (the realised culprit/motive, each testimony act's planted lie, the act structure, and the 暗线 stage must match the plan; the prose may be polished but the logic must not drift). If it all passes, return {{"ok": true}}. Otherwise return the full corrected chapter JSON (same shape)."#,
        campaign = format_campaign_section(campaign, arc_focus),
        outline = outline_json,
        must_cover = if must_cover.is_empty() {
            "(なし)".to_string()
        } else {
            must_cover
                .iter()
                .map(|m| format!("- {m}"))
                .collect::<Vec<_>>()
                .join("\n")
        },
        draft = draft_json,
    )
}
