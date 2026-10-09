//! Seen-state bootstrap, Luna identity, and native dispatch.

use super::*;

pub(in crate::notifier) fn load_seen_state(
    db: &Database,
    source: &str,
) -> (bool, Vec<String>, HashSet<String>) {
    let seen_ids = crate::read_state::get_seen_notif_ids(db, source);
    let seen_set = seen_ids.iter().cloned().collect::<HashSet<_>>();
    let initialized = crate::read_state::is_seen_notif_initialized(db, source);
    (initialized, seen_ids, seen_set)
}

pub(in crate::notifier) fn evaluate_bootstrap_state(
    db: &Database,
    authenticated_sources: &[&str],
) -> BootstrapState {
    if crate::read_state::is_seen_notif_bootstrap_complete(db) {
        return BootstrapState {
            mode: BootstrapMode::Normal,
            should_mark_complete: false,
            should_mark_started_at: None,
        };
    }

    let started_at = crate::read_state::get_seen_notif_bootstrap_started_at(db);
    let all_authenticated_sources_have_seen_state = !authenticated_sources.is_empty()
        && authenticated_sources
            .iter()
            .all(|source| crate::read_state::has_seen_notif_state(db, source));

    if started_at.is_none() && all_authenticated_sources_have_seen_state {
        return BootstrapState {
            mode: BootstrapMode::Normal,
            should_mark_complete: true,
            should_mark_started_at: None,
        };
    }

    if authenticated_sources.is_empty() {
        return BootstrapState {
            mode: BootstrapMode::Silent,
            should_mark_complete: false,
            should_mark_started_at: None,
        };
    }

    let now = epoch_secs();
    let should_mark_started_at = started_at.is_none().then_some(now);
    let started_at = started_at.unwrap_or(now);

    if now.saturating_sub(started_at) >= BOOTSTRAP_GRACE_PERIOD.as_secs() as i64 {
        BootstrapState {
            mode: BootstrapMode::Finalize,
            should_mark_complete: false,
            should_mark_started_at,
        }
    } else {
        BootstrapState {
            mode: BootstrapMode::Silent,
            should_mark_complete: false,
            should_mark_started_at,
        }
    }
}

pub(in crate::notifier) fn resolve_bootstrap_state(
    db: &Database,
    authenticated_sources: &[&str],
) -> BootstrapState {
    let state = evaluate_bootstrap_state(db, authenticated_sources);
    if let Some(started_at) = state.should_mark_started_at {
        crate::read_state::mark_seen_notif_bootstrap_started_at(db, started_at);
    }
    if state.should_mark_complete {
        crate::read_state::mark_seen_notif_bootstrap_complete(db);
    }
    state
}

pub(in crate::notifier) fn bootstrap_mode_label(mode: BootstrapMode) -> &'static str {
    match mode {
        BootstrapMode::Silent => "silent",
        BootstrapMode::Finalize => "finalize",
        BootstrapMode::Normal => "normal",
    }
}

pub(in crate::notifier) fn dispatch_notification(
    app: &AppHandle,
    run: &mut SyncRunDebug,
    source: &str,
    title: String,
    body: String,
    target: Option<NotificationClickTarget>,
) {
    if crate::db::capture_account() != crate::session_coordinator::SESSIONS.account_context() {
        return;
    }
    let result = if let Some(target) = target {
        send_actionable_native_notification(app, &title, &body, target)
    } else {
        crate::ai::send_native_notification(app, &title, &body)
    };

    match result {
        Ok(detail) => {
            run.dispatched += 1;
            record_event(run, source, "dispatched", title, body, detail);
        }
        Err(error) => {
            run.failed += 1;
            record_event(run, source, "failed", title, body, error);
        }
    }
}

pub(in crate::notifier) fn send_actionable_native_notification(
    app: &AppHandle,
    title: &str,
    body: &str,
    _target: NotificationClickTarget,
) -> Result<String, String> {
    let detail = crate::ai::send_native_notification(app, title, body)?;
    Ok(format!(
        "{}; click target not supported on this platform",
        detail
    ))
}

pub(in crate::notifier) fn record_event(
    run: &mut SyncRunDebug,
    source: &str,
    status: &str,
    title: String,
    body: String,
    detail: String,
) {
    run.recent_events.push(NotificationEventDebugInfo {
        at_epoch: epoch_secs(),
        source: source.to_string(),
        status: status.to_string(),
        title,
        body,
        detail,
    });
    if run.recent_events.len() > 12 {
        let drop_count = run.recent_events.len() - 12;
        run.recent_events.drain(0..drop_count);
    }
}

