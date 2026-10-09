use super::*;
use serde_json::{json, Value};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;
use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponse};
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};

struct Temporary(std::path::PathBuf);
impl Temporary {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("selah-cache-ipc-{}", uuid::Uuid::new_v4())))
    }
    fn connection(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.0.join("courses.db")).unwrap()
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// Frozen synchronous commands provide wire-format and argument-error evidence.
mod legacy {
    use crate::db::Database;
    #[tauri::command]
    pub fn get_data_cache(db: tauri::State<'_, Database>, key: String) -> Option<String> {
        db.get_data_cache(&key).ok().flatten().map(|(json, _)| json)
    }
    #[tauri::command]
    pub fn get_data_cache_updated_at(db: tauri::State<'_, Database>, key: String) -> Option<i64> {
        db.cache_updated_at(&key)
    }
    #[tauri::command]
    pub fn save_data_cache(
        db: tauri::State<'_, Database>,
        key: String,
        json: String,
    ) -> Result<(), String> {
        if key.starts_with("seen_notifs_") {
            return Err("reserved cache key".into());
        }
        db.save_data_cache(&key, &json)
    }
    #[tauri::command]
    pub fn mark_notification_read(db: tauri::State<'_, Database>, source: String, id: String) {
        crate::read_state::errors_before::mark_read(&db, &source, &id);
    }
    #[tauri::command]
    pub fn mark_batch_notification_read(
        db: tauri::State<'_, Database>,
        source: String,
        ids: Vec<String>,
    ) {
        crate::read_state::errors_before::mark_batch_read(&db, &source, ids);
    }
    #[tauri::command]
    pub fn get_read_notifications(
        db: tauri::State<'_, Database>,
    ) -> crate::read_state::ReadIdsResponse {
        crate::read_state::errors_before::get_all_read_ids(&db)
    }
}

#[tauri::command]
fn unrelated_ping() -> &'static str {
    "responsive"
}

#[test]
fn cache_arguments_borrow_complete_strings_from_the_owned_ipc_message() {
    let value = json!({"key":"quoted' 🌕", "json":"完整文本\n\"quote\" \\ 🌕".repeat(131_072), "ids":["通知 ID 🌕".repeat(131_072),"same","same"]});
    for key in ["key", "json"] {
        let original = value[key].as_str().unwrap();
        let borrowed = CacheArgument::deserialize(&value[key]).unwrap().0;
        assert_eq!(borrowed, original);
        assert_eq!(borrowed.as_ptr(), original.as_ptr());
    }
    let ids = Vec::<CacheArgument>::deserialize(&value["ids"]).unwrap();
    for (borrowed, original) in ids.iter().zip(value["ids"].as_array().unwrap()) {
        let original = original.as_str().unwrap();
        assert_eq!(borrowed.0, original);
        assert_eq!(borrowed.0.as_ptr(), original.as_ptr());
    }
}

fn app(temporary: &Temporary, queue: Option<Arc<Queue>>) -> tauri::App<MockRuntime> {
    let builder = mock_builder().manage(Database::open(&temporary.0).unwrap());
    let builder = if let Some(queue) = queue {
        builder.invoke_handler(handler(queue, tauri::generate_handler![unrelated_ping]))
    } else {
        builder.invoke_handler(tauri::generate_handler![
            legacy::get_data_cache,
            legacy::get_data_cache_updated_at,
            legacy::save_data_cache,
            legacy::mark_notification_read,
            legacy::mark_batch_notification_read,
            legacy::get_read_notifications,
            unrelated_ping
        ])
    };
    builder.build(mock_context(noop_assets())).unwrap()
}

fn seed_read_state(app: &tauri::App<MockRuntime>) {
    // Never consult the user's file migration path from these IPC fixtures.
    app.state::<Database>()
        .save_data_cache("read_state", r#"{"kgc":[],"luna":[],"kwic":[]}"#)
        .unwrap();
}
fn canonical_ids(response: String) -> Value {
    let mut value: Value = serde_json::from_str(&response).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 3);
    for source in ["kgc", "luna", "kwic"] {
        value[source]
            .as_array_mut()
            .unwrap()
            .sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    }
    value
}

