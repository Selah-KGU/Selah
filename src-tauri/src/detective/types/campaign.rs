use serde::{Deserialize, Serialize};

/// The narrative "bible" for one course. Persisted per course under
/// `DETECTIVE_CAMPAIGN_PREFIX + course_key`. This is the long-running story
/// layer: a world (derived from the lecture subject matter), a recurring cast,
/// and an overarching hidden mystery that every chapter (= one live note)
/// advances. Generated once, then read + nudged forward by each case.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectiveCampaign {
    pub course_key: String,
    pub course_name: String,
    /// Short era/genre label, derived from the course content
    /// (e.g. 「18世紀アメリカ独立戦争」「確率が支配する街」).
    #[serde(default)]
    pub world_label: String,
    /// 世界観 — 2–4 sentence premise that recasts the course as a mystery world.
    #[serde(default)]
    pub setting: String,
    /// One-line hook for the campaign.
    #[serde(default)]
    pub tagline: String,
    /// Recurring cast tied to the meta-mystery.
    #[serde(default)]
    pub cast: Vec<CampaignCharacter>,
    /// The overarching hidden thread spanning all chapters.
    #[serde(default)]
    pub meta_mystery: String,
    /// How far the meta-plot has been revealed (0–100).
    #[serde(default)]
    pub meta_progress: u8,
    /// Staged reveals of the overarching 暗线 — unlocked as meta_progress rises.
    #[serde(default)]
    pub meta_arc: Vec<CampaignRevelation>,
    /// The grand epilogue — shown once the campaign reaches 100% (all chapters
    /// cleared). Conclusively resolves the meta-mystery.
    #[serde(default)]
    pub finale: String,
    /// Chapters (live notes) already turned into cases.
    #[serde(default)]
    pub chapters: Vec<CampaignChapter>,
    /// Web of relationships among the cast + the meta-antagonist — gives the
    /// world social tension that chapters can draw on.
    #[serde(default)]
    pub relationships: Vec<CampaignRelationship>,
    /// Living canon: facts every chapter must stay consistent with, plus the
    /// 暗线 hooks already dropped. Fed into every chapter's outline pass so
    /// independently generated chapters share one coherent world.
    #[serde(default)]
    pub canon: CampaignCanon,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignRelationship {
    /// Two parties (cast names, or the antagonist force) and how they relate.
    pub from: String,
    pub to: String,
    /// e.g. 「兄弟」「師弟」「対立」「秘密の協力者」.
    pub relation: String,
    /// One phrase of the underlying tension / unresolved friction.
    #[serde(default)]
    pub tension: String,
}

/// The shared, append-only story canon for a campaign. Fed into every chapter's
/// outline pass so independently generated chapters stay mutually consistent
/// (weak-continuity model: any play order, but one coherent world).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignCanon {
    /// Hard facts established about the world / cast (timeline, places,
    /// who-did-what) that no chapter may contradict.
    #[serde(default)]
    pub facts: Vec<String>,
    /// 暗线 hooks already dropped, so chapters vary their hints instead of
    /// repeating one detail.
    #[serde(default)]
    pub dropped_hooks: Vec<CanonHook>,
    /// Recurring-cast appearance log — "{name}: 「{chapter}」に登場" — so reused
    /// characters carry forward a felt history across chapters.
    #[serde(default)]
    pub cast_log: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonHook {
    /// Which meta-arc stage this hook served (0 = unknown).
    #[serde(default)]
    pub stage: u8,
    /// The hint that was dropped (kept so it isn't repeated verbatim).
    pub hook: String,
    /// Chapter (case id) that dropped it.
    #[serde(default)]
    pub chapter_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignCharacter {
    pub name: String,
    /// 役回り — e.g. ライバル, 相棒, 黒幕候補, 証人.
    pub role: String,
    /// Relationship to the protagonist / the meta-mystery.
    #[serde(default)]
    pub bond: String,
    /// 2–3 sentences of concrete background — where they came from, what
    /// they've already done, what shaped them. NOT a label, an actual mini-bio.
    #[serde(default)]
    pub background: String,
    /// What this character WANTS right now (in-fiction goal). Every action they
    /// take in the campaign should be traceable to this.
    #[serde(default)]
    pub motivation: String,
    /// What they stand to LOSE if the truth comes out / things go sideways.
    /// Drives why they help, evade, or lie.
    #[serde(default)]
    pub stake: String,
    /// Voice card — speech register / tics / first-person pronoun — so a reused
    /// character sounds the same from chapter to chapter.
    #[serde(default)]
    pub voice: String,
}

/// One staged reveal of the campaign's overarching 暗线. The bible defines an
/// ordered arc (faint hint → deepening → twist → final truth); each entry
/// unlocks once meta_progress reaches its threshold.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignRevelation {
    /// 1-based stage in the arc.
    pub stage: u8,
    /// meta_progress (0–100) at which this reveal unlocks.
    pub threshold: u8,
    pub title: String,
    /// Player-facing reveal text shown when the stage unlocks.
    pub reveal: String,
    #[serde(default)]
    pub unlocked: bool,
    /// Authoring guidance (not shown to the player): the hook chapters at this
    /// stage should plant to seed the 暗线.
    #[serde(default)]
    pub setup: String,
    /// Authoring guidance: the false lead that misdirects from this stage's
    /// truth, so the reveal lands as a fair-play surprise.
    #[serde(default)]
    pub misdirection: String,
    /// Soft guidance for which chapters carry this stage, as a 第N回 band, e.g.
    /// "1-3". Empty ⇒ derive from `threshold`.
    #[serde(default)]
    pub session_band: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignChapter {
    /// Live note id / path used as the chapter source.
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub played_at: i64,
}
