use crate::ai::{self, AiConfig};
use crate::background_refresh::BackendSessionStatusPayload;
use crate::db::{epoch_secs, Database};
use crate::timetable;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

const AI_STATUS_CACHE_KEY: &str = "ai_scheduler_status";
const AI_NOTIF_CACHE_KEY: &str = "ai_notif_analysis";
const AI_REFRESH_CHECK_SECS: u64 = 5 * 60;
const AI_REFRESH_STARTUP_DELAY_SECS: u64 = 35;
const FAST_INPUT_MAX_AGE_SECS: i64 = 15 * 60;
const KWIC_INPUT_MAX_AGE_SECS: i64 = 12 * 60 * 60;
const SCHEDULE_INPUT_MAX_AGE_SECS: i64 = 6 * 60 * 60;
const STABLE_INPUT_MAX_AGE_SECS: i64 = 12 * 60 * 60;

#[derive(Default)]
pub struct AiRefreshState {
    running: AtomicBool,
}

impl AiRefreshState {
    pub fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
        }
    }
}

struct AiRefreshRequest {
    keys: Option<HashSet<String>>,
}

impl AiRefreshRequest {
    fn new(keys: Option<Vec<String>>) -> Self {
        let keys = keys.map(|items| {
            items
                .into_iter()
                .filter(|key| !key.trim().is_empty())
                .collect::<HashSet<_>>()
        });
        Self { keys }
    }

    fn wants(&self, key: &str) -> bool {
        self.keys
            .as_ref()
            .map(|keys| keys.contains(key))
            .unwrap_or(true)
    }