#[test]
fn notification_arguments_replies_and_complete_id_sets_match_synchronous_commands() {
    let old_temporary = Temporary::new();
    let new_temporary = Temporary::new();
    let old_app = app(&old_temporary, None);
    let new_app = app(&new_temporary, Some(Arc::new(Queue::new(FAILURE))));
    seed_read_state(&old_app);
    seed_read_state(&new_app);
    let old = webview(&old_app, "old");
    let new = webview(&new_app, "new");
    for source in ["kgc", "luna", "kwic", "unknown"] {
        let body = json!({"source":source,"id":"完整的通知 ID 🌕\n\"quote\""});
        assert_eq!(
            call(&new, "mark_notification_read", body.clone()).unwrap(),
            call(&old, "mark_notification_read", body).unwrap()
        );
        let ids: Vec<String> = (0..400)
            .map(|i| format!("{source}-{i} 日本語 🌕"))
            .collect();
        let body = json!({"source":source,"ids":ids,"ignored":true});
        assert_eq!(
            call(&new, "mark_batch_notification_read", body.clone()).unwrap(),
            call(&old, "mark_batch_notification_read", body).unwrap()
        );
    }
    assert_eq!(
        canonical_ids(call(&new, "get_read_notifications", json!({})).unwrap()),
        canonical_ids(call(&old, "get_read_notifications", json!({})).unwrap())
    );
    for command in ["mark_notification_read", "mark_batch_notification_read"] {
        for body in [
            json!({}),
            json!({"source":null}),
            json!({"source":[]}),
            json!({"source":"luna"}),
            json!({"source":"luna","id":null,"ids":null}),
            json!({"source":"luna","id":3,"ids":"bad"}),
            json!({"source":"luna","id":{},"ids":["ok",null]}),
            json!({"source":"luna","id":"ok","ids":["ok",3]}),
        ] {
            assert_eq!(call(&new, command, body.clone()), call(&old, command, body));
        }
        assert_eq!(
            reply(request(&new, command, InvokeBody::Raw(vec![1, 2]))),
            reply(request(&old, command, InvokeBody::Raw(vec![1, 2])))
        );
    }
    assert_eq!(
        canonical_ids(
            reply(request(
                &new,
                "get_read_notifications",
                InvokeBody::Raw(vec![1, 2])
            ))
            .unwrap()
        ),
        canonical_ids(
            reply(request(
                &old,
                "get_read_notifications",
                InvokeBody::Raw(vec![1, 2])
            ))
            .unwrap()
        )
    );
}

#[test]
fn notification_and_generic_cache_operations_share_admission_order_across_webviews() {
    let temporary = Temporary::new();
    let queue = Arc::new(Queue::new(FAILURE));
    let app = app(&temporary, Some(queue.clone()));
    seed_read_state(&app);
    let a = webview(&app, "a");
    let b = webview(&app, "b");
    let (entered, started) = channel();
    let (release, released) = channel();
    drop(queue.submit(move || {
        entered.send(()).unwrap();
        released.recv_timeout(Duration::from_secs(10)).unwrap();
        Ok(())
    }));
    started.recv_timeout(Duration::from_secs(10)).unwrap();
    let reset = r#"{"kgc":["seed"],"luna":[],"kwic":[]}"#;
    drop(request(
        &a,
        "save_data_cache",
        InvokeBody::Json(json!({"key":"read_state","json":reset})),
    ));
    drop(request(
        &b,
        "mark_notification_read",
        InvokeBody::Json(json!({"source":"kgc","id":"added"})),
    ));
    let mid = request(&a, "get_read_notifications", InvokeBody::Json(json!({})));
    drop(request(
        &b,
        "mark_batch_notification_read",
        InvokeBody::Json(json!({"source":"luna","ids":["1","2","1"]})),
    ));
    let raw = request(
        &a,
        "get_data_cache",
        InvokeBody::Json(json!({"key":"read_state"})),
    );
    let last = request(&b, "get_read_notifications", InvokeBody::Json(json!({})));
    assert_eq!(
        call(&a, "unrelated_ping", json!({})).unwrap(),
        "\"responsive\""
    );
    release.send(()).unwrap();
    assert_eq!(
        canonical_ids(reply(last).unwrap()),
        json!({"kgc":["added","seed"],"luna":["1","2"],"kwic":[]})
    );
    let raw: String = serde_json::from_str(&reply(raw).unwrap()).unwrap();
    assert_eq!(
        canonical_ids(raw),
        json!({"kgc":["added","seed"],"luna":["1","2"],"kwic":[]})
    );
    assert_eq!(
        canonical_ids(reply(mid).unwrap()),
        json!({"kgc":["added","seed"],"luna":[],"kwic":[]})
    );
}

