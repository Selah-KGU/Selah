use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

#[path = "parser/grades.rs"]
mod grades;
#[path = "parser/notifications.rs"]
mod notifications;
#[path = "parser/registration.rs"]
mod registration;
#[path = "parser/schedule.rs"]
mod schedule;
#[path = "parser/syllabus.rs"]
mod syllabus;

pub use grades::*;
pub use notifications::*;
pub use registration::*;
pub use schedule::*;
#[cfg(test)]
pub(in crate::parser) use syllabus::expand_session_range;
pub use syllabus::*;

// ============ Common Selectors ============

pub(crate) static SEL_TR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("tr").expect("valid selector"));
pub(crate) static SEL_TD: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("td").expect("valid selector"));
pub(crate) static SEL_TH: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("th").expect("valid selector"));
pub(crate) static SEL_HIDDEN_INPUT: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse(r#"input[type="hidden"]"#).expect("valid selector"));
pub(crate) static SEL_TABLE_OUTPUT: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("table.output").expect("valid selector"));
static SEL_TABLE_OUTPUT_SEISEKIT: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("table.output_seisekiT").expect("valid selector"));
static SEL_INPUT: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("input").expect("valid selector"));
static SEL_SELECT: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("select").expect("valid selector"));
static SEL_OPTION_SELECTED: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("option[selected]").expect("valid selector"));
static SESSION_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"第([\d,～~・-]+)回|Session\s+([\d,～~-]+)").unwrap());
static NUM_RE: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\d+").unwrap());

// ============ Shared ============

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct StudentInfo {
    pub student_id: String,
    pub name: String,
    pub name_en: String,
    pub student_type: String,
    pub affiliation_type: String,
    pub status: String,
    pub class: String,
    pub faculty: String,
    pub department: String,
    pub major: String,
    pub address: String,
}

/// Extract value of a hidden input by name attribute.
/// Shared across parser, syllabus, and other KGC page parsing.
pub(crate) fn hidden_input(doc: &Html, name: &str) -> String {
    for el in doc.select(&SEL_HIDDEN_INPUT) {
        if el.value().attr("name") == Some(name) {
            return el.value().attr("value").unwrap_or("").trim().to_string();
        }
    }
    String::new()
}

/// Parse student info primarily from hidden <input> fields (most reliable),
/// with fallback to table.output <th>/<td> pairs.
pub fn parse_student_info(html: &str) -> StudentInfo {
    let doc = Html::parse_document(html);
    let mut info = StudentInfo::default();

    // ---- Strategy 1: Hidden inputs (ARF010 has these) ----
    let sid = hidden_input(&doc, "lblScrgNo");
    if sid.is_empty() {
        // Also try hdnScrgNo
        info.student_id = hidden_input(&doc, "hdnScrgNo");
    } else {
        info.student_id = sid;
    }
    info.faculty = hidden_input(&doc, "lblFclNm");
    info.department = hidden_input(&doc, "lblDprNm");
    info.major = hidden_input(&doc, "lblSpcoNm");
    info.student_type = hidden_input(&doc, "lblStdDvNm");
    info.affiliation_type = hidden_input(&doc, "lblStdAldvNm");
    info.status = hidden_input(&doc, "lblCc001ScrDispNm");
    let addr_full = hidden_input(&doc, "lblCc008ScrDispNmStdAddrStdTelNo");
    if !addr_full.is_empty() {
        info.address = addr_full;
    }

    // Name is only in the table, not in hidden inputs
    // Parse from table.output: find <th> containing 学生氏名, take sibling <td>
    if let Some(table) = doc.select(&SEL_TABLE_OUTPUT).next() {
        for tr in table.select(&SEL_TR) {
            let ths: Vec<_> = tr.select(&SEL_TH).collect();
            let tds: Vec<_> = tr.select(&SEL_TD).collect();
            for (ti, th) in ths.iter().enumerate() {
                let label = th.text().collect::<String>();
                let label = label.trim();
                // Each <th> pairs with the <td> at the same index
                let td_text = tds
                    .get(ti)
                    .map(|td| td.text().collect::<String>().trim().to_string())
                    .unwrap_or_default();
                if td_text.is_empty() {
                    continue;
                }
                if label.contains("学生氏名") || label == "氏名" || label.contains("Student Name")
                {
                    if info.name.is_empty() {
                        parse_name_field(&td_text, &mut info);
                    }
                } else if label.contains("学生番号") && info.student_id.is_empty() {
                    info.student_id = td_text;
                } else if label.contains("学部")
                    && !label.contains("学科")
                    && info.faculty.is_empty()
                {
                    info.faculty = td_text;
                } else if label.contains("学科")
                    && !label.contains("学部")
                    && info.department.is_empty()
                {
                    info.department = td_text;
                } else if label.contains("学生区分") && info.student_type.is_empty() {
                    info.student_type = td_text;
                } else if label.contains("所属区分") && info.affiliation_type.is_empty() {
                    info.affiliation_type = td_text;
                } else if label.contains("学生状態") && info.status.is_empty() {
                    info.status = td_text;
                } else if (label == "クラス" || label.contains("クラス/")) && info.class.is_empty()
                {
                    info.class = td_text;
                } else if (label.contains("専攻") || label.contains("コース"))
                    && info.major.is_empty()
                {
                    info.major = td_text;
                } else if (label.contains("住所") || label.contains("電話番号"))
                    && info.address.is_empty()
                    && td_text.len() > 5
                {
                    info.address = td_text;
                }
            }
        }
    }

    log::debug!(
        "parse_student_info: id={}, name={}, faculty={}",
        info.student_id,
        info.name,
        info.faculty
    );
    info
}

