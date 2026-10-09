use super::*;

struct Temporary(std::path::PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn ipc_mutation_fields_preserve_existing_names_null_options_and_reject_invalid_types() {
    use serde_json::json;
    for title in [
        None,
        Some(Value::Null),
        Some(json!("日本語 🌕")),
        Some(json!("")),
    ] {
        let mut payload = json!({"unused": [1,2,3]});
        if let Some(title) = title {
            payload["title"] = title;
        }
        let request = decode(Kind::Create, &InvokeBody::Json(payload)).unwrap();
        assert!(matches!(request, Mutation::Create(_)));
    }
    assert!(
        matches!(decode(Kind::Select, &InvokeBody::Json(json!({"convId":"c","title":9}))).unwrap(), Mutation::Select(id) if id=="c")
    );
    assert!(
        matches!(decode(Kind::Delete, &InvokeBody::Json(json!({"convId":"c"}))).unwrap(), Mutation::Delete(id) if id=="c")
    );
    assert!(
        matches!(decode(Kind::Rename, &InvokeBody::Json(json!({"convId":"c","title":"full title 🌕"}))).unwrap(), Mutation::Rename(id,title) if id=="c" && title=="full title 🌕")
    );
    for (kind, payload) in [
        (Kind::Create, json!({"title":3})),
        (Kind::Select, json!({})),
        (Kind::Select, json!({"convId":null})),
        (Kind::Delete, json!({"convId":[] })),
        (Kind::Rename, json!({"convId":"c"})),
        (Kind::Rename, json!({"convId":"c","title":null})),
    ] {
        assert!(decode(kind, &InvokeBody::Json(payload)).is_err());
    }
    assert!(decode(Kind::Create, &InvokeBody::Raw(Vec::new())).is_err());
    for command in [
        "agent_send",
        "agent_cancel",
        "agent_load_messages",
        "agent_list_conversations",
    ] {
        assert!(Kind::from_command(command).is_none());
    }
}

#[test]
fn mutation_notifications_follow_successful_commits_and_deleted_scope_invalidation() {
    use crate::agent_turn_scope::RunningTurn;
    use tauri::ipc::IpcResponse;
    let temporary =
        Temporary(std::env::temp_dir().join(format!("selah-mutations-{}", uuid::Uuid::new_v4())));
    let db = Database::open(&temporary.0).unwrap();
    let create = apply(&db, Mutation::Create(None), |_, _| {
        panic!("create added an unexpected event")
    })
    .unwrap();
    let id = create.as_str().unwrap();
    assert_eq!(
        create
            .clone()
            .body()
            .unwrap()
            .deserialize::<String>()
            .unwrap(),
        id
    );
    assert_eq!(
        Value::Null
            .body()
            .unwrap()
            .deserialize::<Option<String>>()
            .unwrap(),
        None
    );
    assert_eq!(uuid::Uuid::parse_str(id).unwrap().get_version_num(), 4);
    assert_eq!(
        db.agent_list_conversations().unwrap()[0].title,
        "新しい会話"
    );
    let mut events = Vec::new();
    assert_eq!(
        apply(&db, Mutation::Select(id.into()), |name, id| events
            .push((name.to_owned(), id.to_owned())))
        .unwrap(),
        Value::Null
    );
    assert_eq!(
        events,
        [("agent-active-conversation-changed".into(), id.into())]
    );
    apply(&db, Mutation::Select(id.into()), |_, _| {
        panic!("same selection emitted an event")
    })
    .unwrap();
    apply(
        &db,
        Mutation::Rename(id.into(), "renamed 🌕".into()),
        |name, got| {
            assert_eq!(name, "agent-conversations-changed");
            assert_eq!(got, id);
            assert_eq!(
                db.agent_list_conversations().unwrap()[0].title,
                "renamed 🌕"
            );
        },
    )
    .unwrap();
    let running = RunningTurn::begin(id, None);
    db.agent_append_message(id, "user", "input", None, None, None)
        .unwrap();
    let conn = rusqlite::Connection::open(temporary.0.join("courses.db")).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_delete BEFORE DELETE ON agent_messages BEGIN SELECT RAISE(ABORT, 'delete failed'); END;").unwrap();
    assert!(apply(&db, Mutation::Delete(id.into()), |_, _| panic!(
        "failed deletion emitted an event"
    ))
    .is_err());
    assert!(!running.turn.cancelled());
    conn.execute_batch("DROP TRIGGER reject_delete;").unwrap();
    events.clear();
    assert_eq!(
        apply(&db, Mutation::Delete(id.into()), |name, got| {
            assert!(!running.turn.accepts_event(true));
            assert!(running.turn.cancelled());
            assert!(db.agent_load_messages(id).unwrap().is_empty());
            assert_eq!(db.agent_active_conversation().unwrap(), None);
            events.push((name.to_owned(), got.to_owned()));
        })
        .unwrap(),
        Value::Null
    );
    assert_eq!(
        events,
        [
            ("agent-conversation-deleted".into(), id.into()),
            ("agent-conversations-changed".into(), id.into())
        ]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn queued_selection_rename_and_deletion_keep_ipc_order_with_reverse_completion_waits() {
    let temporary = Temporary(
        std::env::temp_dir().join(format!("selah-ordered-mutations-{}", uuid::Uuid::new_v4())),
    );
    let db = Arc::new(Database::open(&temporary.0).unwrap());
    for id in ["A", "B"] {
        db.agent_create_conversation(id, id).unwrap();
    }
    let queue = Arc::new(Queue::new("会話の処理に失敗しました"));
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let first_db = db.clone();
    let first = queue.submit(move || {
        entered.send(()).unwrap();
        released.recv().unwrap();
        apply(&first_db, Mutation::Select("A".into()), |_, _| {})
    });
    let mut replies = Vec::new();
    for mutation in [
        Mutation::Rename("A".into(), "first".into()),
        Mutation::Select("B".into()),
        Mutation::Rename("A".into(), "last".into()),
        Mutation::Delete("A".into()),
    ] {
        let db = db.clone();
        replies.push(queue.submit(move || apply(&db, mutation, |_, _| {})));
    }
    started.await.unwrap();
    assert_eq!(db.agent_active_conversation().unwrap(), None);
    release.send(()).unwrap();
    for reply in replies.into_iter().rev() {
        assert_eq!(reply.await.unwrap(), Value::Null);
    }
    assert_eq!(first.await.unwrap(), Value::Null);
    assert_eq!(
        db.agent_active_conversation().unwrap().as_deref(),
        Some("B")
    );
    assert_eq!(db.agent_list_conversations().unwrap().len(), 1);
    assert_eq!(db.agent_list_conversations().unwrap()[0].id, "B");
}

#[tokio::test(flavor = "current_thread")]
async fn sealed_queue_drains_all_admitted_crud_and_delete_cleanup_after_responses_are_dropped() {
    let temporary =
        Temporary(std::env::temp_dir().join(format!("selah-crud-exit-{}", uuid::Uuid::new_v4())));
    let db = Arc::new(Database::open(&temporary.0).unwrap());
    let id = uuid_v4();
    db.agent_create_conversation(&id, "original").unwrap();
    db.agent_append_message(&id, "user", "complete input 🌕", None, None, None)
        .unwrap();
    let running = crate::agent_turn_scope::RunningTurn::begin(&id, None);
    let queue = Arc::new(Queue::new("会話の処理に失敗しました"));
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let created = Arc::new(std::sync::Mutex::new(None));
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let first_db = db.clone();
    let first_id = id.clone();
    let first_events = events.clone();
    drop(queue.submit(move || {
        entered.send(()).unwrap();
        released.recv().unwrap();
        apply(
            &first_db,
            Mutation::Rename(first_id, "accepted rename 🌕".into()),
            |name, id| {
                first_events
                    .lock()
                    .unwrap()
                    .push((name.to_owned(), id.to_owned()));
            },
        )
    }));
    for mutation in [
        Mutation::Select(id.clone()),
        Mutation::Create(Some("新しい会話の完全なタイトル 🌕".into())),
        Mutation::Delete(id.clone()),
    ] {
        let db = db.clone();
        let events = events.clone();
        let created = created.clone();
        let owner = running.turn.clone();
        drop(queue.submit(move || {
            let result = apply(&db, mutation, |name, id| {
                if name == "agent-conversation-deleted" {
                    assert!(owner.cancelled());
                    assert!(!owner.accepts_event(true));
                    assert!(db.agent_load_messages(id).unwrap().is_empty());
                }
                events
                    .lock()
                    .unwrap()
                    .push((name.to_owned(), id.to_owned()));
            })?;
            if let Value::String(id) = &result {
                *created.lock().unwrap() = Some(id.clone());
            }
            Ok(result)
        }));
    }
    started.await.unwrap();
    queue.seal();
    let rejecting_db = db.clone();
    assert!(queue
        .submit(move || apply(
            &rejecting_db,
            Mutation::Create(Some("should not exist".into())),
            |_, _| {}
        ))
        .await
        .is_err());
    release.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), queue.drained())
        .await
        .unwrap();
    let created_id = created.lock().unwrap().clone().unwrap();
    let reopened = Database::open(&temporary.0).unwrap();
    let rows = reopened.agent_list_conversations().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, created_id);
    assert_eq!(rows[0].title, "新しい会話の完全なタイトル 🌕");
    assert_eq!(reopened.agent_active_conversation().unwrap(), None);
    assert!(reopened.agent_load_messages(&id).unwrap().is_empty());
    assert!(running.turn.cancelled());
    assert_eq!(
        *events.lock().unwrap(),
        [
            ("agent-conversations-changed".into(), id.clone()),
            ("agent-active-conversation-changed".into(), id.clone()),
            ("agent-conversation-deleted".into(), id.clone()),
            ("agent-conversations-changed".into(), id),
        ]
    );
}
