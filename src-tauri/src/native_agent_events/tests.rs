use super::{deleted_stream, stream_view, StreamEvent};
use crate::latest_ui_mailbox::CurrentUiValue;
use crate::native_agent_state::{CapsuleMode, SharedState};
use std::borrow::Cow;
use std::sync::Mutex;

fn listening(id: &str) -> SharedState {
    let mut state = SharedState::default();
    state.capture.begin(id.into());
    state.prepare_view(CapsuleMode::Listening, "prompt".into());
    state
}
#[test]
fn speech_is_committed_directly_and_owned_text_can_be_reused() {
    let state = std::sync::Mutex::new(listening("a"));
    let text = "日本語\n\"授業\"".to_owned();
    let pointer = text.as_ptr();
    let view = super::speech_view(&state, "a", Cow::Owned(text), false).unwrap();
    assert_eq!(view.text, "日本語\n\"授業\"");
    assert_eq!(state.lock().unwrap().current_speech.as_ptr(), pointer);
    super::speech_view(&state, "a", Cow::Borrowed("complete"), true);
    assert_eq!(state.lock().unwrap().finals_accumulated, "complete");
}

#[test]
fn direct_events_require_the_native_caller_and_specific_capture() {
    for (caller, id) in [
        ("agent", Some("a")),
        ("live", Some("a")),
        ("native_agent", None),
        ("native_agent", Some("")),
    ] {
        assert!(super::native_owner(caller, id).is_none());
    }
    assert_eq!(super::native_owner("native_agent", Some("a")), Some("a"));
    let state = std::sync::Mutex::new(listening("b"));
    super::speech_view(&state, "b", Cow::Borrowed("new speech"), false);
    assert!(super::speech_view(&state, "a", Cow::Borrowed("old"), true).is_none());
    assert!(super::speech_view(&state, "b", Cow::Borrowed("  "), true).is_none());
    let state = state.lock().unwrap();
    assert_eq!(state.current_speech, "new speech");
    assert!(state.finals_accumulated.is_empty());
}
#[test]
fn releasing_freezes_display_but_keeps_every_final_for_submission() {
    let mut state = listening("a");
    let visible = state
        .accept_speech("a", Cow::Borrowed("first"), true)
        .unwrap();
    state.stop_requested = true;
    assert!(state
        .accept_speech("a", Cow::Borrowed("second"), true)
        .is_none());
    assert!(state
        .accept_speech("a", Cow::Borrowed("tail"), false)
        .is_none());
    assert!(state.speech_ready_view("a").is_none());
    assert!(visible.is_current());
    assert_eq!(
        crate::native_agent_state::consume_all_speech(&mut state),
        "first second tail"
    );
}
#[test]
fn input_ready_updates_are_rejected_after_capture_enters_processing() {
    let mut state = listening("a");
    assert_eq!(state.speech_ready_view("a").unwrap().text, "話してください");
    state.begin_stream("conversation".into());
    assert!(state.speech_ready_view("a").is_none());
}
#[test]
fn direct_stream_rejects_foreign_and_previous_requests_in_the_same_conversation() {
    let state = Mutex::new(SharedState::default());
    let old = state.lock().unwrap().begin_stream("shared".into());
    stream_view(
        &state,
        "shared",
        old.owner.request_id(),
        StreamEvent::Token("old partial".into()),
    );
    let current = state.lock().unwrap().begin_stream("shared".into());
    assert_ne!(old.owner.request_id(), current.owner.request_id());
    assert!(!old.view.is_current());
    for (conversation, request) in [
        ("foreign", current.owner.request_id()),
        ("shared", old.owner.request_id()),
        ("shared", ""),
    ] {
        for event in [
            StreamEvent::Token("foreign token".into()),
            StreamEvent::Done,
            StreamEvent::Error("foreign error".into()),
        ] {
            assert!(stream_view(&state, conversation, request, event).is_none());
        }
    }
    {
        let state = state.lock().unwrap();
        assert!(state.owns_stream("shared"));
        assert!(state.result_accumulated.is_empty());
        assert!(current.view.is_current());
    }
    let answer = "日本語\n\"引用\" 👩🏽‍💻\t";
    stream_view(
        &state,
        "shared",
        current.owner.request_id(),
        StreamEvent::Token(Cow::Borrowed(answer)),
    );
    stream_view(
        &state,
        "shared",
        current.owner.request_id(),
        StreamEvent::Token(Cow::Borrowed("末尾")),
    );
    let completion = stream_view(
        &state,
        "shared",
        current.owner.request_id(),
        StreamEvent::Done,
    )
    .unwrap();
    assert_eq!(completion.view.text, format!("{answer}末尾"));
    assert_eq!(completion.view.mode, CapsuleMode::Result);
    assert!(completion.view.is_current());
    for event in [
        StreamEvent::Token("late".into()),
        StreamEvent::Done,
        StreamEvent::Error("late error".into()),
    ] {
        assert!(stream_view(&state, "shared", current.owner.request_id(), event).is_none());
    }
    assert!(completion.view.is_current());
}

