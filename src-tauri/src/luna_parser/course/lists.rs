use super::super::{
    classify_link, extract_quill_rich_html, ContentsPageResult, Html, Selector, SEL_A_HREF,
    SEL_DISC_LIST, SEL_DISC_NAME, SEL_DISC_NAME_FB, SEL_DISC_PERIOD, SEL_DISC_STATUS,
    SEL_DL_MAT_ID, SEL_EXAM_LIST, SEL_EXAM_NAME, SEL_EXAM_NAME_FB, SEL_EXAM_PERIOD,
    SEL_EXAM_STATUS, SEL_FILENAME, SEL_FILETYPE, SEL_INPUT_SPAN, SEL_LINK_TXT, SEL_MATERIAL_LIST,
    SEL_MAT_CSS, SEL_MAT_FILE_NAME, SEL_MAT_TITLE, SEL_OBJECT_NAME, SEL_OPEN_END_DATE,
    SEL_QL_EDITOR, SEL_REPORT_LIST, SEL_RESOURCE_ID, SEL_RPT_END, SEL_RPT_NAME, SEL_RPT_START,
    SEL_RPT_STATUS, SEL_SCAN_STATUS, SEL_SCRIPT, SEL_SURVEY_LIST, SEL_SURV_NAME, SEL_SURV_NAME_FB,
    SEL_SURV_PERIOD, SEL_SURV_STATUS,
};
use super::model::*;

/// Parse the contents top page (/lms/contents?idnumber=XXX)
/// Extracts materials, reports, examinations, discussions
pub fn parse_luna_contents_page(html: &str) -> ContentsPageResult {
    let doc = Html::parse_document(html);
    let materials = parse_materials(&doc);
    let reports = parse_reports(&doc);
    let examinations = parse_content_list(
        &doc,
        &SEL_EXAM_LIST,
        &SEL_EXAM_NAME,
        &SEL_EXAM_NAME_FB,
        &SEL_EXAM_PERIOD,
        &SEL_EXAM_STATUS,
        "examination",
    );
    let discussions = parse_content_list(
        &doc,
        &SEL_DISC_LIST,
        &SEL_DISC_NAME,
        &SEL_DISC_NAME_FB,
        &SEL_DISC_PERIOD,
        &SEL_DISC_STATUS,
        "discussion",
    );
    let surveys = parse_content_list(
        &doc,
        &SEL_SURVEY_LIST,
        &SEL_SURV_NAME,
        &SEL_SURV_NAME_FB,
        &SEL_SURV_PERIOD,
        &SEL_SURV_STATUS,
        "survey",
    );
    (materials, reports, examinations, discussions, surveys)
}

/// Extract plain text from a Quill Delta JSON embedded in a JS script.
/// The script contains: `_QuillUtil.xxx.setJsonData("{...}", ...)`
#[cfg(test)]
pub(in crate::luna_parser) fn extract_quill_delta_text(script: &str) -> Option<String> {
    let marker = ".setJsonData(\"";
    let start = script.find(marker)? + marker.len();
    let rest = &script[start..];
    // Walk to find the unescaped closing quote
    let mut i = 0;
    let bytes = rest.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2; // skip escape sequence
        } else if bytes[i] == b'"' {
            break;
        } else {
            i += 1;
        }
    }
    if i >= bytes.len() {
        return None;
    }
    let escaped = &rest[..i];
    // Treat as a JSON string body to decode \uXXXX, \", \\n etc.
    let json_lit = format!("\"{}\"", escaped);
    let inner_json: String = serde_json::from_str(&json_lit).ok()?;
    let val: serde_json::Value = serde_json::from_str(&inner_json).ok()?;
    let ops = val.get("ops")?.as_array()?;
    let mut text = String::new();
    for op in ops {
        if let Some(s) = op.get("insert").and_then(|v| v.as_str()) {
            text.push_str(s);
        }
    }
    let trimmed = text.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

pub(in crate::luna_parser) fn extract_quill_delta_html(script: &str) -> Option<String> {
    let marker = ".setJsonData(\"";
    let start = script.find(marker)? + marker.len();
    let rest = &script[start..];
    let mut i = 0;
    let bytes = rest.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
        } else if bytes[i] == b'"' {
            break;
        } else {
            i += 1;
        }
    }
    if i >= bytes.len() {
        return None;
    }
    let escaped = &rest[..i];
    extract_quill_rich_html(escaped)
}

