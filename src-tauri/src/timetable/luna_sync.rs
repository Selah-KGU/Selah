use crate::client;
use crate::db::{Database, SnapshotState};
use crate::luna_client;
use crate::luna_parser;

pub(super) struct LunaSnapshotFields {
    pub(super) communities: Vec<luna_parser::LunaCommunity>,
    pub(super) year_opts: Vec<luna_parser::SelectOption>,
    pub(super) term_opts: Vec<luna_parser::SelectOption>,
    pub(super) year: String,
    pub(super) term: String,
}

pub(super) async fn sync_luna_timetable(
    http: &reqwest::Client,
    db: &Database,
    previous: &SnapshotState,
    base_url: &str,
) -> Result<LunaSnapshotFields, String> {
    let target = target_luna_period(previous);
    let parsed = fetch_current_luna_timetable(http, target.as_ref(), base_url).await?;

    if !courses_fit_target(&parsed.courses, target.as_ref()) {
        log::warn!(
            "sync_schedule_data: Luna courses do not match target term {}/{}; keeping stored rows",
            parsed.year,
            parsed.term
        );
        return Err(
            "Luna の時間割の学期を確認できませんでした。保存済みの時間割を表示しています。".into(),
        );
    }

    // A recognized empty timetable is authoritative too (e.g. withdrawn courses).
    db.replace_luna_courses(&parsed.courses)?;
    log::info!(
        "sync_schedule_data: Luna: {} courses, {} communities, term={}/{}",
        parsed.courses.len(),
        parsed.communities.len(),
        parsed.year,
        parsed.term
    );

    let year = if parsed.year.is_empty() {
        previous.luna_year.clone()
    } else {
        parsed.year
    };
    let term = if parsed.term.is_empty() {
        previous.luna_term.clone()
    } else {
        parsed.term
    };
    Ok(LunaSnapshotFields {
        communities: parsed.communities,
        year_opts: if parsed.year_options.is_empty() {
            previous.luna_year_options.clone()
        } else {
            parsed.year_options
        },
        term_opts: if parsed.term_options.is_empty() {
            previous.luna_term_options.clone()
        } else {
            parsed.term_options
        },
        year,
        term,
    })
}

async fn fetch_current_luna_timetable(
    http: &reqwest::Client,
    target: Option<&(String, String)>,
    base_url: &str,
) -> Result<luna_parser::LunaTimetable, String> {
    let url = format!("{}/lms/timetable", base_url);
    let html = client::fetch_with_redirect(
        http,
        &url,
        base_url,
        luna_client::LUNA_SESSION_EXPIRED_MSG,
        luna_client::is_luna_session_expired,
    )
    .await?;
    let parsed = checked_timetable(&html)?;
    if luna_timetable_matches(&parsed, target) {
        return Ok(parsed);
    }
    let Some((year, term)) = target else {
        return Ok(parsed);
    };
    let Some(request) = luna_parser::build_timetable_switch(&html, year, term) else {
        log::warn!(
            "sync_schedule_data: Luna page is {}/{}, cannot switch to {}/{}",
            parsed.year,
            parsed.term,
            year,
            term
        );
        return Err(
            "Luna の時間割を現在の学期に切り替えられませんでした。もう一度更新してください。"
                .into(),
        );
    };
    log::info!(
        "sync_schedule_data: Luna page is {}/{}, switching to {}/{}",
        parsed.year,
        parsed.term,
        year,
        term
    );
    let switched_html = submit_luna_timetable_switch(http, &request, base_url).await?;
    let switched = checked_timetable(&switched_html)?;
    if luna_timetable_matches(&switched, target) {
        return Ok(switched);
    }
    log::warn!(
        "sync_schedule_data: Luna switch stayed on {}/{}",
        switched.year,
        switched.term
    );
    Err("Luna の時間割を現在の学期に切り替えられませんでした。もう一度更新してください。".into())
}

