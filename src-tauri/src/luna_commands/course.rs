use tauri::State;

use crate::luna_parser;
use crate::LunaState;

use super::{is_safe_param, luna_get, luna_http};

/// Fetch and parse course top page (/lms/course?idnumber=XXX)
#[tauri::command]
pub async fn luna_fetch_course_detail(
    state: State<'_, LunaState>,
    db: crate::db::AccountDb,
    idnumber: String,
) -> Result<luna_parser::LunaCourseContents, String> {
    if !is_safe_param(&idnumber) {
        return Err("無効なパラメータです".into());
    }
    let cache_key = format!("luna_course:{}", idnumber);
    let http = match luna_http(&state).await {
        Ok(h) => h,
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("luna_course: cache fallback ({})", e);
                    return Ok(cached);
                }
            }
            return Err(e);
        }
    };

    let course_path = format!("/lms/course?idnumber={}", idnumber);
    let contents_path = format!("/lms/contents?idnumber={}", idnumber);

    // Fetch course top page — Luna sometimes returns an incomplete/redirect page
    // on the very first access after session restore, so we retry once if menus are empty.
    let course_html = match luna_get(&http, &course_path).await {
        Ok(html) => html,
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("luna_course: cache fallback ({})", e);
                    return Ok(cached);
                }
            }
            return Err(e);
        }
    };
    let mut result = luna_parser::parse_luna_course_contents(&course_html, &idnumber);

    if result.menus.is_empty() {
        log::warn!(
            "Course page for {} returned no menus ({}B), retrying...",
            idnumber,
            course_html.len()
        );
        #[cfg(debug_assertions)]
        {
            if crate::should_dump_debug_html() {
                let dump =
                    std::env::temp_dir().join(format!("luna_course_{}_initial.html", idnumber));
                let _ = std::fs::write(&dump, &course_html);
            }
        }
        // Retry: the first request may have warmed up the Luna session/course state
        if let Ok(retry_html) = luna_get(&http, &course_path).await {
            let retry_result = luna_parser::parse_luna_course_contents(&retry_html, &idnumber);
            if !retry_result.menus.is_empty() {
                log::info!("Retry succeeded for course {}", idnumber);
                result = retry_result;
            }
            #[cfg(debug_assertions)]
            {
                if crate::should_dump_debug_html() {
                    let dump = std::env::temp_dir().join(format!("luna_course_{}.html", idnumber));
                    let _ = std::fs::write(&dump, &retry_html);
                }
            }
        }
    } else {
        #[cfg(debug_assertions)]
        {
            if crate::should_dump_debug_html() {
                let dump = std::env::temp_dir().join(format!("luna_course_{}.html", idnumber));
                let _ = std::fs::write(&dump, &course_html);
            }
        }
    }

    // Fetch contents top page (actual content items)
    let contents_html = match luna_get(&http, &contents_path).await {
        Ok(html) => html,
        Err(e) => {
            if let Ok(Some((json, _))) = db.get_data_cache(&cache_key) {
                if let Ok(cached) = serde_json::from_str(&json) {
                    log::info!("luna_course: cache fallback (contents fetch: {})", e);
                    return Ok(cached);
                }
            }
            return Err(e);
        }
    };

    #[cfg(debug_assertions)]
    {
        if crate::should_dump_debug_html() {
            let dump_path = std::env::temp_dir().join(format!("luna_contents_{}.html", idnumber));
            let _ = std::fs::write(&dump_path, &contents_html);
        }
    }

    // Merge actual content items from contents page
    let (materials, reports, examinations, discussions, surveys) =
        luna_parser::parse_luna_contents_page(&contents_html);
    result.materials = materials;
    result.reports = reports;
    result.examinations = examinations;
    result.discussions = discussions;
    result.surveys = surveys;

    // Cache the complete result
    if let Ok(json) = serde_json::to_string(&result) {
        let _ = db.save_data_cache(&cache_key, &json);
    }

    Ok(result)
}
