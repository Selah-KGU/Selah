//! Campus notifications and notification detail pages.

use super::*;

// ============ Notifications (CPA010/CPA020) ============

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NotificationEntry {
    pub id: String,
    pub title: String,
    pub date: String,
    pub category: String,
    /// Detail page path (relative to KG_COURSE_BASE) extracted from the title
    /// `<a>` href. Empty when the row exposes no link (legacy or non-clickable
    /// rows). Older cached entries that predate this field deserialize as `""`.
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NotificationsData {
    pub entries: Vec<NotificationEntry>,
}

fn is_notification_header(headers: &[String]) -> bool {
    headers.iter().any(|text| {
        text.contains("タイトル") || text.contains("お知らせ") || text.contains("掲示日")
    })
}

/// True only when the response actually contains the campus notice table.
/// A login shell or empty portal page must not be stored as "no notices".
pub fn notifications_list_present(html: &str) -> bool {
    let doc = Html::parse_document(html);
    doc.select(&SEL_TR).any(|tr| {
        let headers: Vec<String> = tr
            .select(&SEL_TH)
            .map(|el| el.text().collect::<String>().trim().to_string())
            .collect();
        is_notification_header(&headers)
    })
}

pub fn parse_notifications(html: &str) -> NotificationsData {
    let doc = Html::parse_document(html);
    let mut entries = Vec::new();

    let a_sel = Selector::parse("a").expect("valid selector");

    let mut headers: Vec<String> = Vec::new();

    for tr in doc.select(&SEL_TR) {
        let ths: Vec<String> = tr
            .select(&SEL_TH)
            .map(|el| el.text().collect::<String>().trim().to_string())
            .collect();

        if is_notification_header(&ths) {
            headers = ths;
            continue;
        }

        if headers.is_empty() {
            continue;
        }

        let tds: Vec<_> = tr.select(&SEL_TD).collect();

        if tds.is_empty() {
            continue;
        }

        // Map columns by header
        let col_idx = |name: &str| -> Option<usize> {
            for (i, h) in headers.iter().enumerate() {
                if h.contains(name) {
                    return Some(i);
                }
            }
            None
        };

        // Get title (may be in a link) and capture the detail href when present.
        let title_i = col_idx("タイトル").or(col_idx("お知らせ")).unwrap_or(0);
        let (title, url) = if let Some(td) = tds.get(title_i) {
            if let Some(a) = td.select(&a_sel).next() {
                let title = a.text().collect::<String>().trim().to_string();
                let url = a
                    .value()
                    .attr("href")
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                (title, url)
            } else {
                (
                    td.text().collect::<String>().trim().to_string(),
                    String::new(),
                )
            }
        } else {
            continue;
        };

        if title.is_empty() {
            continue;
        }

        let date_i = col_idx("掲示日")
            .or(col_idx("日付"))
            .unwrap_or(headers.len().saturating_sub(1));
        let date = tds
            .get(date_i)
            .map(|td| td.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let category_i = col_idx("分類").or(col_idx("区分"));
        let category = category_i
            .and_then(|i| tds.get(i))
            .map(|td| td.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        // Build a stable ID from title+date so read-state tracking survives list changes
        let stable_id = format!("{}|{}", title.trim(), date.trim());
        entries.push(NotificationEntry {
            id: stable_id,
            title,
            date,
            category,
            url,
        });
    }

    NotificationsData { entries }
}

// ============ Notification Detail (CPA020) ============

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct NotificationDetail {
    pub title: String,
    pub date: String,
    pub category: String,
    pub sender: String,
    pub body: String,
    pub attachments: Vec<NotificationAttachment>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NotificationAttachment {
    pub name: String,
    pub url: String,
}

/// Parse a KGC notification detail page (CPA020). The KGC system renders the
/// detail in a label/value table; we walk it generically so future field
/// renames don't break the parser.
pub fn parse_notification_detail(html: &str) -> NotificationDetail {
    let doc = Html::parse_document(html);
    let a_sel = Selector::parse("a").expect("valid selector");

    let mut detail = NotificationDetail::default();

    for tr in doc.select(&SEL_TR) {
        let th_text = tr
            .select(&SEL_TH)
            .map(|el| el.text().collect::<String>().trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let tds: Vec<_> = tr.select(&SEL_TD).collect();
        if tds.is_empty() {
            continue;
        }
        let value_text = tds
            .iter()
            .map(|td| td.text().collect::<String>().trim().to_string())
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string();
        if th_text.contains("タイトル") || th_text.contains("件名") {
            detail.title = value_text.clone();
        } else if th_text.contains("掲示日") || th_text.contains("日付") {
            detail.date = value_text.clone();
        } else if th_text.contains("分類") || th_text.contains("区分") {
            detail.category = value_text.clone();
        } else if th_text.contains("発信") || th_text.contains("送信") || th_text.contains("掲示者")
        {
            detail.sender = value_text.clone();
        } else if th_text.contains("本文") || th_text.contains("内容") || th_text.contains("詳細")
        {
            // Preserve newlines from <br> by inserting separators around block tags.
            let raw_html = tds
                .iter()
                .map(|td| td.inner_html())
                .collect::<Vec<_>>()
                .join("\n");
            detail.body = strip_html_keep_text(&raw_html);
        }
        // Collect any attachment-shaped links (.pdf, .docx, .xlsx, .pptx, .zip).
        for a in tds.iter().flat_map(|td| td.select(&a_sel)) {
            let name = a.text().collect::<String>().trim().to_string();
            let href = a.value().attr("href").unwrap_or_default().trim();
            if name.is_empty() || href.is_empty() {
                continue;
            }
            let lower = name.to_lowercase();
            if [
                ".pdf", ".docx", ".xlsx", ".pptx", ".zip", ".doc", ".xls", ".ppt",
            ]
            .iter()
            .any(|ext| lower.ends_with(ext))
            {
                detail.attachments.push(NotificationAttachment {
                    name,
                    url: href.to_string(),
                });
            }
        }
    }

    // Fallback: if no labelled body row matched, take the largest text block in the page.
    if detail.body.is_empty() {
        if let Ok(sel) = Selector::parse("td, .body, .content, #content") {
            let largest = doc
                .select(&sel)
                .map(|el| el.text().collect::<String>().trim().to_string())
                .max_by_key(|s| s.len())
                .unwrap_or_default();
            detail.body = largest;
        }
    }

    detail
}

pub(super) fn strip_html_keep_text(html: &str) -> String {
    use scraper::Html;
    let mut prepared = html.to_string();
    for tag in ["</p>", "</div>", "</li>", "<br>", "<br/>", "<br />"] {
        prepared = prepared.replace(tag, &format!("{}\n", tag));
    }
    let doc = Html::parse_fragment(&prepared);
    let text: String = doc.root_element().text().collect();
    let mut out = String::with_capacity(text.len());
    let mut last_blank = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !last_blank && !out.is_empty() {
                out.push('\n');
            }
            last_blank = true;
        } else {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(trimmed);
            last_blank = false;
        }
    }
    out.trim().to_string()
}
