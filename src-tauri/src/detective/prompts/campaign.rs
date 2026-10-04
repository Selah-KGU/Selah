use super::super::{truncate_chars, DetectiveCampaign, EvidenceInputEntry};

pub(crate) fn finale_system_prompt() -> &'static str {
    "You are the lead writer closing out a long-running 逆転裁判-style study-mystery campaign. The season is complete. Write the GRAND FINALE — the epilogue shown once, when the player has cleared every chapter.\n\nThis finale must PAY OFF what actually happened, not restate the premise: resolve the 暗线 conspiracy conclusively (name the antagonist force + their motive), land the setups that were planted across the staged reveals, honour the established canon facts, and give the recurring cast a final beat consistent with their motivation/stakes. End on a strong closing image. 4–6 Japanese sentences, evocative and conclusive — no cliffhanger.\n\nALSO refine each staged reveal so the four-stage 暗线 arc reads as one coherent build toward this finale, consistent with the accumulated canon (keep each reveal 1–2 Japanese sentences; do not change the number of stages).\n\nOUTPUT — a single JSON object, nothing else: {\"finale\": \"…4–6文の日本語…\", \"reveals\": [{\"stage\": 1, \"reveal\": \"…\"}, {\"stage\": 2, \"reveal\": \"…\"}, {\"stage\": 3, \"reveal\": \"…\"}, {\"stage\": 4, \"reveal\": \"…\"}]}"
}

