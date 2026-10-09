use super::*;
use crate::latest_ui_mailbox::CurrentUiValue;
use crate::native_agent_state::CapsuleMode;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::oneshot;

#[tokio::test(flavor = "current_thread")]
async fn delayed_storage_does_not_block_async_work_and_submission_waits_for_it() {
    let (entered, started) = oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let ran = Arc::new(AtomicBool::new(false));
    let output = ran.clone();
    let task = tokio::spawn(prepare_and_run(
        move || {
            let _ = entered.send(());
            released.recv().unwrap();
            Ok(())
        },
        move |()| async move {
            output.store(true, Ordering::SeqCst);
            Ok(())
        },
    ));
    started.await.unwrap();
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    assert!(!ran.load(Ordering::SeqCst));
    release.send(()).unwrap();
    assert_eq!(task.await.unwrap(), Ok(()));
    assert!(ran.load(Ordering::SeqCst));
}
#[tokio::test(flavor = "current_thread")]
async fn storage_failure_and_worker_panic_never_register_or_start_inference() {
    for panic in [false, true] {
        let result = prepare_and_run(
            move || {
                if panic {
                    panic!("simulated storage worker failure")
                } else {
                    Err("storage failed".into())
                }
            },
            |(): ()| async {
                panic!("inference started after storage failure");
            },
        )
        .await;
        assert!(result.is_err());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn accepted_input_is_committed_before_delayed_inference() {
    let path = std::env::temp_dir().join(format!("selah-native-commit-{}", uuid::Uuid::new_v4()));
    let db = Arc::new(crate::db::Database::open(&path).unwrap());
    let saving = db.clone();
    let (entered, started) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let task = tokio::spawn(prepare_and_run(
        move || {
            crate::agent::save_voice_input(
                &saving,
                "accepted".into(),
                "all final lines and tail".into(),
            )
        },
        move |input| async move {
            entered.send(()).unwrap();
            released.await.unwrap();
            assert_eq!(input.conversation_id(), "accepted");
            Ok(())
        },
    ));
    started.await.unwrap();
    assert!(
        !task.is_finished(),
        "inference continuation did not remain pending"
    );
    let history = db.agent_load_messages("accepted").unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].content, "all final lines and tail");
    release.send(()).unwrap();
    task.await.unwrap().unwrap();
    drop(db);
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn interrupted_speech_is_saved_while_a_new_capture_rejects_its_delayed_status() {
    let path =
        std::env::temp_dir().join(format!("selah-interrupted-voice-{}", uuid::Uuid::new_v4()));
    let db = Arc::new(crate::db::Database::open(&path).unwrap());
    let state = Arc::new(Mutex::new(SharedState::default()));
    let failure = {
        let mut state = state.lock().unwrap();
        state.capture.begin("input-a".into());
        state.prepare_view(CapsuleMode::Listening, "prompt".into());
        state.accept_speech("input-a", "first final".into(), true);
        state.accept_speech("input-a", "second final".into(), true);
        state.accept_speech("input-a", "partial tail".into(), false);
        state
            .fail_capture("input-a", "recognizer interrupted")
            .unwrap()
    };
    let saves = Arc::new(crate::pending_persistence::PendingPersistence::default());
    let permit = saves.reserve().unwrap();
    saves.begin_shutdown();
    saves.seal();
    let storage = Arc::new(storage::VoiceStorage::default());
    let attempt = storage.accept("recovered".into(), failure.text);
    let (entered, started) = oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let saving = db.clone();
    let queued_state = state.clone();
    let job = tokio::spawn(prepare_and_run(
        move || {
            entered.send(()).unwrap();
            blocked.recv().unwrap();
            let result = attempt.persist_with(|input, _| {
                crate::agent::save_voice_input(
                    &saving,
                    input.conversation_id.clone(),
                    input.text.clone(),
                )
            });
            permit.complete(result.as_ref().map(|_| ()).map_err(String::as_str));
            result
        },
        move |input| async move {
            assert_eq!(input.conversation_id(), "recovered");
            assert!(queued_state
                .lock()
                .unwrap()
                .complete_capture_recovery(&failure.view.lease, &failure.message, None)
                .is_none());
            Ok(())
        },
    ));
    started.await.unwrap();
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    {
        let mut state = state.lock().unwrap();
        assert_eq!(state.mode, Some(CapsuleMode::Notice));
        state.capture.begin("input-b".into());
        state.prepare_view(CapsuleMode::Listening, "new prompt".into());
        state.current_speech = "new speech".into();
    }
    release.send(()).unwrap();
    saves.drained().await.unwrap();
    job.await.unwrap().unwrap();
    let history = db.agent_load_messages("recovered").unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].role, "user");
    assert_eq!(history[0].content, "first final second final partial tail");
    {
        let state = state.lock().unwrap();
        assert_eq!(state.capture.id(), Some("input-b"));
        assert_eq!(state.current_speech, "new speech");
        assert_eq!(state.mode, Some(CapsuleMode::Listening));
        assert!(
            state.active_stream.is_none(),
            "interrupted recovery retained a model stream"
        );
    }
    drop(db);
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn dropping_the_async_continuation_does_not_drop_a_reserved_voice_save() {
    let path = std::env::temp_dir().join(format!("selah-native-drain-{}", uuid::Uuid::new_v4()));
    let db = Arc::new(crate::db::Database::open(&path).unwrap());
    let saves = Arc::new(crate::pending_persistence::PendingPersistence::default());
    let permit = saves.reserve().unwrap();
    saves.begin_shutdown();
    saves.seal();
    let (entered, started) = oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let saving = db.clone();
    let continuation = prepare_and_run(
        move || {
            entered.send(()).unwrap();
            blocked.recv().unwrap();
            let result =
                crate::agent::save_voice_input(&saving, "accepted".into(), "complete voice".into());
            permit.complete(result.as_ref().map(|_| ()).map_err(String::as_str));
            result
        },
        |_| async {
            panic!("abandoned inference started");
        },
    );
    started.await.unwrap();
    drop(continuation);
    let waiting = saves.clone();
    let drain = tokio::spawn(async move { waiting.drained().await });
    tokio::task::yield_now().await;
    assert!(!drain.is_finished());
    release.send(()).unwrap();
    drain.await.unwrap().unwrap();
    assert_eq!(
        db.agent_load_messages("accepted").unwrap()[0].content,
        "complete voice"
    );
    drop(db);
    std::fs::remove_dir_all(path).unwrap();
}
#[tokio::test(flavor = "current_thread")]
async fn hiding_a_pending_submission_preserves_accepted_text_but_rejects_its_late_completion() {
    let state = Arc::new(Mutex::new(SharedState::default()));
    let owner = state.lock().unwrap().begin_stream("a".into()).owner;
    let (entered, started) = oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let input = "first final second final tail".to_owned();
    let delivered = Arc::new(Mutex::new(None));
    let queued_state = state.clone();
    let queued_output = delivered.clone();
    let task = tokio::spawn(prepare_and_run(
        move || {
            entered.send(()).unwrap();
            released.recv().unwrap();
            Ok(())
        },
        move |()| async move {
            assert!(!queued_state.lock().unwrap().owns_stream("a"));
            *queued_output.lock().unwrap() = Some(input);
            Err("old inference failed".into())
        },
    ));
    started.await.unwrap();
    {
        let mut state = state.lock().unwrap();
        state.close_view(None).unwrap();
        state.capture.begin("input-b".into());
        state.prepare_view(CapsuleMode::Listening, "new input".into());
        state.current_speech = "new speech".into();
    }
    release.send(()).unwrap();
    let error = task.await.unwrap().unwrap_err();
    let mut state = state.lock().unwrap();
    assert!(state
        .complete_stream("a", owner.request_id(), Some(&error))
        .is_none());
    assert_eq!(state.current_speech, "new speech");
    assert_eq!(state.capture.id(), Some("input-b"));
    assert_eq!(
        delivered.lock().unwrap().as_deref(),
        Some("first final second final tail")
    );
    assert!(state.active_stream.is_none());
}

