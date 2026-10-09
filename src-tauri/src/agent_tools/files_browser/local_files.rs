//! List, read, write, delete, and open files in the download sandbox.

use super::*;

pub async fn list_downloaded_files(args: &Value) -> Result<Value, String> {
    let keyword = sanitize_text_arg(args, "keyword", 80).unwrap_or_default();
    let keyword_norm = normalize_text(&keyword);
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .min(LIST_CAP as u64) as usize;

    let mut records = crate::background_ipc::run(
        "Downloaded files worker failed",
        crate::commands::list_downloads_snapshot,
    )
    .await?;
    records.retain(|r| r.file_exists);
    if !keyword_norm.is_empty() {
        records.retain(|r| {
            let hay = normalize_text(&format!("{} {} {}", r.filename, r.course_name, r.path));
            hay.contains(&keyword_norm)
        });
    }

    let files: Vec<Value> = records
        .into_iter()
        .take(limit)
        .map(|r| {
            json!({
                "filename": r.filename,
                "path": r.path,
                "course_name": r.course_name,
                "source": r.source,
                "size_bytes": r.size_bytes,
                "downloaded_at": r.downloaded_at,
            })
        })
        .collect();

    Ok(json!({
        "keyword": keyword,
        "files": files,
    }))
}

pub async fn read_downloaded_file(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let mut path = resolve_downloaded_file_arg(args)?;
    if !path.exists() {
        if let Ok(downloaded_path) = auto_download_missing_file(app, &path).await {
            path = downloaded_path;
        } else {
            return Err(format!(
                "ファイルが見つかりません。自動ダウンロードも失敗しました。物理パス: {:?}",
                path
            ));
        }
    }
    let ext = file_extension_lower(&path);
    if !supported_read_extension(&ext) && ext != "doc" {
        return Err(format!("未対応の拡張子です: .{}", ext));
    }
    let metadata = std::fs::metadata(&path).map_err(|e| format!("ファイル情報取得失敗: {}", e))?;
    let text = read_supported_download_file(&path)?;
    Ok(json!({
        "path": path.to_string_lossy(),
        "filename": path.file_name().and_then(|n| n.to_str()).unwrap_or_default(),
        "extension": ext,
        "size_bytes": metadata.len(),
        "content": truncate_chars(&text, 12_000),
    }))
}

fn resolve_downloaded_file_arg(args: &Value) -> Result<PathBuf, String> {
    let raw_path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if !raw_path.is_empty() {
        return resolve_allowed_download_path(raw_path);
    }

    let filename = args
        .get("filename")
        .or_else(|| args.get("file_name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if filename.is_empty() {
        return Err("path または filename を指定してください".into());
    }
    if filename.contains('\0') || filename.contains('/') || filename.contains('\\') {
        return Err("filename が不正です".into());
    }

    let course_hint = args
        .get("course_name")
        .or_else(|| args.get("course"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let filename_norm = normalize_text(filename);
    let course_norm = normalize_text(course_hint);
    let mut records = crate::commands::list_downloads_snapshot()?;
    records.retain(|r| r.file_exists);
    if !course_norm.is_empty() {
        records.retain(|r| normalize_text(&r.course_name).contains(&course_norm));
    }

    records
        .iter()
        .find(|r| r.filename == filename)
        .or_else(|| {
            records
                .iter()
                .find(|r| normalize_text(&r.filename) == filename_norm)
        })
        .or_else(|| {
            records
                .iter()
                .find(|r| normalize_text(&r.filename).contains(&filename_norm))
        })
        .map(|r| PathBuf::from(&r.path))
        .ok_or_else(|| {
            format!(
                "filename に一致するダウンロード済みファイルが見つかりません: {}",
                filename
            )
        })
}

pub async fn write_downloaded_text_file(args: &Value) -> Result<Value, String> {
    let raw_path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
    if raw_path.is_empty() {
        return Err("pathを指定してください".into());
    }
    if content.is_empty() {
        return Err("contentが空です".into());
    }
    let path = resolve_allowed_download_path(raw_path)?;
    let ext = file_extension_lower(&path);
    if !supported_write_extension(&ext) {
        return Err("書き込みできるのは .txt / .md / .json / .csv / .html のみです".into());
    }
    let metadata = std::fs::metadata(&path).map_err(|e| format!("ファイル情報取得失敗: {}", e))?;
    if metadata.len() > 2_000_000 {
        return Err("大きすぎるファイルは編集できません".into());
    }
    std::fs::write(&path, content).map_err(|e| format!("ファイル保存失敗: {}", e))?;
    Ok(json!({
        "path": path.to_string_lossy(),
        "bytes_written": content.len(),
        "status": "saved",
    }))
}

pub async fn delete_downloaded_file(args: &Value) -> Result<Value, String> {
    let path = resolve_downloaded_file_arg(args)?;
    if !path.is_file() {
        return Err("対象はファイルではありません".into());
    }
    let metadata = std::fs::metadata(&path).map_err(|e| format!("ファイル情報取得失敗: {}", e))?;
    std::fs::remove_file(&path).map_err(|e| format!("ファイル削除失敗: {}", e))?;
    Ok(json!({
        "status": "deleted",
        "path": path.to_string_lossy(),
        "filename": path.file_name().and_then(|n| n.to_str()).unwrap_or_default(),
        "size_bytes": metadata.len(),
    }))
}

pub async fn open_downloaded_file(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let mut path = resolve_downloaded_file_arg(args)?;
    if !path.exists() {
        if let Ok(downloaded_path) = auto_download_missing_file(app, &path).await {
            path = downloaded_path;
        } else {
            return Err(format!(
                "ファイルが見つかりません。自動ダウンロードも失敗しました。物理パス: {:?}",
                path
            ));
        }
    }
    crate::commands::open_downloaded_file(app.clone(), path.to_string_lossy().to_string()).await?;
    Ok(json!({
        "status": "opened",
        "path": path.to_string_lossy(),
        "filename": path.file_name().and_then(|n| n.to_str()).unwrap_or_default(),
    }))
}
