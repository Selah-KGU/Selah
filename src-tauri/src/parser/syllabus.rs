//! Course detail, textbooks, session plans, and delivery mode.

use super::*;

// ============ Course Detail (ARF020) ============

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CourseDetail {
    pub fields: Vec<(String, String)>,
}

/// Parse course detail page: extract all th/td pairs from the first table that yields results.
///
/// Values preserve the cell's inner HTML so KGC detail links survive through to
/// the frontend and can be intercepted there instead of being flattened into
/// plain text during parsing.
/// Tries selectors in order: table.output → table.form → table.tbl → table
pub fn parse_course_detail(html: &str) -> CourseDetail {
    let doc = Html::parse_document(html);

    let candidates = ["table.output", "table.form", "table.tbl", "table"];
    let textbook_skip = [
        "教科書",
        "参考書",
        "参考文献",
        "Reference books",
        "Required texts",
    ];
    for selector_str in &candidates {
        let Ok(table_sel) = Selector::parse(selector_str) else {
            continue;
        };
        let mut fields = Vec::new();
        for table in doc.select(&table_sel) {
            for tr in table.select(&SEL_TR) {
                let ths: Vec<_> = tr.select(&SEL_TH).collect();
                let tds: Vec<_> = tr.select(&SEL_TD).collect();
                for (ti, th) in ths.iter().enumerate() {
                    let label = th.text().collect::<String>().trim().to_string();
                    // Skip textbook rows — handled by parse_textbooks()
                    if textbook_skip.iter().any(|kw| label.contains(kw)) {
                        continue;
                    }
                    let value = tds
                        .get(ti)
                        .map(|td| td.inner_html().trim().to_string())
                        .unwrap_or_default();
                    if !label.is_empty() {
                        fields.push((label, value));
                    }
                }
            }
        }
        if !fields.is_empty() {
            return CourseDetail { fields };
        }
    }

    CourseDetail { fields: Vec::new() }
}

/// Structured textbook/reference entry from syllabus detail page.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TextbookEntry {
    pub category: String, // "教科書" or "参考書"
    pub author: String,
    pub title: String,
    pub publisher: String,
    pub year: String,
    pub isbn: String,
    pub text: String, // plain-text fallback (for simple format)
}

/// Parse structured textbook tables from a syllabus detail page.
/// Handles two formats:
/// 1. Structured table with columns: 著者名, タイトル, 発行所, 出版年, ISBN
/// 2. Simple th/td pair with plain text description
pub fn parse_textbooks(html: &str) -> Vec<TextbookEntry> {
    let doc = Html::parse_document(html);
    let mut entries = Vec::new();
    let textbook_keywords = [
        "教科書",
        "参考書",
        "参考文献",
        "Reference books",
        "Required texts",
    ];

    let candidates = ["table.output", "table.form", "table.tbl", "table"];
    for sel_str in &candidates {
        let Ok(table_sel) = Selector::parse(sel_str) else {
            continue;
        };
        let mut found_any = false;
        for table in doc.select(&table_sel) {
            let rows: Vec<_> = table.select(&SEL_TR).collect();
            if rows.is_empty() {
                continue;
            }

            // Check if any th in this table matches textbook keywords
            let mut category = String::new();
            let mut is_structured = false;

            for tr in &rows {
                let ths: Vec<_> = tr.select(&SEL_TH).collect();
                for th in &ths {
                    let th_text = th.text().collect::<String>();
                    let th_trimmed = th_text.trim();
                    for kw in &textbook_keywords {
                        if th_trimmed.contains(kw) {
                            // Determine category
                            if th_trimmed.contains("参考") {
                                category = "参考書".to_string();
                            } else {
                                category = "教科書".to_string();
                            }

                            // Check for rowspan (structured format has rowspan="2"+)
                            if let Some(rs) = th.value().attr("rowspan") {
                                if rs.parse::<i32>().unwrap_or(0) >= 2 {
                                    is_structured = true;
                                }
                            }

                            // Also check if there are header-like tds (著者名, タイトル etc.)
                            let tds: Vec<_> = tr.select(&SEL_TD).collect();
                            if tds.len() >= 3 {
                                let first_td = tds[0].text().collect::<String>();
                                if first_td.contains("著者")
                                    || first_td.contains("Author")
                                    || first_td.contains("タイトル")
                                {
                                    is_structured = true;
                                }
                            }
                            break;
                        }
                    }
                    if !category.is_empty() {
                        break;
                    }
                }
                if !category.is_empty() {
                    break;
                }
            }

            if category.is_empty() {
                continue;
            }
            found_any = true;

            if is_structured {
                // Parse structured rows: skip header rows (those with <th> or bgcolor header tds)
                for tr in &rows {
                    let ths: Vec<_> = tr.select(&SEL_TH).collect();
                    if !ths.is_empty() {
                        continue;
                    } // skip header rows

                    let tds: Vec<_> = tr.select(&SEL_TD).collect();
                    if tds.is_empty() {
                        continue;
                    }

                    // Check if this is a header row (bgcolor tds)
                    if let Some(bg) = tds[0].value().attr("bgcolor") {
                        if !bg.is_empty() {
                            continue;
                        }
                    }
                    let first_text = tds[0].text().collect::<String>();
                    if first_text.trim().contains("著者") || first_text.trim().contains("Author")
                    {
                        continue;
                    }

                    // Data row: author, title, publisher, year, isbn
                    let get_td = |i: usize| -> String {
                        tds.get(i)
                            .map(|td| td.text().collect::<String>().trim().to_string())
                            .unwrap_or_default()
                    };
                    let author = get_td(0);
                    let title = get_td(1);
                    let publisher = get_td(2);
                    let year = get_td(3).trim().replace(" ", "");
                    let isbn = get_td(4);

                    if author.is_empty() && title.is_empty() {
                        continue;
                    }

                    entries.push(TextbookEntry {
                        category: category.clone(),
                        author,
                        title,
                        publisher,
                        year,
                        isbn,
                        text: String::new(),
                    });
                }
            } else {
                // Simple format: th has keyword, td has plain text
                for tr in &rows {
                    let ths: Vec<_> = tr.select(&SEL_TH).collect();
                    let tds: Vec<_> = tr.select(&SEL_TD).collect();
                    for (ti, th) in ths.iter().enumerate() {
                        let th_text = th.text().collect::<String>();
                        let is_textbook = textbook_keywords.iter().any(|kw| th_text.contains(kw));
                        if !is_textbook {
                            continue;
                        }
                        let value = tds
                            .get(ti)
                            .map(|td| td.text().collect::<String>().trim().to_string())
                            .unwrap_or_default();
                        if value.is_empty() {
                            continue;
                        }
                        let cat = if th_text.contains("参考") {
                            "参考書"
                        } else {
                            "教科書"
                        };
                        entries.push(TextbookEntry {
                            category: cat.to_string(),
                            author: String::new(),
                            title: String::new(),
                            publisher: String::new(),
                            year: String::new(),
                            isbn: String::new(),
                            text: value,
                        });
                    }
                }
            }
        }
        if found_any {
            break;
        }
    }

    entries
}

