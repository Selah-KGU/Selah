//! HTML form and Luna report-period helpers.
//!
//! Pulls query parameters, hidden inputs, and submission windows out of
//! Luna pages. The command handlers stay in the parent module.

use super::*;

macro_rules! sel {
    ($name:ident, $s:expr) => {
        static $name: LazyLock<scraper::Selector> =
            LazyLock::new(|| scraper::Selector::parse($s).expect(concat!("bad selector: ", $s)));
    };
}

pub(super) fn extract_url_param(url: &str, key: &str) -> Option<String> {
    let query = url.split('?').nth(1)?;
    for part in query.split('&') {
        let mut kv = part.splitn(2, '=');
        if kv.next()? == key {
            return kv.next().map(|v| v.to_string());
        }
    }
    None
}

const REPORT_SUBMISSION_NOT_OPEN_MESSAGE: &str =
    "提出期間外のため、現在は提出できません。提出期間を確認してから再度お試しください。";

fn normalize_html_text(html: &str) -> String {
    let doc = scraper::Html::parse_document(html);
    doc.root_element()
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_luna_report_datetime(value: &str) -> Option<chrono::DateTime<chrono::Local>> {
    static REPORT_DATETIME_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"^\s*(\d{4})[/-](\d{1,2})[/-](\d{1,2})\s+(\d{1,2}):(\d{2})\s*$")
            .expect("valid report datetime regex")
    });
    let captures = REPORT_DATETIME_RE.captures(value)?;
    let year = captures.get(1)?.as_str().parse::<i32>().ok()?;
    let month = captures.get(2)?.as_str().parse::<u32>().ok()?;
    let day = captures.get(3)?.as_str().parse::<u32>().ok()?;
    let hour = captures.get(4)?.as_str().parse::<u32>().ok()?;
    let minute = captures.get(5)?.as_str().parse::<u32>().ok()?;
    if hour > 24 || minute > 59 || (hour == 24 && minute != 0) {
        return None;
    }

    let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
    let (date, hour) = if hour == 24 {
        (date.succ_opt()?, 0)
    } else {
        (date, hour)
    };
    let naive = date.and_hms_opt(hour, minute, 0)?;
    chrono::Local.from_local_datetime(&naive).single()
}

pub(super) fn parse_luna_report_period(
    period: &str,
) -> Option<(
    chrono::DateTime<chrono::Local>,
    chrono::DateTime<chrono::Local>,
    String,
    String,
)> {
    let parts: Vec<_> = period
        .split(['~', '～'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() < 2 {
        return None;
    }
    let start = parse_luna_report_datetime(parts[0])?;
    let end = parse_luna_report_datetime(parts[1])?;
    Some((start, end, parts[0].to_string(), parts[1].to_string()))
}

pub(super) fn report_period_unavailable_message(period: Option<&str>) -> Option<String> {
    let (start, end, raw_start, raw_end) = parse_luna_report_period(period?)?;
    let now = chrono::Local::now();
    if now < start {
        Some(format!(
            "提出開始前です。提出期間: {} ～ {}",
            raw_start, raw_end
        ))
    } else if now > end {
        Some(format!(
            "提出期間が終了しています。提出期間: {} ～ {}",
            raw_start, raw_end
        ))
    } else {
        None
    }
}

pub(super) fn extract_report_period_from_html(html: &str) -> Option<String> {
    static REPORT_PERIOD_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(\d{4}[/-]\d{1,2}[/-]\d{1,2}\s+\d{1,2}:\d{2})\s*[~～]\s*(\d{4}[/-]\d{1,2}[/-]\d{1,2}\s+\d{1,2}:\d{2})",
        )
        .expect("valid report period regex")
    });
    let text = normalize_html_text(html);
    let captures = REPORT_PERIOD_RE.captures(&text)?;
    let start = captures.get(1)?.as_str();
    let end = captures.get(2)?.as_str();
    Some(format!("{} ～ {}", start, end))
}

pub(super) fn report_submission_unavailable_message(
    html: &str,
    period: Option<&str>,
) -> Option<String> {
    if let Some(message) = report_period_unavailable_message(period).or_else(|| {
        extract_report_period_from_html(html)
            .and_then(|p| report_period_unavailable_message(Some(&p)))
    }) {
        return Some(message);
    }

    let text = normalize_html_text(html);
    if text.contains("提出期間外のため、提出できません")
        || text.contains("提出期間外のため、提出できません。")
        || text.contains("提出期間外")
            && (text.contains("提出できません") || text.contains("提出不可"))
    {
        return Some(REPORT_SUBMISSION_NOT_OPEN_MESSAGE.to_string());
    }
    None
}

