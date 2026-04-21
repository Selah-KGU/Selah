use super::*;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{}…<truncated>", truncated)
}

fn decode_xml_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn normalize_extracted_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_blank = false;
    for line in s.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !prev_blank && !out.is_empty() {
                out.push('\n');
            }
            prev_blank = true;
            continue;
        }
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(trimmed);
        prev_blank = false;
    }
    out.trim().to_string()
}

fn allowed_download_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    roots.push(crate::commands::default_download_dir());
    let cfg = crate::commands::load_download_config();
    if !cfg.download_dir.is_empty() {
        roots.push(PathBuf::from(cfg.download_dir));
    }
    let mut uniq = Vec::new();
    for root in roots {
        let canonical = root.canonicalize().unwrap_or(root);
        if !uniq.iter().any(|p: &PathBuf| p == &canonical) {
            uniq.push(canonical);
        }
    }
    uniq
}

fn resolve_allowed_download_path(raw_path: &str) -> Result<PathBuf, String> {
    let path = Path::new(raw_path);
    if !path.is_absolute() {
        return Err("絶対パスのファイルのみ指定できます".into());
    }
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("ファイルパスを解決できません: {}", e))?;
    let allowed = allowed_download_roots()
        .into_iter()
        .any(|root| canonical.starts_with(&root));
    if !allowed {
        return Err("ダウンロードフォルダ外のファイルは読めません".into());
    }
    Ok(canonical)
}

fn file_extension_lower(path: &Path) -> String {
    path.extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase()
}

fn supported_read_extension(ext: &str) -> bool {
    matches!(
        ext,
        "pdf" | "docx" | "txt" | "md" | "json" | "csv" | "html" | "htm"
    )
}

fn supported_write_extension(ext: &str) -> bool {
    matches!(ext, "txt" | "md" | "json" | "csv" | "html" | "htm")
}

