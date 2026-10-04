use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectiveCase {
    pub id: String,
    pub course_key: String,
    pub course_name: String,
    pub title: String,
    pub case_type: String,
    pub difficulty: u8,
    pub briefing: String,
    pub evidence: Vec<DetectiveCaseEvidence>,
    pub final_question: String,
    #[serde(default)]
    pub testimony: Vec<DetectiveTestimony>,
    /// Narrative prologue (3–5 sentences) that sets up *why* the case is being
    /// investigated and *who* is being cross-examined. Drawn from course content.
    #[serde(default)]
    pub scenario: String,
    /// Witness name as written by the AI (e.g. 「ミナミ」). Japanese only.
    #[serde(default)]
    pub witness_name: String,
    /// Witness's role / relation (e.g. 「同級生」「先輩」「ゼミ仲間」).
    #[serde(default)]
    pub witness_role: String,
    #[serde(default)]
    pub generation_mode: String,
    #[serde(default)]
    pub generation_note: String,
    /// The 授業計画 第N回 this chapter's Live note was matched to BY CONTENT
    /// (0 = could not be determined). Drives chapter numbering + finale.
    #[serde(default)]
    pub session_num: u8,
    /// The knowledge-point checklist this chapter was built to cover.
    #[serde(default)]
    pub knowledge_points: Vec<KnowledgePoint>,
    /// Where each covered knowledge point landed (evidence id or testimony id).
    /// Validated against `knowledge_points` so the player is guaranteed to be
    /// exposed to every must-cover point.
    #[serde(default)]
    pub coverage: Vec<CoverageEntry>,
    /// The chapter's acts (幕) — a mix of investigation and testimony beats
    /// that the player walks through in order. The primary play structure;
    /// `evidence` is the shared Court Record pool referenced by these acts.
    #[serde(default)]
    pub acts: Vec<DetectiveAct>,
    /// The 推理 spine of this chapter (明线): the actual truth, who's
    /// responsible, their motive, planted red herrings, and the deduction chain
    /// the busted contradictions reconstruct. Authored in the outline pass and
    /// kept coherent through drafting + editing.
    #[serde(default)]
    pub case_logic: CaseLogic,
    /// What this chapter contributes to the campaign's 暗线 — one concrete beat
    /// for its position in the arc. Recorded back into the campaign canon so
    /// later (independently generated) chapters stay mutually consistent.
    #[serde(default)]
    pub meta_beat: String,
}

/// The 推理 spine of a chapter — what really happened and why, separate from the
/// surface testimony. Authored in the outline pass, held consistent through
/// drafting + editing, and surfaced on the chapter-clear review.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseLogic {
    /// 1–3 sentences: the actual truth of the case (what really happened).
    #[serde(default)]
    pub truth: String,
    /// Who / what is responsible. Prefer a bible cast member by name.
    #[serde(default)]
    pub culprit: String,
    /// Why they did it / why they lie — means + opportunity folded in.
    #[serde(default)]
    pub motive: String,
    /// Plausible-but-wrong leads planted for fair-play misdirection.
    #[serde(default)]
    pub red_herrings: Vec<String>,
    /// Ordered steps: how the busted contradictions reconstruct the truth.
    /// The final step answers `final_question`.
    #[serde(default)]
    pub deduction_chain: Vec<String>,
}

/// One act (幕) of a chapter. Either an INVESTIGATION beat (the player reads
/// evidence cards revealed here — the teaching moment) or a TESTIMONY beat (a
/// witness testifies with one planted lie — the testing moment). Story prose
/// in `narrative` advances the chapter's main plot between beats.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectiveAct {
    pub id: String,
    /// 1-based 幕番号.
    pub index: u8,
    /// "investigation" | "testimony".
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub location: String,
    /// Story beat shown when the act opens (advances the chapter's main plot).
    #[serde(default)]
    pub narrative: String,
    /// Whether this act subtly seeds the campaign's overarching 暗线.
    #[serde(default)]
    pub seeds_meta: bool,
    /// Investigation acts: the ids (into `case.evidence`) revealed here.
    #[serde(default)]
    pub evidence_ids: Vec<String>,
    /// Testimony acts: who is on the stand for this beat.
    #[serde(default)]
    pub witness_name: String,
    #[serde(default)]
    pub witness_role: String,
    /// Testimony acts: the witness's statements (one planted lie).
    #[serde(default)]
    pub testimony: Vec<DetectiveTestimony>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectiveTestimony {
    pub id: String,
    pub text: String,
    pub is_false: bool,
    pub key_evidence_id: String,
    /// Substrings inside `text` that should be highlighted in the UI — the
    /// terms most likely to either prove or break the statement.
    #[serde(default)]
    pub highlights: Vec<String>,
    /// What the witness says when the player presses (ゆさぶる) this statement.
    /// Used to teach: for TRUE statements it elaborates the concept; for the
    /// FALSE statement it nudges without giving the answer away.
    #[serde(default)]
    pub press_response: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectiveCaseEvidence {
    pub id: String,
    pub source_id: String,
    pub source_type: String,
    pub source: String,
    pub title: String,
    pub date: String,
    pub excerpt: String,
    pub source_path: String,
    pub source_url: String,
    pub information_type: String,
    pub person_category_cd: String,
    pub category_cd: String,
}

/// One distilled knowledge point extracted from a Live note. Drives chapter
/// generation (the AI must place each must-cover point somewhere in the case)
/// + coverage validation. Cached per Live note so it's built once.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgePoint {
    pub id: String,
    /// Short Japanese label (8–30 chars) — the topic headline.
    pub label: String,
    /// One-sentence gist — what the learner must actually know.
    #[serde(default)]
    pub gist: String,
    /// True for load-bearing concepts the teacher emphasised — must be covered.
    #[serde(default)]
    pub must_cover: bool,
}

/// Records which evidence card or testimony statement carries a given
/// knowledge point. `placement` is either an evidence id (e.g. "e3") or a
/// testimony id (e.g. "a4t2").
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageEntry {
    pub point_id: String,
    pub placement: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectiveLiveRecord {
    pub id: String,
    pub filename: String,
    pub path: String,
    pub course_name: String,
    pub downloaded_at: i64,
    pub excerpt: String,
}
