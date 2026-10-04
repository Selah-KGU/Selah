use crate::client;
use crate::config;
use crate::db::{Database, SnapshotState};
use crate::luna_client;
use crate::luna_parser;
use crate::{KgcState, LunaState};

pub(super) struct LunaSnapshotFields {
    pub(super) communities: Vec<luna_parser::LunaCommunity>,
    pub(super) year_opts: Vec<luna_parser::SelectOption>,
    pub(super) term_opts: Vec<luna_parser::SelectOption>,
    pub(super) year: String,
    pub(super) term: String,
    pub(super) replaced: bool,
    pub(super) metadata_changed: bool,
}

fn luna_fields_from_snapshot(snapshot: &SnapshotState) -> LunaSnapshotFields {
    LunaSnapshotFields {
        communities: snapshot.luna_communities.clone(),
        year_opts: snapshot.luna_year_options.clone(),
        term_opts: snapshot.luna_term_options.clone(),
        year: snapshot.luna_year.clone(),
        term: snapshot.luna_term.clone(),
        replaced: false,
        metadata_changed: false,
    }
}

pub(super) async fn sync_luna_timetable(
    luna_state: &LunaState,
    db: &Database,
    previous: &SnapshotState,
) -> Result<LunaSnapshotFields, String> {
    let http = {
        let luna = luna_state.client.lock().await;
        if luna.authenticated {
            Some(luna.http.clone())
        } else {
            None
        }
    };
    let Some(http) = http else {
        log::info!("sync_schedule_data: Luna not authenticated; keeping stored timetable");
        return Ok(luna_fields_from_snapshot(previous));
    };

    let target = target_luna_period(previous);
    let parsed = match fetch_current_luna_timetable(&http, target.as_ref()).await {
        Ok(parsed) => parsed,
        Err(error) if is_luna_auth_error(&error) => return Err(error),
        Err(error) => {
            log::warn!("sync_schedule_data: Luna fetch skipped persist: {}", error);
            return Ok(luna_fields_from_snapshot(previous));
        }
    };

    if !courses_fit_target(&parsed.courses, target.as_ref()) {
        log::warn!(
            "sync_schedule_data: Luna courses do not match target term {}/{}; keeping stored rows",
            parsed.year,
            parsed.term
        );
        return Ok(luna_fields_from_snapshot(previous));
    }

    let replaced = if parsed.courses.is_empty() {
        log::warn!("sync_schedule_data: Luna timetable had no courses; keeping stored rows");
        false
    } else {
        db.replace_luna_courses(&parsed.courses)?;
        log::info!(
            "sync_schedule_data: Luna: {} courses, {} communities, term={}/{}",
            parsed.courses.len(),
            parsed.communities.len(),
            parsed.year,
            parsed.term
        );
        true
    };

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
        communities: if parsed.communities.is_empty() {
            previous.luna_communities.clone()
        } else {
            parsed.communities
        },
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
        replaced,
        metadata_changed: true,
    })
}

async fn fetch_current_luna_timetable(
    http: &reqwest::Client,
    target: Option<&(String, String)>,
) -> Result<luna_parser::LunaTimetable, String> {
    let url = format!("{}/lms/timetable", config::LUNA_BASE);
    let html = client::fetch_with_redirect(
        http,
        &url,
        config::LUNA_BASE,
        luna_client::LUNA_SESSION_EXPIRED_MSG,
        luna_client::is_luna_session_expired,
    )
    .await?;
    let parsed = luna_parser::parse_luna_timetable(&html);
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
        return Err("luna-term-mismatch".into());
    };
    log::info!(
        "sync_schedule_data: Luna page is {}/{}, switching to {}/{}",
        parsed.year,
        parsed.term,
        year,
        term
    );
    let switched_html = submit_luna_timetable_switch(http, &request).await?;
    let switched = luna_parser::parse_luna_timetable(&switched_html);
    if luna_timetable_matches(&switched, target) {
        return Ok(switched);
    }
    log::warn!(
        "sync_schedule_data: Luna switch stayed on {}/{}",
        switched.year,
        switched.term
    );
    Err("luna-term-mismatch".into())
}

async fn submit_luna_timetable_switch(
    http: &reqwest::Client,
    request: &luna_parser::LunaTimetableSwitch,
) -> Result<String, String> {
    let url = resolve_luna_action(&request.action)?;
    let referer = format!("{}/lms/timetable", config::LUNA_BASE);
    if request.method.eq_ignore_ascii_case("get") {
        let builder = http
            .get(&url)
            .query(&request.fields)
            .header("Referer", &referer);
        return client::send_and_follow_redirect(
            http,
            builder,
            config::LUNA_BASE,
            luna_client::LUNA_SESSION_EXPIRED_MSG,
            luna_client::is_luna_session_expired,
        )
        .await;
    }
    client::post_form_with_redirect(
        http,
        &url,
        config::LUNA_BASE,
        luna_client::LUNA_SESSION_EXPIRED_MSG,
        luna_client::is_luna_session_expired,
        request.fields.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        &[("Referer", referer.as_str()), ("Origin", config::LUNA_BASE)],
    )
    .await
}

fn resolve_luna_action(action: &str) -> Result<String, String> {
    let action = action.trim();
    if action.is_empty() || action == "#" || action.starts_with("javascript:") {
        return Ok(format!("{}/lms/timetable", config::LUNA_BASE));
    }
    if action.starts_with("http://") || action.starts_with("https://") {
        if action.starts_with(config::LUNA_BASE) {
            return Ok(action.to_string());
        }
        return Err("Luna timetable form action is outside Luna".into());
    }
    if let Some(path) = action.strip_prefix('/') {
        return Ok(format!("{}/{}", config::LUNA_BASE, path));
    }
    Ok(format!(
        "{}/{}",
        config::LUNA_BASE,
        action.trim_start_matches("./")
    ))
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

pub(super) async fn clear_kgc_if_expired(kgc: &KgcState, error: &str) {
    if error == client::SESSION_EXPIRED_MSG {
        kgc.client.lock().await.clear_session();
        log::info!("KGC session expired during schedule sync; stopped background KGC requests");
    }
}
