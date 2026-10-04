//! Per-source notification diff and dispatch.

use super::*;

pub(in crate::notifier) fn sync_kgc_notifications(
    app: &AppHandle,
    cfg: &NotificationConfig,
    data: NotificationsData,
    suppress_push: bool,
    run: &mut SyncRunDebug,
) {
    let source = "kgc";
    let db = app.state::<Database>();
    let current_ids: Vec<String> = data
        .entries
        .iter()
        .filter_map(|item| (!item.id.is_empty()).then_some(item.id.clone()))
        .collect();
    let (initialized, mut seen_ids, mut seen_set) = load_seen_state(&db, source);

    if should_recover_empty_initialized(initialized, &seen_ids, &current_ids) {
        log::warn!(
            "notification sync: recovering empty initialized state for {}",
            source
        );
        seed_seen_state(&db, source, seen_ids, seen_set, current_ids, run);
        return;
    }

    if !initialized {
        seed_seen_state(&db, source, seen_ids, seen_set, current_ids, run);
        return;
    }

    let mut batch_seen: HashSet<String> = HashSet::new();
    let new_entries: Vec<_> = data
        .entries
        .iter()
        .filter(|item| {
            !item.id.is_empty()
                && !seen_set.contains(&item.id)
                && batch_seen.insert(item.id.clone())
        })
        .collect();

    if suppress_push {
        run.suppressed += new_entries.len();
        for item in &new_entries {
            record_event(
                run,
                source,
                "suppressed",
                item.title.clone(),
                item.date.clone(),
                "bootstrap_silent".to_string(),
            );
        }
    } else if course_notification_allowed(CourseNotificationKind::General, cfg) {
        for item in &new_entries {
            let title = if item.category.is_empty() {
                item.title.clone()
            } else {
                format!("[{}] {}", item.category, item.title)
            };
            dispatch_notification(
                app,
                run,
                source,
                title,
                item.date.clone(),
                Some(NotificationClickTarget {
                    source: source.to_string(),
                    id: item.id.clone(),
                    title: item.title.clone(),
                    date: item.date.clone(),
                    category: item.category.clone(),
                    tab: Some("授業のお知らせ".to_string()),
                    ..Default::default()
                }),
            );
        }
    } else {
        run.muted += new_entries.len();
    }

    extend_seen_ids(&mut seen_ids, &mut seen_set, current_ids);
    crate::read_state::save_seen_notif_ids(&db, source, seen_ids);
    crate::read_state::mark_seen_notif_initialized(&db, source);
}