#[test]
fn completed_answer_or_error_is_not_replaced_by_submission_fallback() {
    for error in [None, Some("provider failed")] {
        let state = Mutex::new(SharedState::default());
        let owner = state.lock().unwrap().begin_stream("voice".into()).owner;
        let completion = {
            let mut state = state.lock().unwrap();
            state.append_stream_token("voice", owner.request_id(), "日本語\n\"回答\" 👩🏽‍💻");
            state
                .complete_stream("voice", owner.request_id(), error)
                .unwrap()
        };
        let result = error.map(|error| Err(error.to_owned())).unwrap_or(Ok(()));
        assert!(submission_finished(&state, &owner, &result).is_none());
        assert!(completion.view.is_current());
        if let Some(error) = error {
            assert_eq!(completion.view.mode, CapsuleMode::Notice);
            assert_eq!(completion.view.text, error);
        } else {
            assert_eq!(completion.view.mode, CapsuleMode::Result);
            assert_eq!(completion.view.text, "日本語\n\"回答\" 👩🏽‍💻");
        }
    }
}

#[test]
fn superseded_backend_request_finishes_its_capsule_when_terminal_delivery_is_suppressed() {
    let conversation = uuid::Uuid::new_v4().to_string();
    let state = Mutex::new(SharedState::default());
    let owner = state
        .lock()
        .unwrap()
        .begin_stream(conversation.clone())
        .owner;
    let mut old =
        crate::agent_turn_scope::RunningTurn::begin(&conversation, Some(owner.request_id().into()));
    assert_eq!(old.turn.request(), owner.request_id());
    let mut replacement = crate::agent_turn_scope::RunningTurn::begin(
        &conversation,
        Some("main-window-request".into()),
    );
    assert!(old.turn.cancelled());
    assert!(
        !old.turn.accepts_event(true),
        "superseded terminal would reach native state"
    );
    let completion = submission_finished(&state, &owner, &Ok(())).unwrap();
    assert_eq!(completion.view.mode, CapsuleMode::Notice);
    assert_eq!(completion.view.text, "応答が取り消されました");
    assert!(submission_finished(&state, &owner, &Ok(())).is_none());
    assert!(!state
        .lock()
        .unwrap()
        .append_stream_token(&conversation, owner.request_id(), "late"));
    assert!(completion.view.is_current());
    assert!(replacement.turn.accepts_event(false));
    old.finish();
    replacement.finish();
}

