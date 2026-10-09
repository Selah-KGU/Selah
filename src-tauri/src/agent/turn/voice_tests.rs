use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::oneshot;

#[tokio::test(flavor = "current_thread")]
async fn voice_is_admitted_before_storage_and_cannot_supersede_a_later_request() {
    let id = uuid::Uuid::new_v4().to_string();
    let path = std::env::temp_dir().join(format!("selah-voice-admission-{id}"));
    let db = Arc::new(Database::open(&path).unwrap());
    let saves = Arc::new(crate::pending_persistence::PendingPersistence::default());
    let permit = saves.reserve().unwrap();
    let saving = db.clone();
    let save_id = id.clone();
    let (entered, started) = oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let mut voice = admit_voice(
        &id,
        "voice-request".into(),
        move || {
            entered.send(()).unwrap();
            released.recv().unwrap();
            let result = save_voice_input(
                &saving,
                save_id,
                "第一段落。\nsecond final \"quotes\" 👩🏽‍💻 末尾".into(),
            );
            permit.complete(result.as_ref().map(|_| ()).map_err(String::as_str));
            result
        },
        |_, _| -> Result<(), AgentError> { panic!("superseded voice resolved model/history") },
    );
    // Admission is authoritative before either storage completion or first poll.
    assert_eq!(voice.running.turn.request(), "voice-request");
    assert!(!voice.running.turn.cancelled());
    started.await.unwrap();
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    let mut later =
        crate::agent_turn_scope::RunningTurn::begin(&id, Some("later-main-request".into()));
    assert!(voice.running.turn.cancelled());
    assert!(!voice.running.turn.accepts_event(true));
    saves.begin_shutdown();
    saves.seal();
    release.send(()).unwrap();
    assert!(matches!(voice.prepared().await, Err(AgentError::Cancelled)));
    saves.drained().await.unwrap();
    assert!(!later.turn.cancelled());
    assert!(later.turn.accepts_event(false));
    assert!(crate::agent_turn_scope::cancel(
        &id,
        Some("later-main-request")
    ));
    let history = db.agent_load_messages(&id).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].role, "user");
    assert_eq!(
        history[0].content,
        "第一段落。\nsecond final \"quotes\" 👩🏽‍💻 末尾"
    );
    voice.running.finish();
    later.finish();
    drop(voice);
    drop(later);
    drop(db);
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn dropped_voice_waiter_keeps_accepted_save_and_skips_model_preparation() {
    let id = uuid::Uuid::new_v4().to_string();
    let path = std::env::temp_dir().join(format!("selah-voice-dropped-{id}"));
    let db = Arc::new(Database::open(&path).unwrap());
    let saving = db.clone();
    let save_id = id.clone();
    let saves = Arc::new(crate::pending_persistence::PendingPersistence::default());
    let permit = saves.reserve().unwrap();
    let prepared = Arc::new(AtomicBool::new(false));
    let observed = prepared.clone();
    let (entered, started) = oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let (finished, saved) = oneshot::channel();
    let voice = admit_voice(
        &id,
        "voice-request".into(),
        move || {
            entered.send(()).unwrap();
            released.recv().unwrap();
            let result = save_voice_input(&saving, save_id, "accepted complete speech".into());
            permit.complete(result.as_ref().map(|_| ()).map_err(String::as_str));
            finished.send(()).unwrap();
            result
        },
        move |_, _| {
            observed.store(true, Ordering::SeqCst);
            Ok(())
        },
    );
    started.await.unwrap();
    let owner = voice.running.turn.clone();
    drop(voice);
    assert!(owner.cancelled());
    saves.begin_shutdown();
    saves.seal();
    release.send(()).unwrap();
    saved.await.unwrap();
    saves.drained().await.unwrap();
    // The dropped waiter detached the worker. Wait for its retained owner to
    // be released before checking that the model continuation never ran.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while Arc::strong_count(&owner) > 1 {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        db.agent_load_messages(&id).unwrap()[0].content,
        "accepted complete speech"
    );
    assert!(!prepared.load(Ordering::SeqCst));
    drop(db);
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn voice_save_receipt_is_consumed_once_and_storage_failure_never_prepares_model() {
    let id = uuid::Uuid::new_v4().to_string();
    let path = std::env::temp_dir().join(format!("selah-voice-receipt-{id}"));
    let db = Arc::new(Database::open(&path).unwrap());
    let saving = db.clone();
    let save_id = id.clone();
    let prepare_id = id.clone();
    let mut voice = admit_voice(
        &id,
        "voice-request".into(),
        move || save_voice_input(&saving, save_id, "complete input".into()),
        move |input, owner| {
            let committed = input::TurnInput::Voice(input)
                .persist_with(&prepare_id, |_, _| panic!("committed voice saved twice"))?;
            owner.set_input_message(committed.message_id)?;
            assert_eq!(owner.request(), "voice-request");
            assert!(!owner.cancelled());
            Ok((committed.message_id, committed.text))
        },
    );
    let (message_id, text) = voice.prepared().await.unwrap();
    assert_eq!(text, "complete input");
    let history = db.agent_load_messages(&id).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, message_id);
    voice.running.finish();
    for panics in [false, true] {
        let mut failed = admit_voice(
            &id,
            "failed-request".into(),
            move || {
                if panics {
                    panic!("storage worker panic")
                }
                Err("storage failed".into())
            },
            |_, _| -> Result<(), AgentError> { panic!("prepared after failed storage") },
        );
        match failed.prepared().await.unwrap_err() {
            AgentError::DbError(message) => {
                assert!(!panics);
                assert_eq!(message, "storage failed");
            }
            AgentError::TaskError(_) => assert!(panics),
            error => panic!("unexpected failure: {error}"),
        }
        failed.running.finish();
    }
    drop(voice);
    drop(db);
    std::fs::remove_dir_all(path).unwrap();
}
