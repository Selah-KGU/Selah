use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager};

use crate::commands::{self, NotificationConfig};
use crate::db::Database;
use crate::kwic_commands::KwicPortalHome;
use crate::mail::MailMessage;
use crate::parser::NotificationsData;
use crate::{KgcState, KwicState, LunaState, MailState};

const INITIAL_SYNC_DELAY: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);
const BOOTSTRAP_GRACE_PERIOD: Duration = Duration::from_secs(6 * 60);

pub struct NotificationPollState {
    running: AtomicBool,
}

impl NotificationPollState {
    pub fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
        }
    }
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

#[tauri::command]
pub async fn notification_sync_now(app: AppHandle) -> Result<(), String> {
    sync_notifications_now(&app).await
}

async fn sync_notifications_now(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<NotificationPollState>();
    if state.running.swap(true, Ordering::SeqCst) {
        return Ok(());
    }

    let result = sync_notifications_inner(app).await;
    state.running.store(false, Ordering::SeqCst);
    result
}

async fn sync_notifications_inner(app: &AppHandle) -> Result<(), String> {
    let cfg = commands::load_notification_config();
    let db = app.state::<Database>();
    let bootstrap_mode = current_bootstrap_mode(&db);
    let suppress_push = !matches!(bootstrap_mode, BootstrapMode::Normal);

    if is_kgc_authenticated(app).await {
        match fetch_kgc_notifications(app).await {
            Ok(data) => sync_kgc_notifications(app, &cfg, data, suppress_push),
            Err(e) => log::warn!("notification sync: kgc fetch failed: {}", e),
        }
    }

    if is_luna_authenticated(app).await {
        match fetch_luna_notifications(app).await {
            Ok(items) => sync_luna_notifications(app, &cfg, items, suppress_push),
            Err(e) => log::warn!("notification sync: luna fetch failed: {}", e),
        }
    }

    if is_kwic_authenticated(app).await {
        match fetch_kwic_home(app).await {
            Ok(home) => sync_kwic_notifications(app, &cfg, home, suppress_push),
            Err(e) => log::warn!("notification sync: kwic fetch failed: {}", e),
        }
    }

    if is_mail_authenticated(app).await {
        match crate::mail_commands::fetch_inbox_internal(app, 20, 0).await {
            Ok(items) => sync_mail_notifications(app, &cfg, items, suppress_push),
            Err(e) => log::warn!("notification sync: mail fetch failed: {}", e),
        }
    }

    if matches!(bootstrap_mode, BootstrapMode::Finalize) {
        crate::read_state::mark_seen_notif_bootstrap_complete(&db);
        log::info!("notification sync: initial bootstrap completed");
    }

    Ok(())
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
) {
    let source = "kgc";
    let db = app.state::<Database>();
    let current_ids: Vec<String> = data
        .entries
        .iter()
        .filter_map(|item| (!item.id.is_empty()).then_some(item.id.clone()))
        .collect();
    let (initialized, mut seen_ids, mut seen_set) = load_seen_state(&db, source);

    if !initialized {
        seed_seen_state(&db, source, seen_ids, seen_set, current_ids);
        return;
    }

    let new_entries: Vec<_> = data
        .entries
        .iter()
        .filter(|item| !item.id.is_empty() && !seen_set.contains(&item.id))
        .collect();

    if !suppress_push && course_notification_allowed(CourseNotificationKind::General, cfg) {
        for item in &new_entries {
            let title = if item.category.is_empty() {
                item.title.clone()
            } else {
                format!("[{}] {}", item.category, item.title)
            };
            let _ = crate::ai::send_native_notification(app, &title, &item.date);
        }
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
) {
    let source = "luna";
    let db = app.state::<Database>();
    let current_ids: Vec<String> = items.iter().map(luna_seen_key).collect();
    let (initialized, mut seen_ids, mut seen_set) = load_seen_state(&db, source);

    if !initialized {
        seed_seen_state(&db, source, seen_ids, seen_set, current_ids);
        return;
    }

    for item in &items {
        let key = luna_seen_key(item);
        if seen_set.contains(&key) {
            continue;
        }
        if !suppress_push
            && course_notification_allowed(classify_course_notification(&item.module), cfg)
        {
            let title = if item.module.is_empty() {
                item.content.clone()
            } else {
                format!("[{}] {}", item.module, item.content)
            };
            let body = format!("{} — {}", item.course_info, item.date);
            let _ = crate::ai::send_native_notification(app, &title, &body);
        }
    }

    extend_seen_ids(&mut seen_ids, &mut seen_set, current_ids);
    crate::read_state::save_seen_notif_ids(&db, source, seen_ids);
    crate::read_state::mark_seen_notif_initialized(&db, source);
}

fn sync_kwic_notifications(
    app: &AppHandle,
    cfg: &NotificationConfig,
    home: KwicPortalHome,
    suppress_push: bool,
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

    if !initialized {
        seed_seen_state(&db, source, seen_ids, seen_set, current_ids);
        return;
    }

    for section in &home.sections {
        for item in &section.items {
            if item.id.is_empty() || seen_set.contains(&item.id) {
                continue;
            }
            if !suppress_push && kwic_section_allowed(&section.title, cfg) {
                let title = if item.category.is_empty() {
                    item.title.clone()
                } else {
                    format!("[{}] {}", item.category, item.title)
                };
                let _ = crate::ai::send_native_notification(app, &title, &item.date);
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
) {
    let source = "mail";
    let db = app.state::<Database>();
    let current_ids: Vec<String> = items
        .iter()
        .filter_map(|item| (!item.id.is_empty()).then_some(item.id.clone()))
        .collect();
    let (initialized, mut seen_ids, mut seen_set) = load_seen_state(&db, source);

    if !initialized {
        seed_seen_state(&db, source, seen_ids, seen_set, current_ids);
        return;
    }

    if !suppress_push && cfg.notify_mail {
        for item in &items {
            if item.id.is_empty() || seen_set.contains(&item.id) || item.is_read.unwrap_or(false) {
                continue;
            }
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
            let _ = crate::ai::send_native_notification(app, &sender, &subject);
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

fn current_bootstrap_mode(db: &Database) -> BootstrapMode {
    if crate::read_state::is_seen_notif_bootstrap_complete(db) {
        return BootstrapMode::Normal;
    }

    let now = epoch_secs();
    let started_at =
        crate::read_state::get_seen_notif_bootstrap_started_at(db).unwrap_or_else(|| {
            crate::read_state::mark_seen_notif_bootstrap_started_at(db, now);
            now
        });

    if now.saturating_sub(started_at) >= BOOTSTRAP_GRACE_PERIOD.as_secs() as i64 {
        BootstrapMode::Finalize
    } else {
        BootstrapMode::Silent
    }
}

fn seed_seen_state(
    db: &Database,
    source: &str,
    mut seen_ids: Vec<String>,
    mut seen_set: HashSet<String>,
    current_ids: Vec<String>,
) {
    extend_seen_ids(&mut seen_ids, &mut seen_set, current_ids);
    crate::read_state::save_seen_notif_ids(db, source, seen_ids);
    crate::read_state::mark_seen_notif_initialized(db, source);
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

fn luna_seen_key(item: &crate::luna_parser::LunaNotification) -> String {
    format!("{}|{}|{}", item.date, item.course_info, item.content)
}

fn classify_course_notification(module: &str) -> CourseNotificationKind {
    let normalized = module.trim().to_lowercase();
    if normalized.is_empty() {
        return CourseNotificationKind::General;
    }
    if normalized.contains("掲示板")
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
    if normalized.contains("出席") || normalized.contains("attendance") {
        return CourseNotificationKind::Attendance;
    }
    if normalized.contains("小テスト")
        || normalized.contains("テスト")
        || normalized.contains("試験")
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
