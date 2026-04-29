use super::*;

#[path = "detail/links.rs"]
mod links;
#[path = "detail/model.rs"]
mod model;
#[path = "detail/notice_filter.rs"]
mod notice_filter;
#[path = "detail/quill.rs"]
mod quill;

pub(super) use links::classify_link;
pub use model::{LunaAttachment, LunaDetailPage, LunaDetailSection};
pub(crate) use notice_filter::is_blacklisted_system_notice_text;
use notice_filter::sanitize_blacklisted_notice_body;
pub(super) use quill::{extract_named_quill_text, extract_quill_rich_html};

fn normalize_detail_text(s: &str) -> String {
    s.split_whitespace().collect::<String>()
}

fn extract_filtered_input_text(area: scraper::ElementRef) -> String {
    let mut text_parts = Vec::new();
    for child in area.text() {
        let trimmed = child.trim();
        if !trimmed.is_empty()
            && !trimmed.starts_with("/*")
            && !trimmed.starts_with("$(")
            && !trimmed.starts_with("_Quill")
            && !trimmed.starts_with("var ")
            && !trimmed.starts_with("*/")
            && !trimmed.contains("setJsonData")
            && !trimmed.contains("function")
            && !trimmed.contains("QuillUtil")
        {
            text_parts.push(trimmed.to_string());
        }
    }
    text_parts.join(" ")
}

fn is_body_label(label: &str) -> bool {
    let label = label.trim();
    matches!(
        label,
        "内容"
            | "本文"
            | "説明"
            | "詳細"
            | "課題内容"
            | "試験内容"
            | "課題説明"
            | "提出内容"
            | "説明文"
            | "問題文"
            | "設問"
    ) || label.contains("内容")
        || label.contains("説明")
        || label.contains("本文")
}

fn push_unique_section(
    sections: &mut Vec<LunaDetailSection>,
    heading: impl Into<String>,
    body: impl Into<String>,
) {
    let Some(body) = sanitize_blacklisted_notice_body(&body.into()) else {
        log::debug!("[luna_detail] dropped blacklisted system notice candidate");
        return;
    };
    let body = body.trim().to_string();
    if body.is_empty() {
        return;
    }
    let normalized = normalize_detail_text(&body);
    if normalized.is_empty()
        || sections
            .iter()
            .any(|s| normalize_detail_text(&s.body) == normalized)
    {
        return;
    }
    sections.push(LunaDetailSection {
        heading: heading.into(),
        body,
    });
}

fn fallback_single_quill_section(html: &str) -> Option<String> {
    let mut unique = Vec::new();
    for text in extract_all_quill_texts(html) {
        let trimmed = text.trim();
        if trimmed.len() <= 5 {
            continue;
        }
        let normalized = normalize_detail_text(trimmed);
        if normalized.is_empty()
            || unique
                .iter()
                .any(|existing: &String| normalize_detail_text(existing) == normalized)
        {
            continue;
        }
        unique.push(trimmed.to_string());
    }
    if unique.len() == 1 {
        unique.into_iter().next()
    } else {
        None
    }
}