pub(crate) fn finale_user_prompt(c: &DetectiveCampaign) -> String {
    let cast = if c.cast.is_empty() {
        "(なし)".to_string()
    } else {
        c.cast
            .iter()
            .map(|m| {
                format!(
                    "- {}（{}）動機:{} 利害:{}",
                    m.name, m.role, m.motivation, m.stake
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let arc = if c.meta_arc.is_empty() {
        "(なし)".to_string()
    } else {
        c.meta_arc
            .iter()
            .map(|r| format!("- 第{}段階「{}」: {}", r.stage, r.title, r.reveal))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let facts = if c.canon.facts.is_empty() {
        "(なし)".to_string()
    } else {
        c.canon
            .facts
            .iter()
            .map(|f| format!("- {f}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let chapters = if c.chapters.is_empty() {
        "(なし)".to_string()
    } else {
        c.chapters
            .iter()
            .map(|ch| format!("- {}", ch.title))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        r#"世界観: {label}
舞台設定: {setting}
暗线（meta-mystery）: {meta}

登場人物:
{cast}

段階的に明かされた真相（metaArc — すべて解禁済み）:
{arc}

積み上がった正典（実際に起きた事実）:
{facts}

辿ってきた章:
{chapters}

═══ TASK ═══
上のすべてを踏まえ、キャンペーンの大団円（finale）を書いてください。暗线を決定的に解決し、各段階の布石を回収し、正典と矛盾せず、登場人物に最後の一拍を与え、強い締めの画で終える。さらに、4段階の reveal を実際に積み上がった正典と一貫するように書き直す（段階数は変えない）。JSON のみで返す: {{"finale": "…", "reveals": [{{"stage": 1, "reveal": "…"}}, {{"stage": 2, "reveal": "…"}}, {{"stage": 3, "reveal": "…"}}, {{"stage": 4, "reveal": "…"}}]}}"#,
        label = c.world_label,
        setting = c.setting,
        meta = c.meta_mystery,
        cast = cast,
        arc = arc,
        facts = facts,
        chapters = chapters,
    )
}
pub(crate) fn campaign_bible_system_prompt() -> &'static str {
    r#"You are the lead writer for a long-running courtroom-mystery game in the spirit of 逆転裁判 (Ace Attorney). You are designing the CAMPAIGN BIBLE — the persistent world (世界観) that every future chapter of one university course lives inside.

═══ THE GOLDEN RULE — anchor to a SIGNATURE HISTORICAL LOCUS ═══

The world is NOT "a vaguely-themed setting matching the subject". It is **the most iconic, historically/culturally representative time-and-place that the subject is associated with** — the specific moment a textbook would cite as the topic's center of gravity. Reach for the concrete locus, not the generic theme.

Good vs. bad anchoring:
- 米国黒人英語(AAVE) → ❌「19世紀のアメリカ」 / ✅「1960年代公民権運動下のミシシッピ・デルタの綿花町」or「1920年代ハーレム・ルネサンス期のニューヨーク」
- 確率論 → ❌「賭博の街」 / ✅「17世紀パスカルとフェルマーが文通した賭博問題のパリ・サロン」
- 量子力学 → ❌「物理学の街」 / ✅「1927年ソルベイ会議直前のコペンハーゲン、ボーア研究所」
- 古代中国法制 → ❌「中華風の都」 / ✅「商鞅変法下の戦国秦・咸陽」
- ピジン/クレオール言語 → ❌「多言語の港」 / ✅「19世紀末ハワイの製糖プランテーションと寄せ集めの労働者集落」

Specificity is non-negotiable. Name the **decade or specific historical event window** + **a real-feeling named locale** + **a concrete community/social structure**. If you cannot identify a signature locus, look harder — every academic subject has one.

═══ CHARACTERS NEED BACKGROUND, MOTIVATION, AND STAKES ═══

Every cast member is a person, not a label. For each, write:
- `background` (2–3 sentences): where they came from, what they've done before the campaign starts, what shaped them. NOT a role tag — a concrete mini-bio.
- `motivation` (1 sentence): what they WANT right now in this world. Every action they take in any chapter should be traceable here.
- `stake` (1 sentence): what they LOSE if the truth comes out / things go wrong. This is why they may lie, evade, or push back when cross-examined.
- `bond` (1 short phrase): their relation to the protagonist or to the meta-mystery.

═══ THE META-MYSTERY NEEDS A FACE ═══

The 暗线 is not just "a hidden truth". Name a **concrete antagonist force** — a person, a society, a guild, an institution — and give IT its own motivation (why are they hiding the truth? what do THEY want?). This makes the conspiracy feel real and the meta-arc reveals concrete.

═══ THE META-ARC IS A STORYBOARD, NOT A SUMMARY ═══

The 暗线 unfolds across the whole season. Design it like a professional serialized-mystery writer: each of the 4 stages is a STORYBOARD BEAT, not a vague summary. For each stage author THREE things that future chapters will execute:
- `setup`: the concrete hook/clue chapters at this stage should plant (a recurring object, a slip of the tongue, an inconsistent record, a name that keeps surfacing). Plantable as a passing detail inside an ordinary chapter.
- `misdirection`: the plausible-but-wrong reading that keeps the audience from guessing the truth too early — fair-play misdirection, not a cheat.
- `reveal`: what the audience actually learns when this stage lands (player-facing text).
Each stage must BUILD ON the previous one: (1) faint hint → (2) deepening clue that complicates stage 1 → (3) twist that recontextualises stages 1–2 → (4) full payoff naming the antagonist's identity + motivation. The reveals must be logically entailed by the setups (no clue appears from nowhere; no reveal contradicts an earlier stage).

═══ RELATIONSHIPS GIVE THE WORLD TENSION ═══

Cast members are not isolated. Author a small relationship web (2–4 edges) among the cast and the antagonist force — alliances, rivalries, debts, secret collaborations — each with the underlying tension that could erupt in a chapter.

═══ OUTPUT — return ONE JSON object, nothing else ═══
{
  "worldLabel": "8–22 char Japanese label naming the SIGNATURE LOCUS (era + place), e.g. 「1965年・ミシシッピ綿花町」「1927年・コペンハーゲン」",
  "setting": "3–5 Japanese sentences establishing the locus: the specific historical moment, the named place, the community structure, and WHY mysteries happen here. Anchor in real history that connects to the course subject.",
  "tagline": "one short Japanese hook line, under 28 chars — should evoke the SPECIFIC locus, not be generic",
  "metaMystery": "3–5 Japanese sentences. Name the antagonist force (who/what is hiding the truth), what they are concealing, and WHY they are concealing it (their motivation). The conspiracy must be thematically tied to the course's core ideas, and must be seedable in early chapters and payable in the finale.",
  "cast": [
    {
      "name": "2–6 char Japanese name (fits the locus's culture)",
      "role": "役回り e.g. 相棒/ライバル/黒幕候補/語り部/情報屋",
      "bond": "one phrase: their relation to the protagonist or the meta-mystery",
      "background": "2–3 Japanese sentences — concrete past, where they came from, what shaped them",
      "motivation": "1 Japanese sentence — what they WANT right now",
      "stake": "1 Japanese sentence — what they LOSE if the truth comes out",
      "voice": "口調カード — 一人称・語尾・口癖など、再登場時に声を一致させるための短いメモ"
    }
  ],
  "relationships": [
    { "from": "人物名/黒幕勢力", "to": "人物名/黒幕勢力", "relation": "関係(兄弟/師弟/対立/秘密の協力者…)", "tension": "燻る火種を1フレーズで" }
  ],
  "metaArc": [
    { "title": "短い見出し(〜12字)", "setup": "この段階で各章が仕込むべき具体的な布石", "misdirection": "観客を真相から逸らす“もっともらしい誤読”", "reveal": "1–2 Japanese sentences — この段階で観客が知る事実(プレイヤー向け表示文)", "sessionBand": "担当する第N回の帯(例 \"1-3\")" }
  ],
  "finale": "3–5 Japanese sentences — the grand epilogue shown when the campaign is 100% complete. The conclusive resolution of the metaMystery: who/what was behind it, what they wanted, how the detective resolves it, the closing image."
}

Provide 2–3 cast members; one of them should plausibly be the antagonist force's local agent or someone whose stake aligns with the meta-mystery. Provide 2–4 `relationships`. `metaArc` = EXACTLY 4 ordered entries with the storyboard fields above: (1) faint hint, (2) deepening clue, (3) twist that recontextualises earlier chapters, (4) full payoff revealing the antagonist's identity + motivation; distribute `sessionBand` to roughly quarter the season. `finale` lands AFTER stage 4 and must pay off the setups planted in stages 1–4. Keep everything in Japanese. Names, places, dates, and references must FEEL historically grounded — avoid totally invented place names when a real locus exists. Never invent facts that contradict the discipline."#
}

pub(crate) fn campaign_bible_user_prompt(
    course_name: &str,
    input: &[EvidenceInputEntry],
) -> String {
    let live: Vec<String> = input
        .iter()
        .filter(|e| e.source_type == "live")
        .map(|e| truncate_chars(e.raw_content.replace('\r', "").trim(), 1200))
        .collect();
    let signals: Vec<String> = input
        .iter()
        .filter(|e| e.source_type == "signal")
        .map(|e| truncate_chars(e.raw_content.trim(), 160))
        .collect();
    let live_block = if live.is_empty() {
        "(none)".to_string()
    } else {
        live.iter()
            .enumerate()
            .map(|(i, b)| format!("[{}]\n{}", i + 1, b))
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    let signal_block = if signals.is_empty() {
        "(none)".to_string()
    } else {
        signals.join("\n")
    };
    format!(
        r#"Course: {course}

═══ COURSE CONTENT (read this to infer the SUBJECT, then build a world around it) ═══

Live lecture notes (what the teacher actually taught):
{live}

Notifications (exam scope / format intel):
{signals}

═══ TASK ═══
Infer the academic SUBJECT of this course from the content above (history? statistics? linguistics? chemistry? law? …). Then design the CAMPAIGN BIBLE: a mystery world whose era/place/genre is DERIVED FROM that subject (see the system rules — American Revolution → 1770s colonial town, etc.). Establish a recurring cast and an overarching hidden conspiracy (暗线) that future chapters will unravel. Return the JSON object exactly as specified in the system message — Japanese, no extra fields, no prose."#,
        course = course_name,
        live = live_block,
        signals = signal_block,
    )
}