// ============ 授業計画 Structured Parser ============

#[derive(Debug, Serialize, Clone)]
pub struct SessionPlan {
    pub session_num: i32,
    pub th_header: String,
    pub topic: String,
    pub delivery_mode: String,
    pub study_outside: String,
}

/// Parse structured 授業計画 from a course detail page.
///
/// **Data-only**: extracts raw text from each table row, no filtering or keyword detection.
/// - `th_header`: text from `<th>` cells after the session number marker
/// - `topic`: text from the first content `<td>`
/// - `delivery_mode`: middle `<td>` columns joined (e.g. "対面", "オンデマンド")
/// - `study_outside`: text from the last `<td>` if there are 2+ content tds
///
/// All additional `<td>` columns are appended to topic in `[brackets]` so no data is lost.
pub fn parse_session_plans(html: &str) -> Vec<SessionPlan> {
    let doc = Html::parse_document(html);
    let mut plans = Vec::new();

    let candidates = ["table.output", "table.form", "table.tbl", "table"];
    for selector_str in &candidates {
        let Ok(table_sel) = Selector::parse(selector_str) else {
            continue;
        };
        for table in doc.select(&table_sel) {
            for tr in table.select(&SEL_TR) {
                let ths: Vec<_> = tr.select(&SEL_TH).collect();
                let tds: Vec<_> = tr.select(&SEL_TD).collect();

                let all_cells: Vec<String> = ths
                    .iter()
                    .chain(tds.iter())
                    .map(|el| el.text().collect::<String>())
                    .collect();
                let full_text = all_cells.join(" ");

                if let Some(caps) = SESSION_RE.captures(&full_text) {
                    let range_str = caps
                        .get(1)
                        .or(caps.get(2))
                        .map(|m| m.as_str())
                        .unwrap_or("");
                    let session_nums = expand_session_range(range_str, &NUM_RE);
                    if session_nums.is_empty() {
                        continue;
                    }

                    // ── th_header: everything after the session number marker ──
                    let th_full: String = ths
                        .iter()
                        .map(|el| el.text().collect::<String>())
                        .collect::<Vec<_>>()
                        .join(" ");
                    let th_header = {
                        let last_end = SESSION_RE
                            .find_iter(&th_full)
                            .last()
                            .map(|m| m.end())
                            .unwrap_or(0);
                        if last_end > 0 && last_end <= th_full.len() {
                            th_full[last_end..].trim().to_string()
                        } else {
                            String::new()
                        }
                    };

                    // ── All td cells as raw text ──
                    let td_texts: Vec<String> = tds
                        .iter()
                        .map(|td| td.text().collect::<String>().trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();

                    let mut topic = String::new();
                    let mut study_outside = String::new();
                    let mut delivery_mode = String::new();

                    if td_texts.len() >= 2 {
                        topic = td_texts[0].clone();
                        study_outside = td_texts[td_texts.len() - 1].clone();
                        // Middle columns → likely delivery mode or short metadata
                        let mid: Vec<_> = td_texts[1..td_texts.len() - 1].to_vec();
                        if !mid.is_empty() {
                            delivery_mode = mid.join(" / ");
                        }
                    } else if td_texts.len() == 1 {
                        topic = td_texts[0].clone();
                    } else if td_texts.is_empty() && ths.len() > 1 {
                        // Fallback: use th cells after the first
                        topic = ths
                            .iter()
                            .skip(1)
                            .map(|el| el.text().collect::<String>().trim().to_string())
                            .collect::<Vec<_>>()
                            .join(" ");
                    }

                    for &sn in &session_nums {
                        plans.push(SessionPlan {
                            session_num: sn,
                            th_header: th_header.clone(),
                            topic: topic.clone(),
                            delivery_mode: delivery_mode.clone(),
                            study_outside: study_outside.clone(),
                        });
                    }
                }
            }
        }
        if !plans.is_empty() {
            break;
        }
    }

    plans.sort_by_key(|p| p.session_num);
    plans.dedup_by_key(|p| p.session_num);
    plans
}

/// Expand a session range string like "1-2", "3～14", "1,2,3", "5・6" into individual numbers.
/// Also handles plain single numbers like "1".
/// Normalizes fullwidth digits (０-９) to ASCII before parsing.
pub(super) fn expand_session_range(range_str: &str, num_re: &regex::Regex) -> Vec<i32> {
    // Normalize fullwidth digits to ASCII (０→0, １→1, ... ９→9)
    let normalized: String = range_str
        .chars()
        .map(|c| match c {
            '\u{FF10}'..='\u{FF19}' => char::from(b'0' + (c as u32 - 0xFF10) as u8),
            _ => c,
        })
        .collect();

    let nums: Vec<i32> = num_re
        .find_iter(&normalized)
        .filter_map(|m| m.as_str().parse::<i32>().ok())
        .filter(|n| (1..=30).contains(n))
        .collect();

    if nums.is_empty() {
        return Vec::new();
    }

    // If exactly 2 numbers and the string contains a range separator, expand
    if nums.len() == 2 {
        let has_range_sep = range_str.contains('-')
            || range_str.contains('～')
            || range_str.contains('~')
            || range_str.contains('\u{FF0D}'); // fullwidth hyphen-minus
        if has_range_sep && nums[0] < nums[1] {
            return (nums[0]..=nums[1]).filter(|n| *n <= 30).collect();
        }
    }

    // Otherwise return all parsed numbers as-is (comma/dot separated list)
    nums
}

/// Detect delivery mode from topic text.
/// Priority: オンデマンド > 同時双方向 > オンライン > 対面
/// "対面" is checked last because it frequently appears in descriptive text
/// (e.g. "対面授業12回中3回以内の欠席") even when the session itself is online.
pub(super) fn detect_delivery_mode(text: &str) -> String {
    if text.contains("オンデマンド") {
        "オンデマンド".to_string()
    } else if text.contains("同時双方向") {
        "同時双方向".to_string()
    } else if text.contains("オンライン") {
        "オンライン".to_string()
    } else if text.contains("対面授業") || text.contains("対面") {
        "対面".to_string()
    } else {
        String::new()
    }
}

/// Extract delivery mode from a course detail page by scanning specific field labels only.
/// Only returns a value when a dedicated field (授業形態, 授業方法, etc.) explicitly states
/// the mode. Does NOT fall back to scanning the full page text, because pages often
/// mention "対面" in session plan rows or descriptions even when individual sessions
/// use a different mode — the per-session delivery_mode in session_plans is authoritative.
pub fn detect_delivery_mode_from_detail(html: &str) -> String {
    let doc = Html::parse_document(html);

    let candidates = ["table.output", "table.form", "table.tbl", "table"];
    for selector_str in &candidates {
        let Ok(table_sel) = Selector::parse(selector_str) else {
            continue;
        };
        for table in doc.select(&table_sel) {
            for tr in table.select(&SEL_TR) {
                let ths: Vec<_> = tr.select(&SEL_TH).collect();
                let tds: Vec<_> = tr.select(&SEL_TD).collect();
                for (ti, th) in ths.iter().enumerate() {
                    let label = th.text().collect::<String>();
                    let label_trimmed = label.trim();
                    if label_trimmed.contains("授業形態")
                        || label_trimmed.contains("授業方法")
                        || label_trimmed.contains("授業の進め方")
                        || label_trimmed.contains("授業スタイル")
                    {
                        if let Some(td) = tds.get(ti) {
                            let value = td.text().collect::<String>();
                            let mode = detect_delivery_mode(&value);
                            if !mode.is_empty() {
                                return mode;
                            }
                        }
                    }
                }
            }
        }
    }

    // No dedicated field found — return empty so session-plan per-session modes are used
    String::new()
}
