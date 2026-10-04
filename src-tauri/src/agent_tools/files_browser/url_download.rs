//! Download an arbitrary URL into the course download directory.

use super::*;

pub async fn download_url(args: &Value) -> Result<Value, String> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if url.is_empty() {
        return Err("urlを指定してください".into());
    }
    let parsed = url::Url::parse(&url).map_err(|e| format!("URL解析失敗: {}", e))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("http/https のみダウンロードできます".into());
    }
    let custom_name = sanitize_text_arg(args, "filename", 200);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("HTTPクライアント生成失敗: {}", e))?;
    let resp = client
        .get(parsed.clone())
        .send()
        .await
        .map_err(|e| format!("ダウンロード失敗: {}", e))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {}", status));
    }

    let content_disposition = resp
        .headers()
        .get(reqwest::header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("レスポンス読み取り失敗: {}", e))?;
    if bytes.len() > 50 * 1024 * 1024 {
        return Err(format!(
            "ファイルが大きすぎます ({} bytes、上限50MB)",
            bytes.len()
        ));
    }

    let filename = custom_name
        .or_else(|| content_disposition.as_deref().and_then(filename_from_cd))
        .unwrap_or_else(|| filename_from_url(&parsed));
    let safe_name = sanitize_filename_basic(&filename);

    let saved = crate::luna_commands::save_to_downloads(&safe_name, &bytes, None)?;
    Ok(json!({
        "status": "downloaded",
        "url": url,
        "saved_path": saved,
        "filename": safe_name,
        "size_bytes": bytes.len(),
    }))
}

fn filename_from_cd(header: &str) -> Option<String> {
    for part in header.split(';') {
        let trimmed = part.trim();
        if let Some(value) = trimmed
            .strip_prefix("filename*=UTF-8''")
            .or_else(|| trimmed.strip_prefix("filename*=utf-8''"))
        {
            if let Ok(decoded) = urlencoding::decode(value) {
                return Some(decoded.into_owned());
            }
        }
        if let Some(value) = trimmed.strip_prefix("filename=") {
            return Some(value.trim_matches('"').to_string());
        }
    }
    None
}

fn filename_from_url(parsed: &url::Url) -> String {
    parsed
        .path_segments()
        .and_then(|mut segs| segs.rfind(|s| !s.is_empty()))
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("download-{}.bin", chrono::Utc::now().format("%Y%m%d%H%M%S")))
}

fn sanitize_filename_basic(name: &str) -> String {
    let trimmed: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = trimmed.trim().trim_matches('.').to_string();
    if trimmed.is_empty() {
        format!("download-{}.bin", chrono::Utc::now().format("%Y%m%d%H%M%S"))
    } else if trimmed.len() > 200 {
        let mut cut = 200;
        while cut > 0 && !trimmed.is_char_boundary(cut) {
            cut -= 1;
        }
        trimmed[..cut].to_string()
    } else {
        trimmed
    }
}