#[test]
fn submission_failure_releases_only_its_request() {
    let state = Mutex::new(SharedState::default());
    let old = state.lock().unwrap().begin_stream("shared".into()).owner;
    let completion = submission_finished(&state, &old, &Err("storage failed".into())).unwrap();
    assert_eq!(completion.view.mode, CapsuleMode::Notice);
    assert_eq!(completion.view.text, "storage failed");
    let current = state.lock().unwrap().begin_stream("shared".into());
    for result in [Ok(()), Err("old failure".into())] {
        assert!(submission_finished(&state, &old, &result).is_none());
    }
    assert!(current.view.is_current());
    assert!(state.lock().unwrap().owns_stream("shared"));
}

#[tokio::test(flavor = "current_thread")]
async fn same_conversation_replacement_during_storage_keeps_full_input_and_rejects_old_finish() {
    let path =
        std::env::temp_dir().join(format!("selah-native-replacement-{}", uuid::Uuid::new_v4()));
    let db = Arc::new(crate::db::Database::open(&path).unwrap());
    let state = Arc::new(Mutex::new(SharedState::default()));
    let old = state.lock().unwrap().begin_stream("shared".into()).owner;
    let (entered, started) = oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let saving = db.clone();
    let queued_state = state.clone();
    let queued_owner = old.clone();
    let full_input = "第一の確定段落。\nsecond final \"quoted\" 👩🏽‍💻 末尾の発話";
    let task = tokio::spawn(async move {
        let result = prepare_and_run(
            move || {
                entered.send(()).unwrap();
                released.recv().unwrap();
                crate::agent::save_voice_input(&saving, "shared".into(), full_input.into())
            },
            move |input| async move {
                assert_eq!(input.conversation_id(), "shared");
                let mut state = queued_state.lock().unwrap();
                assert!(!state.append_stream_token(
                    "shared",
                    queued_owner.request_id(),
                    "old answer"
                ));
                assert!(state
                    .complete_stream("shared", queued_owner.request_id(), Some("old error"))
                    .is_none());
                Err("old inference failed".into())
            },
        )
        .await;
        result
    });
    started.await.unwrap();
    let current = state.lock().unwrap().begin_stream("shared".into());
    release.send(()).unwrap();
    let result = task.await.unwrap();
    assert!(submission_finished(&state, &old, &result).is_none());
    assert!(current.view.is_current());
    {
        let mut state = state.lock().unwrap();
        assert!(state.result_accumulated.is_empty());
        assert!(state.append_stream_token("shared", current.owner.request_id(), "new answer"));
        assert_eq!(
            state
                .complete_stream("shared", current.owner.request_id(), None)
                .unwrap()
                .view
                .text,
            "new answer"
        );
    }
    let history = db.agent_load_messages("shared").unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].role, "user");
    assert_eq!(history[0].content, full_input);
    drop(db);
    std::fs::remove_dir_all(path).unwrap();
}
