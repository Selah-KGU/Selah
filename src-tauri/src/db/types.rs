use super::*;

// ── Row types ──

pub type PlannedSession = (i32, String, String);
pub type PlannedSessionsByName = Vec<(String, Vec<PlannedSession>)>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionPlanRow {
    pub session_num: i32,
    pub th_header: String,
    pub topic: String,
    pub delivery_mode: String,
    pub study_outside: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaCountsRow {
    pub announcements: i32,
    pub new_announcements: i32,
    pub reports: i32,
    pub exams: i32,
    pub discussions: i32,
}

/// Individual Luna activity item (announcement, report, exam, discussion, material).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaActivityRow {
    pub luna_id: String,
    pub activity_type: String, // "announcement", "report", "exam", "discussion", "material"
    pub title: String,
    pub period: String, // deadline / date range
    pub status: String, // e.g. "未提出", "提出済", "未回答", "new"
    #[serde(default)]
    pub detail_path: String, // Luna path for fetching detail on demand
}

/// KGC course detail fields (授業概要, 成績評価 etc.) extracted from detail page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KgcCourseDetailRow {
    pub kgc_code: String,
    pub fields: Vec<(String, String)>, // (label, value) pairs
    pub delivery_mode: String,         // detected from detail page
    pub textbooks: Vec<crate::parser::TextbookEntry>,
}

/// Raw KGC course entry stored in DB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KgcCourseRow {
    pub id: i64,
    pub kgc_code: String,
    pub name: String,
    pub day: i32,
    pub period: i32,
    pub room: String,
    pub detail_path: String,
    pub is_cancelled: bool,
    pub is_makeup: bool,
    pub is_room_changed: bool,
    pub week_label: String,
}

/// Raw Luna course entry stored in DB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaCourseRow {
    pub id: i64,
    pub luna_id: String,
    pub name: String,
    pub teacher: String,
    pub day: i32,
    pub period: i32,
}

/// AI-generated schedule item for a single class session.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiScheduleItem {
    #[serde(default)]
    pub day: i32,
    #[serde(default)]
    pub period: i32,
    #[serde(default)]
    pub course_name: String,
    #[serde(default)]
    pub delivery_mode: String,
    #[serde(default)]
    pub room: String,
    #[serde(default)]
    pub teacher: String,
    #[serde(default)]
    pub session_topic: String,
    #[serde(default)]
    pub is_cancelled: bool,
    #[serde(default)]
    pub notifications: Vec<String>,
    #[serde(default)]
    pub assignments: Vec<String>,
    #[serde(default)]
    pub exams: Vec<String>,
}

/// Full AI schedule result for two weeks.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiScheduleResult {
    #[serde(default)]
    pub current_week_label: String,
    #[serde(default)]
    pub next_week_label: String,
    #[serde(default)]
    pub current_week: Vec<AiScheduleItem>,
    #[serde(default)]
    pub next_week: Vec<AiScheduleItem>,
    #[serde(default)]
    pub weekly_summary: String,
    #[serde(default)]
    pub cross_week_insights: String,
}

/// Raw data collected from both platforms, passed to AI for analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleRawData {
    pub kgc_entries_current: Vec<KgcCourseRow>,
    pub kgc_entries_next: Vec<KgcCourseRow>,
    pub luna_courses: Vec<LunaCourseRow>,
    pub session_plans: Vec<(String, Vec<SessionPlanRow>)>, // (kgc_code, plans)
    pub luna_counts: Vec<(String, LunaCountsRow)>,         // (luna_id, counts)
    pub luna_activities: Vec<LunaActivityRow>,             // detailed activity items
    pub kgc_course_details: Vec<KgcCourseDetailRow>,       // KGC course detail fields
    pub current_week_label: String,
    pub next_week_label: String,
    pub luna_communities: Vec<crate::luna_parser::LunaCommunity>,
}

/// Persisted snapshot metadata: week labels + Luna selector state.
/// Allows rebuilding ScheduleResponse from DB without network.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SnapshotState {
    pub current_week_label: String,
    pub next_week_label: String,
    pub luna_year: String,
    pub luna_term: String,
    pub luna_communities: Vec<luna_parser::LunaCommunity>,
    pub luna_year_options: Vec<luna_parser::SelectOption>,
    pub luna_term_options: Vec<luna_parser::SelectOption>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConversationRow {
    pub id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessageRow {
    pub id: i64,
    pub conv_id: String,
    pub role: String,
    pub content: String,
    pub images_json: Option<String>,
    pub tool_name: Option<String>,
    pub tool_result_json: Option<String>,
    pub created_at: i64,
}
