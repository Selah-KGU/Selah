//! Open and share downloaded files.

use super::*;

#[tauri::command]
pub async fn open_downloaded_file(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let canonical = validate_downloads_path(&path)?;
    if is_markdown_ext(&canonical) {
        return open_markdown_file_window(app, canonical.to_string_lossy().to_string()).await;
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(canonical.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("ファイルを開けませんでした: {}", e))?;
    Ok(())
}

/// Open a downloaded file with the OS default app, bypassing the built-in
/// Markdown reader. Used by the reader's "外部で開く" button.
#[tauri::command]
pub fn open_downloaded_file_external(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let canonical = validate_downloads_path(&path)?;
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(canonical.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("ファイルを開けませんでした: {}", e))?;
    Ok(())
}

/// Share a downloaded/material file through the native OS share surface. The
/// path is restricted to the same managed download roots used by file opening.
#[tauri::command]
pub async fn share_downloaded_file_native(
    app: tauri::AppHandle,
    path: String,
) -> Result<(), String> {
    let canonical = validate_downloads_path(&path)?;
    let file_name = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Markdown")
        .to_string();
    super::super::app_config::share_file_path_native(app, canonical, file_name).await
}

#[tauri::command]
pub async fn share_downloaded_files_native(
    app: tauri::AppHandle,
    paths: Vec<String>,
) -> Result<(), String> {
    if paths.is_empty() {
        return Err("共有するファイルが選択されていません".into());
    }
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let canonical = validate_downloads_path(&path)?;
        let file_name = canonical
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        files.push((canonical, file_name));
    }
    super::super::app_config::share_file_paths_native(app, files).await
}