fn read_utf8ish_file(path: &Path, max_bytes: usize) -> Result<String, String> {
    let metadata = std::fs::metadata(path).map_err(|e| format!("ファイル情報取得失敗: {}", e))?;
    if metadata.len() as usize > max_bytes {
        return Err(format!("ファイルが大きすぎます ({} bytes)", metadata.len()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("ファイル読み込み失敗: {}", e))?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

fn extract_pdf_text(path: &Path) -> Result<String, String> {
    let doc = lopdf::Document::load(path).map_err(|e| format!("PDF読み込み失敗: {}", e))?;
    let pages = doc.get_pages();
    if pages.is_empty() {
        return Err("PDFにページがありません".into());
    }
    let mut out = String::new();
    for page_num in pages.keys().take(20) {
        match doc.extract_text(&[*page_num]) {
            Ok(text) => {
                if !text.trim().is_empty() {
                    if !out.is_empty() {
                        out.push_str("\n\n");
                    }
                    out.push_str(&text);
                }
            }
            Err(e) => {
                log::warn!("pdf text extraction failed for page {}: {}", page_num, e);
            }
        }
    }
    let text = normalize_extracted_text(&out);
    if text.is_empty() {
        Err("PDFからテキストを抽出できませんでした".into())
    } else {
        Ok(text)
    }
}

fn extract_docx_text(path: &Path) -> Result<String, String> {
    let file = File::open(path).map_err(|e| format!("DOCX読み込み失敗: {}", e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("DOCX展開失敗: {}", e))?;
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .map_err(|e| format!("DOCX本文が見つかりません: {}", e))?
        .read_to_string(&mut xml)
        .map_err(|e| format!("DOCX本文読み込み失敗: {}", e))?;

    let para_re = regex::Regex::new(r"</w:p>").unwrap();
    let break_re = regex::Regex::new(r"<w:br\s*/?>").unwrap();
    let tab_re = regex::Regex::new(r"<w:tab\s*/?>").unwrap();
    let tag_re = regex::Regex::new(r"<[^>]+>").unwrap();

    let xml = para_re.replace_all(&xml, "\n");
    let xml = break_re.replace_all(&xml, "\n");
    let xml = tab_re.replace_all(&xml, "\t");
    let text = tag_re.replace_all(&xml, " ");
    let text = decode_xml_entities(&text);
    let text = normalize_extracted_text(&text);
    if text.is_empty() {
        Err("DOCXからテキストを抽出できませんでした".into())
    } else {
        Ok(text)
    }
}

fn read_supported_download_file(path: &Path) -> Result<String, String> {
    let ext = file_extension_lower(path);
    match ext.as_str() {
        "pdf" => extract_pdf_text(path),
        "docx" => extract_docx_text(path),
        "txt" | "md" | "json" | "csv" | "html" | "htm" => {
            read_utf8ish_file(path, 2_000_000).map(|s| normalize_extracted_text(&s))
        }
        "doc" => Err("旧式 .doc は未対応です。.docx か PDF に変換してから試してください".into()),
        _ => Err(format!("未対応の拡張子です: .{}", ext)),
    }
}

pub(super) async fn list_downloaded_files(args: &Value) -> Result<Value, String> {
    let keyword = sanitize_text_arg(args, "keyword", 80).unwrap_or_default();
    let keyword_norm = normalize_text(&keyword);
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .min(LIST_CAP as u64) as usize;

    let mut records = crate::commands::scan_download_dir();
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

pub(super) async fn read_downloaded_file(args: &Value) -> Result<Value, String> {
    let raw_path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if raw_path.is_empty() {
        return Err("pathを指定してください".into());
    }
    let path = resolve_allowed_download_path(raw_path)?;
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

pub(super) async fn write_downloaded_text_file(args: &Value) -> Result<Value, String> {
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

pub(super) async fn open_downloaded_file(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let raw_path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if raw_path.is_empty() {
        return Err("pathを指定してください".into());
    }
    let path = resolve_allowed_download_path(raw_path)?;
    crate::commands::open_downloaded_file(app.clone(), path.to_string_lossy().to_string())?;
    Ok(json!({
        "status": "opened",
        "path": path.to_string_lossy(),
        "filename": path.file_name().and_then(|n| n.to_str()).unwrap_or_default(),
    }))
}

struct LunaAttachmentResolved {
    title: String,
    course_name: String,
    detail_path: String,
    detail_url: String,
    attachment: crate::luna_parser::LunaAttachment,
}

async fn resolve_luna_attachment(
    app: &tauri::AppHandle,
    title: &str,
    attachment_name: &str,
) -> Result<LunaAttachmentResolved, String> {
    let db = app.state::<Database>();
    let acts = db.get_all_luna_activities().unwrap_or_default();
    let needle = title.to_lowercase();
    let row = acts
        .iter()
        .find(|a| a.title == title)
        .or_else(|| {
            acts.iter()
                .find(|a| a.title.to_lowercase().contains(&needle))
        })
        .or_else(|| {
            acts.iter()
                .find(|a| needle.contains(&a.title.to_lowercase()) && !a.title.is_empty())
        })
        .ok_or_else(|| format!("「{}」に一致する活動が見つかりません", title))?;
    if row.detail_path.is_empty() {
        return Err(format!("「{}」には詳細ページのパスがありません", row.title));
    }

    let luna_courses = db.get_luna_courses().unwrap_or_default();
    let course_name = luna_courses
        .iter()
        .find(|c| c.luna_id == row.luna_id)
        .map(|c| c.name.clone())
        .unwrap_or_default();

    let luna_state = app.state::<crate::LunaState>();
    let http = {
        let luna = luna_state.client.lock().await;
        if !luna.authenticated {
            return Err(crate::luna_client::LUNA_AUTH_REQUIRED_MSG.into());
        }
        luna.http.clone()
    };

    let detail_url = format!("{}{}", crate::config::LUNA_BASE, row.detail_path);
    let html = crate::client::fetch_with_redirect(
        &http,
        &detail_url,
        crate::config::LUNA_BASE,
        crate::luna_client::LUNA_SESSION_EXPIRED_MSG,
        crate::luna_client::is_luna_session_expired,
    )
    .await
    .map_err(|e| format!("Luna取得失敗: {}", e))?;

    let detail = if row.activity_type == "announcement" {
        crate::luna_parser::parse_luna_announcement_detail(&html)
    } else {
        crate::luna_parser::parse_luna_detail_page(&html)
    };

    let attachment = if attachment_name.is_empty() {
        detail.attachments.first()
    } else {
        let needle = attachment_name.to_lowercase();
        detail
            .attachments
            .iter()
            .find(|a| a.name == attachment_name)
            .or_else(|| {
                detail
                    .attachments
                    .iter()
                    .find(|a| a.name.to_lowercase().contains(&needle))
            })
            .or_else(|| {
                detail
                    .attachments
                    .iter()
                    .find(|a| needle.contains(&a.name.to_lowercase()))
            })
    }
    .cloned()
    .ok_or_else(|| {
        if attachment_name.is_empty() {
            format!("「{}」には開ける添付がありません", row.title)
        } else {
            format!(
                "「{}」の添付「{}」が見つかりません",
                row.title, attachment_name
            )
        }
    })?;

    Ok(LunaAttachmentResolved {
        title: row.title.clone(),
        course_name,
        detail_path: row.detail_path.clone(),
        detail_url,
        attachment,
    })
}

pub(super) async fn open_luna_attachment(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let attachment_name = args
        .get("attachment_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if title.is_empty() {
        return Err("titleを指定してください".into());
    }

    let resolved = resolve_luna_attachment(app, title, &attachment_name).await?;
    let attachment = &resolved.attachment;

    if attachment.url.starts_with("http") {
        crate::commands::open_external_url(
            app.clone(),
            attachment.url.clone(),
            Some(attachment.name.clone()),
        )
        .await?;
        return Ok(json!({
            "status": "opened_external",
            "title": resolved.title,
            "attachment_name": attachment.name,
            "url": attachment.url,
            "course": resolved.course_name,
            "source": { "service": "luna", "detail_path": resolved.detail_path, "detail_url": resolved.detail_url },
        }));
    }

    let luna_state = app.state::<crate::LunaState>();
    let http = {
        let luna = luna_state.client.lock().await;
        if !luna.authenticated {
            return Err(crate::luna_client::LUNA_AUTH_REQUIRED_MSG.into());
        }
        luna.http.clone()
    };

    let bytes = if attachment.url.is_empty() {
        let action = attachment.download_action.as_str();
        if action.is_empty() {
            return Err("添付のダウンロード情報が不足しています".into());
        }
        let mut params: Vec<String> = Vec::new();
        for (k, v) in &attachment.download_params {
            params.push(format!(
                "{}={}",
                crate::luna_commands::form_encode(k),
                crate::luna_commands::form_encode(v)
            ));
        }
        let path_name = crate::luna_commands::make_down_file_name(&attachment.name);
        let download_url = format!("{}/{}?{}", action, path_name, params.join("&"));
        crate::luna_commands::luna_download(&http, &download_url).await?
    } else {
        crate::luna_commands::luna_download(&http, &attachment.url).await?
    };

    if bytes.is_empty() {
        return Err("添付データが空です".into());
    }
    let saved_path = crate::luna_commands::save_to_downloads(
        &attachment.name,
        &bytes,
        Some(&resolved.course_name),
    )?;
    crate::commands::open_downloaded_file(app.clone(), saved_path.clone())?;

    Ok(json!({
        "status": "downloaded_and_opened",
        "title": resolved.title,
        "attachment_name": attachment.name,
        "saved_path": saved_path,
        "course": resolved.course_name,
        "source": { "service": "luna", "detail_path": resolved.detail_path, "detail_url": resolved.detail_url },
    }))
}

pub(super) async fn download_luna_attachment(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let attachment_name = args
        .get("attachment_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if title.is_empty() {
        return Err("titleを指定してください".into());
    }

    let resolved = resolve_luna_attachment(app, title, &attachment_name).await?;
    let attachment = &resolved.attachment;

    if attachment.url.starts_with("http") {
        return Ok(json!({
            "status": "external_url",
            "title": resolved.title,
            "attachment_name": attachment.name,
            "url": attachment.url,
            "course": resolved.course_name,
            "source": { "service": "luna", "detail_path": resolved.detail_path, "detail_url": resolved.detail_url },
        }));
    }

    let luna_state = app.state::<crate::LunaState>();
    let http = {
        let luna = luna_state.client.lock().await;
        if !luna.authenticated {
            return Err(crate::luna_client::LUNA_AUTH_REQUIRED_MSG.into());
        }
        luna.http.clone()
    };

    let bytes = if attachment.url.is_empty() {
        let action = attachment.download_action.as_str();
        if action.is_empty() {
            return Err("添付のダウンロード情報が不足しています".into());
        }
        let mut params: Vec<String> = Vec::new();
        for (k, v) in &attachment.download_params {
            params.push(format!(
                "{}={}",
                crate::luna_commands::form_encode(k),
                crate::luna_commands::form_encode(v)
            ));
        }
        let path_name = crate::luna_commands::make_down_file_name(&attachment.name);
        let download_url = format!("{}/{}?{}", action, path_name, params.join("&"));
        crate::luna_commands::luna_download(&http, &download_url).await?
    } else {
        crate::luna_commands::luna_download(&http, &attachment.url).await?
    };

    if bytes.is_empty() {
        return Err("添付データが空です".into());
    }
    let saved_path = crate::luna_commands::save_to_downloads(
        &attachment.name,
        &bytes,
        Some(&resolved.course_name),
    )?;
    Ok(json!({
        "status": "downloaded",
        "title": resolved.title,
        "attachment_name": attachment.name,
        "saved_path": saved_path,
        "course": resolved.course_name,
        "source": { "service": "luna", "detail_path": resolved.detail_path, "detail_url": resolved.detail_url },
    }))
}

pub(super) async fn list_browser_windows(app: &tauri::AppHandle) -> Result<Value, String> {
    let items = crate::webview_toolbar::list_browser_windows(app);
    Ok(json!({
        "windows": items.into_iter().map(|w| json!({
            "label": w.label,
            "target": w.target,
            "url": w.url,
        })).collect::<Vec<_>>()
    }))
}

pub(super) async fn open_browser_url(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if url.is_empty() {
        return Err("urlを指定してください".into());
    }
    crate::commands::open_external_url(app.clone(), url.clone(), None).await?;
    Ok(json!({
        "status": "opened",
        "url": url,
    }))
}

fn resolve_browser_target_from_args(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<String, String> {
    crate::webview_toolbar::resolve_browser_target(app, args.get("target").and_then(|v| v.as_str()))
}

pub(super) async fn read_browser_page(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let target = resolve_browser_target_from_args(app, args)?;
    let payload = crate::webview_toolbar::extract_page_text(app, &target).await?;
    Ok(json!({
        "target": target,
        "title": payload.title,
        "url": payload.url,
        "content": truncate_chars(&payload.text, 12_000),
    }))
}

pub(super) async fn browser_back(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let target = resolve_browser_target_from_args(app, args)?;
    crate::webview_toolbar::browser_go_back(app.clone(), target.clone()).await?;
    let url = crate::webview_toolbar::browser_get_url(app.clone(), target.clone())
        .await
        .unwrap_or_default();
    Ok(json!({ "target": target, "status": "ok", "url": url }))
}

pub(super) async fn browser_forward(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    let target = resolve_browser_target_from_args(app, args)?;
    crate::webview_toolbar::browser_go_forward(app.clone(), target.clone()).await?;
    let url = crate::webview_toolbar::browser_get_url(app.clone(), target.clone())
        .await
        .unwrap_or_default();
    Ok(json!({ "target": target, "status": "ok", "url": url }))
}

pub(super) async fn browser_reload_page(
    app: &tauri::AppHandle,
    args: &Value,
) -> Result<Value, String> {
    let target = resolve_browser_target_from_args(app, args)?;
    crate::webview_toolbar::browser_reload(app.clone(), target.clone()).await?;
    let url = crate::webview_toolbar::browser_get_url(app.clone(), target.clone())
        .await
        .unwrap_or_default();
    Ok(json!({ "target": target, "status": "ok", "url": url }))
}
