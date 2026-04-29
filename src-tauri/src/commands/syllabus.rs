use super::{kgc_get, kgc_http, kgc_post};
use crate::KgcState;
use regex::Regex;
use std::sync::LazyLock;
use tauri::{Emitter, Manager, State};

#[tauri::command]
pub async fn search_syllabus(
    params: crate::syllabus::SyllabusSearchParams,
    kgc_state: State<'_, KgcState>,
) -> Result<crate::syllabus::SyllabusSearchResult, String> {
    let _kgc_gate = kgc_state.gate.lock().await;
    let http = kgc_http(kgc_state.inner()).await?;

    let search_html = kgc_get(
        &http,
        "/uniasv2/UnSSOLoginControl2?REQ_LOGIN_NO=2&REQ_ACTION_DO=/AGA030.do&REQ_PRFR_MNU_ID=MNUIDSTD0103011",
    )
    .await?;
    let token = extract_struts_token(&search_html)?;
    let year =
        extract_year_from_search_page(&search_html).unwrap_or_else(|| params.year_from.clone());

    let form_params = vec![
        ("org.apache.struts.taglib.html.TOKEN".into(), token),
        ("selTypeCalLsnOpcFcy".into(), "0".into()),
        (
            "txtLsnOpcFcy".into(),
            if params.year_from.is_empty() {
                year.clone()
            } else {
                params.year_from.clone()
            },
        ),
        ("selTypeCalLsnEndFcy".into(), "0".into()),
        (
            "txtLsnEndFcy".into(),
            if params.year_to.is_empty() {
                year
            } else {
                params.year_to.clone()
            },
        ),
        ("selTacTrmCd".into(), params.term.clone()),
        ("selOpcCmpsCd".into(), params.campus.clone()),
        ("selLsnMngPostCd".into(), params.department.clone()),
        ("txtLsnCd_01".into(), params.class_code.clone()),
        ("txtLsnCd_02".into(), String::new()),
        ("selTmtxCd".into(), params.day_period.clone()),
        ("txtSlbSrchKwd".into(), params.keyword.clone()),
        ("selVolCd1".into(), params.language.clone()),
        ("txtTchKnjfn_01".into(), params.instructor.clone()),
        ("txtTchKnafn_01".into(), String::new()),
        ("txtCbbTchRnmAlpfn_01".into(), String::new()),
        ("hdnClassisyUser".into(), "S".into()),
        ("hdnEsearch".into(), "true".into()),
        ("hdnPhfyPrcFlg".into(), String::new()),
        ("ESearch".into(), "検索/Search".into()),
        ("hdnLoginUrl".into(), String::new()),
    ];

    let html = kgc_post(&http, "/uniasv2/AGA030PSC01EventAction.do", &form_params).await?;

    if html.contains("UNM") {
        if let Some(err) = crate::syllabus::extract_validation_error(&html) {
            return Err(err);
        }
    }
    if !html.contains("結果一覧画面") {
        return Err("検索条件が不足しています。履修期・キャンパス・授業管理部署・曜時のいずれか１つを指定してください。".into());
    }

    let first_page = crate::syllabus::parse_search_results_public(&html)?;
    log::info!(
        "Search page 1: {} entries, page {}/{}",
        first_page.entries.len(),
        first_page.current_page,
        first_page.total_pages
    );

    if first_page.total_pages <= 1 {
        return Ok(first_page);
    }

    let mut all_entries = first_page.entries;
    let total_pages = first_page.total_pages;
    let mut current_html = html;

    for page in 2..=total_pages {
        let mut form_params = extract_all_form_inputs(&current_html);
        form_params.retain(|(k, _)| {
            !k.starts_with("ESearch")
                && !k.starts_with("ENarrowSearch")
                && !k.starts_with("EBack")
                && !k.starts_with("ENext")
                && !k.starts_with("EPrev")
                && !k.starts_with("ERefer")
                && !k.starts_with("ERegister")
                && !k.starts_with("EPageSet")
        });
        form_params.push(("ENext.x".into(), "10".into()));
        form_params.push(("ENext.y".into(), "10".into()));

        log::info!(
            "Fetching page {} with {} form params",
            page,
            form_params.len()
        );

        let next_html = kgc_post(&http, "/uniasv2/AGA030PLS01EventAction.do", &form_params).await?;

        match crate::syllabus::parse_search_results_public(&next_html) {
            Ok(page_result) => {
                log::info!(
                    "Search page {}: {} entries",
                    page,
                    page_result.entries.len()
                );
                if page_result.entries.is_empty() {
                    break;
                }
                all_entries.extend(page_result.entries);
                current_html = next_html;
            }
            Err(e) => {
                log::warn!("Failed to parse page {}: {}", page, e);
                break;
            }
        }
    }

    log::info!(
        "Search total: {} entries across {} pages",
        all_entries.len(),
        total_pages
    );
    Ok(crate::syllabus::SyllabusSearchResult {
        total_count: all_entries.len(),
        entries: all_entries,
        current_page: 1,
        total_pages: 1,
    })
}

