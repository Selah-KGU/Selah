use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

use crate::parser::{self, SEL_HIDDEN_INPUT, SEL_TABLE_OUTPUT, SEL_TR};

// ============ Selectors ============

static SEL_DD_UL_LI: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("dd ul li").expect("valid selector"));
static SEL_IMG: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("img").expect("valid selector"));
static SEL_EREGISTER: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse(r#"input[name="ERegister"]"#).expect("valid selector"));
static SEL_TYPE_IMAGE: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse(r#"input[type="image"]"#).expect("valid selector"));

// ============ Types ============

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SyllabusSearchParams {
    pub year_from: String,
    pub year_to: String,
    pub term: String,
    pub campus: String,
    pub department: String,
    pub class_code: String,
    pub day_period: String,
    pub keyword: String,
    pub instructor: String,
    pub language: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SyllabusEntry {
    pub academic_year: String,
    pub department: String,
    pub class_code: String,
    pub course_title: String,
    pub instructor: String,
    pub term: String,
    pub day_period: String,
    pub campus: String,
    pub credits: String,
    pub bookmarked: bool,
    pub refer_index: String,
    pub register_index: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SyllabusSearchResult {
    pub entries: Vec<SyllabusEntry>,
    pub total_count: usize,
    pub current_page: usize,
    pub total_pages: usize,
}

// ============ Parsing ============

pub fn extract_validation_error(html: &str) -> Option<String> {
    let doc = Html::parse_document(html);
    let errors: Vec<String> = doc
        .select(&SEL_DD_UL_LI)
        .map(|el| el.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if errors.is_empty() {
        None
    } else {
        Some(errors.join("\n"))
    }
}

pub fn parse_search_results_public(html: &str) -> Result<SyllabusSearchResult, String> {
    parse_search_results(html)
}

fn parse_search_results(html: &str) -> Result<SyllabusSearchResult, String> {
    let doc = Html::parse_document(html);

    // Parse pagination info — hidden_input returns "" when not found
    let total_count = parser::hidden_input(&doc, "hdnCount")
        .parse::<usize>()
        .unwrap_or(0);
    let current_page = parser::hidden_input(&doc, "hdnCurrentPage")
        .parse::<usize>()
        .unwrap_or(1);
    let total_pages = parser::hidden_input(&doc, "hdnTotalPage")
        .parse::<usize>()
        .unwrap_or(1);

    // Parse result rows from table.output
    let mut entries = Vec::new();

    if let Some(table) = doc.select(&SEL_TABLE_OUTPUT).next() {
        for tr in table.select(&SEL_TR).skip(1) {
            // Skip header row
            if let Some(entry) = parse_result_row(&tr) {
                entries.push(entry);
            }
        }
    }

    Ok(SyllabusSearchResult {
        total_count: if total_count == 0 && !entries.is_empty() {
            entries.len()
        } else {
            total_count
        },
        entries,
        current_page,
        total_pages,
    })
}

fn parse_result_row(tr: &scraper::ElementRef) -> Option<SyllabusEntry> {
    // Extract values from hidden inputs which have clean data
    let mut fields = std::collections::HashMap::new();
    for input in tr.select(&SEL_HIDDEN_INPUT) {
        if let (Some(name), Some(value)) = (input.value().attr("name"), input.value().attr("value"))
        {
            // Strip the array prefix like "lstSlbinftJ016RList_st[0]."
            if let Some(field) = name.rsplit('.').next() {
                fields.insert(field.to_string(), value.to_string());
            }
        }
    }

    // Need at least a class code to consider it a valid entry
    let class_code = fields.get("lblLsnCd")?.clone();

    // Check bookmark state: Bookmark_1.gif means bookmarked
    let bookmarked = {
        let from_input = tr
            .select(&SEL_EREGISTER)
            .next()
            .and_then(|el| el.value().attr("src"))
            .map(|src| src.contains("Bookmark_1"))
            .unwrap_or(false);
        if !from_input {
            let from_img = tr.select(&SEL_IMG).any(|el| {
                el.value()
                    .attr("src")
                    .map(|s| s.contains("Bookmark_1"))
                    .unwrap_or(false)
            });
            let from_type_img = tr.select(&SEL_TYPE_IMAGE).any(|el| {
                el.value()
                    .attr("src")
                    .map(|s| s.contains("Bookmark_1"))
                    .unwrap_or(false)
            });
            from_img || from_type_img
        } else {
            true
        }
    };

    let refer_index = fields.get("ereferIndex").cloned().unwrap_or_default();
    let register_index = fields.get("eregisterIndex").cloned().unwrap_or_default();

    Some(SyllabusEntry {
        academic_year: fields.get("lblLsnOpcFcy").cloned().unwrap_or_default(),
        department: fields.get("lblLsnMngPostCd").cloned().unwrap_or_default(),
        class_code,
        course_title: fields.get("lblRepSbjKnjShtNm").cloned().unwrap_or_default(),
        instructor: fields.get("lblTchRnmKnjfn_01").cloned().unwrap_or_default(),
        term: fields.get("lblAc201ScrDispNm").cloned().unwrap_or_default(),
        day_period: fields.get("lblTmTx").cloned().unwrap_or_default(),
        campus: fields.get("lblCmps").cloned().unwrap_or_default(),
        credits: fields
            .get("lblTnisu")
            .or_else(|| fields.get("lblCrnum"))
            .cloned()
            .unwrap_or_default(),
        bookmarked,
        refer_index,
        register_index,
    })
}