pub(in crate::notifier) fn finish_sync_debug(
    app: &AppHandle,
    run: SyncRunDebug,
    status: String,
    error: String,
) {
    if let Ok(mut debug) = app.state::<NotificationPollState>().debug.lock() {
        debug.last_sync = NotificationLastSyncDebugInfo {
            started_at_epoch: Some(run.started_at_epoch),
            finished_at_epoch: Some(epoch_secs()),
            status,
            error,
            bootstrap_mode: run.bootstrap_mode,
            suppress_push: run.suppress_push,
            dispatched: run.dispatched,
            failed: run.failed,
            suppressed: run.suppressed,
            muted: run.muted,
            seeded_sources: run.seeded_sources,
            fetch_failures: run.fetch_failures,
        };
        for event in run.recent_events {
            debug.recent_events.push(event);
        }
        if debug.recent_events.len() > 20 {
            let drop_count = debug.recent_events.len() - 20;
            debug.recent_events.drain(0..drop_count);
        }
    }
}

pub(in crate::notifier) fn delivery_note() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macOS: dispatched means UserNotifications accepted the request; OS display happens asynchronously."
    }
    #[cfg(not(target_os = "macos"))]
    {
        "dispatched means the platform notification API accepted the request."
    }
}

pub(in crate::notifier) fn seed_seen_state(
    db: &Database,
    source: &str,
    mut seen_ids: Vec<String>,
    mut seen_set: HashSet<String>,
    current_ids: Vec<String>,
    run: &mut SyncRunDebug,
) {
    let seeded_count = current_ids.len();
    if seeded_count == 0 {
        log::info!(
            "notification sync: deferring baseline init for {} because snapshot is empty",
            source
        );
        record_event(
            run,
            source,
            "deferred",
            format!("empty snapshot for {}", source),
            String::new(),
            "baseline init deferred".to_string(),
        );
        return;
    }
    extend_seen_ids(&mut seen_ids, &mut seen_set, current_ids);
    crate::read_state::save_seen_notif_ids(db, source, seen_ids);
    crate::read_state::mark_seen_notif_initialized(db, source);
    log::info!(
        "notification sync: seeded seen-state baseline for {}",
        source
    );
    run.seeded_sources
        .push(format!("{}({})", source, seeded_count));
    record_event(
        run,
        source,
        "seeded",
        format!("baseline seeded for {}", source),
        format!("{} items", seeded_count),
        "first_sync_baseline".to_string(),
    );
}

pub(in crate::notifier) fn seed_luna_seen_state(
    db: &Database,
    mut seen_ids: Vec<String>,
    mut seen_set: HashSet<String>,
    items: &[crate::luna_parser::LunaNotification],
    run: &mut SyncRunDebug,
    event_detail: &str,
) {
    if items.is_empty() {
        log::info!("notification sync: deferring baseline init for luna because snapshot is empty");
        record_event(
            run,
            "luna",
            "deferred",
            "empty snapshot for luna".to_string(),
            String::new(),
            "baseline init deferred".to_string(),
        );
        return;
    }

    let mut object_entries: Vec<LunaNotifSeenEntry> = Vec::new();
    for item in items {
        let base_key = luna_base_key(item);
        let revision_key = luna_revision_key(item);
        luna_upsert_revision(&mut object_entries, base_key, revision_key.clone());
        if seen_set.insert(revision_key.clone()) {
            seen_ids.push(revision_key);
        }
    }

    crate::read_state::save_luna_notif_seen_entries(db, object_entries);
    crate::read_state::save_seen_notif_ids(db, "luna", seen_ids);
    crate::read_state::mark_seen_notif_initialized(db, "luna");
    log::info!("notification sync: seeded luna object revision baseline");
    run.seeded_sources.push(format!("luna({})", items.len()));
    record_event(
        run,
        "luna",
        "seeded",
        "baseline seeded for luna".to_string(),
        format!("{} items", items.len()),
        event_detail.to_string(),
    );
}

pub(in crate::notifier) fn should_recover_empty_initialized(
    initialized: bool,
    seen_ids: &[String],
    current_ids: &[String],
) -> bool {
    initialized && seen_ids.is_empty() && !current_ids.is_empty()
}

pub(in crate::notifier) fn luna_previous_revision(
    entries: &[LunaNotifSeenEntry],
    base_key: &str,
) -> Option<String> {
    entries
        .iter()
        .find(|entry| entry.base_key == base_key)
        .map(|entry| entry.revision_key.clone())
}

pub(in crate::notifier) fn rebuild_luna_object_entries_from_seen_set(
    items: &[crate::luna_parser::LunaNotification],
    seen_set: &HashSet<String>,
) -> Vec<LunaNotifSeenEntry> {
    let mut entries = Vec::new();
    for item in items {
        let revision_key = luna_revision_key(item);
        if !seen_set.contains(&revision_key) {
            continue;
        }
        luna_upsert_revision(&mut entries, luna_base_key(item), revision_key);
    }
    entries
}

