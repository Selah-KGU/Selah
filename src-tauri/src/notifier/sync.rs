//! Notification poll loop and source refresh orchestration.

use super::*;

pub fn start_notification_loop(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(INITIAL_SYNC_DELAY).await;

        if let Err(e) = sync_notifications_now(&app).await {
            log::warn!("notification sync failed: {}", e);
        }

        // Adaptive backoff: on consecutive failures, sleep longer between
        // attempts (5min → 10 → 20 → 40min, capped). Resets on first success.
        // Hidden windows skip polls, but still refresh about once per 30 minutes.
        // A visible but unfocused window keeps this tick; fast sources stretch
        // to 15 minutes in the due check, and focus catch-up fills the gap.
        let mut consecutive_failures: u32 = 0;
        let mut hidden_streak: u32 = 0;
        loop {
            let delay = match consecutive_failures {
                0 => POLL_INTERVAL,
                1 => POLL_INTERVAL * 2,
                2 => POLL_INTERVAL * 4,
                _ => POLL_INTERVAL * 8,
            };
            tokio::time::sleep(delay).await;
            let visible = crate::background_refresh::is_main_window_visible(&app);
            if !visible && hidden_streak < HIDDEN_SKIP_MAX {
                hidden_streak = hidden_streak.saturating_add(1);
                continue;
            }
            hidden_streak = 0;
            match sync_notifications_now(&app).await {
                Ok(_) => {
                    consecutive_failures = 0;
                }
                Err(e) => {
                    consecutive_failures = consecutive_failures.saturating_add(1);
                    log::warn!(
                        "notification sync failed (attempt {}): {}",
                        consecutive_failures,
                        e
                    );
                }
            }
        }
    });
}

pub async fn debug_snapshot(app: &AppHandle) -> NotificationDebugInfo {
    let (kgc_authenticated, luna_authenticated, kwic_authenticated, mail_authenticated) = tokio::join!(
        is_kgc_authenticated(app),
        is_luna_authenticated(app),
        is_kwic_authenticated(app),
        is_mail_authenticated(app)
    );
    let db = app.state::<Database>();
    let authenticated_sources: Vec<&str> = [
        ("kgc", kgc_authenticated),
        ("luna", luna_authenticated),
        ("kwic", kwic_authenticated),
        ("mail", mail_authenticated),
    ]
    .into_iter()
    .filter_map(|(source, authenticated)| authenticated.then_some(source))
    .collect();
    let bootstrap_state = evaluate_bootstrap_state(&db, &authenticated_sources);
    let started_at = crate::read_state::get_seen_notif_bootstrap_started_at(&db);
    let now = epoch_secs();

    let sources = ["kgc", "luna", "kwic", "mail"]
        .into_iter()
        .map(|source| NotificationSourceDebugInfo {
            source: source.to_string(),
            authenticated: authenticated_sources.contains(&source),
            initialized: crate::read_state::is_seen_notif_initialized(&db, source),
            has_seen_state: crate::read_state::has_seen_notif_state(&db, source),
            seen_count: crate::read_state::get_seen_notif_ids(&db, source).len(),
        })
        .collect();
    let (last_sync, recent_events) = app
        .state::<NotificationPollState>()
        .debug
        .lock()
        .map(|state| (state.last_sync.clone(), state.recent_events.clone()))
        .unwrap_or_default();

    NotificationDebugInfo {
        poll_running: app.state::<NotificationPollState>().is_running(),
        delivery_note: delivery_note().to_string(),
        bootstrap_mode: bootstrap_mode_label(bootstrap_state.mode).to_string(),
        suppress_push: !matches!(bootstrap_state.mode, BootstrapMode::Normal),
        bootstrap_complete: crate::read_state::is_seen_notif_bootstrap_complete(&db),
        bootstrap_started_at_epoch: started_at,
        bootstrap_started_ago_secs: started_at.map(|value| now.saturating_sub(value)),
        grace_period_secs: BOOTSTRAP_GRACE_PERIOD.as_secs(),
        authenticated_sources: authenticated_sources
            .into_iter()
            .map(str::to_string)
            .collect(),
        sources,
        last_sync,
        recent_events,
    }
}