fn parse_materials(doc: &Html) -> Vec<LunaContentItem> {
    let mut items = Vec::new();

    // Each materialList div is a folder with materials
    for folder in doc.select(&SEL_MATERIAL_LIST) {
        let title = folder
            .select(&SEL_MAT_TITLE)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        if title.is_empty() {
            continue;
        }

        let period = folder
            .select(&SEL_INPUT_SPAN)
            .map(|e| e.text().collect::<String>().trim().to_string())
            .find(|s| s.contains('～'))
            .unwrap_or_default();

        // Prefer rendered Quill HTML to preserve rich text formatting.
        // Fallback: parse Quill Delta JSON from <script> tags as rich HTML.
        let description = {
            let mut text = String::new();
            if let Some(editor) = folder.select(&SEL_QL_EDITOR).next() {
                text = editor.inner_html().trim().to_string();
            }
            if text.is_empty() {
                for el in folder.select(&SEL_SCRIPT) {
                    let src = el.inner_html();
                    if let Some(t) = extract_quill_delta_html(&src) {
                        text = t;
                        break;
                    }
                }
            }
            text
        };

        // Parse individual material files with download metadata
        let mut files = Vec::new();
        for row in folder.select(&SEL_MAT_CSS) {
            let display_name = row
                .select(&SEL_MAT_FILE_NAME)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            if display_name.is_empty() {
                continue;
            }

            let file_name = row
                .select(&SEL_FILENAME)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let object_name = row
                .select(&SEL_OBJECT_NAME)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let resource_id = row
                .select(&SEL_RESOURCE_ID)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let file_type = row
                .select(&SEL_FILETYPE)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let material_id = row
                .select(&SEL_DL_MAT_ID)
                .next()
                .and_then(|e| e.value().attr("value"))
                .unwrap_or_default()
                .to_string();
            let end_date = row
                .select(&SEL_OPEN_END_DATE)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let scan_status = row
                .select(&SEL_SCAN_STATUS)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            let mut link_type = if file_type == "0" {
                classify_link(&file_name, &display_name)
            } else {
                let cl = classify_link(&display_name, &file_name);
                if cl == "file" {
                    "web".to_string()
                } else {
                    cl
                }
            };

            // For pure external-link materials (file_type != "0" and no backing file),
            // Luna embeds the URL directly somewhere in the row — scan for it so the
            // frontend can open it without the tempfile flow.
            let mut external_url = String::new();
            if file_type != "0" && file_name.is_empty() {
                for a in row.select(&SEL_A_HREF) {
                    if let Some(href) = a.value().attr("href") {
                        let h = href.trim();
                        if h.starts_with("http") && !h.contains("luna.kwansei.ac.jp") {
                            external_url = h.to_string();
                            break;
                        }
                    }
                }
                if external_url.is_empty() {
                    let row_html = row.html();
                    if let Some(idx) = row_html.find("http") {
                        let tail = &row_html[idx..];
                        let end = tail.find(['"', '\'', '<', ' ']).unwrap_or(tail.len());
                        let candidate = &tail[..end];
                        if candidate.len() > 10 && !candidate.contains("luna.kwansei.ac.jp") {
                            external_url = candidate.to_string();
                        }
                    }
                }
                if !external_url.is_empty() {
                    let cl = classify_link(&external_url, &display_name);
                    if cl != "file" {
                        link_type = cl;
                    } else if link_type == "file" {
                        link_type = "web".to_string();
                    }
                    log::info!(
                        "[material] extracted external URL: resource_id='{}', url='{}'",
                        resource_id,
                        crate::client::safe_truncate(&external_url, 200)
                    );
                } else {
                    let row_html = row.html();
                    log::warn!(
                        "[material] link-type row has no fileName and no extractable URL: resource_id='{}', display='{}'. Raw HTML:\n{}",
                        resource_id,
                        display_name,
                        crate::client::safe_truncate(&row_html, 3000)
                    );
                }
            }

            files.push(LunaMaterialFile {
                display_name,
                file_name,
                object_name,
                resource_id,
                material_id,
                file_type,
                end_date,
                scan_status,
                link_type,
                external_url,
            });
        }

        // Fallback: some materials expose their file purely as a Luna resource
        // link (<a href="/lms/course/display/material/resource?objectName=…
        // &fileName=…">…</a>) with no structured download row, which would leave
        // an empty card. Capture those as web-link files so the frontend shows a
        // button and opens them in the browser (host is completed there).
        if files.is_empty() {
            for a in folder.select(&SEL_A_HREF) {
                let href = a.value().attr("href").unwrap_or("").trim().to_string();
                if !href.contains("/lms/course/display/material/") {
                    continue;
                }
                let display = a.text().collect::<String>().trim().to_string();
                if display.is_empty() {
                    continue;
                }
                files.push(LunaMaterialFile {
                    display_name: display,
                    file_name: String::new(),
                    object_name: String::new(),
                    resource_id: String::new(),
                    material_id: String::new(),
                    file_type: "1".to_string(),
                    end_date: String::new(),
                    scan_status: String::new(),
                    link_type: "web".to_string(),
                    external_url: href,
                });
            }
        }

        let status = if files.is_empty() {
            String::new()
        } else {
            files
                .iter()
                .map(|f| f.display_name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };

        items.push(LunaContentItem {
            title,
            url: String::new(),
            period,
            status,
            description,
            item_type: "material".to_string(),
            files,
        });
    }
    items
}

fn parse_reports(doc: &Html) -> Vec<LunaContentItem> {
    let mut items = Vec::new();

    for row in doc.select(&SEL_REPORT_LIST) {
        let a = match row.select(&SEL_RPT_NAME).next() {
            Some(a) => a,
            None => continue,
        };
        let title = a.text().collect::<String>().trim().to_string();
        let url = a.value().attr("href").unwrap_or_default().to_string();

        let start = row
            .select(&SEL_RPT_START)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        let end = row
            .select(&SEL_RPT_END)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        let period = if !start.is_empty() && !end.is_empty() {
            format!("{} ～ {}", start, end)
        } else {
            String::new()
        };

        let status = row
            .select(&SEL_RPT_STATUS)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        items.push(LunaContentItem {
            title,
            url,
            period,
            status,
            description: String::new(),
            item_type: "report".to_string(),
            files: Vec::new(),
        });
    }
    items
}

fn parse_content_list(
    doc: &Html,
    list_sel: &Selector,
    name_sel: &Selector,
    name_fb_sel: &Selector,
    period_sel: &Selector,
    status_sel: &Selector,
    item_type: &str,
) -> Vec<LunaContentItem> {
    let mut items = Vec::new();

    for row in doc.select(list_sel) {
        let (title, mut url) = if let Some(a) = row.select(name_sel).next() {
            let t = a.text().collect::<String>().trim().to_string();
            let u = a.value().attr("href").unwrap_or_default().to_string();
            (t, u)
        } else if let Some(a) = row.select(&SEL_LINK_TXT).next() {
            let t = a.text().collect::<String>().trim().to_string();
            let u = a.value().attr("href").unwrap_or_default().to_string();
            (t, u)
        } else if let Some(el) = row.select(name_fb_sel).next() {
            let t = el.text().collect::<String>().trim().to_string();
            (t, String::new())
        } else {
            continue;
        };

        if title.is_empty() {
            continue;
        }

        if url.is_empty() || url == "#" || url == "javascript:void(0)" {
            url = extract_url_from_row(&row);
        }

        let period = row
            .select(period_sel)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let status = row
            .select(status_sel)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        items.push(LunaContentItem {
            title,
            url,
            period,
            status,
            description: String::new(),
            item_type: item_type.to_string(),
            files: Vec::new(),
        });
    }
    items
}

/// Extract a URL from onclick attributes or <a> tags within a row element
fn extract_url_from_row(row: &scraper::ElementRef) -> String {
    // Check all <a> tags for href
    for a in row.select(&SEL_A_HREF) {
        let href = a.value().attr("href").unwrap_or_default();
        if !href.is_empty() && href != "#" && !href.starts_with("javascript:") {
            return href.to_string();
        }
    }
    // Check onclick attributes for URL patterns
    let row_html = row.html();
    // Pattern: location.href='...' or window.open('...')
    for pattern in &[
        "location.href='",
        "location.href=\"",
        "window.open('",
        "window.open(\"",
    ] {
        if let Some(start) = row_html.find(pattern) {
            let after = &row_html[start + pattern.len()..];
            let quote = if pattern.ends_with('\'') { '\'' } else { '"' };
            if let Some(end) = after.find(quote) {
                let url = &after[..end];
                if url.starts_with('/') {
                    return url.to_string();
                }
            }
        }
    }
    String::new()
}