/// Parse any Luna detail page (report/submission, examination, forum, etc.)
///
/// Luna detail pages use a consistent pattern:
///   .course-title-txt          → course name
///   .contents-title-txt        → page title (e.g. "テスト 受験トップ")
///   .contents-detail.contents-vertical → each field row:
///     .contents-header-txt .bold-txt → label
///     .contents-input-area           → value
///   .downloadFile              → attachment file names
///   setJsonData("...", ...)    → Quill rich text content (in <script> tags)
pub fn parse_luna_detail_page(html: &str) -> LunaDetailPage {
    let doc = Html::parse_document(html);

    // Course name: .course-title-txt
    let course_name = try_selectors_text(
        &doc,
        &[
            ".course-title-txt",
            ".class-title-txt.course-view-header-txt",
        ],
    );

    // Page title: .contents-title-txt
    let title = try_selectors_text(
        &doc,
        &[
            ".contents-title-txt",
            ".contents-title .contents-title-txt",
            "title",
        ],
    );

    let mut sections = Vec::new();
    let mut meta = Vec::new();
    let mut attachments = Vec::new();

    // === Primary pattern: .contents-detail.contents-vertical rows ===
    {
        for row in doc.select(&SEL_DETAIL_VERT) {
            let label = row
                .select(&SEL_HEADER_COMBO)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            // Skip attachment rows — they'll be handled in the attachments section
            if row.select(&SEL_DOWNLOAD_FILE).next().is_some() {
                continue;
            }

            let value_el = row.select(&SEL_INPUT_AREA).next();
            let value = value_el
                .map(extract_filtered_input_text)
                .unwrap_or_default();
            let row_html = row.html();

            let row_quill_texts = extract_all_quill_texts(&row_html);

            if !label.is_empty() && is_body_label(&label) {
                let heading = if label == "内容" {
                    String::new()
                } else {
                    label.clone()
                };
                if !value.is_empty() {
                    push_unique_section(&mut sections, heading.clone(), value.clone());
                }
                for quill_text in row_quill_texts {
                    push_unique_section(&mut sections, heading.clone(), quill_text);
                }
                continue;
            }

            // Some report pages place the body in an unlabeled row with only Quill/text content.
            // Keep this scoped to the current row so we do not regress back to whole-page guesses.
            if label.is_empty() && (!row_quill_texts.is_empty() || value.len() >= 24) {
                if !value.is_empty() {
                    push_unique_section(&mut sections, String::new(), value.clone());
                }
                for quill_text in row_quill_texts {
                    push_unique_section(&mut sections, String::new(), quill_text);
                }
                continue;
            }

            if !label.is_empty() && !value.is_empty() {
                meta.push((label, value));
            } else if !label.is_empty() {
                // Value might be in Quill rich text (empty div + script)
                if let Some(quill_text) = extract_quill_text(&row_html) {
                    meta.push((label, quill_text));
                }
            }
        }
    }

    if sections.is_empty() {
        if let Some(body) = fallback_single_quill_section(html) {
            push_unique_section(&mut sections, String::new(), body);
        }
    }

    // === Extract attachments ===
    // First, find the download form to determine the correct download endpoint
    // Report pages: #reportDownloadForm -> /lms/course/report/submission_download
    // Forum pages: #forumsPostFile -> /lms/course/forums/thread_postfile
    #[allow(clippy::type_complexity)]
    let download_form_info: Option<(String, Vec<(String, String)>, bool)> = {
        let form_selectors: &[&Selector] = &[&SEL_REPORT_FORM, &SEL_FORUMS_FORM];
        let mut info = None;
        for sel in form_selectors {
            if let Some(form) = doc.select(sel).next() {
                let action = form.value().attr("action").unwrap_or_default().to_string();
                if !action.is_empty() {
                    let is_forum = form.value().id() == Some("forumsPostFile");
                    // Collect static hidden input params. Skip the per-file dynamic fields:
                    // Report: objectName, downloadFileName, downloadMode
                    // Forum: fileId, fileName
                    let mut params = Vec::new();
                    for input in form.select(&SEL_HIDDEN_INPUT) {
                        let iname = input.value().attr("name").unwrap_or_default();
                        let ival = input.value().attr("value").unwrap_or_default();
                        if !ival.is_empty()
                            && iname != "objectName"
                            && iname != "downloadFileName"
                            && iname != "downloadMode"
                            && iname != "fileId"
                            && iname != "fileName"
                        {
                            params.push((iname.to_string(), ival.to_string()));
                        }
                    }
                    log::debug!(
                        "[attachments] download form: action='{}', is_forum={}, params={:?}",
                        action,
                        is_forum,
                        params
                    );
                    info = Some((action, params, is_forum));
                    break;
                }
            }
        }
        info
    };

    // Pattern 1: .downloadFile elements (siblings of .objectName in same parent div)
    // Store objectName + form info so the download command can re-fetch _cid tokens
    {
        for el in doc.select(&SEL_DOWNLOAD_FILE) {
            let name = el.text().collect::<String>().trim().to_string();
            if !name.is_empty() {
                let obj_name = el
                    .parent()
                    .and_then(scraper::ElementRef::wrap)
                    .and_then(|parent_el| {
                        parent_el
                            .select(&SEL_OBJECT_NAME)
                            .next()
                            .map(|e| e.text().collect::<String>().trim().to_string())
                    })
                    .unwrap_or_default();

                if let Some((ref action, ref fixed_params, is_forum)) = download_form_info {
                    let link_type = classify_link("", &name);
                    // Merge static form params with per-file dynamic fields
                    let mut all_params = fixed_params.clone();
                    if is_forum {
                        // Forum: fileId = objectName, fileName = raw filename
                        all_params.push(("fileId".to_string(), obj_name.clone()));
                        all_params.push(("fileName".to_string(), name.clone()));
                    } else {
                        // Report: downloadFileName = raw filename, objectName, downloadMode = ""
                        all_params.push(("downloadFileName".to_string(), name.clone()));
                        all_params.push(("objectName".to_string(), obj_name.clone()));
                        all_params.push(("downloadMode".to_string(), String::new()));
                    }
                    log::debug!(
                        "[attachments] name='{}', objectName='{}', action='{}', type='{}'",
                        name,
                        obj_name,
                        action,
                        link_type
                    );
                    attachments.push(LunaAttachment {
                        name,
                        url: String::new(),
                        link_type,
                        object_name: obj_name,
                        download_action: action.clone(),
                        download_params: all_params,
                    });
                } else {
                    log::warn!(
                        "[attachments] no download form for '{}', objectName='{}'",
                        name,
                        obj_name
                    );
                }
            }
        }
    }

    // Pattern 2: links with download/tempfile in href
    if attachments.is_empty() {
        let file_selectors: &[&Selector] = &[&SEL_TEMPFILE_LINK, &SEL_DOWNLOAD_LINK];
        for file_sel in file_selectors {
            for a in doc.select(file_sel) {
                let name = a.text().collect::<String>().trim().to_string();
                let url = a.value().attr("href").unwrap_or_default().to_string();
                if !name.is_empty() && !url.is_empty() && !url.contains("javascript:") {
                    let link_type = classify_link(&url, &name);
                    attachments.push(LunaAttachment {
                        name,
                        url,
                        link_type,
                        object_name: String::new(),
                        download_action: String::new(),
                        download_params: Vec::new(),
                    });
                }
            }
        }
    }

    // Pattern 3: external links (e.g. SharePoint video links)
    {
        for a in doc.select(&SEL_VIDEO_LINK) {
            let url = a.value().attr("href").unwrap_or_default().to_string();
            if !url.is_empty() && url.starts_with("http") {
                // Show a friendly name for external video links
                let link_type = classify_link(&url, "");
                let display_name = match link_type.as_str() {
                    "cloud" => format!(
                        "動画 ({})",
                        if url.contains("sharepoint") {
                            "SharePoint"
                        } else {
                            "OneDrive"
                        }
                    ),
                    "video" => format!(
                        "動画 ({})",
                        if url.contains("youtube") || url.contains("youtu.be") {
                            "YouTube"
                        } else {
                            "Vimeo"
                        }
                    ),
                    "zoom" => "Zoom ミーティング".to_string(),
                    "panopto" => "Panopto 録画".to_string(),
                    _ => "外部リンク".to_string(),
                };
                attachments.push(LunaAttachment {
                    name: display_name,
                    url,
                    link_type,
                    object_name: String::new(),
                    download_action: String::new(),
                    download_params: Vec::new(),
                });
            }
        }
    }

    // === Extract forum posts (掲示板 thread pages) ===
    // Luna forum threads show posts in .post-body or .thread-post-body etc.
    let forum_post_selectors = [
        ".thread-post-area",
        ".post-list-area .post-body",
        ".forum-post-content",
        ".forums-thread-content",
    ];
    for sel_str in &forum_post_selectors {
        if let Ok(post_sel) = Selector::parse(sel_str) {
            for post_el in doc.select(&post_sel) {
                let post_text = post_el.text().collect::<String>();
                let trimmed = post_text.trim().to_string();
                if !trimmed.is_empty()
                    && trimmed.len() > 3
                    && !sections.iter().any(|s| s.body.contains(&trimmed))
                {
                    sections.push(LunaDetailSection {
                        heading: String::new(),
                        body: trimmed,
                    });
                }
            }
        }
    }
    // Note: forum_post_selectors are rarely-matched CSS that stay dynamic

    // Also extract any links from the page body that might be useful (external links, etc.)
    {
        for a in doc.select(&SEL_BODY_LINK) {
            let url = a.value().attr("href").unwrap_or_default().to_string();
            let name = a.text().collect::<String>().trim().to_string();
            if !url.is_empty()
                && url.starts_with("http")
                && !attachments.iter().any(|att| att.url == url)
            {
                let display = if name.is_empty() { url.clone() } else { name };
                let link_type = classify_link(&url, &display);
                attachments.push(LunaAttachment {
                    name: display,
                    url,
                    link_type,
                    object_name: String::new(),
                    download_action: String::new(),
                    download_params: Vec::new(),
                });
            }
        }
    }

    LunaDetailPage {
        title,
        course_name,
        sections,
        attachments,
        meta,
    }
}

