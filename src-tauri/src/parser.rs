use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

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

// ============ Timetable (ARF010) ============

#[derive(Debug, Serialize, Clone)]
pub struct TimetableEntry {
    pub day: String,
    pub period: i32,
    pub course_name: String,
    pub room: String,
    pub course_code: String,
    pub is_cancelled: bool,
    pub is_makeup: bool,
    pub is_room_changed: bool,
    pub detail_path: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct TimetableData {
    pub student: StudentInfo,
    pub entries: Vec<TimetableEntry>,
    pub week_label: String,
    pub struts_token: String,
    pub form_fields: std::collections::HashMap<String, String>,
}

pub fn parse_timetable(html: &str) -> TimetableData {
    let doc = Html::parse_document(html);
    let student = parse_student_info(html);
    let mut entries = Vec::new();

    // Parse timetable from hidden inputs: lstStdTsCht_st[N]
    // hdnTmtxCd = day letter (A=Mon..F=Sat) + period (1-7)
    // lblSbjKnjNm = course name, lblClrNm = room
    let mut timetable_data: std::collections::HashMap<
        String,
        std::collections::HashMap<String, String>,
    > = Default::default();

    for el in doc.select(&SEL_HIDDEN_INPUT) {
        let name = el.value().attr("name").unwrap_or("");
        let value = el.value().attr("value").unwrap_or("").trim();
        // Match lstStdTsCht_st[N].fieldName
        if let Some(rest) = name.strip_prefix("lstStdTsCht_st[") {
            if let Some(bracket_pos) = rest.find(']') {
                let idx = &rest[..bracket_pos];
                if let Some(field) = rest[bracket_pos..].strip_prefix("].") {
                    timetable_data
                        .entry(idx.to_string())
                        .or_default()
                        .insert(field.to_string(), value.to_string());
                }
            }
        }
    }

    let day_map = [
        ('A', "月"),
        ('B', "火"),
        ('C', "水"),
        ('D', "木"),
        ('E', "金"),
        ('F', "土"),
    ];

    for fields in timetable_data.values() {
        // hdn* fields have the full (untruncated) values; lbl* may be truncated by the server
        let course_name = fields
            .get("hdnSbjKnjNm")
            .cloned()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| fields.get("lblSbjKnjNm").cloned().unwrap_or_default());
        let room = fields
            .get("hdnClrNm")
            .cloned()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| fields.get("lblClrNm").cloned().unwrap_or_default());
        let tmtx = fields.get("hdnTmtxCd").cloned().unwrap_or_default();

        if course_name.is_empty() || tmtx.is_empty() {
            continue;
        }

        // Status flags
        let is_cancelled = fields.get("hdnColFlg1").map(|v| v == "1").unwrap_or(false);
        let is_makeup = fields.get("hdnSplcFlg1").map(|v| v == "1").unwrap_or(false);
        // Room change: hdnClrNm (original) != lblClrNm (current) and hdnColNm1 is non-empty
        let is_room_changed = {
            let col_nm = fields.get("hdnColNm1").cloned().unwrap_or_default();
            !col_nm.is_empty()
        };

        // Build detail URL from fields
        let lsn_cd = fields.get("hdnLsnCd1").cloned().unwrap_or_default();
        let lsn_opc_fcy = fields.get("hdnLsnOpcFcy1").cloned().unwrap_or_default();
        let tac_trm_cd = fields.get("hdnTacTrmCd1").cloned().unwrap_or_default();
        let splc_apl_no = fields.get("hdnSplcAplNo1").cloned().unwrap_or_default();
        let tst_disp_flg = fields.get("hdnTstDispFlg1").cloned().unwrap_or_default();
        let seq_no = fields.get("hdnSeqNo1").cloned().unwrap_or_default();
        let lsc_gap_no = fields.get("hdnLscGapNo").cloned().unwrap_or_default();
        let arf010_flg = fields.get("hdnArf010Flg").cloned().unwrap_or_default();
        let opc_dt = fields.get("hdnOpcDt").cloned().unwrap_or_default();

        let detail_path = format!(
            "/uniasv2/ARF020PVI01Action.do?LSN_CD={}&LSN_OPC_FCY={}&TAC_TRM_CD={}&SPLC_APL_NO={}&TSTINFT_DISP_FLG={}&SEQ_NO={}&TMTX_CD={}&LSCGAP_NO={}&ARF010_FLG={}&OPC_DT={}",
            lsn_cd, lsn_opc_fcy, tac_trm_cd, splc_apl_no, tst_disp_flg, seq_no, tmtx, lsc_gap_no, arf010_flg, opc_dt
        );