#[tauri::command]
pub async fn fetch_syllabus_favorites(
    kgc_state: State<'_, KgcState>,
    db: State<'_, crate::db::Database>,
) -> Result<crate::syllabus::SyllabusSearchResult, String> {
    let _kgc_gate = kgc_state.gate.lock().await;
    let http = match kgc_http(kgc_state.inner()).await {
        Ok(h) => h,
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache("syllabus_favorites") {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("syllabus_favorites: cache fallback ({})", e);
                    return Ok(cached);
                }
            }
            return Err(e);
        }
    };

    let main_terms = ["02", "03", "01"];
    let sub_terms = ["04", "05", "06", "07"];
    let mut all_entries = Vec::new();
    let mut seen_codes = std::collections::HashSet::new();

    for term_code in main_terms.iter().chain(sub_terms.iter()) {
        let search_html = kgc_get(
            &http,
            "/uniasv2/UnSSOLoginControl2?REQ_LOGIN_NO=2&REQ_ACTION_DO=/AGA030.do&REQ_PRFR_MNU_ID=MNUIDSTD0103011",
        )
        .await?;
        let token = match extract_struts_token(&search_html) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let year = extract_year_from_search_page(&search_html).unwrap_or_else(|| "2026".into());

        let params = vec![
            ("org.apache.struts.taglib.html.TOKEN".into(), token),
            ("txtLsnOpcFcy".into(), year.clone()),
            ("txtLsnEndFcy".into(), year),
            ("selTypeCalLsnOpcFcy".into(), "0".into()),
            ("selTypeCalLsnEndFcy".into(), "0".into()),
            ("selTacTrmCd".into(), term_code.to_string()),
            ("selOpcCmpsCd".into(), String::new()),
            ("selLsnMngPostCd".into(), String::new()),
            ("hdnClassisyUser".into(), "S".into()),
            ("hdnEsearch".into(), "true".into()),
            ("hdnPhfyPrcFlg".into(), String::new()),
            ("ENarrowSearch".into(), "お気に入り/Bookmark".into()),
        ];
        let html = kgc_post(&http, "/uniasv2/AGA030PSC01EventAction.do", &params).await?;

        if let Ok(result) = crate::syllabus::parse_search_results_public(&html) {
            for entry in result.entries {
                if seen_codes.insert(entry.class_code.clone()) {
                    all_entries.push(entry);
                }
            }
        }
        if *term_code == "01" && !all_entries.is_empty() {
            break;
        }
    }

    let result = crate::syllabus::SyllabusSearchResult {
        entries: all_entries,
        total_count: 0,
        current_page: 1,
        total_pages: 1,
    };

    if let Ok(json) = serde_json::to_string(&result) {
        let _ = db.save_data_cache("syllabus_favorites", &json);
    }

    Ok(result)
}