pub(super) fn extract_report_token(
    html: &str,
    name: &str,
    period: Option<&str>,
) -> Result<String, String> {
    extract_input_value(html, name).ok_or_else(|| {
        report_submission_unavailable_message(html, period)
            .unwrap_or_else(|| format!("{} トークンが見つかりません", name))
    })
}

/// Extract a hidden input value from HTML by name
pub(super) fn extract_input_value(html: &str, name: &str) -> Option<String> {
    // Use scraper for reliable extraction
    let doc = scraper::Html::parse_document(html);
    let selector_str = format!("input[name=\"{}\"]", name);
    if let Ok(sel) = scraper::Selector::parse(&selector_str) {
        if let Some(el) = doc.select(&sel).next() {
            if let Some(val) = el.value().attr("value") {
                if !val.is_empty() {
                    return Some(val.to_string());
                }
            }
        }
    }
    // Fallback: regex-like search for name="xxx" ... value="yyy"
    let pattern = format!("name=\"{}\"", name);
    let pos = html.find(&pattern)?;
    let region_start = crate::client::floor_char_boundary(html, pos.saturating_sub(200));
    let region_end =
        crate::client::ceil_char_boundary(html, (pos + pattern.len() + 200).min(html.len()));
    let region = &html[region_start..region_end];
    let val_marker = "value=\"";
    let val_pos = region.find(val_marker)?;
    let rest = &region[val_pos + val_marker.len()..];
    let end = rest.find('"')?;
    let val = rest[..end].to_string();
    if !val.is_empty() {
        Some(val)
    } else {
        None
    }
}

/// Extract first matching form action + fields (hidden/text/textarea/select).
pub(super) fn extract_form_fields(
    html: &str,
    action_hint: &str,
) -> Option<(String, Vec<(String, String)>)> {
    sel!(SEL_INPUT_NAME, "input[name]");
    sel!(SEL_TEXTAREA_NAME, "textarea[name]");
    sel!(SEL_SELECT_NAME, "select[name]");
    sel!(SEL_OPT_SELECTED, "option[selected]");
    sel!(SEL_OPTION, "option");

    let doc = scraper::Html::parse_document(html);

    let mut fallback: Option<(String, Vec<(String, String)>)> = None;
    for form in doc.select(&SEL_FORM) {
        let action = form.value().attr("action").unwrap_or_default().to_string();
        let mut fields = Vec::new();

        for input in form.select(&SEL_INPUT_NAME) {
            let name = input.value().attr("name").unwrap_or_default();
            let typ = input
                .value()
                .attr("type")
                .unwrap_or("text")
                .to_ascii_lowercase();
            if (typ == "checkbox" || typ == "radio") && input.value().attr("checked").is_none() {
                continue;
            }
            let value = input.value().attr("value").unwrap_or_default();
            if !name.is_empty() {
                fields.push((name.to_string(), value.to_string()));
            }
        }

        for ta in form.select(&SEL_TEXTAREA_NAME) {
            let name = ta.value().attr("name").unwrap_or_default();
            if !name.is_empty() {
                fields.push((name.to_string(), ta.text().collect::<String>()));
            }
        }

        for se in form.select(&SEL_SELECT_NAME) {
            let name = se.value().attr("name").unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let value = se
                .select(&SEL_OPT_SELECTED)
                .next()
                .or_else(|| se.select(&SEL_OPTION).next())
                .and_then(|o| o.value().attr("value"))
                .unwrap_or_default()
                .to_string();
            fields.push((name.to_string(), value));
        }

        if action.is_empty() || fields.is_empty() {
            continue;
        }

        if fallback.is_none() {
            fallback = Some((action.clone(), fields.clone()));
        }
        if action_hint.is_empty() || action.contains(action_hint) {
            return Some((action, fields));
        }
    }

    fallback
}

pub(super) fn upsert_field(fields: &mut Vec<(String, String)>, key: &str, value: String) {
    for (k, v) in fields.iter_mut() {
        if k == key {
            *v = value;
            return;
        }
    }
    fields.push((key.to_string(), value));
}

pub(super) fn field_value<'a>(fields: &'a [(String, String)], key: &str) -> Option<&'a str> {
    fields
        .iter()
        .find_map(|(k, v)| if k == key { Some(v.as_str()) } else { None })
}
