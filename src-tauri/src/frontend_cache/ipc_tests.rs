use super::{
    load_frontend_cache_batch, reply, CacheStampQuery, FrontendCacheBatch, LIVE_TODO_CACHE_KEY,
};
use crate::db::Database;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc::channel,
    Arc, Mutex,
};
use tauri::Manager;
use tauri::{
    ipc::{CallbackFn, InvokeBody, InvokeResponse, IpcResponse},
    test::{mock_builder, mock_context, noop_assets, MockRuntime},
};

struct Temporary(std::path::PathBuf);
impl Temporary {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("selah-batch-replies-{}", uuid::Uuid::new_v4())))
    }
    fn sql(&self, sql: &str) {
        rusqlite::Connection::open(self.0.join("courses.db"))
            .unwrap()
            .execute_batch(sql)
            .unwrap();
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// The predecessor read/await/reply bodies retain their typed IPC return path.

fn app(temporary: &Temporary, old: bool) -> tauri::App<MockRuntime> {
    let builder = mock_builder().manage(Database::open(&temporary.0).unwrap());
    let builder = if old {
        builder.invoke_handler(crate::cache_reply_legacy::handler())
    } else {
        builder.invoke_handler(tauri::generate_handler![
            crate::frontend_cache::get_backend_task_timestamps,
            crate::frontend_cache::get_frontend_cache_batch,
            crate::timetable::get_schedule_snapshot
        ])
    };
    builder.build(mock_context(noop_assets())).unwrap()
}
fn window(app: &tauri::App<MockRuntime>) -> tauri::WebviewWindow<MockRuntime> {
    tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("index.html".into()))
        .build()
        .unwrap()
}
fn call(
    window: &tauri::WebviewWindow<MockRuntime>,
    command: &str,
    body: Value,
) -> Result<String, Value> {
    let (sent, received) = channel();
    window.as_ref().clone().on_message(
        tauri::webview::InvokeRequest {
            cmd: command.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "http://tauri.localhost".parse().unwrap(),
            body: InvokeBody::Json(body),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.into(),
        },
        Box::new(move |_, _, response, _, _| {
            sent.send(match response {
                InvokeResponse::Ok(tauri::ipc::InvokeResponseBody::Json(json)) => Ok(json),
                InvokeResponse::Err(error) => Err(error.0),
                _ => panic!("cache reply must be JSON"),
            })
            .unwrap();
        }),
    );
    received
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap()
}
fn seed(app: &tauri::App<MockRuntime>, temporary: &Temporary) {
    let db = app.state::<Database>();
    db.save_data_cache(
        "large",
        &json!({"text":"全文\n\"quote\" \\ 👩🏽‍💻".repeat(32768)}).to_string(),
    )
    .unwrap();
    db.save_data_cache(
        "notifications",
        &json!({"entries":[{"id":"notice","title":"全文\n\"引用\" 🌙"}]}).to_string(),
    )
    .unwrap();
    db.save_data_cache(LIVE_TODO_CACHE_KEY, "[]").unwrap();
    temporary.sql("UPDATE data_cache SET updated_at = 42; UPDATE data_cache SET updated_at = -1 WHERE cache_key = 'large';");
}