/// This returns an HTML fragment that typically contains:
///   - .information-detail-title or similar title
///   - Quill Delta JSON via setJsonData() for the main body
///   - .contents-detail.contents-vertical rows for meta (掲示期間, 発信者, etc.)
pub fn parse_luna_announcement_detail(html: &str) -> LunaDetailPage {
    let doc = Html::parse_fragment(html);
    let mut sections = Vec::new();
    let mut meta = Vec::new();
    let mut attachments: Vec<LunaAttachment> = Vec::new();

    // Title from #osiraseTitle or .block-title-txt
    let title = try_selectors_text(
        &doc,
        &["#osiraseTitle", ".block-title-txt", ".contents-title-txt"],
    );

    // Helper: build a LunaAttachment from an announcement-style .downloadFile div.
    // Real Luna HTML looks like:
    //   <div class="link-txt downloadFile downFile">
    //     <p>display name.pdf</p>
    //     <input type="hidden" class="cmtInfoFileName"   value="filename.pdf">
    //     <input type="hidden" class="cmtInfoObjectName" value="2026/ab/cd/ef/uuid">
    //   </div>
    // The browser downloads from:
    //   /lms/information/file/down/{makeDownFileName(name)}?fileName={name}&objectName={obj}
    // We drop this into download_action/download_params so luna_download_file()
    // assembles the URL the same way its report/forum path does.
    let build_announcement_attachment = |container: scraper::ElementRef| -> Option<LunaAttachment> {
        let file_name = container
            .select(&SEL_CMT_FILENAME)
            .next()
            .and_then(|e| e.value().attr("value"))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                let p = container.text().collect::<String>().trim().to_string();
                if p.is_empty() {
                    None
                } else {
                    Some(p)
                }
            })?;
        let obj_name = container
            .select(&SEL_CMT_OBJECTNAME)
            .next()
            .and_then(|e| e.value().attr("value"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        if obj_name.is_empty() {
            log::warn!(
                "[announcement] attachment '{}' has no objectName",
                file_name
            );
            return None;
        }

        let link_type = classify_link("", &file_name);
        log::debug!(
            "[announcement] attachment name='{}', object='{}'",
            file_name,
            obj_name
        );
        Some(LunaAttachment {
            name: file_name.clone(),
            url: String::new(),
            link_type,
            object_name: obj_name.clone(),
            download_action: "/lms/information/file/down".to_string(),
            download_params: vec![
                ("fileName".to_string(), file_name),
                ("objectName".to_string(), obj_name),
            ],
        })
    };

    // Extract meta rows from .contents-detail.contents-vertical
    {
        for row in doc.select(&SEL_DETAIL_VERT) {
            let label = row
                .select(&SEL_HEADER_BOLD)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            // Attachment rows: each .downloadFile div contains hidden inputs with the
            // real file/object name. Read those directly instead of trusting text.
            if row.select(&SEL_DOWNLOAD_FILE).next().is_some() {
                for file_el in row.select(&SEL_DOWNLOAD_FILE) {
                    if let Some(att) = build_announcement_attachment(file_el) {
                        attachments.push(att);
                    }
                }
                continue;
            }

            let value_el = row.select(&SEL_INPUT_AREA).next();
            // Collect text from the value area, filtering out script content
            let value = value_el
                .map(extract_filtered_input_text)
                .unwrap_or_default();
            let row_html = row.html();

            if label == "内容" {
                if !value.is_empty() {
                    push_unique_section(&mut sections, String::new(), value.clone());
                }
                for quill_text in extract_all_quill_texts(&row_html) {
                    push_unique_section(&mut sections, String::new(), quill_text);
                }
                continue;
            }

            // Skip "添付ファイル" if empty
            if label == "添付ファイル" && value.is_empty() {
                continue;
            }

            if !label.is_empty() && !value.is_empty() {
                meta.push((label, value));
            }
        }
    }

    if sections.is_empty() {
        if let Some(body) = fallback_single_quill_section(html) {
            push_unique_section(&mut sections, String::new(), body);
        }
    }

    // Fallback: some layouts may keep .downloadFile blocks outside the
    // .contents-detail rows. Sweep the whole fragment only if nothing was found.
    if attachments.is_empty() {
        for file_el in doc.select(&SEL_DOWNLOAD_FILE) {
            if let Some(att) = build_announcement_attachment(file_el) {
                attachments.push(att);
            }
        }
    }

    LunaDetailPage {
        title,
        course_name: String::new(),
        sections,
        attachments,
        meta,
    }
}