#[test]
fn direct_stream_error_consumes_only_its_request_and_cannot_overwrite_new_speech() {
    let state = Mutex::new(SharedState::default());
    let owner = state.lock().unwrap().begin_stream("voice".into()).owner;
    let message = "モデルの失敗\n\"details\"";
    let completion = stream_view(
        &state,
        "voice",
        owner.request_id(),
        StreamEvent::Error(Cow::Borrowed(message)),
    )
    .unwrap();
    assert_eq!(completion.view.text, message);
    assert_eq!(completion.view.mode, CapsuleMode::Notice);
    {
        let mut state = state.lock().unwrap();
        state.capture.begin("new capture".into());
        state.prepare_view(CapsuleMode::Listening, "new prompt".into());
        state.accept_speech("new capture", "新しい発話".into(), false);
    }
    assert!(!completion.view.is_current());
    for event in [
        StreamEvent::Token("late".into()),
        StreamEvent::Done,
        StreamEvent::Error("old failure".into()),
    ] {
        assert!(stream_view(&state, "voice", owner.request_id(), event).is_none());
    }
    let state = state.lock().unwrap();
    assert_eq!(state.capture.id(), Some("new capture"));
    assert_eq!(state.mode, Some(CapsuleMode::Listening));
    assert_eq!(state.current_speech, "新しい発話");
}

#[test]
fn deletion_finishes_only_its_processing_capsule_and_preserves_active_speech() {
    let state = Mutex::new(SharedState::default());
    let owner = state.lock().unwrap().begin_stream("deleted".into()).owner;
    stream_view(
        &state,
        "deleted",
        owner.request_id(),
        StreamEvent::Token("partial answer".into()),
    );
    assert!(deleted_stream(&state, "other").is_none());
    assert!(state.lock().unwrap().owns_stream("deleted"));
    let completion = deleted_stream(&state, "deleted").unwrap();
    assert_eq!(completion.view.mode, CapsuleMode::Notice);
    assert_eq!(completion.view.text, "会話が削除されました");
    assert!(deleted_stream(&state, "deleted").is_none());
    assert!(!state.lock().unwrap().append_stream_token(
        "deleted",
        owner.request_id(),
        "late answer"
    ));
    state.lock().unwrap().begin_stream("current".into());
    assert!(!completion.view.lease.is_current());
    assert!(deleted_stream(&state, "deleted").is_none());
    assert!(state.lock().unwrap().owns_stream("current"));
    {
        let mut state = state.lock().unwrap();
        state.cancel_stream();
        state.capture.begin("capture".into());
        state.prepare_view(CapsuleMode::Listening, "recognized speech".into());
        state.finals_accumulated = "確定行".into();
        state.current_speech = "認識途中".into();
    }
    assert!(deleted_stream(&state, "current").is_none());
    let state = state.lock().unwrap();
    assert_eq!(state.capture.id(), Some("capture"));
    assert_eq!(state.mode, Some(CapsuleMode::Listening));
    assert_eq!(state.finals_accumulated, "確定行");
    assert_eq!(state.current_speech, "認識途中");
}
