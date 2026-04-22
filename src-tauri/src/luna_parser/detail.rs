use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaDetailSection {
    pub heading: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaDetailPage {
    pub title: String,
    pub course_name: String,
    pub sections: Vec<LunaDetailSection>,
    pub attachments: Vec<LunaAttachment>,
    pub meta: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaAttachment {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub link_type: String, // "file", "external", "video", "zoom", "panopto", "web"
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub object_name: String,
    /// Form action path for download (e.g. /lms/course/report/submission_download)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub download_action: String,
    /// Fixed form params (reportId, idnumber, etc.) serialized as key=value pairs
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub download_params: Vec<(String, String)>,
}

/// Extract Quill text from a specific named variable (e.g. "themeContents", "threadContents0")
pub(super) fn extract_named_quill_text(html: &str, var_name: &str) -> Option<String> {
    // Pattern: _QuillUtil.varName.setJsonData("...", ...)
    // or: _QuillUtil.varName = ... .setJsonData("...", ...)
    let pattern = format!("{}.setJsonData(\"", var_name);
    let pos = html.find(&pattern)?;
    let after = &html[pos + pattern.len()..];
    // Find the closing ", 'reference') or similar
    // The JSON string ends at the last " before the next , or )
    // The JSON is double-escaped: \" inside the JS string
    let mut end = 0;
    let chars: Vec<char> = after.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            i += 2; // skip escaped char
            continue;
        }
        if chars[i] == '"' {
            end = i;
            break;
        }
        i += 1;
    }
    if end == 0 {
        return None;
    }
    let json_str = &after[..end];
    extract_quill_rich_html(json_str)
}

fn escape_html_fragment(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn wrap_quill_inline_attrs(
    text: &str,
    attrs: Option<&serde_json::Map<String, serde_json::Value>>,
) -> String {
    let mut out = escape_html_fragment(text);
    let Some(attrs) = attrs else {
        return out;
    };

    if attrs.get("bold").and_then(|v| v.as_bool()) == Some(true) {
        out = format!("<strong>{}</strong>", out);
    }
    if attrs.get("italic").and_then(|v| v.as_bool()) == Some(true) {
        out = format!("<em>{}</em>", out);
    }
    if attrs.get("underline").and_then(|v| v.as_bool()) == Some(true) {
        out = format!("<u>{}</u>", out);
    }
    if attrs.get("code").and_then(|v| v.as_bool()) == Some(true) {
        out = format!("<code>{}</code>", out);
    }

    let mut style_parts: Vec<String> = Vec::new();
    if let Some(color) = attrs.get("color").and_then(|v| v.as_str()) {
        style_parts.push(format!("color:{}", escape_html_fragment(color)));
    }
    if let Some(bg) = attrs.get("background").and_then(|v| v.as_str()) {
        style_parts.push(format!("background-color:{}", escape_html_fragment(bg)));
    }
    if !style_parts.is_empty() {
        out = format!("<span style=\"{}\">{}</span>", style_parts.join(";"), out);
    }

    if let Some(link) = attrs.get("link").and_then(|v| v.as_str()) {
        let lower = link.to_lowercase();
        if lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.starts_with("mailto:")
            || lower.starts_with("tel:")
        {
            out = format!(
                "<a href=\"{}\" target=\"_blank\" rel=\"noopener\">{}</a>",
                escape_html_fragment(link),
                out
            );
        }
    }

    out
}

pub(super) fn extract_quill_rich_html(json_str: &str) -> Option<String> {
    let unescaped = unescape_js_string(json_str);
    let val: serde_json::Value = serde_json::from_str(&unescaped).ok()?;
    let ops = val.get("ops")?.as_array()?;

    let mut html = String::new();
    for op in ops {
        let Some(insert) = op.get("insert").and_then(|v| v.as_str()) else {
            continue;
        };
        let attrs = op.get("attributes").and_then(|a| a.as_object());

        let mut segment = String::new();
        for ch in insert.chars() {
            if ch == '\n' {
                if !segment.is_empty() {
                    html.push_str(&wrap_quill_inline_attrs(&segment, attrs));
                    segment.clear();
                }
                html.push_str("<br>");
            } else {
                segment.push(ch);
            }
        }
        if !segment.is_empty() {
            html.push_str(&wrap_quill_inline_attrs(&segment, attrs));
        }
    }

    let compact = html
        .replace("<br>", "")
        .replace("<br/>", "")
        .replace("<br />", "")
        .trim()
        .to_string();
    if compact.is_empty() {
        None
    } else {
        Some(html)
    }
}

fn normalize_detail_text(s: &str) -> String {
    s.split_whitespace().collect::<String>()
}

fn normalize_notice_body_lines(s: &str) -> Vec<String> {
    static RE_TAGS: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?is)<[^>]+>").expect("valid regex"));

    let normalized = s
        .replace("<br />", "\n")
        .replace("<br/>", "\n")
        .replace("<br>", "\n")
        .replace("</p>", "\n")
        .replace("</div>", "\n")
        .replace("</li>", "\n")
        .replace("&nbsp;", " ");
    let plain = RE_TAGS.replace_all(&normalized, "");
    plain
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect()
}

