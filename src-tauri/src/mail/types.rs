use serde::{Deserialize, Serialize};

/// Persisted token data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenData {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix timestamp (seconds) when access_token expires
    pub expires_at: i64,
}

/// A single mail message from Graph API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailMessage {
    pub id: String,
    pub subject: Option<String>,
    pub body_preview: Option<String>,
    pub from: Option<MailAddress>,
    pub received_date_time: Option<String>,
    pub is_read: Option<bool>,
    pub has_attachments: Option<bool>,
    /// Plain-text body. Populated by fetch_inbox via Graph `body` field +
    /// `Prefer: outlook.body-content-type="text"`. None for legacy cache entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<MailBody>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailAddress {
    pub email_address: EmailAddress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailAddress {
    pub name: Option<String>,
    pub address: Option<String>,
}

/// A mail attachment entry (metadata only, no content bytes)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailAttachment {
    pub id: String,
    pub name: Option<String>,
    pub content_type: Option<String>,
    pub size: Option<i64>,
}

/// Full mail body for detail view
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailDetail {
    pub id: String,
    pub subject: Option<String>,
    pub body: Option<MailBody>,
    pub from: Option<MailAddress>,
    pub received_date_time: Option<String>,
    pub is_read: Option<bool>,
    pub has_attachments: Option<bool>,
    pub to_recipients: Option<Vec<MailAddress>>,
    pub cc_recipients: Option<Vec<MailAddress>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailBody {
    pub content_type: Option<String>,
    pub content: Option<String>,
}

/// Graph API list response wrapper
#[derive(Debug, Deserialize)]
pub(crate) struct GraphListResponse<T> {
    pub value: Vec<T>,
}

/// User profile from Graph API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailProfile {
    pub display_name: Option<String>,
    pub mail: Option<String>,
    pub user_principal_name: Option<String>,
}
