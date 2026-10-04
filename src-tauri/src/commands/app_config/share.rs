#[cfg(target_os = "macos")]
use super::share_macos::open_macos_share_picker;
#[cfg(target_os = "windows")]
use super::share_windows::open_windows_file_share_picker;

use base64::Engine;

fn decode_image_base64(data_base64: &str) -> Result<Vec<u8>, String> {
    let trimmed = data_base64.trim();
    let payload = trimmed
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(',').map(|(_, data)| data))
        .unwrap_or(trimmed);
    base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .map_err(|err| format!("画像データのデコードに失敗しました: {err}"))
}

// ============ Image Share ============

/// Save PNG image data to a file using the native file dialog.
#[tauri::command]
pub async fn save_image_file(data_base64: String, default_name: String) -> Result<String, String> {
    let data = decode_image_base64(&data_base64)?;
    let result = rfd::AsyncFileDialog::new()
        .set_title("時間割画像を保存")
        .set_file_name(&default_name)
        .add_filter("PNG画像", &["png"])
        .save_file()
        .await;

    match result {
        Some(handle) => {
            let path = handle.path().to_path_buf();
            std::fs::write(&path, &data).map_err(|e| format!("保存に失敗しました: {}", e))?;
            Ok(path.to_string_lossy().to_string())
        }
        None => Err("cancelled".into()),
    }
}

/// Copy PNG image data to the system clipboard using native APIs.
#[tauri::command]
pub async fn copy_image_to_clipboard(data_base64: String) -> Result<(), String> {
    let data = decode_image_base64(&data_base64)?;
    // Write to temp file
    let tmp_dir = std::env::temp_dir().join("selah-share");
    std::fs::create_dir_all(&tmp_dir)
        .map_err(|e| format!("一時ディレクトリの作成に失敗: {}", e))?;
    let tmp_path = tmp_dir.join("clipboard_tmp.png");
    std::fs::write(&tmp_path, &data).map_err(|e| format!("一時ファイルの書き込みに失敗: {}", e))?;

    let result = {
        #[cfg(target_os = "macos")]
        {
            use std::process::Command;
            let script = format!(
                r#"use framework "AppKit"
use framework "Foundation"
set theImage to current application's NSImage's alloc()'s initWithContentsOfFile:"{}"
set pb to current application's NSPasteboard's generalPasteboard()
pb's clearContents()
pb's writeObjects:(current application's NSArray's arrayWithObject:theImage)"#,
                tmp_path.display()
            );
            Command::new("osascript")
                .arg("-e")
                .arg(&script)
                .output()
                .map_err(|e| format!("クリップボードへのコピーに失敗: {}", e))
                .and_then(|out| {
                    if out.status.success() {
                        Ok(())
                    } else {
                        Err(format!(
                            "クリップボードへのコピーに失敗: {}",
                            String::from_utf8_lossy(&out.stderr)
                        ))
                    }
                })
        }

        #[cfg(target_os = "windows")]
        {
            use std::process::Command;
            let path_str = tmp_path.to_string_lossy().replace('\'', "''");
            let script = format!(
                r#"Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; $img = [System.Drawing.Image]::FromFile('{}'); [System.Windows.Forms.Clipboard]::SetImage($img); $img.Dispose()"#,
                path_str
            );
            Command::new("powershell")
                .args(["-NoProfile", "-Command", &script])
                .output()
                .map_err(|e| format!("クリップボードへのコピーに失敗: {}", e))
                .and_then(|out| {
                    if out.status.success() {
                        Ok(())
                    } else {
                        Err(format!(
                            "クリップボードへのコピーに失敗: {}",
                            String::from_utf8_lossy(&out.stderr)
                        ))
                    }
                })
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err::<(), String>("この OS ではクリップボードコピーはサポートされていません".into())
        }
    };

    // Clean up temp file
    let _ = std::fs::remove_file(&tmp_path);

    result
}

/// Share PNG image data via the native OS share sheet.
/// On macOS opens the native share picker.
/// Temp files are cleaned up after the share UI has had time to consume them.
#[tauri::command]
pub async fn share_image_native(
    app: tauri::AppHandle,
    data_base64: String,
    file_name: String,
) -> Result<(), String> {
    let data = decode_image_base64(&data_base64)?;
    // Write to a temp file
    let tmp_dir = std::env::temp_dir().join("selah-share");
    std::fs::create_dir_all(&tmp_dir)
        .map_err(|e| format!("一時ディレクトリの作成に失敗: {}", e))?;
    let tmp_path = tmp_dir.join(&file_name);
    std::fs::write(&tmp_path, &data).map_err(|e| format!("一時ファイルの書き込みに失敗: {}", e))?;
    let result = share_file_path_native(app, tmp_path.clone(), file_name).await;

    // Keep the file around long enough for slower share targets to read it.
    let cleanup_path = tmp_path.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(600));
        let _ = std::fs::remove_file(&cleanup_path);
        // Try to remove the directory if empty
        let _ = std::fs::remove_dir(cleanup_path.parent().unwrap_or(std::path::Path::new("")));
    });

    result
}

pub(crate) async fn share_file_path_native(
    app: tauri::AppHandle,
    path: std::path::PathBuf,
    file_name: String,
) -> Result<(), String> {
    share_file_paths_native(app, vec![(path, file_name)]).await
}

pub(crate) async fn share_file_paths_native(
    app: tauri::AppHandle,
    files: Vec<(std::path::PathBuf, String)>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || share_file_paths_native_blocking(&app, &files))
        .await
        .map_err(|e| format!("Native share task failed: {e}"))?
}

fn share_file_paths_native_blocking(
    app: &tauri::AppHandle,
    files: &[(std::path::PathBuf, String)],
) -> Result<(), String> {
    if files.is_empty() {
        return Err("共有するファイルが選択されていません".into());
    }
    for (path, _) in files {
        if !path.is_file() {
            return Err("共有するファイルが見つかりません".into());
        }
    }

    #[cfg(target_os = "macos")]
    {
        open_macos_share_picker(app, files)
    }

    #[cfg(target_os = "windows")]
    {
        return open_windows_file_share_picker(app, files);
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = app;
        let _ = files;
        Err("この OS では共有機能はサポートされていません".into())
    }
}