pub(in crate::notifier) fn sync_luna_notifications(
    app: &AppHandle,
    cfg: &NotificationConfig,
    items: Vec<crate::luna_parser::LunaNotification>,
    suppress_push: bool,
    run: &mut SyncRunDebug,
) {
    let source = "luna";
    let db = app.state::<Database>();
    let current_ids: Vec<String> = items.iter().map(luna_revision_key).collect();
    let (initialized, mut seen_ids, mut seen_set) = load_seen_state(&db, source);
    let mut object_entries = crate::read_state::get_luna_notif_seen_entries(&db);

    if should_recover_empty_initialized(initialized, &seen_ids, &current_ids) {
        log::warn!(
            "notification sync: recovering empty initialized state for {}",
            source
        );
        seed_luna_seen_state(&db, seen_ids, seen_set, &items, run, "recovered_empty_init");
        return;
    }

    if !initialized {
        seed_luna_seen_state(&db, seen_ids, seen_set, &items, run, "first_sync_baseline");
        return;
    }

    if initialized && !items.is_empty() && object_entries.is_empty() {
        let migrated = rebuild_luna_object_entries_from_seen_set(&items, &seen_set);
        if !migrated.is_empty() {
            log::info!(
                "notification sync: migrated {} luna object revisions from legacy seen ids",
                migrated.len()
            );
            record_event(
                run,
                source,
                "migrated",
                "rebuilt luna object revisions from legacy seen ids".to_string(),
                format!("{} matched items", migrated.len()),
                "legacy_seen_intersection".to_string(),
            );
            crate::read_state::save_luna_notif_seen_entries(&db, migrated.clone());
            object_entries = migrated;
        } else {
            log::info!(
                "notification sync: no luna object revisions could be rebuilt from legacy seen ids"
            );
        }
    }

    for item in &items {
        let base_key = luna_base_key(item);
        let revision_key = luna_revision_key(item);
        if seen_set.contains(&revision_key) {
            luna_upsert_revision(&mut object_entries, base_key, revision_key);
            continue;
        }
        let previous_revision = luna_previous_revision(&object_entries, &base_key);
        if previous_revision.as_deref() == Some(revision_key.as_str()) {
            continue;
        }
        let is_update = previous_revision.is_some();
        if suppress_push {
            run.suppressed += 1;
            record_event(
                run,
                source,
                "suppressed",
                item.content.clone(),
                item.date.clone(),
                if is_update {
                    "bootstrap_silent_update".to_string()
                } else {
                    "bootstrap_silent_new".to_string()
                },
            );
        } else if course_notification_allowed(classify_course_notification(&item.module), cfg) {
            let base_title = if item.module.is_empty() {
                item.content.clone()
            } else {
                format!("[{}] {}", item.module, item.content)
            };
            let title = if is_update {
                format!("[更新] {}", base_title)
            } else {
                base_title
            };
            let body = format!("{} — {}", item.course_info, item.date);
            dispatch_notification(
                app,
                run,
                source,
                title,
                body,
                Some(NotificationClickTarget {
                    source: source.to_string(),
                    id: revision_key.clone(),
                    title: item.content.clone(),
                    date: item.date.clone(),
                    category: item.module.clone(),
                    tab: Some("授業のお知らせ".to_string()),
                    url: (!item.url.is_empty()).then(|| item.url.clone()),
                    course_info: (!item.course_info.is_empty()).then(|| item.course_info.clone()),
                    ..Default::default()
                }),
            );
        } else {
            run.muted += 1;
        }
        luna_upsert_revision(&mut object_entries, base_key, revision_key.clone());
        if seen_set.insert(revision_key.clone()) {
            seen_ids.push(revision_key);
        }
    }

    for revision_key in current_ids {
        if seen_set.insert(revision_key.clone()) {
            seen_ids.push(revision_key);
        }
    }
    crate::read_state::save_luna_notif_seen_entries(&db, object_entries);
    crate::read_state::save_seen_notif_ids(&db, source, seen_ids);
    crate::read_state::mark_seen_notif_initialized(&db, source);
}