#[tokio::test(flavor = "current_thread")]
async fn notification_writes_drain_after_dropped_replies_before_shutdown() {
    let temporary = Temporary::new();
    let queue = Arc::new(Queue::new(FAILURE));
    let app = app(&temporary, Some(queue.clone()));
    seed_read_state(&app);
    let view = webview(&app, "main");
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = channel();
    drop(queue.submit(move || {
        entered.send(()).unwrap();
        released.recv_timeout(Duration::from_secs(10)).unwrap();
        Ok(())
    }));
    started.await.unwrap();
    drop(request(
        &view,
        "mark_notification_read",
        InvokeBody::Json(json!({"source":"kgc","id":"accepted"})),
    ));
    drop(request(
        &view,
        "mark_batch_notification_read",
        InvokeBody::Json(json!({"source":"luna","ids":["one","two"]})),
    ));
    queue.seal();
    assert_eq!(
        call(
            &view,
            "mark_notification_read",
            json!({"source":"kgc","id":"rejected"})
        ),
        Err(json!("アプリケーションを終了中です"))
    );
    release.send(()).unwrap();
    queue.drained().await;
    let reopened = Database::open(&temporary.0).unwrap();
    let data = crate::read_state::get_all_read_ids(&reopened).unwrap();
    assert_eq!(data.kgc, ["accepted"]);
    assert_eq!(data.luna.len(), 2);
    queue.reopen();
    assert_eq!(
        call(
            &view,
            "mark_notification_read",
            json!({"source":"kwic","id":"resumed"})
        )
        .unwrap(),
        "null"
    );
    assert_eq!(
        canonical_ids(call(&view, "get_read_notifications", json!({})).unwrap()),
        json!({"kgc":["accepted"],"luna":["one","two"],"kwic":["resumed"]})
    );
}

#[test]
fn persistence_errors_reach_ipc_and_retries_preserve_every_source() {
    let temporary = Temporary::new();
    let app = app(&temporary, Some(Arc::new(Queue::new(FAILURE))));
    let view = webview(&app, "main");
    let initial = json!({"kgc":["kgc seed"],"luna":["luna seed"],"kwic":["kwic seed"]});
    app.state::<Database>()
        .save_data_cache("read_state", &initial.to_string())
        .unwrap();
    let before = app
        .state::<Database>()
        .get_data_cache("read_state")
        .unwrap();
    let conn = temporary.connection();
    conn.execute_batch("CREATE TRIGGER reject_read_commit BEFORE UPDATE ON data_cache WHEN OLD.cache_key='read_state' BEGIN SELECT RAISE(ABORT,'fixture commit failure'); END;").unwrap();
    let single = call(
        &view,
        "mark_notification_read",
        json!({"source":"luna","id":"new single"}),
    )
    .unwrap_err();
    let batch = call(
        &view,
        "mark_batch_notification_read",
        json!({"source":"kwic","ids":["new batch"]}),
    )
    .unwrap_err();
    assert!(single.as_str().unwrap().contains("fixture commit failure"));
    assert!(batch.as_str().unwrap().contains("fixture commit failure"));
    assert_eq!(
        app.state::<Database>()
            .get_data_cache("read_state")
            .unwrap(),
        before
    );
    assert_eq!(
        call(&view, "unrelated_ping", json!({})).unwrap(),
        "\"responsive\""
    );
    conn.execute_batch("DROP TRIGGER reject_read_commit;")
        .unwrap();
    let retry_single = call(
        &view,
        "mark_notification_read",
        json!({"source":"luna","id":"new single"}),
    )
    .unwrap();
    let retry_batch = call(
        &view,
        "mark_batch_notification_read",
        json!({"source":"kwic","ids":["new batch"]}),
    )
    .unwrap();
    let final_ids = canonical_ids(call(&view, "get_read_notifications", json!({})).unwrap());
    assert_eq!(
        final_ids,
        json!({"kgc":["kgc seed"],"luna":["luna seed","new single"],"kwic":["kwic seed","new batch"]})
    );
    let saved = app.state::<Database>().cache_payload("read_state").unwrap();
    app.state::<Database>()
        .save_data_cache("read_state", "invalid cached JSON")
        .unwrap();
    let read = call(&view, "get_read_notifications", json!({})).unwrap_err();
    assert!(read.as_str().unwrap().starts_with("既読データの解析失敗:"));
    assert!(call(
        &view,
        "mark_notification_read",
        json!({"source":"luna","id":"must not overwrite"})
    )
    .is_err());
    assert_eq!(
        app.state::<Database>()
            .cache_payload("read_state")
            .as_deref(),
        Some("invalid cached JSON")
    );
    app.state::<Database>()
        .save_data_cache("read_state", &saved)
        .unwrap();
    assert_eq!(
        canonical_ids(call(&view, "get_read_notifications", json!({})).unwrap()),
        final_ids
    );
    let wire = json!({"initial":initial,"single_error":single,"batch_error":batch,"read_error":read,"retry_single":serde_json::from_str::<Value>(&retry_single).unwrap(),"retry_batch":serde_json::from_str::<Value>(&retry_batch).unwrap(),"retry_read":final_ids});
    if let Some(path) = std::env::var_os("SELAH_READ_STATE_ERROR_WIRE") {
        std::fs::write(path, serde_json::to_string_pretty(&wire).unwrap() + "\n").unwrap();
    } else {
        let expected: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/read-state-error-wire.json"
        ))
        .unwrap();
        assert_eq!(
            wire, expected,
            "regenerate the native fixture explicitly if the IPC error contract changes"
        );
    }
}

