use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::oneshot;

#[tokio::test(flavor = "current_thread")]
async fn slow_turn_preparation_keeps_the_executor_available() {
    let (entered, started) = oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let task = tokio::spawn(blocking(move || {
        entered.send(()).unwrap();
        released.recv().unwrap();
        Ok(42)
    }));
    started.await.unwrap();
    assert_eq!(tokio::spawn(async { 7 }).await.unwrap(), 7);
    assert!(!task.is_finished());
    release.send(()).unwrap();
    assert_eq!(task.await.unwrap().unwrap(), 42);
}
#[test]
fn persistence_failure_does_not_resolve_provider_or_read_history() {
    let result: Result<((), (), ()), _> = persisted_turn(
        || Err(AgentError::db("write failed")),
        || panic!("provider resolved before persistence"),
        || panic!("history loaded after failed persistence"),
    );
    assert!(matches!(result, Err(AgentError::DbError(_))));
}
#[test]
fn input_is_saved_even_if_model_resolution_fails_and_history_errors_are_reported() {
    let saved = Arc::new(AtomicBool::new(false));
    let result: Result<((), (), ()), _> = persisted_turn(
        || {
            saved.store(true, Ordering::SeqCst);
            Ok(())
        },
        || Err(AgentError::config("AI disabled")),
        || panic!("history loaded after configuration error"),
    );
    assert!(saved.load(Ordering::SeqCst));
    assert!(matches!(result, Err(AgentError::ConfigError(_))));
    let result: Result<((), (), ()), _> =
        persisted_turn(|| Ok(()), || Ok(()), || Err(AgentError::db("read failed")));
    assert!(matches!(result, Err(AgentError::DbError(_))));
}

#[test]
fn real_sqlite_retains_the_complete_message_and_images_when_provider_startup_fails() {
    struct TempDir(std::path::PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let temporary =
        TempDir(std::env::temp_dir().join(format!("selah-agent-prepare-{}", uuid::Uuid::new_v4())));
    let db = crate::db::Database::open(&temporary.0).unwrap();
    db.agent_create_conversation("voice-a", "Voice Shortcut")
        .unwrap();
    let text = "第一の確定行。第二の確定行。最後の発話。";
    let images = vec![crate::ai::ImagePart {
        mime: "image/png".into(),
        data_base64: "AA==".into(),
    }];
    let result: Result<(i64, (), ()), _> = persisted_turn(
        || crate::agent::turn::persist_user_body(&db, "voice-a", text, &images),
        || Err(AgentError::config("simulated model unavailable")),
        || panic!("history read after startup failure"),
    );
    assert!(matches!(result, Err(AgentError::ConfigError(_))));
    let history = db.agent_load_messages("voice-a").unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].role, "user");
    assert_eq!(history[0].content, text);
    assert_eq!(
        history[0].images_json.as_deref(),
        Some(serde_json::to_string(&images).unwrap().as_str())
    );
}

#[test]
fn automatic_title_changes_only_default_titles_and_respects_manual_renames() {
    struct TempDir(std::path::PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let temporary =
        TempDir(std::env::temp_dir().join(format!("selah-agent-title-{}", uuid::Uuid::new_v4())));
    let db = crate::db::Database::open(&temporary.0).unwrap();
    for (id, title) in [
        ("empty", ""),
        ("new", "新しい会話"),
        ("agent", "エージェント"),
        ("voice", "Voice Shortcut"),
        ("manual", "my title"),
    ] {
        db.agent_create_conversation(id, title).unwrap();
        assert_eq!(
            db.agent_autotitle_conversation(id, "generated").unwrap(),
            matches!(id, "empty" | "new" | "agent")
        );
    }
    assert!(!db
        .agent_autotitle_conversation("missing", "generated")
        .unwrap());
    db.agent_create_conversation("race", "新しい会話").unwrap();
    db.agent_rename_conversation("race", "manual choice")
        .unwrap();
    assert!(!db
        .agent_autotitle_conversation("race", "late automatic title")
        .unwrap());
    let rows = db.agent_list_conversations().unwrap();
    assert_eq!(
        rows.iter().find(|row| row.id == "race").unwrap().title,
        "manual choice"
    );
    assert_eq!(
        rows.iter().find(|row| row.id == "voice").unwrap().title,
        "Voice Shortcut"
    );
    assert_eq!(
        rows.iter().find(|row| row.id == "manual").unwrap().title,
        "my title"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_request_keeps_its_admitted_input_and_images_before_skipping_provider() {
    struct TempDir(std::path::PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let temporary =
        TempDir(std::env::temp_dir().join(format!("selah-cancel-save-{}", uuid::Uuid::new_v4())));
    let db = Arc::new(crate::db::Database::open(&temporary.0).unwrap());
    let id = format!("cancel-save-{}", uuid::Uuid::new_v4());
    db.agent_create_conversation(&id, "saved input").unwrap();
    let running = crate::agent_turn_scope::RunningTurn::begin(&id, Some("request".into()));
    assert!(crate::agent_turn_scope::cancel(&id, Some("request")));
    let owner = running.turn.clone();
    let worker_db = db.clone();
    let worker_id = id.clone();
    let text = "第一の確定行。第二の確定行。最後の発話。";
    let images = vec![crate::ai::ImagePart {
        mime: "image/png".into(),
        data_base64: "AA==".into(),
    }];
    let worker_images = images.clone();
    let result: Result<(i64, (), ()), _> = blocking(move || {
        persisted_turn(
            || crate::agent::turn::persist_user_body(&worker_db, &worker_id, text, &worker_images),
            || {
                if owner.cancelled() {
                    Err(AgentError::Cancelled)
                } else {
                    panic!("cancelled request resolved the model")
                }
            },
            || panic!("cancelled request loaded inference history"),
        )
    })
    .await;
    assert!(matches!(result, Err(AgentError::Cancelled)));
    let history = db.agent_load_messages(&id).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].content, text);
    assert_eq!(
        history[0].images_json.as_deref(),
        Some(serde_json::to_string(&images).unwrap().as_str())
    );
}