pub(in crate::notifier) fn sync_kwic_notifications(
    app: &AppHandle,
    cfg: &NotificationConfig,
    home: KwicPortalHome,
    suppress_push: bool,
    run: &mut SyncRunDebug,
) {
    let source = "kwic";
    let db = app.state::<Database>();
    let current_ids: Vec<String> = home
        .sections
        .iter()
        .flat_map(|section| {
            section
                .items
                .iter()
                .filter_map(|item| (!item.id.is_empty()).then_some(item.id.clone()))
        })
        .collect();
    let (initialized, mut seen_ids, mut seen_set) = load_seen_state(&db, source);

    if should_recover_empty_initialized(initialized, &seen_ids, &current_ids) {
        log::warn!(
            "notification sync: recovering empty initialized state for {}",
            source
        );
        seed_seen_state(&db, source, seen_ids, seen_set, current_ids, run);
        return;
    }

    if !initialized {
        seed_seen_state(&db, source, seen_ids, seen_set, current_ids, run);
        return;
    }

    for section in &home.sections {
        for item in &section.items {
            if item.id.is_empty() || !seen_set.insert(item.id.clone()) {
                continue;
            }
            seen_ids.push(item.id.clone());
            if suppress_push {
                run.suppressed += 1;
                record_event(
                    run,
                    source,
                    "suppressed",
                    item.title.clone(),
                    item.date.clone(),
                    "bootstrap_silent".to_string(),
                );
            } else if kwic_section_allowed(&section.title, cfg) {
                let title = if item.category.is_empty() {
                    item.title.clone()
                } else {
                    format!("[{}] {}", item.category, item.title)
                };
                dispatch_notification(
                    app,
                    run,
                    source,
                    title,
                    item.date.clone(),
                    Some(NotificationClickTarget {
                        source: source.to_string(),
                        id: item.id.clone(),
                        title: item.title.clone(),
                        date: item.date.clone(),
                        category: item.category.clone(),
                        tab: Some(section.title.clone()),
                        information_type: (!item.information_type.is_empty())
                            .then(|| item.information_type.clone()),
                        person_category_cd: (!item.person_category_cd.is_empty())
                            .then(|| item.person_category_cd.clone()),
                        category_cd: (!item.category_cd.is_empty())
                            .then(|| item.category_cd.clone()),
                        ..Default::default()
                    }),
                );
            } else {
                run.muted += 1;
            }
        }
    }

    extend_seen_ids(&mut seen_ids, &mut seen_set, current_ids);
    crate::read_state::save_seen_notif_ids(&db, source, seen_ids);
    crate::read_state::mark_seen_notif_initialized(&db, source);
}

pub(in crate::notifier) fn sync_mail_notifications(
    app: &AppHandle,
    cfg: &NotificationConfig,
    items: Vec<MailMessage>,
    suppress_push: bool,
    run: &mut SyncRunDebug,
) {
    let source = "mail";
    let db = app.state::<Database>();
    let current_ids: Vec<String> = items
        .iter()
        .filter_map(|item| (!item.id.is_empty()).then_some(item.id.clone()))
        .collect();
    let (initialized, mut seen_ids, mut seen_set) = load_seen_state(&db, source);

    if should_recover_empty_initialized(initialized, &seen_ids, &current_ids) {
        log::warn!(
            "notification sync: recovering empty initialized state for {}",
            source
        );
        seed_seen_state(&db, source, seen_ids, seen_set, current_ids, run);
        return;
    }

    if !initialized {
        seed_seen_state(&db, source, seen_ids, seen_set, current_ids, run);
        return;
    }

    for item in &items {
        if item.id.is_empty() || item.is_read.unwrap_or(false) {
            continue;
        }
        if !seen_set.insert(item.id.clone()) {
            continue;
        }
        seen_ids.push(item.id.clone());
        if suppress_push {
            run.suppressed += 1;
            record_event(
                run,
                source,
                "suppressed",
                item.subject
                    .clone()
                    .unwrap_or_else(|| "(件名なし)".to_string()),
                item.id.clone(),
                "bootstrap_silent".to_string(),
            );
            continue;
        }
        if cfg.notify_mail {
            let sender = item
                .from
                .as_ref()
                .and_then(|from| {
                    from.email_address
                        .name
                        .clone()
                        .or(from.email_address.address.clone())
                })
                .unwrap_or_else(|| "新着メール".to_string());
            let subject = item
                .subject
                .clone()
                .unwrap_or_else(|| "(件名なし)".to_string());
            dispatch_notification(
                app,
                run,
                source,
                sender.clone(),
                subject.clone(),
                Some(NotificationClickTarget {
                    source: source.to_string(),
                    id: item.id.clone(),
                    title: subject,
                    date: item.received_date_time.clone().unwrap_or_default(),
                    category: sender,
                    tab: Some("その他".to_string()),
                    ..Default::default()
                }),
            );
        } else {
            run.muted += 1;
        }
    }

    extend_seen_ids(&mut seen_ids, &mut seen_set, current_ids);
    crate::read_state::save_seen_notif_ids(&db, source, seen_ids);
    crate::read_state::mark_seen_notif_initialized(&db, source);
}
