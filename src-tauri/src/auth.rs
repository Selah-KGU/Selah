use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSession {
    pub username: String,
    pub display_name: String,
    pub student_id: String,
    pub faculty: String,
    pub department: String,
}
