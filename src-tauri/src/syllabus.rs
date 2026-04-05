use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Serialize)]
pub struct SyllabusSearchResult {
    pub entries: Vec<SyllabusEntry>,
    pub total_count: usize,
    pub current_page: usize,
    pub total_pages: usize,
}

// ============ Parsing ============

pub fn extract_validation_error(html: &str) -> Option<String> {
    let doc = Html::parse_document(html);
    // Error messages appear in <dd><ul><li> elements
    let sel = Selector::parse("dd ul li").ok()?;
    let errors: Vec<String> = doc.select(&sel)
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

    // Parse pagination info
    let total_count = extract_hidden_value(&doc, "hdnCount")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    let current_page = extract_hidden_value(&doc, "hdnCurrentPage")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1);
    let total_pages = extract_hidden_value(&doc, "hdnTotalPage")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1);

    // Parse result rows from table.output
    let table_sel = Selector::parse("table.output").unwrap();
    let tr_sel = Selector::parse("tr").unwrap();

    let mut entries = Vec::new();

    if let Some(table) = doc.select(&table_sel).next() {
        for tr in table.select(&tr_sel).skip(1) {
            // Skip header row
            if let Some(entry) = parse_result_row(&tr) {
                entries.push(entry);
            }
        }
    }

    Ok(SyllabusSearchResult {
        total_count: if total_count == 0 && !entries.is_empty() { entries.len() } else { total_count },
        entries,
        current_page,
        total_pages,
    })
}

fn extract_hidden_value(doc: &Html, name: &str) -> Option<String> {
    let selector_str = format!(r#"input[name="{}"]"#, name);
    let sel = Selector::parse(&selector_str).ok()?;
    doc.select(&sel).next()?.value().attr("value").map(|s| s.to_string())
}

fn parse_result_row(tr: &scraper::ElementRef) -> Option<SyllabusEntry> {
    let hidden_sel = Selector::parse("input[type=hidden]").unwrap();

    // Extract values from hidden inputs which have clean data
    let mut fields = std::collections::HashMap::new();
    for input in tr.select(&hidden_sel) {
        if let (Some(name), Some(value)) = (input.value().attr("name"), input.value().attr("value")) {
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
        // Try input[name="ERegister"] (type=image with src)
        let img_sel = Selector::parse(r#"input[name="ERegister"]"#).unwrap();
        let from_input = tr.select(&img_sel).next()
            .and_then(|el| el.value().attr("src"))
            .map(|src| src.contains("Bookmark_1"))
            .unwrap_or(false);
        if !from_input {
            // Also try img tags inside any element that may contain bookmark icon
            let any_img_sel = Selector::parse("img").unwrap();
            let from_img = tr.select(&any_img_sel)
                .any(|el| el.value().attr("src").map(|s| s.contains("Bookmark_1")).unwrap_or(false));
            // Also check input[type=image]
            let type_img_sel = Selector::parse(r#"input[type="image"]"#).unwrap();
            let from_type_img = tr.select(&type_img_sel)
                .any(|el| el.value().attr("src").map(|s| s.contains("Bookmark_1")).unwrap_or(false));
            // Debug: log what we found
            let all_inputs: Vec<String> = tr.select(&Selector::parse("input[type=image]").unwrap())
                .map(|el| format!("name={:?} src={:?}", el.value().attr("name"), el.value().attr("src")))
                .collect();
            if !all_inputs.is_empty() {
                log::info!("Bookmark detection: input[name=ERegister]={}, img={}, input[type=image]={}, inputs={:?}", from_input, from_img, from_type_img, all_inputs);
            }
            from_img || from_type_img
        } else {
            true
        }
    };

    let refer_index = fields.get("ereferIndex").cloned().unwrap_or_default();
    let register_index = fields.get("eregisterIndex").cloned().unwrap_or_default();

    // Debug: log all hidden field names from first row to discover available fields
    if fields.len() > 5 {
        let field_names: Vec<&String> = fields.keys().collect();
        log::debug!("Syllabus row hidden fields: {:?}", field_names);
    }

    Some(SyllabusEntry {
        academic_year: fields.get("lblLsnOpcFcy").cloned().unwrap_or_default(),
        department: fields.get("lblLsnMngPostCd").cloned().unwrap_or_default(),
        class_code,
        course_title: fields.get("lblRepSbjKnjShtNm").cloned().unwrap_or_default(),
        instructor: fields.get("lblTchRnmKnjfn_01").cloned().unwrap_or_default(),
        term: fields.get("lblAc201ScrDispNm").cloned().unwrap_or_default(),
        day_period: fields.get("lblTmTx").cloned().unwrap_or_default(),
        campus: fields.get("lblCmps").cloned().unwrap_or_default(),
        credits: fields.get("lblTnisu").or_else(|| fields.get("lblCrnum")).cloned().unwrap_or_default(),
        bookmarked,
        refer_index,
        register_index,
    })
}
