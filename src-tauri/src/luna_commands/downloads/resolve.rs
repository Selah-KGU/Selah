use tauri::State;

use crate::{config, LunaState};

use super::super::{luna_http, SEL_A_HREF, SEL_BODY, SEL_IFRAME_SRC, SEL_META_REFRESH, SEL_SCRIPT};
use super::prepare::{build_material_download_query, prepare_material_tempfile};
use super::save::make_down_file_name;

/// Resolve an HTML-type material to its actual external URL.
/// Same tempfile+sethtmlfiledown flow as download, but parses the HTML for the link.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn luna_resolve_material_link(
    state: State<'_, LunaState>,
    idnumber: String,
    file_name: String,
    object_name: String,
    resource_id: String,
    file_type: String,
    material_id: Option<String>,
    display_name: Option<String>,
    end_date: Option<String>,
) -> Result<String, String> {
    let http = luna_http(&state).await?;

    log::info!(
        "Material link resolve: file='{}', resource='{}', type='{}'",
        file_name,
        resource_id,
        file_type
    );

    let file_id = prepare_material_tempfile(
        &http,
        &idnumber,
        &file_name,
        &object_name,
        &resource_id,
        "Failed to prepare tempfile",
    )
    .await?;

    let path_encoded_name = make_down_file_name(&file_name);
    let base_path = format!(
        "/lms/course/materialref/sethtmlfiledown/{}",
        path_encoded_name
    );
    let dl_title = display_name.unwrap_or_default();
    let content_id = material_id.unwrap_or_default();
    let end_date_val = end_date.unwrap_or_default();
    let query_string = build_material_download_query(
        &file_name,
        &file_id,
        &idnumber,
        &resource_id,
        &content_id,
        &end_date_val,
        &dl_title,
    );
    let full_url = format!("{}{}?{}", config::LUNA_BASE, base_path, query_string);

    let resp = http
        .get(&full_url)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    let final_url = resp.url().to_string();
    let html = resp.text().await.unwrap_or_default();

    log::info!(
        "Material link HTML (len={}, final_url={}): {}",
        html.len(),
        final_url,
        crate::client::safe_truncate(&html, 1000)
    );

    if !final_url.contains("luna.kwansei.ac.jp") {
        return Ok(final_url);
    }

    let doc = scraper::Html::parse_document(&html);

    if let Some(meta) = doc.select(&SEL_META_REFRESH).next() {
        if let Some(content) = meta.value().attr("content") {
            if let Some(idx) = content.to_lowercase().find("url=") {
                let url = content[idx + 4..]
                    .trim()
                    .trim_matches(|c| c == '\'' || c == '"');
                if !url.is_empty() {
                    return Ok(url.to_string());
                }
            }
        }
    }

    if let Some(iframe) = doc.select(&SEL_IFRAME_SRC).next() {
        if let Some(src) = iframe.value().attr("src") {
            if src.starts_with("http") {
                return Ok(src.to_string());
            }
        }
    }

    for script in doc.select(&SEL_SCRIPT) {
        let text = script.text().collect::<String>();
        for pattern in &[
            "window.location.href",
            "window.location",
            "location.href",
            "window.open(",
        ] {
            if let Some(idx) = text.find(pattern) {
                let after = &text[idx + pattern.len()..];
                let start = after.find(['\'', '"']);
                if let Some(s) = start {
                    let quote = after.as_bytes()[s] as char;
                    if let Some(e) = after[s + 1..].find(quote) {
                        let url = &after[s + 1..s + 1 + e];
                        if url.starts_with("http") {
                            return Ok(url.to_string());
                        }
                    }
                }
            }
        }
    }

    for a in doc.select(&SEL_A_HREF) {
        if let Some(href) = a.value().attr("href") {
            if href.starts_with("http") && !href.contains("luna.kwansei.ac.jp") {
                return Ok(href.to_string());
            }
        }
    }

    let body_text = doc
        .select(&SEL_BODY)
        .next()
        .map(|b| b.text().collect::<String>().trim().to_string())
        .unwrap_or_default();
    if body_text.starts_with("http") && !body_text.contains(' ') {
        return Ok(body_text);
    }

    Err("リンク先のURLを抽出できませんでした".into())
}
