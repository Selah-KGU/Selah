use super::*;
use std::sync::Arc;

struct Temporary(std::path::PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn encoded_metadata_replies_keep_the_existing_array_string_and_null_shapes() {
    use tauri::ipc::IpcResponse;
    let temporary = Temporary(
        std::env::temp_dir().join(format!("selah-metadata-reply-{}", uuid::Uuid::new_v4())),
    );
    let db = Database::open(&temporary.0).unwrap();
    db.agent_create_conversation("c", "日本語 title 🌕")
        .unwrap();
    let rows = db
        .agent_list_conversations()
        .unwrap()
        .into_iter()
        .map(super::super::AgentConversationSummary::from)
        .collect::<Vec<_>>();
    let result = json_response(&rows)
        .unwrap()
        .body()
        .unwrap()
        .deserialize::<Vec<super::super::AgentConversationSummary>>()
        .unwrap();
    assert_eq!(
        serde_json::to_value(result).unwrap(),
        serde_json::to_value(rows).unwrap()
    );
    let current = json_response(&db.agent_active_conversation().unwrap())
        .unwrap()
        .body()
        .unwrap()
        .deserialize::<Option<String>>()
        .unwrap();
    assert_eq!(current, None);
    db.agent_set_active_conversation("c").unwrap();
    let current = json_response(&db.agent_active_conversation().unwrap())
        .unwrap()
        .body()
        .unwrap()
        .deserialize::<String>()
        .unwrap();
    assert_eq!(current, "c");
}

#[tokio::test(flavor = "current_thread")]
async fn locked_sqlite_command_keeps_the_executor_available_and_returns_the_committed_result() {
    let temporary = Temporary(
        std::env::temp_dir().join(format!("selah-command-worker-{}", uuid::Uuid::new_v4())),
    );
    let db = Arc::new(Database::open(&temporary.0).unwrap());
    db.agent_create_conversation("c", "test").unwrap();
    let conn = rusqlite::Connection::open(temporary.0.join("courses.db")).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE;").unwrap();
    let worker_db = db.clone();
    let (entered, started) = tokio::sync::oneshot::channel();
    let caller_thread = std::thread::current().id();
    let task = tokio::spawn(run(move || {
        assert_ne!(std::thread::current().id(), caller_thread);
        entered.send(()).unwrap();
        worker_db.agent_append_message("c", "user", "全文 🌕", None, None, None)
    }));
    started.await.unwrap();
    assert_eq!(
        tokio::spawn(async { "executor available" }).await.unwrap(),
        "executor available"
    );
    assert!(
        !task.is_finished(),
        "SQL completed while another writer held the write lock"
    );
    conn.execute_batch("COMMIT;").unwrap();
    let message_id = tokio::time::timeout(std::time::Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(db.agent_load_messages("c").unwrap()[0].id, message_id);
    assert_eq!(db.agent_load_messages("c").unwrap()[0].content, "全文 🌕");
    let failure = run(|| Err::<(), _>("original database error".into()))
        .await
        .unwrap_err();
    assert_eq!(failure, "original database error");
    let panic = run(|| -> Result<(), String> { panic!("worker failure") })
        .await
        .unwrap_err();
    assert!(panic.starts_with("会話の処理に失敗しました:"));
}

#[tokio::test(flavor = "current_thread")]
async fn aborting_the_response_waiter_does_not_skip_committed_delete_cleanup() {
    use crate::agent_turn_scope::RunningTurn;
    let id = format!("selah-delete-worker-{}", uuid::Uuid::new_v4());
    let temporary = Temporary(std::env::temp_dir().join(&id));
    let db = Arc::new(Database::open(&temporary.0).unwrap());
    db.agent_create_conversation(&id, "test").unwrap();
    db.agent_append_message(&id, "user", "input", None, None, None)
        .unwrap();
    let running = RunningTurn::begin(&id, None);
    let worker_db = db.clone();
    let worker_id = id.clone();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let (committed, ended) = tokio::sync::oneshot::channel();
    let queue = Arc::new(crate::background_queue::Queue::new(
        "会話の処理に失敗しました",
    ));
    let completion = queue.submit(move || {
        entered.send(()).unwrap();
        released.recv().unwrap();
        super::super::lifecycle::delete_conversation(&worker_db, &worker_id)?;
        committed.send(()).unwrap();
        Ok(())
    });
    let task = tokio::spawn(completion);
    started.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(!running.turn.cancelled());
    release.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), ended)
        .await
        .unwrap()
        .unwrap();
    assert!(running.turn.cancelled());
    assert!(!running.turn.accepts_event(true));
    assert!(!running.turn.accepts_event(false));
    assert!(db.agent_load_messages(&id).unwrap().is_empty());
}
