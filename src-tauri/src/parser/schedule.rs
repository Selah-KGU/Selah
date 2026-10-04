//! Timetable, cancellations, makeup classes, room changes, and exams.

use super::*;

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