pub(in crate::notifier) fn luna_upsert_revision(
    entries: &mut Vec<LunaNotifSeenEntry>,
    base_key: String,
    revision_key: String,
) {
    if let Some(index) = entries.iter().position(|entry| entry.base_key == base_key) {
        entries.remove(index);
    }
    entries.push(LunaNotifSeenEntry {
        base_key,
        revision_key,
    });
}

pub(in crate::notifier) fn extend_seen_ids(
    seen_ids: &mut Vec<String>,
    seen_set: &mut HashSet<String>,
    current_ids: Vec<String>,
) {
    for id in current_ids {
        if !id.is_empty() && seen_set.insert(id.clone()) {
            seen_ids.push(id);
        }
    }
}

pub(in crate::notifier) fn luna_revision_key(
    item: &crate::luna_parser::LunaNotification,
) -> String {
    // Strip the volatile trailing "(YYYY/MM/DD HH:MM)" timestamp that LUNA
    // appends to some notification bodies. It can change between polls without
    // the item being genuinely updated, which otherwise makes the revision key
    // drift and re-fires the same item as an "[更新]" notification every cycle.
    // The action suffix (追加/更新/提出…) is deliberately kept so real updates
    // still produce a new revision.
    format!(
        "{}|{}|{}",
        normalize_luna_key_part(&item.date),
        normalize_luna_key_part(&item.course_info),
        normalize_luna_key_part(&strip_trailing_luna_timestamp(&item.content))
    )
}

pub(in crate::notifier) fn strip_trailing_luna_timestamp(content: &str) -> String {
    let trimmed = content.trim_end();
    if trimmed.ends_with(')') {
        if let Some(open) = trimmed.rfind('(') {
            let inner = &trimmed[open + 1..trimmed.len() - 1];
            if looks_like_luna_timestamp(inner) {
                return trimmed[..open].trim_end().to_string();
            }
        }
    }
    content.to_string()
}

pub(in crate::notifier) fn luna_base_key(item: &crate::luna_parser::LunaNotification) -> String {
    let course = normalize_luna_key_part(&item.course_info);
    let module = normalize_luna_key_part(&item.module);
    let idnumber = normalize_luna_key_part(&item.idnumber);
    let url_identity = luna_url_identity(&item.url);
    let subject = normalize_luna_notification_subject(&item.content);

    if let Some(url_identity) = url_identity {
        format!("{}|{}|{}|{}", idnumber, course, module, url_identity)
    } else if !subject.is_empty() {
        format!("{}|{}|{}|{}", idnumber, course, module, subject)
    } else {
        format!(
            "{}|{}|{}|{}",
            idnumber,
            course,
            module,
            normalize_luna_key_part(&item.content)
        )
    }
}

pub(in crate::notifier) fn luna_url_identity(raw_url: &str) -> Option<String> {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return None;
    }

    let without_origin = trimmed
        .strip_prefix("https://luna.kwansei.ac.jp")
        .or_else(|| trimmed.strip_prefix("http://luna.kwansei.ac.jp"))
        .unwrap_or(trimmed);
    let (path_and_query, fragment) = without_origin
        .split_once('#')
        .map(|(path, fragment)| (path, Some(fragment)))
        .unwrap_or((without_origin, None));
    let (path, query) = path_and_query
        .split_once('?')
        .map(|(path, query)| (path, Some(query)))
        .unwrap_or((path_and_query, None));

    let path = normalize_luna_key_part(path);
    let interesting_keys = [
        "informationId",
        "reportId",
        "surveyId",
        "forumId",
        "threadId",
        "attendanceId",
        "contentId",
        "resourceId",
        "materialId",
        "questionnaireId",
        "examinationId",
    ];
    let mut parts = Vec::new();
    let mut matched_object_key = false;
    if let Some(query) = query {
        for key in interesting_keys {
            if let Some(value) = extract_query_param(query, key) {
                if !path.is_empty() && parts.is_empty() {
                    parts.push(path.clone());
                }
                parts.push(format!("{}={}", key, normalize_luna_key_part(&value)));
                matched_object_key = true;
            }
        }
    }
    if matched_object_key {
        if let Some(fragment) = fragment {
            let fragment = normalize_luna_key_part(fragment);
            if !fragment.is_empty() {
                parts.push(format!("#{}", fragment));
            }
        }
    }

    if !matched_object_key {
        return None;
    }

    if let Some(fragment) = fragment {
        let fragment = normalize_luna_key_part(fragment);
        if !fragment.is_empty() {
            let marker = format!("#{}", fragment);
            if !parts.contains(&marker) {
                parts.push(marker);
            }
        }
    }

    (!parts.is_empty()).then(|| parts.join("|"))
}