pub(crate) fn is_blacklisted_system_notice_text(s: &str) -> bool {
    let text = s.trim();
    if text.is_empty() {
        return false;
    }

    // Luna occasionally returns university-wide support/maintenance notices in place
    // of the actual activity body on the first request. These are stable enough to
    // blacklist explicitly instead of further tightening the structural parser.
    const STRONG_PATTERNS: &[&str] = &[
        "ゲストアクセス」と「履修登録」は違います",
        "履修データ連携に関する補足",
        "このセッションの表示アクセス権がありません。",
        "学生キャビネット ＞ 教務機構 ＞ LUNA・ポートフォリオ",
        "macを利用されている学生は注意ください",
        "LUNAサポートへお問い合わせいただく前に",
        "メンテナンス時間帯において、接続が切れる場合があります。",
    ];
    if STRONG_PATTERNS.iter().any(|pattern| text.contains(pattern)) {
        return true;
    }

    let grouped_patterns: &[&[&str]] = &[
        &["時間割", "ゲストアクセス", "履修登録"],
        &["KG Chatbot", "学生向け動画マニュアル"],
        &["Panoptoボタン", "アクセス権をリクエスト"],
        &[
            "Panoptoボタン",
            "このセッションの表示アクセス権がありません。",
        ],
        &["教務連携スケジュール", "履修データ連携"],
        &["LUNAの定期メンテナンスについて", "AM2:00 - AM2:30"],
        &["学生キャビネット", "LUNA・ポートフォリオ"],
    ];
    grouped_patterns
        .iter()
        .any(|group| group.iter().all(|pattern| text.contains(pattern)))
}

fn is_system_notice_line(line: &str) -> bool {
    const LINE_PATTERNS: &[&str] = &[
        "時間割",
        "ゲストアクセス",
        "履修登録",
        "履修データ連携",
        "教務連携スケジュール",
        "このセッションの表示アクセス権がありません。",
        "アクセス権をリクエスト",
        "学生キャビネット",
        "LUNA・ポートフォリオ",
        "KG Chatbot",
        "学生向け動画マニュアル",
        "LUNAの定期メンテナンスについて",
        "メンテナンス時間帯において、接続が切れる場合があります。",
        "LUNAサポートへお問い合わせいただく前に",
        "macを利用されている学生は注意ください",
        "AM2:00 - AM2:30",
    ];
    LINE_PATTERNS.iter().any(|pattern| line.contains(pattern))
}

fn sanitize_blacklisted_notice_body(body: &str) -> Option<String> {
    let body = body.trim();
    if body.is_empty() {
        return None;
    }
    if !is_blacklisted_system_notice_text(body) {
        return Some(body.to_string());
    }

    let kept_lines: Vec<String> = normalize_notice_body_lines(body)
        .into_iter()
        .filter(|line| !is_system_notice_line(line))
        .collect();
    if kept_lines.is_empty() {
        return None;
    }

    let candidate = kept_lines.join("\n").trim().to_string();
    if candidate.is_empty() || is_blacklisted_system_notice_text(&candidate) {
        return None;
    }
    Some(candidate)
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
/// Classify a URL into a link type for display purposes.
pub(super) fn classify_link(url: &str, name: &str) -> String {
    let u = url.to_lowercase();
    let n = name.to_lowercase();

    // Internal Luna download paths → file
    if !u.starts_with("http") {
        return "file".into();
    }

    // Zoom
    if u.contains("zoom.us") || u.contains("zoom.") || u.contains("/lti/zoom") {
        return "zoom".into();
    }
    // Panopto
    if u.contains("panopto") || u.contains("/lti/panopto") || u.contains("/Panopto/") {
        return "panopto".into();
    }
    // Video platforms
    if u.contains("youtube.com") || u.contains("youtu.be") || u.contains("vimeo.com") {
        return "video".into();
    }
    // SharePoint / OneDrive (often used for video/file sharing)
    if u.contains("sharepoint.com") || u.contains("onedrive.live.com") || u.contains("1drv.ms") {
        return "cloud".into();
    }
    // Google (Drive, Docs, Slides, Sheets, Forms)
    if u.contains("drive.google.com")
        || u.contains("docs.google.com")
        || u.contains("forms.gle")
        || u.contains("forms.google.com")
    {
        return "google".into();
    }
    // Microsoft Teams
    if u.contains("teams.microsoft.com") || u.contains("teams.live.com") {
        return "teams".into();
    }
    // Known file extensions in URL or name → treat as downloadable external file
    let file_exts = [
        ".pdf", ".doc", ".docx", ".ppt", ".pptx", ".xls", ".xlsx", ".zip", ".rar", ".7z", ".mp4",
        ".mp3", ".wav", ".png", ".jpg", ".jpeg",
    ];
    for ext in &file_exts {
        if u.ends_with(ext) || n.ends_with(ext) {
            return "file".into();
        }
    }

    "web".into()
}

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
