use super::campaign::DetectiveCampaign;
use super::case::DetectiveLiveRecord;
use crate::db::AiScheduleItem;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectiveContext {
    pub courses: Vec<DetectiveCourse>,
    pub review_queue: Vec<DetectiveReviewItem>,
    pub recent_results: Vec<DetectiveCaseResult>,
    pub generated_at: i64,
    /// Course keys the user has explicitly opted into Detective. Empty by
    /// default — when empty the frontend shows the selection screen.
    pub included_course_keys: Vec<String>,
    /// Cross-session memory shown in the Detective HQ home — busted topics,
    /// pending doubts, recently-used evidence.
    pub memory: DetectiveMemory,
    /// Per-course campaign bibles (世界観 layer) that have already been
    /// generated and cached. Empty entries are omitted — the title screen
    /// uses these to show the world/tagline/meta-progress for selected courses.
    #[serde(default)]
    pub campaigns: Vec<DetectiveCampaign>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectiveCourse {
    pub name: String,
    pub key: String,
    pub live_records: Vec<DetectiveLiveRecord>,
    pub exam_signals: Vec<DetectiveSignal>,
    pub schedule_items: Vec<AiScheduleItem>,
    pub latest_at: i64,
    pub doubts: Vec<DetectiveDoubt>,
    pub recent_results: Vec<DetectiveCaseResult>,
    pub case_type: String,
    pub readiness: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectiveSignal {
    pub id: String,
    pub source_id: String,
    pub title: String,
    pub date: String,
    pub category: String,
    pub source: String,
    pub course_info: String,
    pub source_url: String,
    pub information_type: String,
    pub person_category_cd: String,
    pub category_cd: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectiveDoubt {
    pub id: String,
    pub course_name: String,
    pub note: String,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
    pub created_at: i64,
    pub due_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectiveCaseResult {
    pub id: String,
    pub case_id: String,
    pub course_key: String,
    pub course_name: String,
    pub case_title: String,
    pub case_type: String,
    #[serde(default)]
    pub selected_evidence_ids: Vec<String>,
    pub relation: String,
    pub deduction: String,
    pub closed_at: i64,
    pub confidence: u8,
}

/// One chapter of a course's campaign — backed by a single Live note.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectiveChapterInfo {
    pub live_id: String,
    /// 1-based chapter number (oldest lecture = chapter 1).
    pub index: u8,
    /// Chapter title — from the cached case if generated, else 「第N章」.
    pub title: String,
    /// True when this chapter's case has already been generated + cached.
    pub generated: bool,
    /// True when the player has an archived result for this chapter.
    pub played: bool,
    /// Best (latest) confidence 1–5, or 0 if never played.
    pub best_confidence: u8,
    pub played_at: i64,
    /// True for a PLANNED-but-not-yet-delivered lecture (per 授業計画) that has
    /// no Live note yet — shown locked so the player sees the full arc length.
    #[serde(default)]
    pub locked: bool,
    /// True when `index` is a content-confirmed 第N回 (vs. a not-yet-aligned note).
    #[serde(default)]
    pub aligned: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectiveReviewItem {
    pub id: String,
    pub course_key: String,
    pub course_name: String,
    pub reason: String,
    pub priority: u8,
    pub due_at: i64,
}

/// Cross-session memory: drives content continuity across Detective sessions.
/// Persisted as JSON under DETECTIVE_MEMORY_KEY.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectiveMemory {
    /// Topics the player has consistently busted — AI should de-emphasise.
    #[serde(default)]
    pub mastered: Vec<MemoryItem>,
    /// Topics the player failed on — AI should re-emphasise next session.
    #[serde(default)]
    pub mistakes: Vec<MemoryItem>,
    /// Evidence ids used in the last several sessions; AI tries to vary picks.
    #[serde(default)]
    pub recent_evidence_titles: Vec<String>,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryItem {
    /// A short tag describing the topic — derived from the busted/failed lie text.
    pub topic: String,
    /// Course this came from (best-effort).
    #[serde(default)]
    pub course_name: String,
    pub at: i64,
}

#[derive(Default)]
pub(crate) struct CourseBuilder {
    pub(crate) name: String,
    pub(crate) key: String,
    pub(crate) live_records: Vec<DetectiveLiveRecord>,
    pub(crate) exam_signals: Vec<DetectiveSignal>,
    pub(crate) schedule_items: Vec<AiScheduleItem>,
    pub(crate) doubts: Vec<DetectiveDoubt>,
    pub(crate) recent_results: Vec<DetectiveCaseResult>,
    pub(crate) latest_at: i64,
}
