use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaDetailSection {
    pub heading: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaDetailPage {
    pub title: String,
    pub course_name: String,
    pub sections: Vec<LunaDetailSection>,
    pub attachments: Vec<LunaAttachment>,
    pub meta: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaAttachment {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub link_type: String, // "file", "external", "video", "zoom", "panopto", "web"
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub object_name: String,
    /// Form action path for download (e.g. /lms/course/report/submission_download)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub download_action: String,
    /// Fixed form params (reportId, idnumber, etc.) serialized as key=value pairs
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub download_params: Vec<(String, String)>,
}
