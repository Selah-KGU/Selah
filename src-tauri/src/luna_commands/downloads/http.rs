use crate::{config, luna_client};

/// Luna download: download a file without holding the lock. Returns bytes.
pub(crate) async fn luna_download(http: &reqwest::Client, path: &str) -> Result<Vec<u8>, String> {
    let url = if path.starts_with("http") {
        path.to_string()
    } else {
        format!("{}{}", config::LUNA_BASE, path)
    };

    let mut current_url = url;
    for i in 0..10 {
        let resp = http
            .get(&current_url)
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header("Sec-Fetch-Dest", "document")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-Site", "same-origin")
            .send()
            .await
            .map_err(|e| format!("ダウンロード失敗: {}", e))?;

        let status = resp.status();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string();
        let content_len = resp
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string();
        let content_disp = resp
            .headers()
            .get("content-disposition")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        log::info!(
            "luna_download #{}: status={}, type={}, len={}, disp='{}'",
            i,
            status,
            content_type,
            content_len,
            content_disp
        );

        if status.is_redirection() {
            if let Some(loc) = resp.headers().get("location") {
                let loc_str = loc.to_str().unwrap_or_default();
                current_url = if loc_str.starts_with('/') {
                    format!("{}{}", config::LUNA_BASE, loc_str)
                } else {
                    loc_str.to_string()
                };
                if current_url.contains("sso.kwansei.ac.jp") {
                    return Err(luna_client::LUNA_SESSION_EXPIRED_MSG.into());
                }
                log::info!("luna_download: redirect -> {}", current_url);
                continue;
            }
        }

        if !status.is_success() {
            return Err(format!("HTTP {}", status));
        }

        if content_type.contains("text/html") {
            let text = resp
                .text()
                .await
                .map_err(|e| format!("読み取り失敗: {}", e))?;
            if luna_client::is_luna_session_expired(&text) {
                return Err(luna_client::LUNA_SESSION_EXPIRED_MSG.into());
            }
            return Ok(text.into_bytes());
        }

        return resp
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| format!("ダウンロード読み取り失敗: {}", e));
    }
    Err("リダイレクトが多すぎます".into())
}