#[tauri::command]
pub async fn notification_sync_now(app: AppHandle) -> Result<(), String> {
    sync_notification_sources(&app, None, true)
        .await
        .map(|_| ())
}

pub async fn sync_notifications_now(app: &AppHandle) -> Result<Vec<String>, String> {
    sync_notification_sources(app, None, false).await
}

pub async fn sync_notification_sources(
    app: &AppHandle,
    keys: Option<&std::collections::BTreeSet<String>>,
    force: bool,
) -> Result<Vec<String>, String> {
    let state = app.state::<NotificationPollState>();
    if state.running.swap(true, Ordering::SeqCst) {
        return Ok(Vec::new());
    }

    struct RunningGuard<'a>(&'a AtomicBool);
    impl Drop for RunningGuard<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }
    let _guard = RunningGuard(&state.running);

    sync_notifications_inner(app, keys, force).await
}

fn source_wanted(keys: Option<&std::collections::BTreeSet<String>>, key: &str) -> bool {
    keys.map(|set| set.contains(key)).unwrap_or(true)
}

pub(in crate::notifier) fn cache_refresh_due(
    updated_at: Option<i64>,
    now: i64,
    max_age_secs: i64,
    force: bool,
) -> bool {
    if force {
        return true;
    }
    match updated_at {
        Some(ts) => now.saturating_sub(ts) >= max_age_secs,
        None => true,
    }
}

pub(in crate::notifier) fn notifications_json_is_empty(json: Option<&str>) -> bool {
    let Some(json) = json else {
        return true;
    };
    serde_json::from_str::<NotificationsData>(json)
        .map(|data| data.entries.is_empty())
        .unwrap_or(true)
}

/// An empty KGC notice cache is not proof the campus has nothing to show.
/// Retry on the fast interval instead of waiting out the 12 hour stable age.
pub(in crate::notifier) fn kgc_notification_max_age(payload_empty: bool, fast_max_age: i64) -> i64 {
    if payload_empty {
        fast_max_age
    } else {
        KGC_NOTIFICATION_MAX_AGE_SECS
    }
}