async fn submit_luna_timetable_switch(
    http: &reqwest::Client,
    request: &luna_parser::LunaTimetableSwitch,
    base_url: &str,
) -> Result<String, String> {
    let url = resolve_luna_action(&request.action, base_url)?;
    let referer = format!("{}/lms/timetable", base_url);
    if request.method.eq_ignore_ascii_case("get") {
        let builder = http
            .get(&url)
            .query(&request.fields)
            .header("Referer", &referer);
        return client::send_and_follow_redirect(
            http,
            builder,
            base_url,
            luna_client::LUNA_SESSION_EXPIRED_MSG,
            luna_client::is_luna_session_expired,
        )
        .await;
    }
    client::post_form_with_redirect(
        http,
        &url,
        base_url,
        luna_client::LUNA_SESSION_EXPIRED_MSG,
        luna_client::is_luna_session_expired,
        request.fields.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        &[("Referer", referer.as_str()), ("Origin", base_url)],
    )
    .await
}

fn resolve_luna_action(action: &str, base_url: &str) -> Result<String, String> {
    let action = action.trim();
    if action.is_empty() || action == "#" || action.starts_with("javascript:") {
        return Ok(format!("{}/lms/timetable", base_url));
    }
    if action.starts_with("http://") || action.starts_with("https://") {
        if url::Url::parse(action)
            .ok()
            .zip(url::Url::parse(base_url).ok())
            .is_some_and(|(action, base)| action.origin() == base.origin())
        {
            return Ok(action.to_string());
        }
        return Err("Luna timetable form action is outside Luna".into());
    }
    if let Some(path) = action.strip_prefix('/') {
        return Ok(format!("{}/{}", base_url, path));
    }
    Ok(format!("{}/{}", base_url, action.trim_start_matches("./")))
}

fn checked_timetable(html: &str) -> Result<luna_parser::LunaTimetable, String> {
    if !luna_parser::is_luna_timetable(html) {
        return Err("Luna の時間割ページを確認できませんでした。もう一度更新してください。".into());
    }
    let parsed = luna_parser::parse_luna_timetable(html);
    if parsed.year.is_empty()
        || parsed.term.is_empty()
        || parsed.courses.iter().any(|course| {
            course.idnumber.is_empty()
                || course.name.is_empty()
                || !(1..=6).contains(&course.day)
                || !(1..=7).contains(&course.period)
        })
    {
        return Err("Luna の時間割を読み取れませんでした。もう一度更新してください。".into());
    }
    Ok(parsed)
}

fn target_luna_period(snapshot: &SnapshotState) -> Option<(String, String)> {
    let today = chrono::Local::now().date_naive();
    if let Some(period) = crate::academic_period::calendar_academic_period(today) {
        return Some((period.year, period.term));
    }
    if !snapshot.luna_year.is_empty() && !snapshot.luna_term.is_empty() {
        return Some((snapshot.luna_year.clone(), snapshot.luna_term.clone()));
    }
    None
}

pub(super) fn retain_current_communities(
    communities: &mut Vec<luna_parser::LunaCommunity>,
    year: &str,
    term: &str,
) {
    communities.retain(|community| {
        crate::db::luna_course_matches_snapshot(&community.idnumber, year, term)
    });
}

fn luna_timetable_matches(
    parsed: &luna_parser::LunaTimetable,
    target: Option<&(String, String)>,
) -> bool {
    let Some((year, term)) = target else {
        return true;
    };
    let year_ok = parsed.year.is_empty() || &parsed.year == year;
    parsed.term == *term && year_ok
}

fn courses_fit_target(
    courses: &[luna_parser::LunaCourse],
    target: Option<&(String, String)>,
) -> bool {
    let Some((year, term)) = target else {
        return true;
    };
    courses
        .iter()
        .all(|course| crate::db::luna_course_matches_snapshot(&course.idnumber, year, term))
}

pub(super) fn is_luna_auth_error(error: &str) -> bool {
    error == luna_client::LUNA_SESSION_EXPIRED_MSG || error.contains("Lunaセッションが期限切れ")
}

pub(super) async fn clear_kgc_if_expired(
    lease: &crate::session_coordinator::SessionLease,
    error: &str,
) {
    if error == client::SESSION_EXPIRED_MSG {
        let _ = crate::session_coordinator::SESSIONS.validated(
            lease,
            crate::session_coordinator::Health::NeedsLogin,
            None,
        );
        log::info!("KGC session expired during schedule sync; stopped background KGC requests");
    }
}