pub(in crate::notifier) fn extract_query_param(query: &str, target_key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (key == target_key).then(|| value.to_string())
    })
}

pub(in crate::notifier) fn normalize_luna_notification_subject(content: &str) -> String {
    let mut subject = content.trim().trim_start_matches('・').trim().to_string();

    if let Some((prefix, _)) = subject.rsplit_once("が更新されました。") {
        subject = prefix.trim().to_string();
    } else if let Some((prefix, _)) = subject.rsplit_once("が追加されました。") {
        subject = prefix.trim().to_string();
    } else if let Some((prefix, _)) = subject.rsplit_once("を提出しました。") {
        subject = prefix.trim().to_string();
    } else if let Some((prefix, _)) = subject.rsplit_once("で解答しました。") {
        subject = prefix.trim().to_string();
    } else if let Some((prefix, _)) = subject.rsplit_once("が削除されました。") {
        subject = prefix.trim().to_string();
    }

    if let Some(index) = subject.rfind(")(") {
        if subject.ends_with(')') {
            let tail = &subject[index + 2..subject.len() - 1];
            if looks_like_luna_timestamp(tail) {
                subject.truncate(index + 1);
            }
        }
    }

    normalize_luna_key_part(&subject)
}

pub(in crate::notifier) fn looks_like_luna_timestamp(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.len() >= 10
        && trimmed.contains('/')
        && trimmed.contains(':')
        && trimmed
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '/' | ':' | ' ' | '(' | ')' | '　'))
}

pub(in crate::notifier) fn normalize_luna_key_part(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

pub(in crate::notifier) fn classify_course_notification(module: &str) -> CourseNotificationKind {
    let normalized = module.trim().to_lowercase();
    if normalized.is_empty() {
        return CourseNotificationKind::General;
    }
    if normalized.contains("掲示板")
        || normalized.contains("ディスカッション")
        || normalized.contains("フォーラム")
        || normalized.contains("forum")
        || normalized.contains("discussion")
        || normalized.contains("comment")
        || normalized.contains("返信")
    {
        return CourseNotificationKind::Discussion;
    }
    if normalized.contains("アンケート")
        || normalized.contains("survey")
        || normalized.contains("questionnaire")
    {
        return CourseNotificationKind::Survey;
    }
    if normalized.contains("出席")
        || normalized.contains("出欠")
        || normalized.contains("attendance")
    {
        return CourseNotificationKind::Attendance;
    }
    if normalized.contains("小テスト")
        || normalized.contains("テスト")
        || normalized.contains("試験")
        || normalized.contains("examination")
        || normalized.contains("exam")
        || normalized.contains("quiz")
    {
        return CourseNotificationKind::Exam;
    }
    if normalized.contains("課題")
        || normalized.contains("レポート")
        || normalized.contains("assignment")
        || normalized.contains("report")
        || normalized.contains("提出")
    {
        return CourseNotificationKind::Assignment;
    }
    if normalized.contains("お知らせ")
        || normalized.contains("資料")
        || normalized.contains("教材")
        || normalized.contains("information")
        || normalized.contains("announcement")
        || normalized.contains("material")
        || normalized.contains("連絡")
    {
        return CourseNotificationKind::Announcement;
    }
    CourseNotificationKind::General
}

pub(in crate::notifier) fn course_notification_allowed(
    kind: CourseNotificationKind,
    cfg: &NotificationConfig,
) -> bool {
    if !cfg.notify_class {
        return false;
    }
    match kind {
        CourseNotificationKind::General => cfg.notify_class_general,
        CourseNotificationKind::Announcement => cfg.notify_class_announcement,
        CourseNotificationKind::Assignment => cfg.notify_class_assignment,
        CourseNotificationKind::Exam => cfg.notify_class_exam,
        CourseNotificationKind::Discussion => cfg.notify_class_discussion,
        CourseNotificationKind::Survey => cfg.notify_class_survey,
        CourseNotificationKind::Attendance => cfg.notify_class_attendance,
    }
}

pub(in crate::notifier) fn kwic_section_allowed(section: &str, cfg: &NotificationConfig) -> bool {
    match section {
        "呼出し・重要なお知らせ" => cfg.notify_important,
        "学部・研究科からのお知らせ" => cfg.notify_faculty,
        "授業のお知らせ" => {
            course_notification_allowed(CourseNotificationKind::General, cfg)
        }
        _ => cfg.notify_other,
    }
}

pub(in crate::notifier) fn epoch_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