    fn is_all(&self) -> bool {
        self.keys.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiRefreshStatus {
    pub running: bool,
    pub last_run: Option<i64>,
    pub last_ok: Option<bool>,
    #[serde(default)]
    pub last_error: String,
    #[serde(default)]
    pub interval_minutes: u32,
    #[serde(default)]
    pub items: Vec<AiRefreshItemStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiRefreshItemStatus {
    pub key: String,
    pub label: String,
    pub status: String,
    #[serde(default)]
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnifiedNotif {
    source: String,
    title: String,
    category: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    course_info: String,
    date: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    section: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    kwic_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    information_type: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    person_category_cd: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    category_cd: String,
}

pub fn start_ai_refresh_loop(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(AI_REFRESH_STARTUP_DELAY_SECS)).await;
        loop {
            if let Err(err) = run_due_ai_refresh(&app).await {
                log::warn!("[ai_refresh] scheduled check failed: {}", err);
            }
            tokio::time::sleep(Duration::from_secs(AI_REFRESH_CHECK_SECS)).await;
        }
    });
}

#[tauri::command]
pub async fn get_backend_ai_refresh_status(
    db: State<'_, Database>,
    state: State<'_, AiRefreshState>,
) -> Result<AiRefreshStatus, String> {
    let mut status = load_status(&db);
    status.running = state.running.load(Ordering::SeqCst);
    status.interval_minutes = ai::load_ai_config().ai_refresh_interval;
    Ok(status)
}

#[tauri::command]
pub async fn backend_ai_refresh_now(
    app: AppHandle,
    force: bool,
    keys: Option<Vec<String>>,
) -> Result<AiRefreshStatus, String> {
    run_ai_refresh(&app, force, AiRefreshRequest::new(keys), "manual").await
}

async fn run_due_ai_refresh(app: &AppHandle) -> Result<(), String> {
    let config = ai::load_ai_config();
    let db = app.state::<Database>();
    let mut status = load_status(&db);
    status.interval_minutes = config.ai_refresh_interval;

    if config.ai_refresh_interval == 0 {
        save_status_and_emit(app, &db, &status)?;
        return Ok(());
    }

    if let Some(reason) = ai_unavailable_reason(&config) {
        status.running = false;
        status.last_ok = None;
        status.last_error = reason;
        save_status_and_emit(app, &db, &status)?;
        return Ok(());
    }

    if let Some(last_run) = status.last_run {
        let due_after = i64::from(config.ai_refresh_interval) * 60;
        if epoch_secs() - last_run < due_after {
            save_status_and_emit(app, &db, &status)?;
            return Ok(());
        }
    }

    if let Some(reason) = ai_session_block_reason(app).await? {
        status.running = false;
        status.last_ok = None;
        status.last_error = reason;
        save_status_and_emit(app, &db, &status)?;
        return Ok(());
    }

    run_ai_refresh(app, true, AiRefreshRequest::new(None), "scheduled")
        .await
        .map(|_| ())
}

async fn run_ai_refresh(
    app: &AppHandle,
    force: bool,
    request: AiRefreshRequest,
    trigger: &str,
) -> Result<AiRefreshStatus, String> {
    let state = app.state::<AiRefreshState>();
    if state.running.swap(true, Ordering::SeqCst) {
        let db = app.state::<Database>();
        let mut status = load_status(&db);
        status.running = true;
        status.interval_minutes = ai::load_ai_config().ai_refresh_interval;
        return Ok(status);
    }

    let db = app.state::<Database>();
    let config = ai::load_ai_config();
    let mut status = load_status(&db);
    let record_scheduler_status = request.is_all();
    status.running = true;
    status.interval_minutes = config.ai_refresh_interval;
    status.last_error.clear();
    status.items.clear();
    if let Err(err) = record_status(app, &db, &status, record_scheduler_status) {
        state.running.store(false, Ordering::SeqCst);
        return Err(err);
    }

    let mut changed_keys: Vec<String> = Vec::new();
    let mut any_error = false;

    let outcome = async {
        if let Some(reason) = ai_unavailable_reason(&config) {
            return Err(reason);
        }
        let session = current_ai_session(app).await?;
        if let Some(reason) = ai_session_block_reason_from_status(&session) {
            return Err(reason);
        }

        log::info!(
            "[ai_refresh] starting {} refresh (force={})",
            trigger,
            force
        );

        if request.wants("ai_notif") {
            status
                .items
                .push(item_status("ai_notif", "AI 通知分析", "running", ""));
            record_status(app, &db, &status, record_scheduler_status)?;
            match refresh_notification_analysis(&db, &config, &session).await {
                Ok(()) => {
                    changed_keys.push(AI_NOTIF_CACHE_KEY.to_string());
                    update_item_status(&mut status, "ai_notif", "done", "");
                }
                Err(err) if is_no_data_error(&err) => {
                    update_item_status(&mut status, "ai_notif", "skipped", &err);
                }
                Err(err) => {
                    any_error = true;
                    update_item_status(&mut status, "ai_notif", "error", &err);
                }
            }
            record_status(app, &db, &status, record_scheduler_status)?;
        }

        // AI 課題分析は定期スケジューラでは実行しない。AI 補助モードの
        // 「再分析」ボタンを押したときだけ ai_analyze_todo が呼ばれる。

        if request.wants("ai_schedule") {
            status
                .items
                .push(item_status("ai_schedule", "AI 時間割分析", "running", ""));
            record_status(app, &db, &status, record_scheduler_status)?;
            match refresh_schedule_analysis(&db, force, &session).await {
                Ok(()) => {
                    changed_keys.push("schedule_data".to_string());
                    update_item_status(&mut status, "ai_schedule", "done", "");
                }
                Err(err) if is_no_data_error(&err) => {
                    update_item_status(&mut status, "ai_schedule", "skipped", &err);
                }
                Err(err) => {
                    any_error = true;
                    update_item_status(&mut status, "ai_schedule", "error", &err);
                }
            }
            record_status(app, &db, &status, record_scheduler_status)?;
        }

        Ok::<(), String>(())
    }
    .await;

    state.running.store(false, Ordering::SeqCst);
    status.running = false;
    let attempted_ai = status.items.iter().any(item_attempted);
    if attempted_ai && request.is_all() {
        status.last_run = Some(epoch_secs());
    }

    if let Err(err) = outcome {
        status.last_ok = Some(false);
        status.last_error = err;
    } else if any_error {
        status.last_ok = Some(false);
        status.last_error = status
            .items
            .iter()
            .find(|item| item.status == "error")
            .map(|item| item.error.clone())
            .unwrap_or_else(|| "AI refresh failed".to_string());
    } else if attempted_ai {
        status.last_ok = Some(true);
        status.last_error.clear();
    } else {
        status.last_ok = None;
        status.last_error = status
            .items
            .iter()
            .find(|item| item.status == "skipped")
            .map(|item| item.error.clone())
            .unwrap_or_default();
    }

    record_status(app, &db, &status, record_scheduler_status)?;
    if !changed_keys.is_empty() {
        emit_cache_updated(app, changed_keys);
    }
    Ok(status)
}

async fn refresh_schedule_analysis(
    db: &Database,
    force: bool,
    session: &BackendSessionStatusPayload,
) -> Result<(), String> {
    if !session.kgc_session_present {
        return Err("KGC未ログインのため時間割AI分析をスキップします".to_string());
    }
    let snap = db.get_snapshot_state()?.unwrap_or_default();
    let scope = crate::academic_period::visible_weeks(
        &snap.current_week_label,
        &snap.next_week_label,
        &snap.luna_year,
        &snap.luna_term,
        chrono::Local::now().date_naive(),
    );
    if scope.current.trim().is_empty() {
        return Err("今学期の時間割がまだありません".to_string());
    }
    if !timestamp_is_fresh(snap.updated_at, SCHEDULE_INPUT_MAX_AGE_SECS) {
        return Err("時間割データが最新ではないためAI分析をスキップします".to_string());
    }
    timetable::ai_generate_schedule_internal(db, scope.current, scope.next, force)
        .await
        .map(|_| ())
}

async fn refresh_notification_analysis(
    db: &Database,
    config: &AiConfig,
    session: &BackendSessionStatusPayload,
) -> Result<(), String> {
    let has_fresh_kgc =
        session.kgc_session_present && cache_is_fresh(db, "notifications", FAST_INPUT_MAX_AGE_SECS);
    let has_fresh_luna =
        session.luna_authenticated && cache_is_fresh(db, "luna_updates", FAST_INPUT_MAX_AGE_SECS);
    let has_fresh_kwic =
        session.kwic_authenticated && cache_is_fresh(db, "kwic_home", KWIC_INPUT_MAX_AGE_SECS);

    if !has_fresh_kgc && !has_fresh_luna && !has_fresh_kwic {
        return Err("通知データが最新ではないためAI分析をスキップします".to_string());
    }

    let notifications: crate::parser::NotificationsData = load_cache_json(db, "notifications")
        .unwrap_or(crate::parser::NotificationsData {
            entries: Vec::new(),
        });
    let luna_updates: Vec<crate::luna_parser::LunaNotification> =
        load_cache_json(db, "luna_updates").unwrap_or_default();
    let kwic_home: Option<crate::kwic_commands::KwicPortalHome> = load_cache_json(db, "kwic_home");
    let sources = build_unified_notifications(
        if has_fresh_kgc {
            Some(&notifications)
        } else {
            None
        },
        if has_fresh_luna {
            Some(&luna_updates)
        } else {
            None
        },
        if has_fresh_kwic {
            kwic_home.as_ref()
        } else {
            None
        },
    );

    if sources.is_empty() {
        return Err("通知データがまだありません".to_string());
    }

    let course_names = fresh_course_names(db)?;

    let todo_items: Vec<crate::luna_parser::LunaTodoItem> =
        if session.luna_authenticated && cache_is_fresh(db, "luna_todo", FAST_INPUT_MAX_AGE_SECS) {
            load_cache_json(db, "luna_todo").unwrap_or_default()
        } else {
            Vec::new()
        };
    let todo_summary = todo_items
        .iter()
        .take(8)
        .map(|item| {
            format!(
                "- {} / {} / {} / {}",
                item.course_name, item.content_type, item.content_name, item.deadline
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let profile_json =
        fresh_cache_json(db, "student_profile", STABLE_INPUT_MAX_AGE_SECS).unwrap_or_default();

    let system_prompt = build_notification_system_prompt(config);
    let user_prompt =
        build_notification_user_prompt(&profile_json, &course_names, &todo_summary, &sources);

    let response = ai::chat_completion_public(
        config,
        vec![
            ai::ChatMessage {
                role: "system".into(),
                content: system_prompt,
                images: Vec::new(),
            },
            ai::ChatMessage {
                role: "user".into(),
                content: user_prompt,
                images: Vec::new(),
            },
        ],
    )
    .await?;

    let result = parse_notification_response(&response)?;
    let payload = json!({
        "result": result,
        "sources": sources,
        "generated_at": epoch_secs(),
    });
    db.save_data_cache(AI_NOTIF_CACHE_KEY, &payload.to_string())
}

fn fresh_course_names(db: &Database) -> Result<String, String> {
    let snap = db.get_snapshot_state()?.unwrap_or_default();
    if !timestamp_is_fresh(snap.updated_at, SCHEDULE_INPUT_MAX_AGE_SECS) {
        return Ok(String::new());
    }
    let scope = crate::academic_period::visible_weeks(
        &snap.current_week_label,
        &snap.next_week_label,
        &snap.luna_year,
        &snap.luna_term,
        chrono::Local::now().date_naive(),
    );
    let raw = db.build_raw_data(&scope.current, &scope.next, Vec::new())?;
    Ok(raw
        .kgc_entries_current
        .iter()
        .chain(raw.kgc_entries_next.iter())
        .map(|entry| entry.name.trim())
        .filter(|name| !name.is_empty())
        .collect::<HashSet<_>>()
        .into_iter()
        .take(24)
        .collect::<Vec<_>>()
        .join("、"))
}

fn build_notification_system_prompt(config: &AiConfig) -> String {
    let lang_hint = ai::reply_language_hint(
        &config.reply_language,
        "\n\nsummary, important.title, important.reason, suggestions は中国語（简体字）で書く。",
        "\n\nWrite summary, important.title, important.reason, and suggestions in English.",
        "\n\nsummary, important.title, important.reason, suggestions 는 한국어로 작성.",
    );
    format!(
        r#"あなたは関西学院大学の学生向けパーソナル通知アシスタントです。
学生のプロフィール、履修科目、課題状況、通知一覧を受け取り、今この学生にとって重要な情報をJSON形式だけで返します。

判定基準:
- 日程が現在より前なら終了済みとして低優先または除外
- 学生の履修科目・学部・キャンパスと関係が強いものを優先
- 似たタイトルでも各通知を個別に扱う
- 通知文そのままの繰り返しではなく、学生が次に動ける一歩を提案する

出力JSON形式:
{{"summary":"80〜150字","important":[{{"title":"20字以内","reason":"15字以内","index":1}}],"suggestions":["10〜20字の行動提案"]}}

JSON以外の文字、Markdown、コードブロックは出力しない。{}"#,
        lang_hint
    )
}

fn build_notification_user_prompt(
    profile_json: &str,
    course_names: &str,
    todo_summary: &str,
    sources: &[UnifiedNotif],
) -> String {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    let notif_text = sources
        .iter()
        .enumerate()
        .map(|(idx, notif)| {
            format!(
                "{}. [{}] {} / {} / {}{}",
                idx + 1,
                notif.source,
                notif.date,
                notif.category,
                notif.title,
                if notif.course_info.is_empty() {
                    String::new()
                } else {
                    format!(" / 科目: {}", notif.course_info)
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "現在日時: {now}\n\n学生プロフィールJSON:\n{profile_json}\n\n履修科目:\n{course_names}\n\n未完了課題:\n{todo_summary}\n\n通知一覧:\n{notif_text}"
    )
}

fn build_unified_notifications(
    notifications: Option<&crate::parser::NotificationsData>,
    luna_updates: Option<&[crate::luna_parser::LunaNotification]>,
    kwic_home: Option<&crate::kwic_commands::KwicPortalHome>,
) -> Vec<UnifiedNotif> {
    let mut seen = HashSet::new();
    let mut merged = Vec::new();

    if let Some(notifications) = notifications {
        for notif in notifications.entries.iter().take(12) {
            push_unique(
                &mut merged,
                &mut seen,
                UnifiedNotif {
                    source: "kgc".into(),
                    title: notif.title.clone(),
                    category: notif.category.clone(),
                    course_info: String::new(),
                    date: notif.date.clone(),
                    section: String::new(),
                    url: notif.url.clone(),
                    kwic_id: String::new(),
                    information_type: String::new(),
                    person_category_cd: String::new(),
                    category_cd: String::new(),
                },
            );
        }
    }

    if let Some(luna_updates) = luna_updates {
        for notif in luna_updates.iter().take(12) {
            push_unique(
                &mut merged,
                &mut seen,
                UnifiedNotif {
                    source: "luna".into(),
                    title: notif.content.clone(),
                    category: if notif.module.is_empty() {
                        notif.course_info.clone()
                    } else {
                        notif.module.clone()
                    },
                    course_info: notif.course_info.clone(),
                    date: notif.date.clone(),
                    section: String::new(),
                    url: notif.url.clone(),
                    kwic_id: String::new(),
                    information_type: String::new(),
                    person_category_cd: String::new(),
                    category_cd: String::new(),
                },
            );
        }
    }

    if let Some(home) = kwic_home {
        for section in home.sections.iter() {
            if section.title == "メインリンク" || section.title == "注目コンテンツ" {
                continue;
            }
            for item in section.items.iter().take(8) {
                push_unique(
                    &mut merged,
                    &mut seen,
                    UnifiedNotif {
                        source: "kwic".into(),
                        title: item.title.clone(),
                        category: if item.category.is_empty() {
                            section.title.clone()
                        } else {
                            item.category.clone()
                        },
                        course_info: String::new(),
                        date: item.date.clone(),
                        section: section.title.clone(),
                        url: String::new(),
                        kwic_id: item.id.clone(),
                        information_type: item.information_type.clone(),
                        person_category_cd: item.person_category_cd.clone(),
                        category_cd: item.category_cd.clone(),
                    },
                );
            }
        }
    }

    merged.sort_by(|a, b| b.date.cmp(&a.date));
    merged.truncate(12);
    merged
}

fn push_unique(merged: &mut Vec<UnifiedNotif>, seen: &mut HashSet<String>, notif: UnifiedNotif) {
    let title_key = notif.title.split_whitespace().collect::<Vec<_>>().join("");
    let key = format!("{}|{}|{}", notif.source, title_key, notif.date);
    if seen.insert(key) {
        merged.push(notif);
    }
}

fn parse_notification_response(response: &str) -> Result<serde_json::Value, String> {
    let cleaned = strip_think_blocks(response);
    let start = cleaned
        .find('{')
        .ok_or_else(|| "AI通知分析のJSON開始位置が見つかりません".to_string())?;
    let end = cleaned
        .rfind('}')
        .ok_or_else(|| "AI通知分析のJSON終了位置が見つかりません".to_string())?;
    let mut value: serde_json::Value = serde_json::from_str(&cleaned[start..=end])
        .map_err(|e| format!("AI通知分析のJSON解析に失敗: {}", e))?;
    if let Some(obj) = value.as_object_mut() {
        obj.remove("_check");
    }
    Ok(value)
}

fn strip_think_blocks(text: &str) -> String {
    let mut out = text.to_string();
    loop {
        let lower = out.to_ascii_lowercase();
        let Some(start) = lower.find("<think") else {
            break;
        };
        let Some(rel_end) = lower[start..].find("</think>") else {
            out.replace_range(start.., "");
            break;
        };
        let end = start + rel_end + "</think>".len();
        out.replace_range(start..end, "");
    }
    out.replace("<think>", "")
        .replace("</think>", "")
        .trim()
        .to_string()
}

fn ai_unavailable_reason(config: &AiConfig) -> Option<String> {
    if !config.ai_enabled {
        return Some("AI機能が無効です".to_string());
    }
    if config.provider == "local" {
        if let Err(reason) = crate::local_ai_support::ensure_supported() {
            return Some(reason);
        }
    } else if config.api_key.trim().is_empty() {
        return Some("AI APIキーが未設定です".to_string());
    }
    None
}

async fn ai_session_block_reason(app: &AppHandle) -> Result<Option<String>, String> {
    let status = current_ai_session(app).await?;
    Ok(ai_session_block_reason_from_status(&status))
}

async fn current_ai_session(app: &AppHandle) -> Result<BackendSessionStatusPayload, String> {
    // Scheduling only needs a local availability snapshot. It must not trigger
    // network validation or hidden SAML recovery on its own.
    crate::background_refresh::sync_backend_session_status(app, false).await
}

fn ai_session_block_reason_from_status(status: &BackendSessionStatusPayload) -> Option<String> {
    if status.session_expired {
        return Some("セッション期限切れのためAI定期更新を実行しません".to_string());
    }
    if !status.kgc_session_present {
        return Some("未ログインのためAI定期更新を実行しません".to_string());
    }
    None
}

fn is_no_data_error(err: &str) -> bool {
    err.contains("まだありません")
        || err.contains("TODO項目がありません")
        || err.contains("読み込んでください")
        || err.contains("未ログイン")
        || err.contains("最新ではない")
}

fn item_status(key: &str, label: &str, status: &str, error: &str) -> AiRefreshItemStatus {
    AiRefreshItemStatus {
        key: key.to_string(),
        label: label.to_string(),
        status: status.to_string(),
        error: error.to_string(),
    }
}

fn update_item_status(status: &mut AiRefreshStatus, key: &str, next_status: &str, error: &str) {
    if let Some(item) = status.items.iter_mut().rev().find(|item| item.key == key) {
        item.status = next_status.to_string();
        item.error = error.to_string();
    }
}

fn item_attempted(item: &AiRefreshItemStatus) -> bool {
    matches!(item.status.as_str(), "done" | "error")
}

fn load_cache_json<T: for<'de> Deserialize<'de>>(db: &Database, key: &str) -> Option<T> {
    db.get_data_cache(key)
        .ok()
        .flatten()
        .and_then(|(json, _)| serde_json::from_str(&json).ok())
}

fn fresh_cache_json(db: &Database, key: &str, max_age_secs: i64) -> Option<String> {
    db.get_data_cache(key)
        .ok()
        .flatten()
        .and_then(|(json, updated_at)| timestamp_is_fresh(updated_at, max_age_secs).then_some(json))
}

fn cache_is_fresh(db: &Database, key: &str, max_age_secs: i64) -> bool {
    db.get_data_cache(key)
        .ok()
        .flatten()
        .map(|(_, updated_at)| timestamp_is_fresh(updated_at, max_age_secs))
        .unwrap_or(false)
}

fn timestamp_is_fresh(updated_at: i64, max_age_secs: i64) -> bool {
    updated_at > 0 && epoch_secs() - updated_at <= max_age_secs
}

fn load_status(db: &Database) -> AiRefreshStatus {
    let mut status: AiRefreshStatus = db
        .get_data_cache(AI_STATUS_CACHE_KEY)
        .ok()
        .flatten()
        .and_then(|(json, _)| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    // running is process-local state. Never trust a persisted value after
    // restart or crash; active calls set it from AiRefreshState instead.
    status.running = false;
    status
}

fn save_status_and_emit(
    app: &AppHandle,
    db: &Database,
    status: &AiRefreshStatus,
) -> Result<(), String> {
    let json = serde_json::to_string(status).map_err(|e| e.to_string())?;
    db.save_data_cache(AI_STATUS_CACHE_KEY, &json)?;
    let _ = app.emit("backend-ai-refresh-status", status);
    Ok(())
}

fn record_status(
    app: &AppHandle,
    db: &Database,
    status: &AiRefreshStatus,
    should_record: bool,
) -> Result<(), String> {
    if should_record {
        save_status_and_emit(app, db, status)?;
    }
    Ok(())
}

fn emit_cache_updated(app: &AppHandle, keys: Vec<String>) {
    if let Err(e) = app.emit("backend-cache-updated", json!({ "keys": keys })) {
        log::warn!("[ai_refresh] backend-cache-updated emit failed: {}", e);
    }
}
