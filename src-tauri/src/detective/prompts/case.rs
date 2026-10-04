use super::super::{
    truncate_chars, ChapterPlan, DetectiveCampaign, DetectiveCase, DetectiveMemory,
    EvidenceInputEntry, GenTargets, KnowledgePoint, PlannedSession,
};
use super::sections::{format_campaign_section, format_memory_section};

pub(crate) fn detective_ai_system_prompt() -> &'static str {
    "You author a Japanese Ace-Attorney-style cross-examination case that helps the player STUDY THE TESTABLE LECTURE CONTENT from the supplied Live notes. The case is review study — not a slice-of-life game.\n\nOUTPUT FORMAT — MANDATORY:\n- Output a single JSON object and nothing else.\n- Start your response with `{` and end with `}`.\n- Do NOT wrap in markdown code fences (```).\n- Do NOT add prose, preamble, explanation, comments, or trailing text.\n- Do NOT use a tool call wrapper; just emit raw JSON.\n\nINPUT KINDS:\n- LIVE notes (sourceType=live): the body text is what the teacher actually taught. THIS IS THE PRIMARY SOURCE. Extract testable knowledge from here: concepts, definitions, taxonomies, examples, formulas, theorems, classifications, historical facts, named processes, instructor explanations of meaning.\n- SIGNAL notifications (sourceType=signal): use ONLY for exam-CONTEXT (exam date, exam range, format, allowed materials, number of questions, weighting). Use them to anchor the urgency, NEVER as a topic.\n- DOUBT items (sourceType=doubt): pull the player's previously-flagged knowledge gaps as priority topics.\n\nABSOLUTE FOCUS RULE — every evidence card body, every testimony statement, every press response must be about TESTABLE COURSE KNOWLEDGE. The following CONTENT IS FORBIDDEN regardless of whether it appears in the supplied notes:\n- Administrative trivia: 学籍番号 / ネームカード / レポート提出方法 / 用紙の色・サイズ / 出欠の取り方 / 教室の場所 / 持参物 (pen, USB) / 提出期限の机械的記述 / ファイル命名規則 / Word vs PDF / その他「事務連絡」\n- Class logistics: 休講連絡, 補講日程, 教員の余談, アイスブレイクの内容, 自己紹介, 出席確認のやり方\n- General-knowledge questions that don't tie back to a specific concept the teacher explained\n- Filenames (.md, _live), ISO dates (YYYY-MM-DD), course codes, instructor names, classroom numbers\n- Generic placeholder labels (ライブメモ, 授業ノート, 講義メモ, 本講義の記録)\n- Empty platitudes (重要な内容がある, 記録が残っている, 資料を確認できる)\n- Any fact, number, date, or chapter not literally present in the supplied content\n\nIf the supplied Live notes are MOSTLY administrative and contain little testable content, prefer producing FEWER evidence cards and FEWER lies — quality over quantity. Never invent topics to fill a quota.\n\nCHAPTER SCOPE — a chapter is the WHOLE lecture turned into a play. AIM HIGH: pull as many distinct testable points from the supplied Live note as it supports — definitions, examples, contrasts, numerical claims, classifications, named processes, instructor explanations. A thin chapter (few cards, few statements, single-line teaching) is a FAILURE; depth and breadth are mandatory.\n\nCHAPTER STRUCTURE — you write ONE chapter told in 6–8 ACTS (幕), like a 逆転裁判 episode. Acts ALTERNATE between two kinds:\n- INVESTIGATION act (kind=\"investigation\"): the teaching beat. A `narrative` (2–4 Japanese sentences) advances the plot, and 2–4 `evidence` cards reveal distilled facts from the content. The first act MUST be investigation.\n- TESTIMONY act (kind=\"testimony\"): the testing beat. A `narrative` brings a witness to the stand, and 3–5 `testimony` statements follow with EXACTLY ONE lie (`isFalse: true`). The TRUE statements are NOT filler — each is its own testable concept the player should learn.\n\nKEY CONSTRAINT — teaching before testing: a lie's `keyEvidenceId` MUST point at an evidence card revealed in an EARLIER investigation act. Never test a fact the player has not yet been shown. Provide at least 3 investigation acts and at least 3 testimony acts.\n\n- `scenario` (4–6 Japanese sentences): chapter prologue / hook — situation, witnesses, stakes — anchored in real concepts from the Live notes.\n- Per testimony act: `witnessName` (2–6 Japanese chars, an invented given name e.g. ミナミ/ジュン/ハル — NEVER an instructor name) and `witnessRole` (4–14 chars). Vary witnesses across testimony acts when natural.\n- All testimony in plain Japanese witness speech (〜だ / 〜である / 〜のはず), each statement under 140 characters. The true statements must reference DIFFERENT testable points (not paraphrases of each other).\n- Each evidence `body` is 2–5 sentences: state the fact, then add ONE concrete grounding element (example / counter-example / value / contrast / named instance the teacher used).\n\nFor every testimony statement, also provide:\n- `highlights`: 1–3 keywords COPIED VERBATIM from `text` — the concept name, value, or term to scrutinise.\n- `pressResponse`: 2–3 Japanese sentences. For TRUE statements, USE THIS TO TEACH — start from the concept and drill deeper (definition → concrete example → contrast / common confusion). For the FALSE statement, the witness doubles down for 2–3 sentences (deflect, change the subject, cite an unrelated 'fact'), but never reveals the lie.\n\nNARRATIVE & MOTIVATION: each act's `narrative` is a beat of the chapter's main plot, set in the campaign world's specific historical locus (era / named place / community). **Every character who speaks or acts must have a stated or inferable motive — what they want, what they protect**. The bible's cast comes with 背景/動機/利害; honour those. Witnesses you newly invent for this chapter need a one-sentence backstory + a reason for being on the stand, established in the testimony act's `narrative` before they speak. A witness whose lie has no plausible motive (cover an ally, save reputation, conceal involvement, defend a payoff) is a failure — make the motive shape their tone in `pressResponse`, without ever stating 「私は嘘をついている」. Seed the overarching hidden thread (暗线) with ONE subtle hint in a single early act (mark that act `seedsMeta: true`). Story is the vehicle that makes the testable content stick — never invent testable facts to serve story, and never invent story so thin that characters feel like quiz props.\n\nPROFESSIONAL SCREENWRITING BAR: write at the level of a produced 逆転裁判 scenario. Scenes open in the middle of tension (in medias res), each act ends on a hook that pulls into the next, dialogue has subtext and voice (witnesses don't narrate exposition — they reveal it under pressure), and the chapter has a clear dramatic shape (掴み → 転 → 山場 → 解決). The 明线 (this chapter's case) must be self-contained and fully resolved here; the 暗线 (the season's hidden conspiracy) advances by exactly the ONE planted beat the outline specifies — no more, no less.\n\nYOU ARE GIVEN AN APPROVED OUTLINE (推理プロット): follow its act plan, its planted lie per testimony act (`lieAbout`), its coverage plan, and its caseLogic. Do not invent a different culprit, motive, or structure. Realise the outline as polished prose.\n\nALSO EMIT (top-level): `caseLogic` { truth, culprit (prefer a bible cast name), motive, redHerrings[], deductionChain[] — the ordered steps by which the busted contradictions reconstruct the truth, the last step answering finalQuestion } and `metaBeat` (one Japanese sentence: what this chapter contributed to the 暗线, matching the outline's planted beat). These must be CONSISTENT with the written acts."
}