#[test]
fn actual_ipc_objects_omissions_complete_json_and_arguments_match_the_typed_commands() {
    let old_dir = Temporary::new();
    let new_dir = Temporary::new();
    let old_app = app(&old_dir, true);
    let new_app = app(&new_dir, false);
    seed(&old_app, &old_dir);
    seed(&new_app, &new_dir);
    let old = window(&old_app);
    let new = window(&new_app);
    for (command, body) in [
        ("get_schedule_snapshot", json!({})),
        (
            "get_backend_task_timestamps",
            json!({"keys":["notifications","missing","large","notifications"],"includeSchedule":true}),
        ),
        (
            "get_backend_task_timestamps",
            json!({"keys":[],"includeSchedule":false}),
        ),
        (
            "get_frontend_cache_batch",
            json!({"queries":[{"key":"large"},{"key":"notifications"},{"key":"missing"},{"key":"large"}],"includeSchedule":false}),
        ),
        (
            "get_frontend_cache_batch",
            json!({"queries":[{"key":"notifications","knownRevision":2}],"includeSchedule":true}),
        ),
        (
            "get_frontend_cache_batch",
            json!({"queries":[],"includeSchedule":false}),
        ),
    ] {
        let before = call(&old, command, body.clone()).unwrap();
        let after = call(&new, command, body).unwrap();
        assert_eq!(after, before);
        assert!(serde_json::from_str::<Value>(&after).unwrap().is_object());
    }
    let body = json!({"queries":[],"includeSchedule":true});
    let schedule: Value =
        serde_json::from_str(&call(&new, "get_frontend_cache_batch", body).unwrap()).unwrap();
    let body = json!({"queries":[],"includeSchedule":true,"knownScheduleStamp":schedule["schedule_stamp"]});
    let unchanged = call(&new, "get_frontend_cache_batch", body.clone()).unwrap();
    assert_eq!(
        unchanged,
        call(&old, "get_frontend_cache_batch", body).unwrap()
    );
    let unchanged: Value = serde_json::from_str(&unchanged).unwrap();
    assert_eq!(unchanged["schedule_unchanged"], true);
    assert!(unchanged.get("schedule").is_none());
    for (command, bodies) in [
        (
            "get_backend_task_timestamps",
            vec![
                json!({}),
                json!({"keys":null,"includeSchedule":false}),
                json!({"keys":[3],"includeSchedule":false}),
            ],
        ),
        (
            "get_frontend_cache_batch",
            vec![
                json!({}),
                json!({"queries":null,"includeSchedule":true}),
                json!({"queries":[{"key":3}],"includeSchedule":true}),
                json!({"queries":[],"includeSchedule":"true"}),
            ],
        ),
    ] {
        for body in bodies {
            assert_eq!(call(&new, command, body.clone()), call(&old, command, body));
        }
    }
    let notification = call(
        &new,
        "get_frontend_cache_batch",
        json!({"queries":[{"key":"notifications"}],"includeSchedule":false}),
    )
    .unwrap();
    let timestamps = call(
        &new,
        "get_backend_task_timestamps",
        json!({"keys":["notifications","missing","large","notifications"],"includeSchedule":true}),
    )
    .unwrap();
    let notification_value: Value = serde_json::from_str(&notification).unwrap();
    let unchanged_notification = call(&new, "get_frontend_cache_batch", json!({
        "queries":[{"key":"notifications","knownRevision":notification_value["rows"][0]["revision"]}],
        "includeSchedule":false
    })).unwrap();
    let unchanged_notification: Value = serde_json::from_str(&unchanged_notification).unwrap();
    assert_eq!(unchanged_notification["rows"][0]["unchanged"], true);
    assert!(unchanged_notification["rows"][0].get("json").is_none());
    let fixture = json!({"notification_batch":notification_value,
        "notification_unchanged_batch":unchanged_notification,
        "timestamp_batch":serde_json::from_str::<Value>(&timestamps).unwrap(),
        "schedule_snapshot":serde_json::from_str::<Value>(&call(&new,"get_schedule_snapshot",json!({})).unwrap()).unwrap()});
    if let Ok(path) = std::env::var("SELAH_CACHE_BATCH_REPLY_WIRE") {
        std::fs::write(path, serde_json::to_string_pretty(&fixture).unwrap() + "\n").unwrap();
    }
}

#[test]
fn actual_database_failures_keep_their_ipc_errors_instead_of_empty_successes() {
    let old_dir = Temporary::new();
    let new_dir = Temporary::new();
    let old_app = app(&old_dir, true);
    let new_app = app(&new_dir, false);
    old_dir
        .sql("DROP TABLE data_cache; DROP TABLE schedule_snapshot_state; DROP TABLE luna_courses;");
    new_dir
        .sql("DROP TABLE data_cache; DROP TABLE schedule_snapshot_state; DROP TABLE luna_courses;");
    let old = window(&old_app);
    let new = window(&new_app);
    for (command, body) in [
        ("get_schedule_snapshot", json!({})),
        (
            "get_backend_task_timestamps",
            json!({"keys":["missing"],"includeSchedule":false}),
        ),
        (
            "get_frontend_cache_batch",
            json!({"queries":[{"key":"missing"}],"includeSchedule":false}),
        ),
    ] {
        let before = call(&old, command, body.clone()).unwrap_err();
        assert!(before.is_string());
        assert_eq!(call(&new, command, body).unwrap_err(), before);
    }
}