pub(crate) async fn find_syllabus_results_by_class_code(
    http: &reqwest::Client,
    class_code: &str,
) -> Result<String, String> {
    let terms = ["02", "03", "01", "04", "05", "06", "07"];
    for term_code in &terms {
        let search_html = kgc_get(
            http,
            "/uniasv2/UnSSOLoginControl2?REQ_LOGIN_NO=2&REQ_ACTION_DO=/AGA030.do&REQ_PRFR_MNU_ID=MNUIDSTD0103011",
        )
        .await?;
        let token = match extract_struts_token(&search_html) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let year = extract_year_from_search_page(&search_html).unwrap_or_else(|| "2026".into());

        let search_params = vec![
            ("org.apache.struts.taglib.html.TOKEN".into(), token),
            ("selTypeCalLsnOpcFcy".into(), "0".into()),
            ("txtLsnOpcFcy".into(), year.clone()),
            ("selTypeCalLsnEndFcy".into(), "0".into()),
            ("txtLsnEndFcy".into(), year),
            ("selTacTrmCd".into(), term_code.to_string()),
            ("selOpcCmpsCd".into(), String::new()),
            ("selLsnMngPostCd".into(), String::new()),
            ("txtLsnCd_01".into(), class_code.to_string()),
            ("txtLsnCd_02".into(), String::new()),
            ("selTmtxCd".into(), String::new()),
            ("txtSlbSrchKwd".into(), String::new()),
            ("selVolCd1".into(), String::new()),
            ("txtTchKnjfn_01".into(), String::new()),
            ("txtTchKnafn_01".into(), String::new()),
            ("txtCbbTchRnmAlpfn_01".into(), String::new()),
            ("hdnClassisyUser".into(), "S".into()),
            ("hdnEsearch".into(), "true".into()),
            ("hdnPhfyPrcFlg".into(), String::new()),
            ("ESearch".into(), "検索/Search".into()),
            ("hdnLoginUrl".into(), String::new()),
        ];
        let html = kgc_post(http, "/uniasv2/AGA030PSC01EventAction.do", &search_params).await?;

        if html.contains("結果一覧画面") {
            if let Ok(parsed) = crate::syllabus::parse_search_results_public(&html) {
                if parsed.entries.iter().any(|e| e.class_code == class_code) {
                    return Ok(html);
                }
            }
        }
    }
    Err(format!("科目コード {} が見つかりません", class_code))
}

#[tauri::command]
pub async fn toggle_syllabus_bookmark(
    kgc_state: State<'_, KgcState>,
    class_code: String,
) -> Result<bool, String> {
    let _kgc_gate = kgc_state.gate.lock().await;
    let http = kgc_http(kgc_state.inner()).await?;

    let html = find_syllabus_results_by_class_code(&http, &class_code).await?;

    let parsed = crate::syllabus::parse_search_results_public(&html)?;
    let target_entry = parsed
        .entries
        .iter()
        .find(|e| e.class_code == class_code)
        .ok_or_else(|| format!("科目コード {} が見つかりません", class_code))?;
    let target_index = target_entry.register_index.clone();

    let mut form_params = extract_all_form_inputs(&html);

    form_params.retain(|(k, _)| {
        !k.starts_with("ESearch")
            && !k.starts_with("ENarrowSearch")
            && !k.starts_with("EBack")
            && !k.starts_with("ENext")
            && !k.starts_with("EPrev")
            && !k.starts_with("ERefer")
            && !k.starts_with("ERegister")
            && !k.starts_with("EPageSet")
            && k != "hdnEsearch"
    });

    form_params.retain(|(k, _)| k != "eregisterIndex");
    form_params.push(("eregisterIndex".into(), target_index.clone()));
    form_params.push(("ERegister.x".into(), "10".into()));
    form_params.push(("ERegister.y".into(), "10".into()));

    log::info!(
        "Bookmark toggle: class_code={}, eregisterIndex={}, params_count={}",
        class_code,
        target_index,
        form_params.len()
    );

    let toggle_html = kgc_post(&http, "/uniasv2/AGA030PLS01EventAction.do", &form_params).await?;

    let success = !toggle_html.contains("UNM000480E") && !toggle_html.contains("不正アクセス");
    log::info!(
        "Bookmark toggle result: success={}, len={}",
        success,
        toggle_html.len()
    );

    Ok(success)
}

