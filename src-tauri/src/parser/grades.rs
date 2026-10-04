//! Curriculum rows and grade tables.

use super::*;

// ============ Grades / Curriculum (ARF140) ============

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CurriculumRow {
    pub category: String,
    pub level: i32,
    pub required_credits: String,
    pub enrolled_acquired_credits: String,
    pub enrolled_credits: String,
    pub earned_credits: String,
    pub is_deficit: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GradesData {
    pub student: StudentInfo,
    pub curriculum: Vec<CurriculumRow>,
}

pub fn parse_grades(html: &str) -> GradesData {
    let doc = Html::parse_document(html);
    let student = parse_student_info(html);
    let mut curriculum = Vec::new();

    // Strategy: extract data from hidden inputs in output_seisekiT tables
    // Each row has: hdnLv (level), lblRqgpNm (name), lblRqgpLlCrnum (required),
    // lblRqgpAcqTacCrnum (enrolled+acquired), lblRqgpTacCrnum (enrolled), lblRqgpAcqCrnum (acquired)
    // Collect all hidden inputs and group by index
    let mut rows_map: std::collections::BTreeMap<usize, std::collections::HashMap<String, String>> =
        std::collections::BTreeMap::new();

    for input in doc.select(&SEL_HIDDEN_INPUT) {
        let name = input.value().attr("name").unwrap_or("");
        let value = input.value().attr("value").unwrap_or("").trim().to_string();

        // Pattern: lstAchInfPelDispData_st[N].fieldName
        if !name.starts_with("lstAchInfPelDispData_st[") {
            continue;
        }
        let rest = &name["lstAchInfPelDispData_st[".len()..];
        let bracket_end = match rest.find(']') {
            Some(i) => i,
            None => continue,
        };
        let idx: usize = match rest[..bracket_end].parse() {
            Ok(n) => n,
            Err(_) => continue,
        };
        let field = &rest[bracket_end + 1..];
        let field = field.strip_prefix('.').unwrap_or(field);

        rows_map
            .entry(idx)
            .or_default()
            .insert(field.to_string(), value);
    }

    // Also check for deficit: red background on td cells
    // We look at each output_seisekiT table's tr for style containing #FF0000
    let mut deficit_indices: std::collections::HashSet<usize> = std::collections::HashSet::new();

    for (table_idx, table) in doc.select(&SEL_TABLE_OUTPUT_SEISEKIT).enumerate() {
        for tr in table.select(&SEL_TR) {
            for td in tr.select(&SEL_TD) {
                if let Some(style) = td.value().attr("style") {
                    if style.contains("#FF0000") {
                        deficit_indices.insert(table_idx);
                        break;
                    }
                }
            }
        }
    }

    for (idx, fields) in &rows_map {
        let name = fields.get("lblRqgpNm").cloned().unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let level: i32 = fields
            .get("hdnLv")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        let required = fields.get("lblRqgpLlCrnum").cloned().unwrap_or_default();
        let enrolled_acquired = fields
            .get("lblRqgpAcqTacCrnum")
            .cloned()
            .unwrap_or_default();
        let enrolled = fields.get("lblRqgpTacCrnum").cloned().unwrap_or_default();
        let acquired = fields.get("lblRqgpAcqCrnum").cloned().unwrap_or_default();

        // Check if this row has deficit (required > 0 and acquired < required)
        let is_deficit = deficit_indices.contains(idx) || {
            let req: f64 = required.parse().unwrap_or(0.0);
            let acq: f64 = acquired.parse().unwrap_or(0.0);
            req > 0.0 && acq < req
        };

        curriculum.push(CurriculumRow {
            category: name,
            level,
            required_credits: required,
            enrolled_acquired_credits: enrolled_acquired,
            enrolled_credits: enrolled,
            earned_credits: acquired,
            is_deficit,
        });
    }

    // Fallback: if no hidden inputs found, try the old table approach
    if curriculum.is_empty() {
        let mut headers: Vec<String> = Vec::new();
        for tr in doc.select(&SEL_TR) {
            let ths: Vec<String> = tr
                .select(&SEL_TH)
                .map(|el| el.text().collect::<String>().trim().to_string())
                .collect();
            if ths
                .iter()
                .any(|t| t.contains("必要単位") || t.contains("修得"))
            {
                headers = ths;
                continue;
            }
            if headers.is_empty() {
                continue;
            }
            let tds: Vec<String> = tr
                .select(&SEL_TD)
                .map(|el| el.text().collect::<String>().trim().to_string())
                .collect();
            if tds.is_empty() {
                continue;
            }
            let row_ths: Vec<String> = tr
                .select(&SEL_TH)
                .map(|el| el.text().collect::<String>().trim().to_string())
                .collect();
            let category = if !row_ths.is_empty() {
                row_ths[0].clone()
            } else if !tds.is_empty() {
                tds[0].clone()
            } else {
                continue;
            };
            if category.is_empty() {
                continue;
            }
            let col = |name: &str| -> String {
                for (i, h) in headers.iter().enumerate() {
                    if h.contains(name) {
                        let td_offset = if !row_ths.is_empty() {
                            i.saturating_sub(1)
                        } else {
                            i
                        };
                        if td_offset < tds.len() {
                            return tds[td_offset].clone();
                        }
                    }
                }
                String::new()
            };
            curriculum.push(CurriculumRow {
                category,
                level: 1,
                required_credits: col("必要単位"),
                enrolled_acquired_credits: String::new(),
                enrolled_credits: col("履修"),
                earned_credits: col("修得"),
                is_deficit: false,
            });
        }
    }

    GradesData {
        student,
        curriculum,
    }
}