#[test]
fn broken_snapshot_metadata_rejects_schedule_ipc_without_breaking_unrelated_cache_reads() {
    let mut errors = Vec::new();
    for (sql, expected) in [
        ("DROP TABLE schedule_snapshot_state", "DB snapshot read:"),
        ("DROP TABLE ai_schedule_cache", "DB AI cache read:"),
        (
            "INSERT INTO schedule_snapshot_state (id, updated_at) VALUES (1, 'broken')",
            "DB snapshot read:",
        ),
        (
            "INSERT INTO ai_schedule_cache (id, result_json, updated_at) VALUES (1, '{}', 'broken')",
            "DB AI cache read:",
        ),
    ] {
        let temporary = Temporary::new();
        let application = app(&temporary, false);
        seed(&application, &temporary);
        let view = window(&application);
        let cached = application
            .state::<Database>()
            .get_data_cache("notifications")
            .unwrap();
        temporary.sql(sql);
        let error = call(&view, "get_schedule_snapshot", json!({})).unwrap_err();
        assert!(error.as_str().unwrap().starts_with(expected), "{error}");
        // Batched reads fail too, sometimes earlier in their version lookup.
        let batch_error = call(
            &view,
            "get_frontend_cache_batch",
            json!({"queries":[],"includeSchedule":true}),
        )
        .unwrap_err();
        errors.push(json!({"snapshot_error":error, "batch_error":batch_error}));
        let notification = call(
            &view,
            "get_frontend_cache_batch",
            json!({"queries":[{"key":"notifications"}],"includeSchedule":false}),
        )
        .unwrap();
        let notification: Value = serde_json::from_str(&notification).unwrap();
        assert_eq!(
            notification["rows"][0]["json"].as_str(),
            cached.as_ref().map(|(json, _)| json.as_str())
        );
        assert_eq!(
            application
                .state::<Database>()
                .get_data_cache("notifications")
                .unwrap(),
            cached
        );
    }
    if let Ok(path) = std::env::var("SELAH_SCHEDULE_READ_ERROR_WIRE") {
        std::fs::write(path, serde_json::to_string_pretty(&errors).unwrap() + "\n").unwrap();
    }
}

struct EncodingProbe {
    batch: FrontendCacheBatch,
    caller: std::thread::ThreadId,
    count: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
    entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
}
impl Serialize for EncodingProbe {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        assert_ne!(std::thread::current().id(), self.caller);
        self.count.fetch_add(1, Ordering::SeqCst);
        self.entered
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(())
            .unwrap();
        self.release
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        self.batch.serialize(serializer)
    }
}
impl Drop for EncodingProbe {
    fn drop(&mut self) {
        assert_ne!(std::thread::current().id(), self.caller);
        self.dropped.store(true, Ordering::SeqCst);
    }
}
#[tokio::test(flavor = "current_thread")]
async fn batch_read_encoding_and_large_row_disposal_run_on_the_worker_once() {
    let temporary = Temporary::new();
    let app = app(&temporary, false);
    seed(&app, &temporary);
    let handle = app.handle().clone();
    let queries = || {
        vec![CacheStampQuery {
            key: "large".into(),
            known_revision: None,
        }]
    };
    let expected =
        serde_json::to_string(&load_frontend_cache_batch(&handle, queries(), false, None).unwrap())
            .unwrap();
    let caller = std::thread::current().id();
    let count = Arc::new(AtomicUsize::new(0));
    let counted = count.clone();
    let dropped = Arc::new(AtomicBool::new(false));
    let drop_probe = dropped.clone();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = channel();
    let reading = tokio::spawn(reply(
        "キャッシュの読み込みに失敗しました",
        move || {
            assert_ne!(std::thread::current().id(), caller);
            let batch = load_frontend_cache_batch(&handle, queries(), false, None)?;
            Ok(EncodingProbe {
                batch,
                caller,
                count: counted,
                dropped: drop_probe,
                entered: Mutex::new(Some(entered)),
                release: Mutex::new(Some(released)),
            })
        },
    ));
    started.await.unwrap();
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    let pending = !reading.is_finished();
    release.send(()).unwrap();
    assert!(pending);
    let body = reading.await.unwrap().unwrap().body().unwrap();
    match body {
        tauri::ipc::InvokeResponseBody::Json(json) => assert_eq!(json, expected),
        _ => panic!("batch must be raw JSON"),
    }
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(dropped.load(Ordering::SeqCst));
}