#[test]
fn previous_commands_falsely_acknowledge_failed_writes_and_overwrite_unreadable_cache() {
    let old_temporary = Temporary::new();
    let new_temporary = Temporary::new();
    let old_app = app(&old_temporary, None);
    let new_app = app(&new_temporary, Some(Arc::new(Queue::new(FAILURE))));
    seed_read_state(&old_app);
    seed_read_state(&new_app);
    let old = webview(&old_app, "old");
    let new = webview(&new_app, "new");
    for temporary in [&old_temporary, &new_temporary] {
        temporary.connection().execute_batch("CREATE TRIGGER reject_read_commit BEFORE UPDATE ON data_cache WHEN OLD.cache_key='read_state' BEGIN SELECT RAISE(ABORT,'fixture commit failure'); END;").unwrap();
    }
    for (command, body) in [
        (
            "mark_notification_read",
            json!({"source":"luna","id":"single"}),
        ),
        (
            "mark_batch_notification_read",
            json!({"source":"luna","ids":["batch"]}),
        ),
    ] {
        assert_eq!(call(&old, command, body.clone()).unwrap(), "null");
        assert!(call(&new, command, body)
            .unwrap_err()
            .as_str()
            .unwrap()
            .contains("fixture commit failure"));
    }
    for (temporary, app) in [(&old_temporary, &old_app), (&new_temporary, &new_app)] {
        temporary
            .connection()
            .execute_batch("DROP TRIGGER reject_read_commit;")
            .unwrap();
        app.state::<Database>()
            .save_data_cache("read_state", "invalid cached JSON")
            .unwrap();
    }
    assert_eq!(
        canonical_ids(call(&old, "get_read_notifications", json!({})).unwrap()),
        json!({"kgc":[],"luna":[],"kwic":[]})
    );
    assert!(call(&new, "get_read_notifications", json!({})).is_err());
    assert_eq!(
        call(
            &old,
            "mark_notification_read",
            json!({"source":"luna","id":"only new ID"})
        )
        .unwrap(),
        "null"
    );
    assert!(call(
        &new,
        "mark_notification_read",
        json!({"source":"luna","id":"only new ID"})
    )
    .is_err());
    assert_eq!(
        canonical_ids(
            old_app
                .state::<Database>()
                .cache_payload("read_state")
                .unwrap()
        ),
        json!({"kgc":[],"luna":["only new ID"],"kwic":[]})
    );
    assert_eq!(
        new_app
            .state::<Database>()
            .cache_payload("read_state")
            .as_deref(),
        Some("invalid cached JSON")
    );
}

fn webview(app: &tauri::App<MockRuntime>, label: &str) -> tauri::WebviewWindow<MockRuntime> {
    tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App("index.html".into()))
        .build()
        .unwrap()
}

