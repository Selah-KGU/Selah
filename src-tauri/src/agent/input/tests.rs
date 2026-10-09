use super::*;
use std::sync::Arc;

struct TempDatabase {
    db: Database,
    path: std::path::PathBuf,
}

impl TempDatabase {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("selah-voice-input-{}", uuid::Uuid::new_v4()));
        Self {
            db: Database::open(&path).unwrap(),
            path,
        }
    }
}

impl Drop for TempDatabase {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn committed_voice_receipt_preserves_input_without_a_second_message_on_provider_failure() {
    let temporary = TempDatabase::new();
    let text = "第一の確定行。\n第二の確定行。末尾の発話。";
    let saved = save_voice_input(&temporary.db, "voice-a".into(), text.into()).unwrap();
    assert_eq!(saved.conversation_id(), "voice-a");
    let input = TurnInput::Voice(saved);
    let result: Result<(CommittedInput, (), ()), _> = super::super::prepare::persisted_turn(
        || input.persist_with("voice-a", |_, _| panic!("committed voice saved twice")),
        || Err(AgentError::config("simulated unavailable provider")),
        || panic!("history loaded after configuration failure"),
    );
    assert!(matches!(result, Err(AgentError::ConfigError(_))));
    let history = temporary.db.agent_load_messages("voice-a").unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].content, text);
    assert_eq!(history[0].role, "user");
    assert!(history[0].images_json.is_none());
    let reopened = Database::open(&temporary.path).unwrap();
    assert_eq!(
        reopened.agent_load_messages("voice-a").unwrap()[0].content,
        text
    );
    assert_eq!(
        reopened.agent_list_conversations().unwrap()[0].title,
        "Voice Shortcut"
    );
}

#[test]
fn voice_receipt_cannot_be_used_for_another_conversation() {
    let temporary = TempDatabase::new();
    let saved = save_voice_input(&temporary.db, "voice-a".into(), "accepted".into()).unwrap();
    let result =
        TurnInput::Voice(saved).persist_with("voice-b", |_, _| panic!("wrong conversation saved"));
    assert!(matches!(result, Err(AgentError::DbError(_))));
    assert!(temporary
        .db
        .agent_load_messages("voice-b")
        .unwrap()
        .is_empty());
    assert_eq!(
        temporary.db.agent_load_messages("voice-a").unwrap().len(),
        1
    );
}

#[test]
fn voice_retry_receipt_keeps_the_original_committed_id_after_followups() {
    let temporary = TempDatabase::new();
    let saved = save_voice_input(&temporary.db, "voice".into(), "same speech".into()).unwrap();
    let original_id = saved.message_id;
    let later = temporary
        .db
        .agent_append_message("voice", "user", "same speech", None, None, None)
        .unwrap();
    assert!(later > original_id);
    let retry = retry_voice_input(&temporary.db, "voice".into(), "same speech".into()).unwrap();
    for receipt in [saved, retry] {
        let committed = TurnInput::Voice(receipt)
            .persist_with("voice", |_, _| panic!("receipt saved twice"))
            .unwrap();
        assert_eq!(committed.message_id, original_id);
        assert_eq!(committed.text, "same speech");
        assert!(committed.images.is_empty());
    }
    assert_eq!(temporary.db.agent_load_messages("voice").unwrap().len(), 2);
}

#[test]
fn ordinary_input_still_saves_the_complete_message_and_images_once() {
    let temporary = TempDatabase::new();
    temporary
        .db
        .agent_create_conversation("chat", "new")
        .unwrap();
    let images = vec![ImagePart {
        mime: "image/png".into(),
        data_base64: "AA==".into(),
    }];
    let expected_json = serde_json::to_string(&images).unwrap();
    let committed = TurnInput::New {
        text: "new message".into(),
        images,
        save: std::sync::Arc::new(crate::pending_persistence::PendingPersistence::default())
            .reserve()
            .unwrap(),
    }
    .persist_with("chat", |text, images| {
        crate::agent::turn::persist_user_body(&temporary.db, "chat", text, images)
    })
    .unwrap();
    assert_eq!(committed.text, "new message");
    assert_eq!(
        serde_json::to_string(&committed.images).unwrap(),
        expected_json
    );
    let history = temporary.db.agent_load_messages("chat").unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(committed.message_id, history[0].id);
    assert_eq!(
        history[0].images_json.as_deref(),
        Some(expected_json.as_str())
    );
}

#[test]
fn unpolled_or_dropped_completion_cannot_discard_admitted_sqlite_input_or_save_reservation() {
    struct TempDir(std::path::PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let temporary = TempDir(
        std::env::temp_dir().join(format!("selah-admission-save-{}", uuid::Uuid::new_v4())),
    );
    let db = Arc::new(crate::db::Database::open(&temporary.0).unwrap());
    let id = uuid::Uuid::new_v4().to_string();
    db.agent_create_conversation(&id, "admission").unwrap();
    let saves = Arc::new(crate::pending_persistence::PendingPersistence::default());
    let save = saves.reserve().unwrap();
    saves.begin_shutdown();
    saves.seal();
    let worker_db = db.clone();
    let worker_id = id.clone();
    let (entered, started) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let (finished, ended) = std::sync::mpsc::channel();
    let mut admitted = crate::agent_turn_scope::Admission::start(
        &id,
        Some("admitted-input".into()),
        move |owner| {
            entered.send(()).unwrap();
            released.recv().unwrap();
            // Use the production input/permit path: persistence completes before
            // provider admission checks the cancellation signalled by root drop.
            let result = TurnInput::New {
                text: "第一の確定行。第二の確定行。末尾の全文。".into(),
                images: vec![crate::ai::ImagePart {
                    mime: "image/png".into(),
                    data_base64: "AA==".into(),
                }],
                save,
            }
            .persist_with(&worker_id, |text, images| {
                crate::agent::turn::persist_user_body(&worker_db, &worker_id, text, images)
            });
            assert!(owner.cancelled());
            assert!(result.is_ok());
            drop(owner);
            finished.send(()).unwrap();
            Err::<(), _>(AgentError::Cancelled)
        },
    );
    let generation = admitted.running.turn.generation().to_owned();
    started
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap();
    let completion = async move { admitted.prepared().await };
    // Drop before its first poll. The worker is already submitted, owns the
    // permit and retains this exact request until its durable input is saved.
    drop(completion);
    assert!(crate::agent_turn_scope::generation_cancelled(&generation));
    release.send(()).unwrap();
    ended
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap();
    tauri::async_runtime::block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(3), saves.drained())
            .await
            .unwrap()
            .unwrap();
    });
    let history = db.agent_load_messages(&id).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(
        history[0].content,
        "第一の確定行。第二の確定行。末尾の全文。"
    );
    assert_eq!(
        history[0].images_json.as_deref(),
        Some(r#"[{"mime":"image/png","data_base64":"AA=="}]"#)
    );
    assert!(!crate::agent_turn_scope::generation_cancelled(&generation));
    assert!(!crate::agent_turn_scope::cancel(&id, None));
}
