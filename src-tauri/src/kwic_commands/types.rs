//! KWIC portal response types.

use serde::{Deserialize, Serialize};

// ============ Types ============

/// A notification/information entry from the KWIC Portal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KwicPortalNotification {
    pub id: String,
    pub title: String,
    pub date: String,
    pub category: String,
    pub important: bool,
    /// data2: informationType (e.g. "10")
    pub information_type: String,
    /// data3: personCategoryCd (e.g. "0")
    pub person_category_cd: String,
    /// data4: categoryCd (e.g. "02")
    pub category_cd: String,
}

/// The home page data from KWIC Portal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KwicPortalHome {
    /// Category sections on the home page
    pub sections: Vec<KwicPortalSection>,
    /// Raw HTML for debug/exploration (only in debug mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_html_debug: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KwicPortalSection {
    pub title: String,
    pub items: Vec<KwicPortalItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KwicPortalItem {
    pub id: String,
    pub title: String,
    pub date: String,
    pub category: String,
    pub url: String,
    pub important: bool,
    #[serde(default)]
    pub information_type: String,
    #[serde(default)]
    pub person_category_cd: String,
    #[serde(default)]
    pub category_cd: String,
}

/// Parsed detail content of a KWIC Portal notification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KwicNotificationDetail {
    pub title: String,
    pub date: String,
    pub sender: String,
    pub body_html: String,
    /// Attachment file names / links (if any)
    pub attachments: Vec<KwicAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KwicAttachment {
    pub name: String,
    pub url: String,
}

/// A link/item from a KWIC Portal subportal page
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KwicSubportalLink {
    pub title: String,
    pub url: String,
    pub icon_url: String,
    pub description: String,
}

/// Subportal page data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KwicSubportalData {
    pub title: String,
    pub links: Vec<KwicSubportalLink>,
    /// Notification items on this subportal
    pub notifications: Vec<KwicPortalNotification>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KwicCabinetItem {
    pub cabinet_id: String,
    pub list_id: String,
    pub name: String,
    pub level: u32,
    pub updated_at: String,
    pub is_new: bool,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KwicCabinetReference {
    pub title: String,
    pub items: Vec<KwicCabinetItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_html_debug: Option<String>,
}
