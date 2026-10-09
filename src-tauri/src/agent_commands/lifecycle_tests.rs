use super::*;
use crate::agent_turn_scope::RunningTurn;

struct Temporary(std::path::PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn failed_delete_preserves_inference_until_the_complete_transaction_commits() {
    for table in [
        "agent_messages",
        "agent_conversations",
        "data_cache",
        "deferred_constraint",
    ] {
        let id = format!("delete-lifecycle-{}", uuid::Uuid::new_v4());
        let temporary = Temporary(std::env::temp_dir().join(&id));
        let db = Database::open(&temporary.0).unwrap();
        db.agent_create_conversation(&id, "keep me").unwrap();
        db.agent_set_active_conversation(&id).unwrap();
        db.agent_append_message(&id, "user", "全文", Some("[]"), None, None)
            .unwrap();
        let running = RunningTurn::begin(&id, None);
        let other = RunningTurn::begin(&format!("other-{id}"), None);
        let before = serde_json::to_value(db.agent_load_messages(&id).unwrap()).unwrap();
        let conn = rusqlite::Connection::open(temporary.0.join("courses.db")).unwrap();
        if table == "deferred_constraint" {
            conn.execute_batch("CREATE TABLE delete_guard(conv_id TEXT REFERENCES agent_conversations(id) DEFERRABLE INITIALLY DEFERRED);").unwrap();
            conn.execute("INSERT INTO delete_guard(conv_id) VALUES (?1)", [&id])
                .unwrap();
        } else {
            let operation = if table == "data_cache" {
                "UPDATE"
            } else {
                "DELETE"
            };
            conn.execute_batch(&format!("CREATE TRIGGER reject_delete BEFORE {operation} ON {table} BEGIN SELECT RAISE(ABORT, 'delete failed'); END;")).unwrap();
        }
        assert!(delete_conversation(&db, &id).is_err(), "{table}");
        assert!(
            !running.turn.cancelled(),
            "failed {table} deletion cancelled valid inference"
        );
        assert!(running.turn.accepts_event(false));
        assert!(running.turn.accepts_event(true));
        assert_eq!(
            serde_json::to_value(db.agent_load_messages(&id).unwrap()).unwrap(),
            before
        );
        assert_eq!(
            db.agent_active_conversation().unwrap().as_deref(),
            Some(id.as_str())
        );
        if table == "deferred_constraint" {
            conn.execute_batch("DROP TABLE delete_guard;").unwrap();
        } else {
            conn.execute_batch("DROP TRIGGER reject_delete;").unwrap();
        }
        delete_conversation(&db, &id).unwrap();
        assert!(running.turn.cancelled());
        assert!(!running.turn.accepts_event(false));
        assert!(!running.turn.accepts_event(true));
        assert!(!other.turn.cancelled());
        assert!(other.turn.accepts_event(false));
        assert!(db.agent_load_messages(&id).unwrap().is_empty());
        assert_eq!(db.agent_active_conversation().unwrap(), None);
        delete_conversation(&db, &id).unwrap();
        // A later request owns a different generation even if a caller reuses
        // this conversation ID; finishing old workers cannot cancel it.
        let next = RunningTurn::begin(&id, None);
        drop(running);
        assert!(!next.turn.cancelled());
        assert!(next.turn.accepts_event(true));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn committed_delete_closes_a_real_stalled_http_provider_without_frontend_cancellation() {
    use crate::agent_error::AgentError;
    use crate::agent_provider::AgentProvider;
    use crate::agent_turn_scope::CURRENT;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for streaming in [false, true] {
        let id = format!("delete-http-{}", uuid::Uuid::new_v4());
        let temporary = Temporary(std::env::temp_dir().join(&id));
        let db = Database::open(&temporary.0).unwrap();
        db.agent_create_conversation(&id, "test").unwrap();
        db.agent_append_message(&id, "user", "input", None, None, None)
            .unwrap();
        let conn = rusqlite::Connection::open(temporary.0.join("courses.db")).unwrap();
        conn.execute_batch("CREATE TRIGGER reject_delete BEFORE DELETE ON agent_messages BEGIN SELECT RAISE(ABORT, 'delete failed'); END;").unwrap();
        let running = RunningTurn::begin(&id, None);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = crate::ai::AiConfig {
            provider: "openai".into(),
            base_url: format!("http://{}", listener.local_addr().unwrap()),
            api_key: "test-placeholder".into(),
            model: "local-http-fixture".into(),
            ..Default::default()
        };
        let task_id = id.clone();
        let (token_received, first_token) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(CURRENT.scope(running.turn.clone(), async move {
            let provider = AgentProvider::Remote { config };
            if streaming {
                let mut token_received = Some(token_received);
                provider
                    .answer(Vec::new(), &task_id, 0, move |text, _| {
                        if !text.is_empty() {
                            if let Some(sender) = token_received.take() {
                                let _ = sender.send(());
                            }
                        }
                    })
                    .await
            } else {
                provider.plan(Vec::new(), 32, 0.0, "", 0, &task_id).await
            }
        }));
        let deadline = std::time::Duration::from_secs(3);
        let (mut socket, _) = tokio::time::timeout(deadline, listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut request = Vec::new();
        loop {
            let mut bytes = [0; 4096];
            let count = tokio::time::timeout(deadline, socket.read(&mut bytes))
                .await
                .unwrap()
                .unwrap();
            assert_ne!(count, 0);
            request.extend_from_slice(&bytes[..count]);
            if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&request[..end]).to_lowercase();
                let length: usize = header
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        assert!(String::from_utf8_lossy(&request).contains("/chat/completions"));
        if streaming {
            let data = format!(
                "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}}}}]}}\n\n",
                "live token ".repeat(20)
            );
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n", data.len(), data);
            socket.write_all(response.as_bytes()).await.unwrap();
            tokio::time::timeout(deadline, first_token)
                .await
                .unwrap()
                .unwrap();
        }
        assert!(delete_conversation(&db, &id).is_err());
        assert!(!running.turn.cancelled());
        assert!(!task.is_finished());
        conn.execute_batch("DROP TRIGGER reject_delete;").unwrap();
        delete_conversation(&db, &id).unwrap();
        // No headers/next token are sent, and no UI cancellation RPC is needed.
        let result = tokio::time::timeout(deadline, task).await.unwrap().unwrap();
        assert!(matches!(result, Err(AgentError::Cancelled)));
        assert!(!running.turn.accepts_event(true));
        let mut byte = [0; 1];
        let closed = tokio::time::timeout(deadline, socket.read(&mut byte))
            .await
            .unwrap();
        assert!(
            matches!(closed, Ok(0) | Err(_)),
            "deleted request kept HTTP open"
        );
    }
}
