//! Course registration page parsing.

use super::*;

// ============ Course Registration (ARD010) ============

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CreditSummary {
    pub semester: String,
    pub enrolled: String,
    pub limit: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LanguageOption {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct RegisteredCourse {
    pub period: String,
    pub day: String,
    pub semester: String,
    pub course_name: String,
    pub course_code: String,
    pub instructor: String,
    pub campus: String,
    pub credits: String,
    pub room: String,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct RegistrationData {
    pub student: StudentInfo,
    pub credit_summary: Vec<CreditSummary>,
    pub courses: Vec<RegisteredCourse>,
    pub year_semester: String,
    pub last_applied: String,
    pub language_options: Vec<LanguageOption>,
}

pub fn parse_registration(html: &str) -> RegistrationData {
    let doc = Html::parse_document(html);
    let student = parse_student_info(html);

    // Year / semester label
    let year = hidden_input(&doc, "hdnTcapFcy");
    let term = hidden_input(&doc, "hdnTcapDtm");
    let term_label = match term.as_str() {
        "1" => "春学期",
        "2" => "秋学期",
        _ => "",
    };
    let year_semester = if !year.is_empty() && !term_label.is_empty() {
        format!("{}年度 {}", year, term_label)
    } else {
        String::new()
    };

    // Last applied datetime
    let full_text = doc.root_element().text().collect::<String>();
    let marker = "前回申請日時：";
    let last_applied = if let Some(pos) = full_text.find(marker) {
        let after = &full_text[pos + marker.len()..];
        let trimmed = after.trim();
        // Take date + time (e.g. "2026/04/11 09:13:48")
        let mut parts = trimmed.splitn(3, char::is_whitespace);
        let date_part = parts.next().unwrap_or("");
        let time_part = parts.next().unwrap_or("");
        if date_part.contains('/') {
            format!("{} {}", date_part, time_part).trim().to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    // Language options from hidden inputs
    let mut language_options = Vec::new();
    let mut opt_names: std::collections::BTreeMap<usize, String> =
        std::collections::BTreeMap::new();
    let mut opt_values: std::collections::BTreeMap<usize, String> =
        std::collections::BTreeMap::new();
    for el in doc.select(&SEL_HIDDEN_INPUT) {
        let name = el.value().attr("name").unwrap_or("");
        let value = el.value().attr("value").unwrap_or("").trim().to_string();
        if name.contains("lblTacOptAstNm") {
            if let Some(idx) = name
                .split('[')
                .nth(1)
                .and_then(|s| s.split(']').next())
                .and_then(|s| s.parse::<usize>().ok())
            {
                opt_names.insert(idx, value);
            }
        } else if name.contains("lblTacOptValNm") {
            if let Some(idx) = name
                .split('[')
                .nth(1)
                .and_then(|s| s.split(']').next())
                .and_then(|s| s.parse::<usize>().ok())
            {
                opt_values.insert(idx, value);
            }
        }
    }
    for (idx, oname) in &opt_names {
        if let Some(oval) = opt_values.get(idx) {
            if !oname.is_empty() && !oval.is_empty() {
                language_options.push(LanguageOption {
                    name: oname.clone(),
                    value: oval.clone(),
                });
            }
        }
    }

    // Credit summary from hidden inputs
    let credit_summary = vec![
        CreditSummary {
            semester: "春学期".into(),
            enrolled: hidden_input(&doc, "lblFtsmTacInsmCrnum"),
            limit: hidden_input(&doc, "lblFtsmTacUlCrnum"),
        },
        CreditSummary {
            semester: "秋学期".into(),
            enrolled: hidden_input(&doc, "lblScsmTacInsmCrnum"),
            limit: hidden_input(&doc, "lblScsmTacUlCrnum"),
        },
        CreditSummary {
            semester: "年間".into(),
            enrolled: hidden_input(&doc, "lblYptcInsmCrnum"),
            limit: hidden_input(&doc, "lblYptcUlCrnum"),
        },
    ];

    // Parse courses from curriculum grid (table.output_curriculum)
    let mut courses = Vec::new();
    let table_sel = Selector::parse("table.output_curriculum").expect("valid selector");

    let caption_sel = Selector::parse("caption").expect("valid selector");

    let days = ["月", "火", "水", "木", "金", "土"];

    for table in doc.select(&table_sel) {
        // Skip icon legend table (first output_curriculum)
        if table.select(&caption_sel).next().is_none() {
            continue;
        }

        let mut current_period = String::new();

        for tr in table.select(&SEL_TR) {
            let ths: Vec<_> = tr.select(&SEL_TH).collect();
            let tds: Vec<_> = tr.select(&SEL_TD).collect();

            // Update period from th with "N時限"
            for th in &ths {
                let text = th.text().collect::<String>().trim().to_string();
                if text.contains("時限") {
                    current_period = text.clone();
                }
            }

            // Skip rows that are add-button rows (they have icon_plus images)
            let row_html = tr.html();
            if row_html.contains("icon_plus_on") || row_html.contains("icon_plus_off") {
                // This is an add-button row, check if it also has data cells we need
                if tds.is_empty() || !row_html.contains("icon_detail_") {
                    continue;
                }
            }

            // Process data cells (td.segment)
            if tds.is_empty() || current_period.is_empty() {
                continue;
            }

            for (i, td) in tds.iter().enumerate() {
                let cell_html = td.html();
                // Only process cells with actual course icons
                if !cell_html.contains("icon_detail_application")
                    && !cell_html.contains("icon_detail_curriculum")
                    && !cell_html.contains("icon_sentakutyu")
                    && !cell_html.contains("icon_detail_over")
                {
                    continue;
                }

                // Extract text lines from cell
                let full_text = td.text().collect::<String>();
                let lines: Vec<&str> = full_text
                    .split('\n')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect();

                if lines.is_empty() {
                    continue;
                }

                // Determine status from icon
                let status = if cell_html.contains("icon_detail_over") {
                    "履修済".to_string()
                } else if cell_html.contains("icon_detail_curriculum") {
                    "履修".to_string()
                } else if cell_html.contains("icon_sentakutyu") {
                    "選択中".to_string()
                } else {
                    "申請".to_string()
                };

                // Parse fields from text lines
                let mut semester = String::new();
                let mut course_name = String::new();
                let mut instructor = String::new();
                let mut credits = String::new();
                let mut campus = String::new();
                let mut room = String::new();

                for line in &lines {
                    if line.contains("学期") || *line == "通年" {
                        semester = line.to_string();
                    } else if line.contains("単位") {
                        credits = line
                            .trim_start_matches('(')
                            .trim_start_matches('（')
                            .trim_end_matches(')')
                            .trim_end_matches('）')
                            .to_string();
                    } else if line.contains("キャンパス") {
                        campus = line.to_string();
                    } else if course_name.is_empty() && !line.contains("科目の") {
                        course_name = line.to_string();
                    } else if instructor.is_empty() && !line.contains("科目の") {
                        instructor = line.to_string();
                    } else if campus.is_empty() && !line.contains("科目の") {
                        campus = line.to_string();
                    }
                }

                // Try to get room from hidden input
                let room_inputs: Vec<_> = td
                    .select(&SEL_HIDDEN_INPUT)
                    .filter(|el| {
                        el.value().attr("name").unwrap_or("").contains("lblClrNm")
                            && !el.value().attr("name").unwrap_or("").contains("lblClrNm2")
                    })
                    .collect();
                if let Some(el) = room_inputs.first() {
                    room = el.value().attr("value").unwrap_or("").trim().to_string();
                }
                // Fallback: last text line if not yet assigned
                if room.is_empty() {
                    if let Some(last) = lines.last() {
                        if !last.contains("単位")
                            && !last.contains("キャンパス")
                            && !last.contains("学期")
                            && !last.contains("科目の")
                        {
                            room = last.to_string();
                        }
                    }
                }

                // Get full subject name from hidden input if truncated
                let full_name_inputs: Vec<_> = td
                    .select(&SEL_HIDDEN_INPUT)
                    .filter(|el| {
                        el.value()
                            .attr("name")
                            .unwrap_or("")
                            .contains("lblSbjNmTmtx2")
                    })
                    .collect();
                if let Some(el) = full_name_inputs.first() {
                    let full = el.value().attr("value").unwrap_or("").trim().to_string();
                    if !full.is_empty() {
                        course_name = full;
                    }
                }

                let day = days.get(i % days.len()).unwrap_or(&"").to_string();

                // Extract course code from ARF020 link (LSN_CD=XXXXX)
                let course_code = cell_html
                    .split("LSN_CD=")
                    .nth(1)
                    .and_then(|s| s.split('&').next())
                    .unwrap_or("")
                    .to_string();

                if !course_name.is_empty() {
                    courses.push(RegisteredCourse {
                        period: current_period.clone(),
                        day,
                        semester,
                        course_name,
                        course_code,
                        instructor,
                        campus,
                        credits,
                        room,
                        status,
                    });
                }
            }
        }
    }

    RegistrationData {
        student,
        credit_summary,
        courses,
        year_semester,
        last_applied,
        language_options,
    }
}
