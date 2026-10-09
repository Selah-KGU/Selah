use super::sources::*;
use super::types::*;
use crate::db::Database;
use std::collections::HashMap;

pub(in crate::detective) fn build_context(db: &Database) -> Result<DetectiveContext, String> {
    let doubts = load_doubts(db);
    let results = load_case_results(db);
    let mut courses: HashMap<String, CourseBuilder> = HashMap::new();

    let mut schedule_items = Vec::new();
    let snap = db.get_snapshot_state().ok().flatten();
    let scope = snap.as_ref().map(|snap| {
        crate::academic_period::visible_weeks(
            &snap.current_week_label,
            &snap.next_week_label,
            &snap.luna_year,
            &snap.luna_term,
            chrono::Local::now().date_naive(),
        )
    });
    if let (Some(scope), Ok(Some((result, _)))) = (scope.as_ref(), db.get_ai_schedule_cache()) {
        if crate::academic_period::week_belongs_to_visible_label(
            &scope.current,
            &result.current_week_label,
        ) {
            schedule_items.extend(result.current_week);
        }
        if crate::academic_period::week_belongs_to_visible_label(
            &scope.next,
            &result.next_week_label,
        ) {
            schedule_items.extend(result.next_week);
        }
    }
    for item in schedule_items {
        if item.course_name.trim().is_empty() {
            continue;
        }
        let course = ensure_course(&mut courses, &item.course_name);
        course.schedule_items.push(item);
    }

    if let Some(scope) = scope.as_ref() {
        if let Some(snap) = snap {
            let mut communities = snap.luna_communities;
            communities.retain(|community| {
                crate::db::luna_course_matches_snapshot(
                    &community.idnumber,
                    &scope.year,
                    &scope.term,
                )
            });
            if let Ok(raw) = db.build_raw_data(&scope.current, &scope.next, communities) {
                for row in raw
                    .kgc_entries_current
                    .iter()
                    .chain(raw.kgc_entries_next.iter())
                {
                    if !row.name.trim().is_empty() {
                        ensure_course(&mut courses, &row.name);
                    }
                }
                for row in raw.luna_courses.iter() {
                    if !row.name.trim().is_empty() {
                        ensure_course(&mut courses, &row.name);
                    }
                }
            }
        }
    }

    let mut records = crate::commands::list_downloads_snapshot()?;
    if !records.iter().any(is_live_record) {
        records = crate::commands::scan_download_dir_snapshot()?;
    }
    for record in records
        .into_iter()
        .filter(|record| record.file_exists && is_live_record(record))
    {
        let course_name = if record.course_name.trim().is_empty() {
            infer_course_name_from_record(&record)
        } else {
            record.course_name.clone()
        };
        let course = ensure_course(&mut courses, &course_name);
        course.latest_at = course.latest_at.max(record.downloaded_at);
        course.live_records.push(DetectiveLiveRecord {
            id: record.id,
            filename: record.filename,
            path: record.path.clone(),
            course_name: course.name.clone(),
            downloaded_at: record.downloaded_at,
            excerpt: live_excerpt(&record.path),
        });
    }

    let signals = collect_exam_signals(db);
    for signal in signals {
        if let Some(key) = match_signal_course_key(&signal, &courses) {
            if let Some(course) = courses.get_mut(&key) {
                course.latest_at = course.latest_at.max(date_score(&signal.date));
                course.exam_signals.push(signal);
            }
        } else if !signal.course_info.trim().is_empty() {
            let course = ensure_course(&mut courses, &signal.course_info);
            course.exam_signals.push(signal);
        }
    }

    for doubt in doubts {
        let course = ensure_course(&mut courses, &doubt.course_name);
        course.doubts.push(doubt);
    }

    for result in results.iter().cloned() {
        let course = ensure_course(&mut courses, &result.course_name);
        course.latest_at = course.latest_at.max(result.closed_at);
        course.recent_results.push(result);
    }

    let mut out: Vec<DetectiveCourse> = courses
        .into_values()
        .filter(|course| {
            !course.live_records.is_empty()
                || !course.exam_signals.is_empty()
                || !course.doubts.is_empty()
                || !course.recent_results.is_empty()
        })
        .map(|mut course| {
            course
                .live_records
                .sort_by(|a, b| b.downloaded_at.cmp(&a.downloaded_at));
            course.exam_signals = dedupe_signals(course.exam_signals);
            course
                .recent_results
                .sort_by(|a, b| b.closed_at.cmp(&a.closed_at));
            course.recent_results.truncate(5);
            let case_type = course_case_type(&course).to_string();
            let readiness = readiness_text(&course);
            DetectiveCourse {
                name: course.name,
                key: course.key,
                live_records: course.live_records,
                exam_signals: course.exam_signals,
                schedule_items: course.schedule_items,
                latest_at: course.latest_at,
                doubts: course.doubts,
                recent_results: course.recent_results,
                case_type,
                readiness,
            }
        })
        .collect();

    out.sort_by(|a, b| course_score(b).total_cmp(&course_score(a)));

    let review_queue = build_review_queue(&out);

    let included_course_keys = load_included_courses(db);
    let memory = load_memory(db);

    // Surface any campaign bibles already cached for the visible courses.
    let campaigns: Vec<DetectiveCampaign> = out
        .iter()
        .filter_map(|course| load_campaign(db, &course.key))
        .collect();

    Ok(DetectiveContext {
        courses: out,
        review_queue,
        recent_results: results.into_iter().take(12).collect(),
        generated_at: crate::db::epoch_secs(),
        included_course_keys,
        memory,
        campaigns,
    })
}