        // Parse hdnTmtxCd: first char = day letter, rest = period number
        let day_char = tmtx.chars().next().unwrap_or(' ');
        let period_str = &tmtx[1..];
        let day = day_map
            .iter()
            .find(|(c, _)| *c == day_char)
            .map(|(_, d)| d.to_string())
            .unwrap_or_default();
        let period: i32 = period_str.parse().unwrap_or(0);

        if !day.is_empty() && (1..=7).contains(&period) {
            entries.push(TimetableEntry {
                day,
                period,
                course_name,
                room,
                course_code: lsn_cd,
                is_cancelled,
                is_makeup,
                is_room_changed,
                detail_path,
            });
        }
    }

    // Sort by day order then period
    let day_order = |d: &str| -> i32 {
        match d {
            "月" => 0,
            "火" => 1,
            "水" => 2,
            "木" => 3,
            "金" => 4,
            "土" => 5,
            _ => 6,
        }
    };
    entries.sort_by(|a, b| {
        day_order(&a.day)
            .cmp(&day_order(&b.day))
            .then(a.period.cmp(&b.period))
    });

    // Extract week label and Struts token for navigation
    let week_label = hidden_input(&doc, "lblSpcfProd");
    let struts_token = hidden_input(&doc, "org.apache.struts.taglib.html.TOKEN");

    // Collect ALL input fields from the form for resubmission
    // The Struts form requires all fields to be present when POSTing
    let all_input_sel = &*SEL_INPUT;
    let mut form_fields = std::collections::HashMap::new();
    for el in doc.select(all_input_sel) {
        let name = el.value().attr("name").unwrap_or("").trim();
        let value = el.value().attr("value").unwrap_or("").trim();
        if name.is_empty() {
            continue;
        }
        // Skip image submit buttons (EPrevious, ENext, EBack, EPageSet)
        let input_type = el.value().attr("type").unwrap_or("").to_lowercase();
        if input_type == "image" {
            continue;
        }
        // For duplicate names, keep the first one
        form_fields
            .entry(name.to_string())
            .or_insert_with(|| value.to_string());
    }
    // Also collect <select> values
    for select_el in doc.select(&SEL_SELECT) {
        let name = select_el.value().attr("name").unwrap_or("").trim();
        if !name.is_empty() {
            let value = select_el
                .select(&SEL_OPTION_SELECTED)
                .next()
                .and_then(|o| o.value().attr("value"))
                .unwrap_or("")
                .trim();
            form_fields
                .entry(name.to_string())
                .or_insert_with(|| value.to_string());
        }
    }

    TimetableData {
        student,
        entries,
        week_label,
        struts_token,
        form_fields,
    }
}

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