fn request(
    webview: &tauri::WebviewWindow<MockRuntime>,
    command: &str,
    body: InvokeBody,
) -> Receiver<(std::thread::ThreadId, Result<String, Value>)> {
    let (sender, receiver) = channel();
    webview.as_ref().clone().on_message(
        tauri::webview::InvokeRequest {
            cmd: command.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "http://tauri.localhost".parse().unwrap(),
            body,
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.into(),
        },
        Box::new(move |_, _, response, _, _| {
            let response = match response {
                InvokeResponse::Ok(tauri::ipc::InvokeResponseBody::Json(json)) => Ok(json),
                InvokeResponse::Ok(_) => panic!("cache reply must be JSON"),
                InvokeResponse::Err(error) => Err(error.0),
            };
            // A closed webview can drop a reply without revoking the write.
            let _ = sender.send((std::thread::current().id(), response));
        }),
    );
    receiver
}
fn reply(
    receiver: Receiver<(std::thread::ThreadId, Result<String, Value>)>,
) -> Result<String, Value> {
    receiver.recv_timeout(Duration::from_secs(10)).unwrap().1
}
fn call(
    webview: &tauri::WebviewWindow<MockRuntime>,
    command: &str,
    body: Value,
) -> Result<String, Value> {
    reply(request(webview, command, InvokeBody::Json(body)))
}

#[test]
fn complete_cache_strings_null_and_scalar_timestamps_match_the_old_ipc() {
    let old_temporary = Temporary::new();
    let new_temporary = Temporary::new();
    let old_app = app(&old_temporary, None);
    let new_app = app(&new_temporary, Some(Arc::new(Queue::new(FAILURE))));
    let old = webview(&old_app, "old");
    let new = webview(&new_app, "new");
    let huge = "日本語 中文 👩🏽‍💻\n\"quoted\" \\ \0\r\t".repeat(65_536);
    for (key, text) in [
        ("", ""),
        ("quotes' 🌕", "not JSON"),
        ("large", huge.as_str()),
    ] {
        let body = json!({"key":key,"json":text,"unused":42});
        assert_eq!(call(&old, "save_data_cache", body.clone()).unwrap(), "null");
        assert_eq!(call(&new, "save_data_cache", body).unwrap(), "null");
        let previous = call(&old, "get_data_cache", json!({"key":key})).unwrap();
        let current = call(&new, "get_data_cache", json!({"key":key})).unwrap();
        assert_eq!(current, previous);
        assert_eq!(serde_json::from_str::<String>(&current).unwrap(), text);
    }
    for temporary in [&old_temporary, &new_temporary] {
        let conn = temporary.connection();
        for (key, timestamp) in [("large", 91_i64), ("zero", 0), ("negative", -3)] {
            conn.execute("INSERT INTO data_cache(cache_key,data_json,updated_at) VALUES (?1,'x',?2) ON CONFLICT(cache_key) DO UPDATE SET updated_at=?2", rusqlite::params![key,timestamp]).unwrap();
        }
        conn.execute(
            "INSERT INTO data_cache(cache_key,data_json,updated_at) VALUES ('invalid_utf8',?1,123)",
            rusqlite::params![vec![0xff_u8; 2 * 1024 * 1024]],
        )
        .unwrap();
    }
    for key in ["large", "zero", "negative", "absent", "invalid_utf8"] {
        for command in ["get_data_cache", "get_data_cache_updated_at"] {
            assert_eq!(
                call(&new, command, json!({"key":key})),
                call(&old, command, json!({"key":key}))
            );
        }
    }
    assert_eq!(
        call(&new, "get_data_cache", json!({"key":"absent"})).unwrap(),
        "null"
    );
    assert_eq!(
        call(&new, "get_data_cache", json!({"key":"invalid_utf8"})).unwrap(),
        "null"
    );
    assert_eq!(
        call(
            &new,
            "get_data_cache_updated_at",
            json!({"key":"invalid_utf8"})
        )
        .unwrap(),
        "123"
    );
}

#[test]
fn malformed_arguments_reserved_keys_and_db_errors_match_the_old_ipc() {
    let old_temporary = Temporary::new();
    let new_temporary = Temporary::new();
    let old_app = app(&old_temporary, None);
    let new_app = app(&new_temporary, Some(Arc::new(Queue::new(FAILURE))));
    let old = webview(&old_app, "old");
    let new = webview(&new_app, "new");
    for command in [
        "get_data_cache",
        "get_data_cache_updated_at",
        "save_data_cache",
    ] {
        for body in [
            json!({}),
            json!({"key":null}),
            json!({"key":3}),
            json!({"key":[]}),
            json!({"key":"x"}),
            json!({"key":"x","json":null}),
            json!({"key":"x","json":{}}),
            json!({"key":"seen_notifs_luna","json":"original"}),
            json!({"key":"seen_notifs_luna"}),
        ] {
            assert_eq!(call(&new, command, body.clone()), call(&old, command, body));
        }
        assert_eq!(
            reply(request(&new, command, InvokeBody::Raw(vec![1, 2, 3]))),
            reply(request(&old, command, InvokeBody::Raw(vec![1, 2, 3])))
        );
    }
    for temporary in [&old_temporary, &new_temporary] {
        temporary
            .connection()
            .execute_batch("DROP TABLE data_cache;")
            .unwrap();
    }
    for command in [
        "get_data_cache",
        "get_data_cache_updated_at",
        "save_data_cache",
    ] {
        let body = json!({"key":"x","json":"complete"});
        assert_eq!(call(&new, command, body.clone()), call(&old, command, body));
    }
}