/// Open the syllabus detail window *immediately* (showing a loading state)
/// and run the slow KGC fetch in the background. The window listens for
/// `syllabus-ready` / `syllabus-error` events emitted to its own label.
///
/// The previous implementation blocked window creation on 2-3 sequential KGC
/// HTTP requests, which made click→window take 1-3 seconds with no feedback.
#[tauri::command]
pub async fn open_syllabus_detail(
    app: tauri::AppHandle,
    class_code: String,
    course_name: String,
) -> Result<(), String> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(1000);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = format!("syllabus-detail-{}", id);

    // 1. Create the window first so the user gets instant visual feedback.
    let encoded_name = urlencoding::encode(&course_name);
    let encoded_label = urlencoding::encode(&label);
    let url_str = format!(
        "university-detail.html?mode=syllabus&name={}&wlabel={}",
        encoded_name, encoded_label
    );

    tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App(url_str.into()))
        .title(&course_name)
        .inner_size(480.0, 560.0)
        .resizable(true)
        .build()
        .map_err(|e| format!("ウィンドウ作成失敗: {}", e))?;

    // 2. Spawn the actual KGC fetch in the background. When it finishes (or
    //    errors out), emit an event to the window so it can swap loading →
    //    rendered content.
    let app_clone = app.clone();
    let label_clone = label.clone();
    tauri::async_runtime::spawn(async move {
        let kgc_state = app_clone.state::<KgcState>();
        let result = fetch_syllabus_detail(&kgc_state, &class_code).await;
        match result {
            Ok(detail) => {
                let store = app_clone.state::<SyllabusDetailData>();
                let stored = match store.0.lock() {
                    Ok(mut map) => {
                        if map.len() > 20 {
                            map.clear();
                        }
                        map.insert(label_clone.clone(), detail);
                        true
                    }
                    Err(_) => false,
                };
                drop(store);
                if stored {
                    let _ = app_clone.emit_to(&label_clone, "syllabus-ready", &label_clone);
                } else {
                    let _ = app_clone.emit_to(
                        &label_clone,
                        "syllabus-error",
                        "internal: state lock poisoned",
                    );
                }
            }
            Err(e) => {
                log::error!("Syllabus detail fetch failed for '{}': {}", class_code, e);
                let _ = app_clone.emit_to(&label_clone, "syllabus-error", e);
            }
        }
    });

    Ok(())
}

/// Inner KGC fetch, factored out so `open_syllabus_detail` can `spawn` it.
async fn fetch_syllabus_detail(
    kgc_state: &KgcState,
    class_code: &str,
) -> Result<crate::parser::CourseDetail, String> {
    let _kgc_gate = kgc_state.gate.lock().await;
    let http = kgc_http(kgc_state).await?;

    let html = find_syllabus_results_by_class_code(&http, class_code).await?;

    let results = crate::syllabus::parse_search_results_public(&html)
        .map_err(|e| format!("検索結果の解析に失敗: {}", e))?;
    let target_entry = results
        .entries
        .iter()
        .find(|e| e.class_code == class_code)
        .ok_or("授業が見つかりませんでした")?;
    let fresh_refer_index = target_entry.refer_index.clone();

    let mut form_params = extract_all_form_inputs(&html);

    let token_key = "org.apache.struts.taglib.html.TOKEN";
    let token_count = form_params.iter().filter(|(k, _)| k == token_key).count();
    if token_count > 1 {
        let last_token = form_params
            .iter()
            .rev()
            .find(|(k, _)| k == token_key)
            .map(|(_, v)| v.clone());
        form_params.retain(|(k, _)| k != token_key);
        if let Some(tok) = last_token {
            form_params.insert(0, (token_key.into(), tok));
        }
        log::warn!(
            "open_syllabus_detail: deduped Struts tokens: {} -> 1",
            token_count
        );
    }

    form_params.retain(|(k, _)| {
        !k.starts_with("ESearch")
            && !k.starts_with("ENarrowSearch")
            && !k.starts_with("EBack")
            && !k.starts_with("ENext")
            && !k.starts_with("EPrev")
            && !k.starts_with("ERefer")
            && !k.starts_with("ERegister")
            && !k.starts_with("EPageSet")
            && k != "hdnEsearch"
    });

    form_params.retain(|(k, _)| k != "ereferIndex");
    form_params.push(("ereferIndex".into(), fresh_refer_index.clone()));
    form_params.push(("ERefer.x".into(), "10".into()));
    form_params.push(("ERefer.y".into(), "10".into()));

    log::info!(
        "Syllabus detail: ereferIndex={}, params_count={}",
        fresh_refer_index,
        form_params.len()
    );

    let detail_html = kgc_post(&http, "/uniasv2/AGA030PLS01EventAction.do", &form_params).await?;

    let detail = crate::parser::parse_course_detail(&detail_html);
    log::info!(
        "Syllabus detail: {} fields (HTML {} bytes)",
        detail.fields.len(),
        detail_html.len()
    );
    Ok(detail)
}