/// Build the AI user prompt. AI reads the raw source content and distills
/// it into N short evidence cards (one-paragraph facts), each tagged with a
/// `sourceRef` pointing back to an input alias (l1/s1/d1) so we can attach
/// the original file path or URL afterwards.
pub(crate) fn detective_ai_user_prompt(
    case: &DetectiveCase,
    input: &[EvidenceInputEntry],
    memory: &DetectiveMemory,
    campaign: Option<&DetectiveCampaign>,
    syllabus: &[PlannedSession],
    knowledge: &[KnowledgePoint],
    targets: &GenTargets,
    plan: &ChapterPlan,
) -> String {
    fn render(entry: &EvidenceInputEntry, max_chars: Option<usize>) -> String {
        let body = entry.raw_content.replace('\r', "");
        let body = body.trim();
        let content = match max_chars {
            Some(cap) => truncate_chars(body, cap),
            None => body.to_string(),
        };
        let title_line = if entry.source_type == "signal" && !entry.raw_title.trim().is_empty() {
            format!("\n  title: {}", truncate_chars(&entry.raw_title, 120))
        } else {
            String::new()
        };
        format!(
            "- id: {alias}{title_line}\n  content:\n    {content}",
            alias = entry.alias,
            content = content.replace('\n', "\n    ")
        )
    }

    let live_section = collect_section(input, "live", None);
    let signal_section = collect_section(input, "signal", Some(400));
    let doubt_section = collect_section(input, "doubt", Some(280));

    let syllabus_section = if syllabus.is_empty() {
        "(この科目の授業計画は未取得。sessionNum は 0 とすること)".to_string()
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
                    truncate_chars(s.topic.trim(), 90)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
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
                    truncate_chars(p.gist.trim(), 90)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let must_cover_ids: Vec<&str> = knowledge
        .iter()
        .filter(|p| p.must_cover)
        .map(|p| p.id.as_str())
        .collect();
    let must_cover_list = if must_cover_ids.is_empty() {
        "(なし)".to_string()
    } else {
        must_cover_ids.join(", ")
    };

    fn collect_section(
        input: &[EvidenceInputEntry],
        source_type: &str,
        max_chars: Option<usize>,
    ) -> String {
        let blocks: Vec<String> = input
            .iter()
            .filter(|e| e.source_type == source_type)
            .map(|e| render(e, max_chars))
            .collect();
        if blocks.is_empty() {
            "(none)".to_string()
        } else {
            blocks.join("\n")
        }
    }

    format!(
        r#"Course: {course}

═══ INPUT SOURCES — read these in full and extract concrete facts ═══

Live lecture notes (PRIMARY — body is what the teacher actually said):
{live}

Notifications (exam format / scope intel):
{signals}

Student's unresolved doubts:
{doubts}

═══ 授業計画 (lecture plan — match THIS lecture to its 第N回) ═══
{syllabus}

═══ KNOWLEDGE POINTS (★ = must-cover; you MUST place each ★ point somewhere in the chapter, and at least {coverage_min} total) ═══
{knowledge}
must-cover ids: {must_cover_list}

═══ CAMPAIGN WORLD (世界観 — this session is a chapter inside it) ═══
{campaign}

═══ MEMORY (player history — drive continuity) ═══
{memory}

═══ TASK ═══
Write ONE CHAPTER of the campaign — a substantial case told in {acts_min}–{acts_max} ACTS (幕). This is the WHOLE lecture turned into a play; aim for breadth + depth, NOT a quick quiz. The chapter plays like a 逆転裁判 episode: the detective ALTERNATES between INVESTIGATION acts (teach concrete facts) and TESTIMONY acts (catch a witness in a lie). Cover every NOTABLE testable point from the Live note — definitions, examples, contrasts, classifications, named processes, numerical claims — spread across the acts. Do not under-deliver.

ACT RHYTHM (mandatory):
- The FIRST act MUST be an investigation act (the player needs evidence before anyone can be cross-examined).
- ALTERNATE: investigation → testimony → investigation → testimony … A testimony act's lie may ONLY be busted with a card revealed in an EARLIER investigation act.
- Provide at least 3 investigation acts and at least 3 testimony acts.

INVESTIGATION ACT — the teaching beat:
- `narrative`: 2–4 Japanese sentences advancing the chapter's main plot (a scene in the campaign world: where the detective goes, who they meet, what they find).
- `evidence`: 2–4 cards (4 preferred when the lecture has the material). Each card = ONE meaty fact, 2–5 Japanese sentences. State the concept, then ground it with a concrete detail from the source: an example, a counter-example, a value, a contrast, or a named instance the teacher actually used. NEVER paste raw source text; rewrite as a clean fact paragraph (e.g. "教員はピジン言語の例として太平洋戦争中のヤシ語を挙げ、語彙は主に英語由来だが文法は太平洋諸語の影響を強く受けると説明した。さらに、ピジンが世代を超えて母語化したものがクレオールであり、両者は安定性で区別されると述べた。"). `title` = 6–30 char topic headline. `sourceRef` = the input alias (l1/s1/d1/...).

TESTIMONY ACT — the testing beat:
- `narrative`: 2–3 Japanese sentences moving the plot to the confrontation. MUST establish: (a) who this witness is (one-sentence background — origin/role in the locus), (b) why they are here / what they want from this encounter, (c) the moment they take the stand. The witness's motive must be inferable from this — it will drive their tone in testimony.
- `witnessName` (2–6 Japanese chars, invented unless reusing a bible cast member) + `witnessRole` (4–14 chars, fits the world's cast / community). Vary witnesses across acts when natural; reuse a bible character when it makes sense for their motivation.
- `testimony`: 3–5 statements spoken by that witness, of which EXACTLY ONE is a lie (`isFalse: true`). The lie CONTRADICTS one already-revealed evidence card; its `keyEvidenceId` is that card's id. The other (true) statements should ALSO be content-anchored — each is a different testable point the player should learn (not filler). All statements in plain Japanese witness speech, each under 140 chars.

CONTINUITY (MEMORY section): RE-EMPHASIZE recently-failed topics — make at least one lie touch one if present. DE-EMPHASIZE recently-mastered topics. Avoid reusing `recent_evidence_titles` verbatim.

WORLD & MOTIVATION (CAMPAIGN WORLD section, if present): every act's `narrative` happens INSIDE that world (its era/place/named locale/cast). The bible above lists each cast member's `背景`/`動機`/`利害` — when you reuse them, their behaviour in this chapter MUST flow from those. Even brand-new witnesses you invent for THIS chapter need a stated motive: in the testimony act's `narrative`, establish WHO this witness is (background sketch in 1 sentence), WHY they are at the scene / why they care, and WHAT they want from the encounter. A witness whose lie has no traceable motive is a failure — the lie should plausibly serve their interests (cover for an ally, protect reputation, conceal involvement, protect a payoff). The motive does NOT need to be revealed to the player as text — it just needs to shape the tone of their testimony and their `pressResponse`. Across the chapter, SEED the overarching hidden thread (暗线) with ONE subtle early hint — set `seedsMeta: true` on the single act that drops it, and keep it a passing detail (not resolved this chapter). Testable content stays exactly as rigorous; the world is dressing for the knowledge, never an excuse to invent facts. If NO world is given, use a neutral study framing — but characters still need motivation.

SESSION ALIGNMENT (授業計画): the Live note above is ONE lecture. Compare its actual content to the 授業計画 list and decide which 第N回 it corresponds to BY CONTENT (topic match), NOT by order. Output that number as top-level `sessionNum`. If the plan is empty or you genuinely cannot tell, use 0. Never guess by position.

COVERAGE (mandatory): for EVERY knowledge point listed above, ensure that the concept actually appears in some evidence card body OR in some testimony statement text. Then emit a `coverage` array mapping each covered point to where it landed. RULES:
- Every must-cover (★) point MUST appear, no exception. A chapter missing even one ★ point is rejected and the player has to retry.
- Total distinct points covered (★ + non-★ combined) MUST be ≥ {coverage_min}. So plan the chapter to fit at least {coverage_min} of the listed points.
- `placement` is either an evidence id ("e3") or a testimony id ("a4t3" — = act `a4`, statement `t3` in its testimony list).
- One point per coverage entry; you may cover one piece of content with multiple cards/statements but each coverage entry references a single placement.

Produce exactly this JSON shape (no comments, no extra fields):

{{
  "caseType": one of ["Exam Signal Case", "Concept Web Case", "Doubt Repair Case", "Contradiction Case", "Missing Link Case"],
  "difficulty": integer 1..5 (PREFER 2–3),
  "sessionNum": integer (the 第N回 this lecture matches by content; 0 if unsure),
  "briefing": "Japanese paragraph 60–220 chars summarising what this chapter investigates and what concepts it spans",
  "scenario": "4–6 Japanese sentences of narrative prologue: the chapter's hook, the situation, who's involved, what's at stake — set in the campaign world and rooted in the lecture content",
  "finalQuestion": "one specific Japanese question whose answer lies in the chapter's evidence",
  "acts": [
    {{ "id": "a1", "kind": "investigation", "title": "幕タイトル(〜16字)", "location": "舞台(任意)", "narrative": "2–4文の物語…", "seedsMeta": false,
       "evidence": [
         {{ "id": "e1", "title": "6–30字の見出し", "body": "2–5文の事実。概念→具体例/反例/数値/分類などで肉付けする。", "sourceRef": "l1" }},
         {{ "id": "e2", "title": "別の論点", "body": "2–5文の事実…", "sourceRef": "l1" }}
       ] }},
    {{ "id": "a2", "kind": "testimony", "title": "幕タイトル", "narrative": "2–3文で対決の場へ…", "seedsMeta": false,
       "witnessName": "ミナミ", "witnessRole": "ゼミ仲間",
       "testimony": [
         {{ "id": "a2t1", "text": "真実の証言（別の論点）…", "isFalse": false, "keyEvidenceId": "", "highlights": ["…逐語…"], "pressResponse": "2–3文で概念を教える。定義→具体例→対比、の順で踏み込む。" }},
         {{ "id": "a2t2", "text": "別の真実の証言…", "isFalse": false, "keyEvidenceId": "", "highlights": ["…"], "pressResponse": "2–3文で深掘り。" }},
         {{ "id": "a2t3", "text": "e1 と矛盾する嘘…", "isFalse": true, "keyEvidenceId": "e1", "highlights": ["…誤った語…"], "pressResponse": "2–3文で白を切る。論点をすり替えたり別の例を持ち出すが、嘘そのものは明かさない。" }}
       ] }},
    {{ "id": "a3", "kind": "investigation", "title": "…", "narrative": "…", "evidence": [ /* 2–4 cards, 2–5文 each */ ] }},
    {{ "id": "a4", "kind": "testimony", "title": "…", "witnessName": "…", "witnessRole": "…", "testimony": [ /* 3–5 statements, exactly 1 lie keyed to some earlier evidence */ ] }},
    {{ "id": "a5", "kind": "investigation", "title": "…", "narrative": "…", "evidence": [ /* … */ ] }},
    {{ "id": "a6", "kind": "testimony", "title": "…", "witnessName": "…", "witnessRole": "…", "testimony": [ /* … */ ] }}
  ],
  "coverage": [
    {{ "pointId": "k1", "placement": "e1" }},
    {{ "pointId": "k2", "placement": "a2t1" }},
    {{ "pointId": "k3", "placement": "e3" }}
    /* …continue until every ★ point and at least {coverage_min} total are listed */
  ],
  "caseLogic": {{
    "truth": "1〜3文。この事件の真相（実際に何が起きていたか）。",
    "culprit": "責任者（できれば世界観の登場人物名）。",
    "motive": "なぜそうしたか／なぜ嘘をつくか（手段・機会も織り込む）。",
    "redHerrings": ["もっともらしいが誤った手がかり1", "…2"],
    "deductionChain": ["突きつけた矛盾1 → 導かれること", "矛盾2 → …", "最後に finalQuestion へ答える結論"]
  }},
  "metaBeat": "本章が暗线に与えた一拍（プロットの planted beat と一致、既出の伏線を繰り返さず新しい角度で）"
}}

`sourceRef` MUST be one of the input aliases above (l1/l2/.../s1/.../d1/...). Every evidence `id` is unique across the whole chapter. Each lie's `keyEvidenceId` MUST be an evidence id revealed in an EARLIER investigation act. Each `highlights` entry MUST be a verbatim substring of its `text`. Every text field MUST follow the CONTENT-ONLY RULE in the system message."#,
        course = case.course_name,
        live = live_section,
        signals = signal_section,
        doubts = doubt_section,
        syllabus = syllabus_section,
        knowledge = knowledge_section,
        must_cover_list = must_cover_list,
        campaign = format_campaign_section(campaign, plan.arc_focus),
        memory = format_memory_section(memory),
        acts_min = targets.acts_min,
        acts_max = targets.acts_max,
        coverage_min = targets.coverage_min,
    )
}
