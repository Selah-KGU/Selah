//! Read-only tool implementations for the Selah agent.
//!
//! Each tool takes a JSON-encoded argument object (often empty `{}`) and
//! returns a JSON value.  Tools are intentionally few and semantically
//! narrow so a 2B model can reliably pick among them.

use serde_json::{json, Value};
use tauri::Manager;

use crate::db::Database;

#[derive(Debug, Clone)]
pub(crate) struct ReusableCourseDownload {
    pub path: String,
    pub source_fingerprint: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ReusableActivityDetail {
    pub list_fingerprint: String,
    pub source_fingerprint: String,
    pub checked_at: i64,
}

#[path = "agent_tools/academic.rs"]
mod academic;
#[path = "agent_tools/calendar.rs"]
mod calendar;
#[path = "agent_tools/copilot_page.rs"]
mod copilot_page;
#[path = "agent_tools/files_browser.rs"]
mod files_browser;
#[path = "agent_tools/insights.rs"]
mod insights;
#[path = "agent_tools/mail_lookup.rs"]
mod mail_lookup;
#[path = "agent_tools/records.rs"]
mod records;
#[path = "agent_tools/sanitize.rs"]
mod sanitize;

use academic::*;
pub use sanitize::sanitize_tool_args;
pub(in crate::agent_tools) use sanitize::*;

pub(crate) async fn fetch_luna_course_contents(
    app: &tauri::AppHandle,
    luna_id: &str,
) -> Result<crate::luna_parser::LunaCourseContents, String> {
    files_browser::fetch_luna_course_contents_for_download(app, luna_id).await
}

pub(crate) async fn download_luna_course_material(
    app: &tauri::AppHandle,
    luna_id: &str,
    filename: &str,
) -> Result<Option<Value>, String> {
    files_browser::download_course_material_from_contents(app, luna_id, filename).await
}

pub(crate) fn read_downloaded_text(path: &std::path::Path) -> Result<String, String> {
    files_browser::read_supported_download_file(path)
}

/// Full-document extraction (no PDF page cap) — used by the paper checker,
/// which must analyse the whole file rather than an agent-sized excerpt.
pub(crate) fn read_downloaded_text_full(path: &std::path::Path) -> Result<String, String> {
    files_browser::read_supported_download_file_full(path)
}

/// Embedded page images for a scanned PDF whose text layer is empty, so a
/// vision model can still read it. Errors for non-PDF or image-free files.
pub(crate) fn read_downloaded_images(
    path: &std::path::Path,
) -> Result<Vec<crate::ai::ImagePart>, String> {
    files_browser::extract_pdf_images(path)
}

/// Last resort for a PDF with no text layer and no embedded images (e.g. a
/// vector slide deck): rasterize its pages so a vision model can read them.
pub(crate) fn render_pdf_images(
    path: &std::path::Path,
) -> Result<Vec<crate::ai::ImagePart>, String> {
    files_browser::render_pdf_to_images(path)
}

pub(crate) async fn download_luna_activity_attachments(
    app: &tauri::AppHandle,
    luna_id: &str,
    contents: &crate::luna_parser::LunaCourseContents,
    kinds: &[&str],
    reusable_paths: &std::collections::HashMap<String, ReusableCourseDownload>,
    reusable_details: &std::collections::HashMap<String, ReusableActivityDetail>,
    detail_cache_ttl_secs: i64,
    now: i64,
    force_detail_fetch: bool,
) -> Result<Vec<Value>, String> {
    files_browser::download_all_luna_activity_attachments(
        app,
        luna_id,
        contents,
        kinds,
        reusable_paths,
        reusable_details,
        detail_cache_ttl_secs,
        now,
        force_detail_fetch,
    )
    .await
}

/// Maximum number of list items returned by any single tool.
const LIST_CAP: usize = 15;
/// Mail body truncation threshold (bytes).
const MAIL_BODY_CAP: usize = 4096;

#[path = "agent_tools/catalog.rs"]
mod catalog;
#[path = "agent_tools/cjk.rs"]
mod cjk;
#[path = "agent_tools/dispatch.rs"]
mod tool_dispatch;

pub use catalog::{
    canonical_tool_name, exact_tool_name, tool_catalog_prompt, tool_catalog_signatures,
};
#[cfg(test)]
pub use catalog::{dispatched_tool_names, is_known_tool, registered_tool_names};
pub(crate) use cjk::normalize_cjk_char;
pub use tool_dispatch::dispatch;
