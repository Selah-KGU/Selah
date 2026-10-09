use crate::commands;
use crate::KgcState;

/// Syllabus search URL — enters the syllabus system through SSO.
const SYLLABUS_SSO_URL: &str =
    "/uniasv2/UnSSOLoginControl2?REQ_LOGIN_NO=2&REQ_ACTION_DO=/AGA030.do&REQ_PRFR_MNU_ID=MNUIDSTD0103011";

/// Batch-fetch syllabus detail pages for multiple class codes.
///
/// Each course holds the KGC Struts gate only for its own token sequence
/// (search form GET + search POST + detail POST). The gate is released before
/// the inter-course delay so a foreground KGC command can run.
/// Returns `(class_code, Ok(detail_html))` or `(class_code, Err(reason))`.
pub(super) async fn batch_fetch_syllabi(
    kgc: &KgcState,
    codes: &[String],
) -> Vec<(String, Result<String, String>)> {
    let mut results = Vec::new();
    let terms = ["02", "03", "01", "04", "05"];

    for code in codes {
        let gate = kgc.gate.lock().await;
        let http_owned = {
            let client = kgc.session();
            if !client.has_credentials() {
                drop(gate);
                results.push((code.clone(), Err("KGC not authenticated".into())));
                break;
            }
            client.http().clone()
        };
        let http = &http_owned;
        let mut found = false;
        for term_code in &terms {
            // Get a fresh search form (each search POST consumes the Struts token)
            let search_html = match commands::kgc_get(http, SYLLABUS_SSO_URL).await {
                Ok(h) => h,
                Err(e) => {
                    log::warn!(
                        "batch_fetch_syllabi: {} GET search page failed: {}",
                        code,
                        e
                    );
                    break; // session broken, skip remaining terms for this code
                }
            };
            let token = match commands::extract_struts_token(&search_html) {
                Ok(t) => t,
                Err(_) => continue, // try next term
            };
            let year = commands::extract_year_from_search_page(&search_html)
                .unwrap_or_else(|| "2026".into());

            // POST search for this class_code + term
            let search_params = vec![
                ("org.apache.struts.taglib.html.TOKEN".into(), token),
                ("selTypeCalLsnOpcFcy".into(), "0".into()),
                ("txtLsnOpcFcy".into(), year.clone()),
                ("selTypeCalLsnEndFcy".into(), "0".into()),
                ("txtLsnEndFcy".into(), year),
                ("selTacTrmCd".into(), term_code.to_string()),
                ("selOpcCmpsCd".into(), String::new()),
                ("selLsnMngPostCd".into(), String::new()),
                ("txtLsnCd_01".into(), code.to_string()),
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

            let results_html = match commands::kgc_post(
                http,
                "/uniasv2/AGA030PSC01EventAction.do",
                &search_params,
            )
            .await
            {
                Ok(h) => h,
                Err(e) => {
                    log::warn!("batch_fetch_syllabi: {} POST search failed: {}", code, e);
                    continue;
                }
            };

            if !results_html.contains("結果一覧画面") {
                continue; // no results for this term
            }

            let parsed = match crate::syllabus::parse_search_results_public(&results_html) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let target = match parsed
                .entries
                .iter()
                .find(|e| e.class_code == code.as_str())
            {
                Some(t) => t,
                None => continue,
            };
            let refer_index = target.refer_index.clone();
            log::info!(
                "batch_fetch_syllabi: {} found in term {}, refer_index='{}', total_results={}, course_title='{}'",
                code, term_code, refer_index, parsed.entries.len(), target.course_title
            );

            // Guard: empty refer_index means the hidden input wasn't in the row HTML
            // (likely set by JavaScript onclick). Use positional index as fallback.
            let effective_refer_index = if refer_index.is_empty() {
                let pos = parsed
                    .entries
                    .iter()
                    .position(|e| e.class_code == code.as_str())
                    .unwrap_or(0);
                log::warn!(
                    "batch_fetch_syllabi: {} ereferIndex is empty, using positional fallback: {}",
                    code,
                    pos
                );
                pos.to_string()
            } else {
                refer_index.clone()
            };

            log::info!(
                "batch_fetch_syllabi: {} ereferIndex={}",
                code,
                effective_refer_index
            );

            // Navigate to syllabus detail page.
            // Extract inputs ONLY from the results list form (AGA030PLS01Form),
            // not from the search form which shares the page and has conflicting params.
            let mut form_params =
                commands::extract_named_form_inputs(&results_html, "AGA030PLS01Form");

            // Log diagnostic info about extracted params
            let token_count = form_params
                .iter()
                .filter(|(k, _)| k == "org.apache.struts.taglib.html.TOKEN")
                .count();
            log::info!(
                "batch_fetch_syllabi: {} extracted {} form params (AGA030PLS01Form), {} Struts tokens",
                code, form_params.len(), token_count
            );

            // With targeted form extraction, token dedup is unnecessary
            // (only one form's token is extracted).

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
            form_params.push(("ereferIndex".into(), effective_refer_index.clone()));
            form_params.push(("ERefer.x".into(), "10".into()));
            form_params.push(("ERefer.y".into(), "10".into()));

            // Dump params for debugging
            log::debug!(
                "batch_fetch_syllabi: {} POST params: {:?}",
                code,
                form_params
                    .iter()
                    .map(|(k, v)| {
                        if v.len() > 60 {
                            format!("{}={}...", k, &v[..60])
                        } else {
                            format!("{}={}", k, v)
                        }
                    })
                    .collect::<Vec<_>>()
            );

            match commands::kgc_post(http, "/uniasv2/AGA030PLS01EventAction.do", &form_params).await
            {
                Ok(detail_html) => {
                    if !detail_html.contains("AGA030PVI01Form") {
                        log::warn!(
                            "batch_fetch_syllabi: {} detail POST did not reach detail page (term {}), {} bytes",
                            code, term_code, detail_html.len()
                        );
                        #[cfg(debug_assertions)]
                        {
                            if crate::should_dump_debug_html() {
                                let _ = std::fs::write(
                                    std::env::temp_dir()
                                        .join(format!("kwic_detail_fail_{}.html", code)),
                                    &detail_html,
                                );
                            }
                        }
                        continue;
                    }
                    log::info!(
                        "batch_fetch_syllabi: {} -> {} bytes (term {})",
                        code,
                        detail_html.len(),
                        term_code
                    );

                    results.push((code.clone(), Ok(detail_html)));
                    found = true;
                    break;
                }
                Err(e) => {
                    log::warn!(
                        "batch_fetch_syllabi: {} POST detail failed (term {}): {}",
                        code,
                        term_code,
                        e
                    );
                    continue;
                }
            }
        }

        if !found && !results.iter().any(|(c, _)| c == code) {
            results.push((
                code.clone(),
                Err(format!("科目コード {} が見つかりません", code)),
            ));
        }

        drop(gate);
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    results
}