// ============ Cancellations (APB020) ============

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CancellationEntry {
    pub date: String,
    pub period: String,
    pub campus: String,
    pub department: String,
    pub course_code: String,
    pub year: String,
    pub course_name: String,
    pub instructor: String,
    pub room: String,
    pub comment: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CancellationsData {
    pub student: StudentInfo,
    pub entries: Vec<CancellationEntry>,
}

pub fn parse_cancellations(html: &str) -> CancellationsData {
    let doc = Html::parse_document(html);
    let student = parse_student_info(html);
    let mut entries = Vec::new();

    let mut headers: Vec<String> = Vec::new();

    for tr in doc.select(&SEL_TR) {
        let ths: Vec<String> = tr
            .select(&SEL_TH)
            .map(|el| el.text().collect::<String>().trim().to_string())
            .collect();

        // Detect header row
        if ths
            .iter()
            .any(|t| t.contains("休講日付") || t.contains("休講時限"))
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

        // The first column (項番) is a <th> in data rows, so tds starts after it
        // Map by header names (skipping 項番 header)
        let col = |name: &str| -> String {
            // Find header index, then get corresponding td
            // Headers include 項番 as first, but tds don't include it (it's a th)
            for (i, h) in headers.iter().enumerate() {
                if h.contains(name) {
                    // The th (項番) takes index 0, tds start from index 1 in headers
                    if i > 0 && i - 1 < tds.len() {
                        return tds[i - 1].clone();
                    }
                }
            }
            String::new()
        };

        let date = col("休講日付");
        let course_name = col("授業名称");
        if date.is_empty() && course_name.is_empty() {
            continue;
        }

        entries.push(CancellationEntry {
            date,
            period: col("休講時限"),
            campus: col("キャンパス"),
            department: col("授業管理部署"),
            course_code: col("授業コード"),
            year: col("開講年度"),
            course_name,
            instructor: col("教員"),
            room: col("教室"),
            comment: col("コメント"),
        });
    }

    CancellationsData { student, entries }
}

// ============ Makeup Classes (APC020) ============

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MakeupEntry {
    pub date: String,
    pub period: String,
    pub campus: String,
    pub department: String,
    pub course_code: String,
    pub year: String,
    pub course_name: String,
    pub instructor: String,
    pub room: String,
    pub comment: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MakeupData {
    pub student: StudentInfo,
    pub entries: Vec<MakeupEntry>,
}

pub fn parse_makeup_classes(html: &str) -> MakeupData {
    let doc = Html::parse_document(html);
    let student = parse_student_info(html);
    let mut entries = Vec::new();

    let mut headers: Vec<String> = Vec::new();

    for tr in doc.select(&SEL_TR) {
        let ths: Vec<String> = tr
            .select(&SEL_TH)
            .map(|el| el.text().collect::<String>().trim().to_string())
            .collect();

        if ths
            .iter()
            .any(|t| t.contains("補講日付") || t.contains("補講時限"))
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

        let col = |name: &str| -> String {
            for (i, h) in headers.iter().enumerate() {
                if h.contains(name) && i > 0 && i - 1 < tds.len() {
                    return tds[i - 1].clone();
                }
            }
            String::new()
        };

        let date = col("補講日付");
        let course_name = col("授業名称");
        if date.is_empty() && course_name.is_empty() {
            continue;
        }

        entries.push(MakeupEntry {
            date,
            period: col("時限"),
            campus: col("キャンパス"),
            department: col("授業管理部署"),
            course_code: col("授業コード"),
            year: col("開講年度"),
            course_name,
            instructor: col("教員"),
            room: col("教室"),
            comment: col("コメント"),
        });
    }

    MakeupData { student, entries }
}

// ============ Room Changes (APA960) ============

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RoomChangeEntry {
    pub date: String,
    pub department: String,
    pub course_code: String,
    pub year: String,
    pub course_name: String,
    pub room: String,
    pub instructor: String,
    pub schedule: String,
    pub comment: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RoomChangesData {
    pub student: StudentInfo,
    pub entries: Vec<RoomChangeEntry>,
}

pub fn parse_room_changes(html: &str) -> RoomChangesData {
    let doc = Html::parse_document(html);
    let student = parse_student_info(html);
    let mut entries = Vec::new();

    let mut headers: Vec<String> = Vec::new();

    for tr in doc.select(&SEL_TR) {
        let ths: Vec<String> = tr
            .select(&SEL_TH)
            .map(|el| el.text().collect::<String>().trim().to_string())
            .collect();

        if ths.iter().any(|t| t.contains("変更日付")) && ths.iter().any(|t| t.contains("授業名称"))
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

        let col = |name: &str| -> String {
            for (i, h) in headers.iter().enumerate() {
                if h.contains(name) && i > 0 && i - 1 < tds.len() {
                    return tds[i - 1].clone();
                }
            }
            String::new()
        };

        let date = col("変更日付");
        let course_name = col("授業名称");
        if date.is_empty() && course_name.is_empty() {
            continue;
        }

        entries.push(RoomChangeEntry {
            date,
            department: col("授業管理部署"),
            course_code: col("授業コード"),
            year: col("開講年度"),
            course_name,
            room: col("教室名称"),
            instructor: col("教員氏名"),
            schedule: col("曜時"),
            comment: col("コメント"),
        });
    }

    RoomChangesData { student, entries }
}

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

// ============ Exam Timetable (ARF010PVL01) ============

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExamEntry {
    pub day: String,
    pub period: i32,
    pub course_name: String,
    pub room: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExamTimetableData {
    pub student: StudentInfo,
    pub entries: Vec<ExamEntry>,
}

pub fn parse_exam_timetable(html: &str) -> ExamTimetableData {
    // Exam timetable has similar structure to regular timetable
    let timetable = parse_timetable(html);
    ExamTimetableData {
        student: timetable.student,
        entries: timetable
            .entries
            .into_iter()
            .map(|e| ExamEntry {
                day: e.day,
                period: e.period,
                course_name: e.course_name,
                room: e.room,
            })
            .collect(),
    }
}

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

        if ths
            .iter()
            .any(|t| t.contains("タイトル") || t.contains("お知らせ") || t.contains("掲示日"))
        {
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

fn strip_html_keep_text(html: &str) -> String {
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

// ============ Course Detail (ARF020) ============

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CourseDetail {
    pub fields: Vec<(String, String)>,
}

/// Parse course detail page: extract all th/td pairs from the first table that yields results.
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
                        .map(|td| td.text().collect::<String>().trim().to_string())
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

/// Locate the "履修基準年度 / Standard Year for Registration" field in a parsed
/// CourseDetail. Returns the index into `fields`, if any.
#[allow(dead_code)]
pub fn standard_year_field_index(fields: &[(String, String)]) -> Option<usize> {
    fields.iter().position(|(label, _)| {
        let l = label.to_lowercase();
        label.contains("履修基準年度")
            || label.contains("履修基準")
            || l.contains("standard year for registration")
            || l.contains("standard year")
    })
}

/// Convert full-width digits (０-９) to ASCII digits in-place. Other chars pass through.
/// KGC pages render numerals as full-width, so any digit-aware parsing must
/// normalize first or it sees zero digits.
#[allow(dead_code)]
pub fn normalize_fullwidth_digits(s: &str) -> String {
    s.chars()
        .map(|c| {
            if ('\u{FF10}'..='\u{FF19}').contains(&c) {
                char::from_u32(c as u32 - 0xFF10 + b'0' as u32).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

/// Parse the value of "履修基準年度" into an inclusive year range.
/// Returns `(min, max)` where `max = 99` means "no upper bound".
/// Returns `None` if no digits could be parsed.
#[allow(dead_code)]
pub fn parse_standard_year_range(value: &str) -> Option<(u32, u32)> {
    // Normalize full-width digits first — KGC writes "１年" not "1年".
    let v_owned = normalize_fullwidth_digits(value);
    let v = v_owned.trim();
    if v.is_empty() {
        return None;
    }
    if v.contains("全学年") || v.contains("不問") {
        return Some((1, 99));
    }
    let mut years: Vec<u32> = Vec::new();
    let mut digits = String::new();
    for ch in v.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else if !digits.is_empty() {
            if let Ok(n) = digits.parse::<u32>() {
                if (1..=10).contains(&n) {
                    years.push(n);
                }
            }
            digits.clear();
        }
    }
    if !digits.is_empty() {
        if let Ok(n) = digits.parse::<u32>() {
            if (1..=10).contains(&n) {
                years.push(n);
            }
        }
    }
    if years.is_empty() {
        return None;
    }
    let min = *years.iter().min().unwrap();
    let max = if v.contains("以上") {
        99
    } else {
        *years.iter().max().unwrap()
    };
    Some((min, max))
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
fn expand_session_range(range_str: &str, num_re: &regex::Regex) -> Vec<i32> {
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
fn detect_delivery_mode(text: &str) -> String {
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

#[cfg(test)]
mod standard_year_tests {
    use super::*;

    #[test]
    fn parses_simple_year() {
        assert_eq!(parse_standard_year_range("1年"), Some((1, 1)));
        assert_eq!(parse_standard_year_range("3年生"), Some((3, 3)));
    }

    #[test]
    fn parses_open_ended_above() {
        assert_eq!(parse_standard_year_range("2年以上"), Some((2, 99)));
        assert_eq!(parse_standard_year_range("1年生以上"), Some((1, 99)));
    }

    #[test]
    fn parses_range_and_list() {
        assert_eq!(parse_standard_year_range("1～4年"), Some((1, 4)));
        assert_eq!(parse_standard_year_range("1・2・3・4年"), Some((1, 4)));
        assert_eq!(parse_standard_year_range("2年〜3年"), Some((2, 3)));
    }

    #[test]
    fn parses_unrestricted() {
        assert_eq!(parse_standard_year_range("全学年"), Some((1, 99)));
    }

    #[test]
    fn rejects_no_digits() {
        assert_eq!(parse_standard_year_range(""), None);
        assert_eq!(parse_standard_year_range("—"), None);
    }

    #[test]
    fn parses_fullwidth_digits() {
        assert_eq!(parse_standard_year_range("１年"), Some((1, 1)));
        assert_eq!(parse_standard_year_range("２年以上"), Some((2, 99)));
        assert_eq!(parse_standard_year_range("１〜４年"), Some((1, 4)));
    }

    #[test]
    fn locates_field_by_label() {
        let fields = vec![
            ("授業科目".to_string(), "Demo".to_string()),
            (
                "履修基準年度 Standard Year for Registration".to_string(),
                "１年".to_string(),
            ),
        ];
        assert_eq!(standard_year_field_index(&fields), Some(1));
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