pub struct SyllabusDetailData(
    pub std::sync::Mutex<std::collections::HashMap<String, crate::parser::CourseDetail>>,
);

#[tauri::command]
pub async fn get_syllabus_detail(
    state: State<'_, SyllabusDetailData>,
    label: String,
) -> Result<crate::parser::CourseDetail, String> {
    let mut map = state.0.lock().map_err(|e| e.to_string())?;
    map.remove(&label).ok_or("詳細データがありません".into())
}

#[tauri::command]
pub async fn get_kgc_syllabus_fields(
    db: State<'_, crate::db::Database>,
    kgc_code: String,
) -> Result<Option<serde_json::Value>, String> {
    Ok(db.get_kgc_course_detail(&kgc_code)?.map(|d| {
        serde_json::json!({
            "fields": d.fields,
            "textbooks": d.textbooks,
        })
    }))
}

static STRUTS_TOKEN_RE1: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"name="org\.apache\.struts\.taglib\.html\.TOKEN"[^>]*value="([^"]+)""#)
        .expect("valid regex")
});
static STRUTS_TOKEN_RE2: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"value="([^"]+)"[^>]*name="org\.apache\.struts\.taglib\.html\.TOKEN""#)
        .expect("valid regex")
});

pub(crate) fn extract_struts_token(html: &str) -> Result<String, String> {
    STRUTS_TOKEN_RE1
        .captures(html)
        .or_else(|| STRUTS_TOKEN_RE2.captures(html))
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| "Strutsトークンが見つかりません".into())
}

static YEAR_RE1: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"name="txtLsnOpcFcy"[^>]*value="(\d{4})""#).expect("valid regex")
});
static YEAR_RE2: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"value="(\d{4})"[^>]*name="txtLsnOpcFcy""#).expect("valid regex")
});

pub(crate) fn extract_year_from_search_page(html: &str) -> Option<String> {
    YEAR_RE1
        .captures(html)
        .or_else(|| YEAR_RE2.captures(html))
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

pub(crate) fn extract_all_form_inputs(html: &str) -> Vec<(String, String)> {
    extract_form_inputs_impl(html, "form")
}

pub(crate) fn extract_named_form_inputs(html: &str, form_name: &str) -> Vec<(String, String)> {
    let selector = format!("form[name=\"{}\"]", form_name);
    let params = extract_form_inputs_impl(html, &selector);
    if params.is_empty() {
        log::warn!(
            "extract_named_form_inputs: form '{}' not found, falling back to all forms",
            form_name
        );
        extract_form_inputs_impl(html, "form")
    } else {
        params
    }
}

fn extract_form_inputs_impl(html: &str, form_selector: &str) -> Vec<(String, String)> {
    use scraper::{Html, Selector};

    let document = Html::parse_document(html);
    let mut params: Vec<(String, String)> = Vec::new();

    let input_sel = Selector::parse(&format!("{} input", form_selector)).expect("valid selector");
    for el in document.select(&input_sel) {
        let name = match el.value().attr("name") {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => continue,
        };
        let input_type = el.value().attr("type").unwrap_or("text").to_lowercase();
        if input_type == "submit" || input_type == "image" || input_type == "button" {
            continue;
        }
        if (input_type == "checkbox" || input_type == "radio")
            && el.value().attr("checked").is_none()
        {
            continue;
        }
        let value = el.value().attr("value").unwrap_or("").to_string();
        params.push((name, value));
    }

    let select_sel = Selector::parse(&format!("{} select", form_selector)).expect("valid selector");
    let option_sel = Selector::parse("option[selected]").expect("valid selector");
    for sel_el in document.select(&select_sel) {
        let name = match sel_el.value().attr("name") {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => continue,
        };
        if let Some(opt) = sel_el.select(&option_sel).next() {
            let value = opt.value().attr("value").unwrap_or("").to_string();
            params.push((name, value));
        }
    }

    params
}
