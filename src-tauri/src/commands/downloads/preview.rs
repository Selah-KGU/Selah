//! In-app previews for downloaded files.

use super::*;
use base64::Engine;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct DownloadPreview {
    pub kind: String,
    pub mime: String,
    pub data_url: Option<String>,
    pub text: Option<String>,
}

pub(super) fn is_markdown_ext(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
        .unwrap_or(false)
}

fn preview_mime(path: &std::path::Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "svg" => Some("image/svg+xml"),
        _ => None,
    }
}

fn is_text_preview_ext(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "md" | "markdown" | "txt" | "csv" | "json" | "log"
            )
        })
        .unwrap_or(false)
}

#[tauri::command]
pub fn get_download_preview(path: String) -> Result<Option<DownloadPreview>, String> {
    const IMAGE_PREVIEW_MAX_BYTES: u64 = 10 * 1024 * 1024;
    const TEXT_PREVIEW_MAX_BYTES: u64 = 512 * 1024;
    const TEXT_PREVIEW_CHARS: usize = 700;

    let canonical = validate_downloads_path(&path)?;
    let meta = std::fs::metadata(&canonical).map_err(|e| format!("読み込み失敗: {}", e))?;
    if !meta.is_file() {
        return Ok(None);
    }

    if let Some(mime) = preview_mime(&canonical) {
        if meta.len() > IMAGE_PREVIEW_MAX_BYTES {
            return Ok(None);
        }
        let bytes = std::fs::read(&canonical).map_err(|e| format!("読み込み失敗: {}", e))?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        return Ok(Some(DownloadPreview {
            kind: "image".to_string(),
            mime: mime.to_string(),
            data_url: Some(format!("data:{};base64,{}", mime, encoded)),
            text: None,
        }));
    }

    if is_text_preview_ext(&canonical) {
        if meta.len() > TEXT_PREVIEW_MAX_BYTES {
            return Ok(None);
        }
        let bytes = std::fs::read(&canonical).map_err(|e| format!("読み込み失敗: {}", e))?;
        let raw = String::from_utf8_lossy(&bytes);
        let preview = raw
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let text: String = preview.chars().take(TEXT_PREVIEW_CHARS).collect();
        if text.trim().is_empty() {
            return Ok(None);
        }
        return Ok(Some(DownloadPreview {
            kind: "text".to_string(),
            mime: "text/plain".to_string(),
            data_url: None,
            text: Some(text),
        }));
    }

    Ok(None)
}