/// Parse name field that may contain English name in parentheses
fn parse_name_field(v: &str, info: &mut StudentInfo) {
    let paren_pos = v.find('(').or_else(|| v.find('（'));
    if let Some(pos) = paren_pos {
        info.name = v[..pos].trim().to_string();
        let en = v[pos..]
            .trim_matches(|c: char| c == '(' || c == ')' || c == '（' || c == '）')
            .trim()
            .to_string();
        if !en.is_empty() {
            info.name_en = en;
        }
    } else {
        info.name = v.trim().to_string();
    }
}

#[cfg(test)]
mod session_plan_tests {
    use super::*;

    #[test]
    fn test_parse_from_dump_file() {
        let path = std::path::Path::new("/tmp/kwic_detail_fail_34001001.html");
        if !path.exists() {
            return; // dump file not available
        }
        let html = std::fs::read_to_string(path).unwrap();
        let plans = parse_session_plans(&html);
        assert_eq!(
            plans.len(),
            15,
            "Expected 15 session plans, got {}",
            plans.len()
        );
        assert_eq!(plans[0].session_num, 1);
        assert_eq!(plans[14].session_num, 15);
        assert!(!plans[0].topic.is_empty());
    }

    #[test]
    fn test_parse_notifications_captures_href() {
        let html = r#"
        <table>
          <tr><th>掲示日</th><th>分類</th><th>タイトル</th></tr>
          <tr>
            <td>2026-05-01</td>
            <td>事務</td>
            <td><a href="/uniasv2/CPA020Action.do?id=42">休講のお知らせ</a></td>
          </tr>
          <tr>
            <td>2026-05-02</td>
            <td>授業</td>
            <td>リンクのない掲示</td>
          </tr>
        </table>"#;
        let data = parse_notifications(html);
        assert_eq!(data.entries.len(), 2);
        assert_eq!(data.entries[0].title, "休講のお知らせ");
        assert_eq!(data.entries[0].url, "/uniasv2/CPA020Action.do?id=42");
        assert_eq!(data.entries[1].title, "リンクのない掲示");
        assert!(data.entries[1].url.is_empty());
        assert!(notifications_list_present(html));
        assert!(!notifications_list_present(
            "<html><body>login</body></html>"
        ));
    }

    #[test]
    fn test_parse_notification_detail_extracts_body_and_attachment() {
        let html = r#"
        <table>
          <tr><th>タイトル</th><td>休講のお知らせ</td></tr>
          <tr><th>掲示日</th><td>2026-05-01</td></tr>
          <tr><th>発信元</th><td>教務課</td></tr>
          <tr><th>本文</th><td>5月10日(月)1限の経済学は休講です。<br>補講は別途連絡します。</td></tr>
          <tr><th>添付</th><td><a href="/uniasv2/dl?f=notice.pdf">notice.pdf</a></td></tr>
        </table>"#;
        let detail = parse_notification_detail(html);
        assert_eq!(detail.title, "休講のお知らせ");
        assert_eq!(detail.date, "2026-05-01");
        assert_eq!(detail.sender, "教務課");
        assert!(detail.body.contains("5月10日"));
        assert!(detail.body.contains("補講"));
        assert_eq!(detail.attachments.len(), 1);
        assert_eq!(detail.attachments[0].name, "notice.pdf");
    }

    #[test]
    fn test_parse_course_detail_preserves_detail_link_html() {
        let html = r#"
                <table class="output">
                    <tr>
                        <th>関連資料</th>
                        <td><a href="/uniasv2/ARF020PVI01Action.do?LSN_CD=28550600">授業詳細を見る</a></td>
                    </tr>
                </table>
                "#;
        let detail = parse_course_detail(html);
        assert_eq!(detail.fields.len(), 1);
        assert_eq!(detail.fields[0].0, "関連資料");
        assert!(detail.fields[0]
            .1
            .contains("href=\"/uniasv2/ARF020PVI01Action.do?LSN_CD=28550600\""));
        assert!(detail.fields[0].1.contains("授業詳細を見る"));
    }

    #[test]
    fn test_expand_fullwidth_digits() {
        let num_re = regex::Regex::new(r"\d+").unwrap();
        assert_eq!(expand_session_range("１", &num_re), vec![1]);
        assert_eq!(expand_session_range("１５", &num_re), vec![15]);
        assert_eq!(
            expand_session_range("１～１５", &num_re),
            (1..=15).collect::<Vec<_>>()
        );
        assert_eq!(expand_session_range("3", &num_re), vec![3]);
        assert_eq!(expand_session_range("1-3", &num_re), vec![1, 2, 3]);
    }
}