async fn sync_notifications_inner(
    app: &AppHandle,
    keys: Option<&std::collections::BTreeSet<String>>,
    force: bool,
) -> Result<Vec<String>, String> {
    let cfg = commands::load_notification_config();
    let (mut kgc_authenticated, luna_authenticated, kwic_authenticated, mail_authenticated) = tokio::join!(
        is_kgc_authenticated(app),
        is_luna_authenticated(app),
        is_kwic_authenticated(app),
        is_mail_authenticated(app)
    );
    let db = app.state::<Database>();
    let now = epoch_secs();
    let fast_max_age = crate::background_refresh::fast_cache_max_age_secs(
        crate::background_refresh::is_app_focused(app),
    );
    let notifications_empty =
        notifications_json_is_empty(db.cache_payload("notifications").as_deref());
    let kgc_notifications_due = source_wanted(keys, "notifications")
        && cache_refresh_due(
            cache_updated_at(&db, "notifications"),
            now,
            kgc_notification_max_age(notifications_empty, fast_max_age),
            force,
        );
    let luna_updates_due = source_wanted(keys, "luna_updates")
        && cache_refresh_due(
            cache_updated_at(&db, "luna_updates"),
            now,
            fast_max_age,
            force,
        );
    let kwic_home_due = source_wanted(keys, "kwic_home")
        && cache_refresh_due(cache_updated_at(&db, "kwic_home"), now, fast_max_age, force);
    let mail_inbox_due = source_wanted(keys, "mail_inbox")
        && cache_refresh_due(
            cache_updated_at(&db, "mail_inbox"),
            now,
            fast_max_age,
            force,
        );
    if kgc_notifications_due {
        kgc_authenticated = crate::commands::ensure_kgc_session_for_automatic_request(app).await;
    }

    let stored_version = crate::read_state::get_seen_notif_format_version(&db);
    if stored_version != crate::read_state::CURRENT_SEEN_NOTIF_FORMAT_VERSION {
        log::info!(
            "notification sync: seen-state format upgraded ({} -> {}), resetting baseline",
            stored_version,
            crate::read_state::CURRENT_SEEN_NOTIF_FORMAT_VERSION
        );
        crate::read_state::reset_all_seen_notif_state(&db);
        crate::read_state::mark_seen_notif_format_version(
            &db,
            crate::read_state::CURRENT_SEEN_NOTIF_FORMAT_VERSION,
        );
    }

    let authenticated_sources: Vec<&str> = [
        ("kgc", kgc_authenticated),
        ("luna", luna_authenticated),
        ("kwic", kwic_authenticated),
        ("mail", mail_authenticated),
    ]
    .into_iter()
    .filter_map(|(source, authenticated)| authenticated.then_some(source))
    .collect();
    let bootstrap_state = resolve_bootstrap_state(&db, &authenticated_sources);
    let bootstrap_mode = bootstrap_state.mode;
    let suppress_push = !matches!(bootstrap_mode, BootstrapMode::Normal);
    let mut run = SyncRunDebug {
        started_at_epoch: epoch_secs(),
        bootstrap_mode: bootstrap_mode_label(bootstrap_mode).to_string(),
        suppress_push,
        ..Default::default()
    };
    let mut updated_keys = Vec::new();

    // A fresh cache only skips the network fetch. Page and agent writers can
    // store the payload without diffing seen ids; touching updated_at must not
    // hide those items until the max age elapses.
    if kgc_authenticated && source_wanted(keys, "notifications") {
        if kgc_notifications_due {
            let before = db.cache_payload("notifications");
            match fetch_kgc_notifications(app).await {
                Ok(data) => {
                    sync_kgc_notifications(app, &cfg, data, suppress_push, &mut run);
                    if db.cache_payload("notifications") != before {
                        updated_keys.push("notifications".to_string());
                    }
                }
                Err(e) => {
                    log::warn!("notification sync: kgc fetch failed: {}", e);
                    run.fetch_failures.push(format!("kgc: {}", e));
                    sync_cached_notifications(&db, "notifications", |data: NotificationsData| {
                        sync_kgc_notifications(app, &cfg, data, suppress_push, &mut run);
                    });
                }
            }
        } else {
            sync_cached_notifications(&db, "notifications", |data: NotificationsData| {
                sync_kgc_notifications(app, &cfg, data, suppress_push, &mut run);
            });
        }
    }

    if luna_authenticated && source_wanted(keys, "luna_updates") {
        if luna_updates_due {
            let before = db.cache_payload("luna_updates");
            match fetch_luna_notifications(app).await {
                Ok(items) => {
                    sync_luna_notifications(app, &cfg, items, suppress_push, &mut run);
                    if db.cache_payload("luna_updates") != before {
                        updated_keys.push("luna_updates".to_string());
                    }
                }
                Err(e) => {
                    log::warn!("notification sync: luna fetch failed: {}", e);
                    run.fetch_failures.push(format!("luna: {}", e));
                    sync_cached_notifications(
                        &db,
                        "luna_updates",
                        |items: Vec<crate::luna_parser::LunaNotification>| {
                            sync_luna_notifications(app, &cfg, items, suppress_push, &mut run);
                        },
                    );
                }
            }
        } else {
            sync_cached_notifications(
                &db,
                "luna_updates",
                |items: Vec<crate::luna_parser::LunaNotification>| {
                    sync_luna_notifications(app, &cfg, items, suppress_push, &mut run);
                },
            );
        }
    }

    if kwic_authenticated && source_wanted(keys, "kwic_home") {
        if kwic_home_due {
            let before = db.cache_payload("kwic_home");
            match fetch_kwic_home(app).await {
                Ok(home) => {
                    sync_kwic_notifications(app, &cfg, home, suppress_push, &mut run);
                    if db.cache_payload("kwic_home") != before {
                        updated_keys.push("kwic_home".to_string());
                    }
                }
                Err(e) => {
                    log::warn!("notification sync: kwic fetch failed: {}", e);
                    run.fetch_failures.push(format!("kwic: {}", e));
                    sync_cached_notifications(&db, "kwic_home", |home: KwicPortalHome| {
                        sync_kwic_notifications(app, &cfg, home, suppress_push, &mut run);
                    });
                }
            }
        } else {
            sync_cached_notifications(&db, "kwic_home", |home: KwicPortalHome| {
                sync_kwic_notifications(app, &cfg, home, suppress_push, &mut run);
            });
        }
    }

    if mail_authenticated && source_wanted(keys, "mail_inbox") {
        if mail_inbox_due {
            let before = db.cache_payload("mail_inbox");
            match crate::mail_commands::fetch_inbox_internal(app, 20, 0).await {
                Ok(items) => {
                    sync_mail_notifications(app, &cfg, items, suppress_push, &mut run);
                    if db.cache_payload("mail_inbox") != before {
                        updated_keys.push("mail_inbox".to_string());
                    }
                }
                Err(e) => {
                    log::warn!("notification sync: mail fetch failed: {}", e);
                    run.fetch_failures.push(format!("mail: {}", e));
                    sync_cached_notifications(&db, "mail_inbox", |items: Vec<MailMessage>| {
                        sync_mail_notifications(app, &cfg, items, suppress_push, &mut run);
                    });
                }
            }
        } else {
            sync_cached_notifications(&db, "mail_inbox", |items: Vec<MailMessage>| {
                sync_mail_notifications(app, &cfg, items, suppress_push, &mut run);
            });
        }
    }

    if matches!(bootstrap_mode, BootstrapMode::Finalize) {
        crate::read_state::mark_seen_notif_bootstrap_complete(&db);
        log::info!("notification sync: initial bootstrap completed");
    }

    if !updated_keys.is_empty() {
        background_refresh::emit_cache_updates(app, updated_keys.clone());
    }

    let status = if run.failed > 0 || !run.fetch_failures.is_empty() {
        "partial_error".to_string()
    } else {
        "ok".to_string()
    };
    let mut error_parts = Vec::new();
    if !run.fetch_failures.is_empty() {
        error_parts.push(run.fetch_failures.join(" | "));
    }
    if run.failed > 0 {
        error_parts.push(format!("dispatch failures: {}", run.failed));
    }
    let error = error_parts.join(" | ");
    finish_sync_debug(app, run, status, error);
    db.checkpoint_passive();
    Ok(updated_keys)
}

