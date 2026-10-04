use super::super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaTimetable {
    pub year: String,
    pub term: String,
    pub year_label: String,
    pub term_label: String,
    pub year_options: Vec<SelectOption>,
    pub term_options: Vec<SelectOption>,
    pub courses: Vec<LunaCourse>,
    pub communities: Vec<LunaCommunity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaCourse {
    pub idnumber: String,
    pub name: String,
    pub teacher: String,
    pub period: u32, // 1-7
    pub day: u32,    // 1=月 ... 6=土
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LunaCommunity {
    pub idnumber: String,
    pub name: String,
}

pub fn parse_luna_timetable(html: &str) -> LunaTimetable {
    let doc = Html::parse_document(html);

    let (year, year_label) = extract_selected_value(&doc, "#nendo");
    let (term, term_label) = extract_selected_value(&doc, "#kikanCd");
    let year_options = extract_select_options(&doc, "#nendo");
    let term_options = extract_select_options(&doc, "#kikanCd");

    let mut courses = Vec::new();

    for row in doc.select(&SEL_DATA_ROW) {
        // Extract period number from text like "１時限"
        let period_text = row
            .select(&SEL_PERIOD_COL)
            .next()
            .map(|e| e.text().collect::<String>())
            .unwrap_or_default();
        let period = parse_japanese_number(&period_text);
        if period == 0 {
            continue;
        }

        // Each cell corresponds to a day (1=月 through 6=土)
        for (i, cell) in row.select(&SEL_TABLE_CELL).enumerate() {
            let day = (i + 1) as u32;
            if let Some(btn) = cell.select(&SEL_COURSE_BTN).next() {
                let idnumber = btn.value().attr("id").unwrap_or_default().to_string();
                let name = btn.text().collect::<String>().trim().to_string();
                let teacher = cell
                    .select(&SEL_CELL_DETAIL)
                    .map(|s| s.text().collect::<String>().trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ");

                courses.push(LunaCourse {
                    idnumber,
                    name,
                    teacher,
                    period,
                    day,
                });
            }
        }
    }

    // Parse communities
    let mut communities = Vec::new();
    for el in doc.select(&SEL_COMMUNITY_BTN) {
        let idnumber = el.value().attr("id").unwrap_or_default().to_string();
        let name = el.text().collect::<String>().trim().to_string();
        communities.push(LunaCommunity { idnumber, name });
    }

    LunaTimetable {
        year,
        term,
        year_label,
        term_label,
        year_options,
        term_options,
        courses,
        communities,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LunaTimetableSwitch {
    pub action: String,
    pub method: String,
    pub fields: Vec<(String, String)>,
}

/// Build a form request that changes the timetable page to the target year/term.
/// Returns None when the page has no usable form or the target is not an option.
pub fn build_timetable_switch(html: &str, year: &str, term: &str) -> Option<LunaTimetableSwitch> {
    if year.is_empty() || term.is_empty() {
        return None;
    }
    let doc = Html::parse_document(html);
    let form_sel = scraper::Selector::parse("form").ok()?;
    let nendo_sel = scraper::Selector::parse("select#nendo, select[name='nendo']").ok()?;
    let term_sel = scraper::Selector::parse("select#kikanCd, select[name='kikanCd']").ok()?;

    if let Some(form) = doc.select(&form_sel).find(|form| {
        form.select(&nendo_sel).next().is_some() && form.select(&term_sel).next().is_some()
    }) {
        let nendo = form.select(&nendo_sel).next()?;
        let term_el = form.select(&term_sel).next()?;
        return switch_from_selects(form, nendo, term_el, year, term);
    }

    let nendo = doc.select(&nendo_sel).next()?;
    let term_el = doc.select(&term_sel).next()?;
    switch_from_selects(doc.root_element(), nendo, term_el, year, term)
}

fn switch_from_selects(
    scope: scraper::ElementRef<'_>,
    nendo: scraper::ElementRef<'_>,
    term_el: scraper::ElementRef<'_>,
    year: &str,
    term: &str,
) -> Option<LunaTimetableSwitch> {
    if !select_has_value(nendo, year) || !select_has_value(term_el, term) {
        return None;
    }
    let nendo_name = nendo.value().attr("name").unwrap_or("nendo").to_string();
    let term_name = term_el
        .value()
        .attr("name")
        .unwrap_or("kikanCd")
        .to_string();
    let mut fields = Vec::new();
    let input_sel = scraper::Selector::parse("input[name]").ok()?;
    for input in scope.select(&input_sel) {
        let name = input.value().attr("name").unwrap_or_default();
        if name.is_empty() || name == nendo_name || name == term_name {
            continue;
        }
        let typ = input
            .value()
            .attr("type")
            .unwrap_or("text")
            .to_ascii_lowercase();
        if matches!(typ.as_str(), "submit" | "button" | "image" | "file") {
            continue;
        }
        if matches!(typ.as_str(), "checkbox" | "radio") && input.value().attr("checked").is_none() {
            continue;
        }
        fields.push((
            name.to_string(),
            input.value().attr("value").unwrap_or_default().to_string(),
        ));
    }

    let select_sel = scraper::Selector::parse("select[name]").ok()?;
    for select in scope.select(&select_sel) {
        let name = select.value().attr("name").unwrap_or_default();
        if name.is_empty() || name == nendo_name || name == term_name {
            continue;
        }
        if let Some(value) = selected_option_value(select) {
            fields.push((name.to_string(), value));
        }
    }
    fields.push((nendo_name, year.to_string()));
    fields.push((term_name, term.to_string()));

    let action = scope
        .value()
        .attr("action")
        .unwrap_or("/lms/timetable")
        .trim()
        .to_string();
    let action = if action.is_empty() || action.starts_with("javascript:") || action == "#" {
        "/lms/timetable".to_string()
    } else {
        action
    };
    let method = scope
        .value()
        .attr("method")
        .unwrap_or("post")
        .trim()
        .to_ascii_lowercase();
    let method = if method.is_empty() {
        "post".to_string()
    } else {
        method
    };
    Some(LunaTimetableSwitch {
        action,
        method,
        fields,
    })
}

fn select_has_value(select: scraper::ElementRef<'_>, value: &str) -> bool {
    let Ok(option_sel) = scraper::Selector::parse("option") else {
        return false;
    };
    select
        .select(&option_sel)
        .any(|opt| opt.value().attr("value") == Some(value))
}

fn selected_option_value(select: scraper::ElementRef<'_>) -> Option<String> {
    let selected_sel = scraper::Selector::parse("option[selected]").ok()?;
    let option_sel = scraper::Selector::parse("option").ok()?;
    select
        .select(&selected_sel)
        .next()
        .or_else(|| select.select(&option_sel).next())
        .and_then(|opt| opt.value().attr("value"))
        .map(|value| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_selected_term_and_keeps_csrf() {
        let html = r#"
            <form id="timetableForm" action="/lms/timetable" method="post">
              <input type="hidden" name="_csrf" value="csrf-token">
              <select id="nendo" name="nendo">
                <option value="2025">2025</option>
                <option value="2026" selected>2026</option>
              </select>
              <select id="kikanCd" name="kikanCd">
                <option value="02" selected>春学期</option>
                <option value="03">秋学期</option>
              </select>
            </form>
        "#;
        let request = build_timetable_switch(html, "2026", "03").unwrap();
        assert_eq!(request.action, "/lms/timetable");
        assert_eq!(request.method, "post");
        assert!(request
            .fields
            .iter()
            .any(|(k, v)| k == "_csrf" && v == "csrf-token"));
        assert!(request
            .fields
            .iter()
            .any(|(k, v)| k == "nendo" && v == "2026"));
        assert!(request
            .fields
            .iter()
            .any(|(k, v)| k == "kikanCd" && v == "03"));
    }

    #[test]
    fn refuses_a_term_that_is_not_an_option() {
        let html = r#"
            <form action="/lms/timetable" method="get">
              <select id="nendo" name="nendo"><option value="2026" selected>2026</option></select>
              <select id="kikanCd" name="kikanCd"><option value="02" selected>春学期</option></select>
            </form>
        "#;
        assert!(build_timetable_switch(html, "2026", "03").is_none());
    }
}
