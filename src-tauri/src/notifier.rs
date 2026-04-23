use serde::Serialize;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

use crate::background_refresh;
use crate::commands::{self, NotificationConfig};
use crate::db::Database;
use crate::kwic_commands::KwicPortalHome;
use crate::mail::MailMessage;
use crate::parser::NotificationsData;
use crate::read_state::LunaNotifSeenEntry;
use crate::{KgcState, KwicState, LunaState, MailState};

const INITIAL_SYNC_DELAY: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);
const BOOTSTRAP_GRACE_PERIOD: Duration = Duration::from_secs(6 * 60);

pub struct NotificationPollState {
    running: AtomicBool,
    debug: Mutex<NotificationRuntimeDebugState>,
}

impl NotificationPollState {
    pub fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
            debug: Mutex::new(NotificationRuntimeDebugState::default()),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct NotificationEventDebugInfo {
    pub at_epoch: i64,
    pub source: String,
    pub status: String,
    pub title: String,
    pub body: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct NotificationLastSyncDebugInfo {
    pub started_at_epoch: Option<i64>,
    pub finished_at_epoch: Option<i64>,
    pub status: String,
    pub error: String,
    pub bootstrap_mode: String,
    pub suppress_push: bool,
    pub dispatched: usize,
    pub failed: usize,
    pub suppressed: usize,
    pub muted: usize,
    pub seeded_sources: Vec<String>,
    pub fetch_failures: Vec<String>,
}

#[derive(Debug, Default)]
struct NotificationRuntimeDebugState {
    last_sync: NotificationLastSyncDebugInfo,
    recent_events: Vec<NotificationEventDebugInfo>,
}

#[derive(Clone, Copy)]
enum CourseNotificationKind {
    General,
    Announcement,
    Assignment,
    Exam,
    Discussion,
    Survey,
    Attendance,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BootstrapMode {
    Silent,
    Finalize,
    Normal,
}

#[derive(Clone, Copy)]
struct BootstrapState {
    mode: BootstrapMode,
    should_mark_complete: bool,
    should_mark_started_at: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct NotificationSourceDebugInfo {
    pub source: String,
    pub authenticated: bool,
    pub initialized: bool,
    pub has_seen_state: bool,
    pub seen_count: usize,
}

#[derive(Debug, Serialize)]
pub struct NotificationDebugInfo {
    pub poll_running: bool,
    pub delivery_note: String,
    pub bootstrap_mode: String,
    pub suppress_push: bool,
    pub bootstrap_complete: bool,
    pub bootstrap_started_at_epoch: Option<i64>,
    pub bootstrap_started_ago_secs: Option<i64>,
    pub grace_period_secs: u64,
    pub authenticated_sources: Vec<String>,
    pub sources: Vec<NotificationSourceDebugInfo>,
    pub last_sync: NotificationLastSyncDebugInfo,
    pub recent_events: Vec<NotificationEventDebugInfo>,
}

#[derive(Default)]
struct SyncRunDebug {
    started_at_epoch: i64,
    bootstrap_mode: String,
    suppress_push: bool,
    dispatched: usize,
    failed: usize,
    suppressed: usize,
    muted: usize,
    seeded_sources: Vec<String>,
    fetch_failures: Vec<String>,
    recent_events: Vec<NotificationEventDebugInfo>,
}

pub fn start_notification_loop(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(INITIAL_SYNC_DELAY).await;

        if let Err(e) = sync_notifications_now(&app).await {
            log::warn!("notification sync failed: {}", e);
        }

        let mut interval = tokio::time::interval(POLL_INTERVAL);
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(e) = sync_notifications_now(&app).await {
                log::warn!("notification sync failed: {}", e);
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
    sync_notifications_now(&app).await.map(|_| ())
}

pub async fn sync_notifications_now(app: &AppHandle) -> Result<Vec<String>, String> {
    let state = app.state::<NotificationPollState>();
    if state.running.swap(true, Ordering::SeqCst) {
        return Ok(Vec::new());
    }

    let result = sync_notifications_inner(app).await;
    state.running.store(false, Ordering::SeqCst);
    result
}

async fn sync_notifications_inner(app: &AppHandle) -> Result<Vec<String>, String> {
    let cfg = commands::load_notification_config();
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

    if kgc_authenticated {
        match fetch_kgc_notifications(app).await {
            Ok(data) => {
                sync_kgc_notifications(app, &cfg, data, suppress_push, &mut run);
                updated_keys.push("notifications".to_string());
            }
            Err(e) => {
                log::warn!("notification sync: kgc fetch failed: {}", e);
                run.fetch_failures.push(format!("kgc: {}", e));
            }
        }
    }

    if luna_authenticated {
        match fetch_luna_notifications(app).await {
            Ok(items) => {
                sync_luna_notifications(app, &cfg, items, suppress_push, &mut run);
                updated_keys.push("luna_updates".to_string());
            }
            Err(e) => {
                log::warn!("notification sync: luna fetch failed: {}", e);
                run.fetch_failures.push(format!("luna: {}", e));
            }
        }
    }

    if kwic_authenticated {
        match fetch_kwic_home(app).await {
            Ok(home) => {
                sync_kwic_notifications(app, &cfg, home, suppress_push, &mut run);
                updated_keys.push("kwic_home".to_string());
            }
            Err(e) => {
                log::warn!("notification sync: kwic fetch failed: {}", e);
                run.fetch_failures.push(format!("kwic: {}", e));
            }
        }
    }

    if mail_authenticated {
        match crate::mail_commands::fetch_inbox_internal(app, 20, 0).await {
            Ok(items) => {
                sync_mail_notifications(app, &cfg, items, suppress_push, &mut run);
                updated_keys.push("mail_inbox".to_string());
            }
            Err(e) => {
                log::warn!("notification sync: mail fetch failed: {}", e);
                run.fetch_failures.push(format!("mail: {}", e));
            }
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
    Ok(updated_keys)
}

async fn fetch_kgc_notifications(app: &AppHandle) -> Result<NotificationsData, String> {
    crate::commands::fetch_notifications(app.state::<KgcState>(), app.state::<Database>()).await
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

fn sync_kgc_notifications(
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

    let new_entries: Vec<_> = data
        .entries
        .iter()
        .filter(|item| !item.id.is_empty() && !seen_set.contains(&item.id))
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
            dispatch_notification(app, run, source, title, item.date.clone());
        }
    } else {
        run.muted += new_entries.len();
    }

    extend_seen_ids(&mut seen_ids, &mut seen_set, current_ids);
    crate::read_state::save_seen_notif_ids(&db, source, seen_ids);
    crate::read_state::mark_seen_notif_initialized(&db, source);
}

fn sync_luna_notifications(
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
            dispatch_notification(app, run, source, title, body);
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

fn sync_kwic_notifications(
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
            if item.id.is_empty() || seen_set.contains(&item.id) {
                continue;
            }
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
                dispatch_notification(app, run, source, title, item.date.clone());
            } else {
                run.muted += 1;
            }
        }
    }

    extend_seen_ids(&mut seen_ids, &mut seen_set, current_ids);
    crate::read_state::save_seen_notif_ids(&db, source, seen_ids);
    crate::read_state::mark_seen_notif_initialized(&db, source);
}

fn sync_mail_notifications(
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
        if item.id.is_empty() || seen_set.contains(&item.id) || item.is_read.unwrap_or(false) {
            continue;
        }
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
            dispatch_notification(app, run, source, sender, subject);
        } else {
            run.muted += 1;
        }
    }

    extend_seen_ids(&mut seen_ids, &mut seen_set, current_ids);
    crate::read_state::save_seen_notif_ids(&db, source, seen_ids);
    crate::read_state::mark_seen_notif_initialized(&db, source);
}

fn load_seen_state(db: &Database, source: &str) -> (bool, Vec<String>, HashSet<String>) {
    let seen_ids = crate::read_state::get_seen_notif_ids(db, source);
    let seen_set = seen_ids.iter().cloned().collect::<HashSet<_>>();
    let initialized = crate::read_state::is_seen_notif_initialized(db, source);
    (initialized, seen_ids, seen_set)
}

fn evaluate_bootstrap_state(db: &Database, authenticated_sources: &[&str]) -> BootstrapState {
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

fn resolve_bootstrap_state(db: &Database, authenticated_sources: &[&str]) -> BootstrapState {
    let state = evaluate_bootstrap_state(db, authenticated_sources);
    if let Some(started_at) = state.should_mark_started_at {
        crate::read_state::mark_seen_notif_bootstrap_started_at(db, started_at);
    }
    if state.should_mark_complete {
        crate::read_state::mark_seen_notif_bootstrap_complete(db);
    }
    state
}

fn bootstrap_mode_label(mode: BootstrapMode) -> &'static str {
    match mode {
        BootstrapMode::Silent => "silent",
        BootstrapMode::Finalize => "finalize",
        BootstrapMode::Normal => "normal",
    }
}

fn dispatch_notification(
    app: &AppHandle,
    run: &mut SyncRunDebug,
    source: &str,
    title: String,
    body: String,
) {
    match crate::ai::send_native_notification(app, &title, &body) {
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

fn record_event(
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

fn finish_sync_debug(app: &AppHandle, run: SyncRunDebug, status: String, error: String) {
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

fn delivery_note() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macOS: dispatched means notify-rust accepted the request; OS display happens asynchronously."
    }
    #[cfg(not(target_os = "macos"))]
    {
        "dispatched means the platform notification API accepted the request."
    }
}

fn seed_seen_state(
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

fn seed_luna_seen_state(
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

fn should_recover_empty_initialized(
    initialized: bool,
    seen_ids: &[String],
    current_ids: &[String],
) -> bool {
    initialized && seen_ids.is_empty() && !current_ids.is_empty()
}

fn luna_previous_revision(entries: &[LunaNotifSeenEntry], base_key: &str) -> Option<String> {
    entries
        .iter()
        .find(|entry| entry.base_key == base_key)
        .map(|entry| entry.revision_key.clone())
}

fn rebuild_luna_object_entries_from_seen_set(
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

fn luna_upsert_revision(
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

fn extend_seen_ids(
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

fn luna_revision_key(item: &crate::luna_parser::LunaNotification) -> String {
    format!("{}|{}|{}", item.date, item.course_info, item.content)
}

fn luna_base_key(item: &crate::luna_parser::LunaNotification) -> String {
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

fn luna_url_identity(raw_url: &str) -> Option<String> {
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

fn extract_query_param(query: &str, target_key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (key == target_key).then(|| value.to_string())
    })
}

fn normalize_luna_notification_subject(content: &str) -> String {
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

fn looks_like_luna_timestamp(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.len() >= 10
        && trimmed.contains('/')
        && trimmed.contains(':')
        && trimmed
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '/' | ':' | ' ' | '(' | ')' | '　'))
}

fn normalize_luna_key_part(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn classify_course_notification(module: &str) -> CourseNotificationKind {
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

fn course_notification_allowed(kind: CourseNotificationKind, cfg: &NotificationConfig) -> bool {
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

fn kwic_section_allowed(section: &str, cfg: &NotificationConfig) -> bool {
    match section {
        "呼出し・重要なお知らせ" => cfg.notify_important,
        "学部・研究科からのお知らせ" => cfg.notify_faculty,
        "授業のお知らせ" => {
            course_notification_allowed(CourseNotificationKind::General, cfg)
        }
        _ => cfg.notify_other,
    }
}

fn epoch_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
