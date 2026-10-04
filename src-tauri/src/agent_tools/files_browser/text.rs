//! Shared text, path, and download-sandbox helpers.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{}…<truncated>", truncated)
}

pub fn decode_xml_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

pub fn normalize_extracted_text(s: &str) -> String {
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

pub fn compact_text(s: &str, max_chars: usize) -> Option<String> {
    let normalized = normalize_extracted_text(s);
    if normalized.is_empty() {
        None
    } else {
        Some(truncate_chars(&normalized, max_chars))
    }
}

pub fn compact_string_list(items: &[String], max_items: usize, max_chars: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in items {
        let Some(value) = compact_text(item, max_chars) else {
            continue;
        };
        let key = value.to_lowercase();
        if !seen.insert(key) {
            continue;
        }
        out.push(value);
        if out.len() >= max_items {
            break;
        }
    }
    out
}

pub fn allowed_download_roots() -> Vec<PathBuf> {
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

pub fn resolve_allowed_download_path(raw_path: &str) -> Result<PathBuf, String> {
    let path = Path::new(raw_path);
    if !path.is_absolute() {
        return Err("絶対パスのファイルのみ指定できます".into());
    }
    let canonical = if path.exists() {
        path.canonicalize()
            .map_err(|e| format!("ファイルパスを解決できません: {}", e))?
    } else if let Some(parent) = path.parent() {
        if parent.exists() {
            let parent_canonical = parent
                .canonicalize()
                .map_err(|e| format!("親ディレクトリを解決できません: {}", e))?;
            if let Some(file_name) = path.file_name() {
                parent_canonical.join(file_name)
            } else {
                return Err("ファイル名が不正です".into());
            }
        } else {
            path.to_path_buf()
        }
    } else {
        path.to_path_buf()
    };
    let allowed = allowed_download_roots()
        .into_iter()
        .any(|root| canonical.starts_with(&root));
    if !allowed {
        return Err("ダウンロードフォルダ外のファイルは読めません".into());
    }
    Ok(canonical)
}

pub fn file_extension_lower(path: &Path) -> String {
    path.extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase()
}

pub fn supported_read_extension(ext: &str) -> bool {
    matches!(
        ext,
        "pdf" | "docx" | "pptx" | "xlsx" | "txt" | "md" | "json" | "csv" | "html" | "htm"
    )
}

pub fn supported_write_extension(ext: &str) -> bool {
    matches!(ext, "txt" | "md" | "json" | "csv" | "html" | "htm")
}

pub fn read_utf8ish_file(path: &Path, max_bytes: usize) -> Result<String, String> {
    let metadata = std::fs::metadata(path).map_err(|e| format!("ファイル情報取得失敗: {}", e))?;
    if metadata.len() as usize > max_bytes {
        return Err(format!("ファイルが大きすぎます ({} bytes)", metadata.len()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("ファイル読み込み失敗: {}", e))?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}
