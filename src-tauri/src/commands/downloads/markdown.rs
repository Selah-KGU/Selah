//! In-app markdown reader window.

use super::*;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{LazyLock, Mutex};

/// Max size of a markdown file the in-app reader will load. Larger files are
/// pushed to the external opener.
const MARKDOWN_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// Pending payloads keyed by window label. The markdown reader window pulls
/// from here on startup to avoid a race where the backend emits the
/// `markdown-content` event before the page's listener attaches.
static PENDING_MARKDOWN_PAYLOADS: LazyLock<Mutex<HashMap<String, serde_json::Value>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn markdown_window_label(canonical: &std::path::Path) -> String {
    let mut hasher = DefaultHasher::new();
    canonical.to_string_lossy().hash(&mut hasher);
    format!("md-reader-{:x}", hasher.finish())
}

fn markdown_payload_for_file(canonical: &std::path::Path, filename: &str) -> serde_json::Value {
    let path_str = canonical.to_string_lossy().to_string();
    match std::fs::read(canonical) {
        Ok(bytes) => serde_json::json!({
            "path": path_str,
            "filename": filename,
            "markdown": String::from_utf8_lossy(&bytes).to_string(),
            "error": serde_json::Value::Null,
        }),
        Err(e) => serde_json::json!({
            "path": path_str,
            "filename": filename,
            "markdown": "",
            "error": format!("読み込み失敗: {}", e),
        }),
    }
}

fn queue_markdown_payload_emit(
    app: tauri::AppHandle,
    label: String,
    canonical: std::path::PathBuf,
    filename: String,
) {
    use tauri::Emitter;
    tauri::async_runtime::spawn(async move {
        let error_path = canonical.to_string_lossy().to_string();
        let error_filename = filename.clone();
        let payload = match tokio::task::spawn_blocking(move || {
            markdown_payload_for_file(&canonical, &filename)
        })
        .await
        {
            Ok(payload) => payload,
            Err(e) => serde_json::json!({
                "path": error_path,
                "filename": error_filename,
                "markdown": "",
                "error": format!("読み込み失敗: {}", e),
            }),
        };
        if let Ok(mut map) = PENDING_MARKDOWN_PAYLOADS.lock() {
            map.insert(label.clone(), payload.clone());
        }
        let _ = app.emit_to(
            tauri::EventTarget::AnyLabel {
                label: label.clone(),
            },
            "markdown-content",
            &payload,
        );

        let payload_clone = payload.clone();
        let label_clone = label.clone();
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
            let _ = app_clone.emit_to(
                tauri::EventTarget::AnyLabel { label: label_clone },
                "markdown-content",
                &payload_clone,
            );
        });
    });
}

/// Open (or focus) the in-app Markdown reader window for the given file.
#[tauri::command]
pub async fn open_markdown_file_window(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let canonical = validate_downloads_path(&path)?;
    let meta = std::fs::metadata(&canonical).map_err(|e| format!("読み込み失敗: {}", e))?;
    if meta.len() > MARKDOWN_MAX_BYTES {
        // Fall back to the system opener for oversized files so the user can
        // still get to them.
        return open_downloaded_file_external(app, canonical.to_string_lossy().to_string());
    }
    let filename = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Markdown")
        .to_string();
    let label = markdown_window_label(&canonical);

    let source_path = canonical.to_string_lossy().to_string();
    let tab = crate::document_tabs::open_markdown_reader_tab(
        &app,
        label,
        filename.clone(),
        Some(source_path),
    )?;

    // Read and parse the file payload after the tab is created so opening a
    // large-but-supported note does not block the native window from appearing.
    queue_markdown_payload_emit(app, tab.target, canonical, filename);

    Ok(())
}

/// Called by the markdown reader on init to fetch its payload synchronously.
/// Removes the entry so subsequent calls return null.
#[tauri::command]
pub fn get_pending_markdown_payload(label: String) -> Option<serde_json::Value> {
    PENDING_MARKDOWN_PAYLOADS
        .lock()
        .ok()
        .and_then(|mut m| m.remove(&label))
}

/// Write Markdown contents back to disk. Restricted to .md/.markdown files
/// inside the allowed download roots, with a size cap matching the reader's.
#[tauri::command]
pub fn write_markdown_file(path: String, contents: String) -> Result<(), String> {
    let canonical = validate_downloads_path(&path)?;
    if !is_markdown_ext(&canonical) {
        return Err("Markdown ファイルのみ編集できます".into());
    }
    if contents.len() as u64 > MARKDOWN_MAX_BYTES {
        return Err("ファイルが大きすぎます（8MBを超えるMarkdownはサポートしていません）".into());
    }
    std::fs::write(&canonical, contents.as_bytes()).map_err(|e| format!("保存失敗: {}", e))?;
    Ok(())
}
