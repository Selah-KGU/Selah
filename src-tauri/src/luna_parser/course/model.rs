use serde::{Deserialize, Serialize};

// ──────────────────────────────────────────────
// Course top page (/lms/course?idnumber=)
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaCourseContents {
    pub course_name: String,
    pub semester: String,
    pub teachers: String,
    pub ta_info: String,
    pub la_info: String,
    pub syllabus_url: String,
    pub grade_url: String,
    pub menus: Vec<LunaCourseMenu>,
    pub announcements: Vec<LunaCourseAnnouncement>,
    pub online_tools: Vec<LunaOnlineTool>,
    pub materials: Vec<LunaContentItem>,
    pub reports: Vec<LunaContentItem>,
    pub examinations: Vec<LunaContentItem>,
    pub discussions: Vec<LunaContentItem>,
    pub surveys: Vec<LunaContentItem>,
    #[serde(default)]
    pub attendances: Vec<LunaAttendanceItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaAttendanceItem {
    pub title: String,
    pub date: String,
    pub status: String,
    #[serde(default)]
    pub can_register: bool,
    #[serde(default)]
    pub idnumber: String,
    #[serde(default)]
    pub attendance_id: String,
    #[serde(default)]
    pub log_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaCourseMenu {
    pub name: String,
    pub module_type: String,
    pub icon: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaContentItem {
    pub title: String,
    pub url: String,
    pub period: String,
    pub status: String,
    pub item_type: String, // material, report, examination, discussion
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub files: Vec<LunaMaterialFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaMaterialFile {
    pub display_name: String, // link text (e.g. "2026日本語Ⅰ金_配布用シラバス")
    pub file_name: String,    // actual filename (e.g. "2026日本語Ⅰ金_配布用シラバス.pdf")
    pub object_name: String,  // storage path (e.g. "2026/ee/3c/1b/...")
    pub resource_id: String,  // resource ID
    pub material_id: String,  // dlMaterialId
    pub file_type: String,    // "0" = file, else HTML
    pub end_date: String,     // open end date (e.g. "2026-07-04 00:00:00.0")
    pub scan_status: String,  // virus scan status ("1" = clean)
    #[serde(default)]
    pub link_type: String, // "file", "zoom", "panopto", "video", "cloud", "google", "teams", "web"
    /// Direct external URL for pure link-type materials (Zoom, YouTube, etc.).
    /// When set, frontend should open this directly instead of using the tempfile flow.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub external_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaCourseAnnouncement {
    pub title: String,
    pub info_id: String,
    pub start_date: String,
    pub end_date: String,
    pub is_new: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaOnlineTool {
    pub name: String,
    pub url: String,
    pub icon: String,
}

// ── Survey (questionnaire) detail types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaSurveyDetail {
    pub title: String,
    pub description: String,
    pub period: String,
    pub anonymity: String,
    pub allow_edit: String,
    pub answer_status: String,
    pub respondent: String,
    pub attachments: Vec<LunaSurveyAttachment>,
    pub questions: Vec<LunaSurveyQuestion>,
    /// Hidden form fields needed for submission (_cid, _csrf, idnumber, surveyId, takeFlag,
    /// and per-question answer[N].surveyNo / answer[N].surveyNoSub)
    #[serde(default)]
    pub form_fields: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub form_action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaSurveyAttachment {
    pub file_name: String,
    pub object_name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub download_action: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub download_params: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaSurveyQuestion {
    pub number: String,
    pub body: String,
    pub required: bool,
    pub answer_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub answer_name: String,
    pub options: Vec<LunaSurveyOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaSurveyOption {
    pub value: String,
    pub label: String,
}