fn sync_cached_notifications<T, F>(db: &Database, key: &str, sync: F)
where
    T: serde::de::DeserializeOwned,
    F: FnOnce(T),
{
    let Some(json) = db.cache_payload(key) else {
        return;
    };
    match serde_json::from_str::<T>(&json) {
        Ok(value) => sync(value),
        Err(err) => log::warn!("notification sync: cached {key} is unreadable: {err}"),
    }
}

async fn fetch_kgc_notifications(app: &AppHandle) -> Result<NotificationsData, String> {
    crate::commands::fetch_notifications(app.state::<KgcState>(), app.state::<Database>()).await
}

fn cache_updated_at(db: &Database, key: &str) -> Option<i64> {
    db.cache_updated_at(key)
}

async fn fetch_luna_notifications(
    app: &AppHandle,
) -> Result<Vec<crate::luna_parser::LunaNotification>, String> {
    crate::luna_commands::luna_fetch_updates(app.state::<LunaState>(), app.state::<Database>())
        .await
}

async fn fetch_kwic_home(app: &AppHandle) -> Result<KwicPortalHome, String> {
    crate::kwic_commands::kwic_fetch_home(app.state::<KwicState>(), app.state::<Database>()).await
}

async fn is_kgc_authenticated(app: &AppHandle) -> bool {
    app.state::<KgcState>()
        .client
        .lock()
        .await
        .is_authenticated()
}

async fn is_luna_authenticated(app: &AppHandle) -> bool {
    app.state::<LunaState>().client.lock().await.authenticated
}

async fn is_kwic_authenticated(app: &AppHandle) -> bool {
    app.state::<KwicState>().client.lock().await.authenticated
}

async fn is_mail_authenticated(app: &AppHandle) -> bool {
    app.state::<MailState>()
        .client
        .lock()
        .await
        .is_authenticated()
}