#[test]
fn admission_is_nonblocking_and_cache_order_survives_reverse_waits_and_dropped_replies() {
    let temporary = Temporary::new();
    let queue = Arc::new(Queue::new(FAILURE));
    let app = app(&temporary, Some(queue.clone()));
    let a = webview(&app, "a");
    let b = webview(&app, "b");
    let (entered, started) = channel();
    let (release, released) = channel();
    drop(queue.submit(move || {
        entered.send(()).unwrap();
        released.recv_timeout(Duration::from_secs(10)).unwrap();
        Ok(())
    }));
    started.recv_timeout(Duration::from_secs(10)).unwrap();
    let caller = std::thread::current().id();
    let mut replies = Vec::new();
    for index in 0..32 {
        let target = if index % 2 == 0 { &a } else { &b };
        let save = request(
            target,
            "save_data_cache",
            InvokeBody::Json(json!({"key":"ordered","json":format!("{index}: full data 🌕")})),
        );
        if index % 3 == 0 {
            drop(save);
        } else {
            replies.push((save, "null".to_string()));
        }
        let read = request(
            target,
            "get_data_cache",
            InvokeBody::Json(json!({"key":"ordered"})),
        );
        replies.push((
            read,
            serde_json::to_string(&format!("{index}: full data 🌕")).unwrap(),
        ));
    }
    // All admissions returned with the worker held. A generated command still
    // responds synchronously, and no queued cache job has touched the DB.
    assert_eq!(
        call(&a, "unrelated_ping", json!({})).unwrap(),
        "\"responsive\""
    );
    assert!(app.state::<Database>().cache_payload("ordered").is_none());
    for (receiver, _) in &replies {
        assert!(receiver.try_recv().is_err());
    }
    release.send(()).unwrap();
    for (receiver, expected) in replies.into_iter().rev() {
        let (thread, result) = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_ne!(thread, caller);
        assert_eq!(result.unwrap(), expected);
    }
    assert_eq!(
        app.state::<Database>().cache_payload("ordered").unwrap(),
        "31: full data 🌕"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn seal_drains_admitted_cache_writes_and_reopen_accepts_new_requests() {
    let temporary = Temporary::new();
    let queue = Arc::new(Queue::new(FAILURE));
    let app = app(&temporary, Some(queue.clone()));
    let view = webview(&app, "main");
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = channel();
    drop(queue.submit(move || {
        entered.send(()).unwrap();
        released.recv_timeout(Duration::from_secs(10)).unwrap();
        Ok(())
    }));
    started.await.unwrap();
    drop(request(
        &view,
        "save_data_cache",
        InvokeBody::Json(json!({"key":"saved","json":"admitted 🌕"})),
    ));
    drop(request(
        &view,
        "save_data_cache",
        InvokeBody::Json(json!({"key":"seen_notifs_luna","json":"rejected"})),
    ));
    queue.seal();
    assert_eq!(
        call(
            &view,
            "save_data_cache",
            json!({"key":"rejected","json":"never"})
        ),
        Err(json!("アプリケーションを終了中です"))
    );
    let waiting = queue.clone();
    let drained = tokio::spawn(async move { waiting.drained().await });
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    assert!(!drained.is_finished());
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), drained)
        .await
        .unwrap()
        .unwrap();
    let reopened = Database::open(&temporary.0).unwrap();
    assert_eq!(
        reopened.cache_payload("saved").as_deref(),
        Some("admitted 🌕")
    );
    assert!(reopened.cache_payload("rejected").is_none());
    assert!(reopened.cache_payload("seen_notifs_luna").is_none());
    queue.reopen();
    assert_eq!(
        call(
            &view,
            "save_data_cache",
            json!({"key":"saved","json":"resumed"})
        )
        .unwrap(),
        "null"
    );
    assert_eq!(
        call(&view, "get_data_cache", json!({"key":"saved"})).unwrap(),
        "\"resumed\""
    );
    queue.seal();
    queue.drained().await;
}
