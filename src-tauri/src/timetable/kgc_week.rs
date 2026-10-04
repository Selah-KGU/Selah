use super::*;

use crate::client;
use crate::config;
use crate::db::Database;
use crate::parser;

/// Fetch next week's KGC data by navigating the Struts form.
pub(super) async fn fetch_next_week_kgc(
    kgc_http: &reqwest::Client,
    current_data: &parser::TimetableData,
    db: &Database,
) -> Result<String, String> {
    if current_data.form_fields.is_empty() {
        return Ok(String::new());
    }

    let fresh_url = format!(
        "{}/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014",
        config::KG_COURSE_BASE
    );
    let fresh_html = client::fetch_page_with(kgc_http, &fresh_url).await?;
    let fresh_data = parser::parse_timetable(&fresh_html);

    let mut params: Vec<(String, String)> = fresh_data.form_fields.into_iter().collect();
    params.push(("ENext.x".into(), "1".into()));
    params.push(("ENext.y".into(), "1".into()));

    let post_url = format!(
        "{}/uniasv2/ARF010PCT01EventAction.do",
        config::KG_COURSE_BASE
    );
    let html = client::post_form_with_redirect(
        kgc_http,
        &post_url,
        config::KG_COURSE_BASE,
        client::SESSION_EXPIRED_MSG,
        client::is_session_expired_body,
        params.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        &[
            (
                "Referer",
                &format!("{}/uniasv2/ARF010.do", config::KG_COURSE_BASE),
            ),
            ("Origin", config::KG_COURSE_BASE),
        ],
    )
    .await?;

    let next_data = parser::parse_timetable(&html);
    let next_week_label = next_data.week_label.clone();
    log::info!(
        "fetch_next_week_kgc: next page: {} entries, week_label='{}'",
        next_data.entries.len(),
        next_week_label
    );

    if next_data.entries.is_empty() && next_week_label.is_empty() {
        return Ok(String::new());
    }

    for entry in &next_data.entries {
        let day_int = day_str_to_int(&entry.day);
        if day_int == 0 {
            continue;
        }
        db.upsert_kgc_course(
            &entry.course_code,
            &entry.course_name,
            day_int,
            entry.period,
            &entry.room,
            &entry.detail_path,
            entry.is_cancelled,
            entry.is_makeup,
            entry.is_room_changed,
            &next_week_label,
        )?;
    }

    Ok(next_week_label)
}
