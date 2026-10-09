use super::*;
use crate::latest_ui_mailbox::LatestUiMailbox;
use std::sync::Mutex;

fn processing(id: &str) -> SharedState {
    let mut state = SharedState::default();
    state.begin_stream(id.into());
    state
}

fn request(state: &SharedState, id: &str) -> String {
    state
        .active_stream
        .as_ref()
        .filter(|owner| owner.conversation_id() == id)
        .map(|owner| owner.request_id().to_owned())
        .unwrap_or_default()
}

fn capturing(id: &str) -> SharedState {
    let mut state = SharedState::default();
    state.capture.begin(id.into());
    state.prepare_view(CapsuleMode::Listening, "prompt".into());
    state
}

#[test]
fn capture_error_keeps_every_recognized_segment_and_completes_only_once() {
    let mut state = capturing("input-a");
    state.accept_speech("input-a", Cow::Borrowed("第一の確定行。"), true);
    state.accept_speech("input-a", Cow::Borrowed("第二の確定行。"), true);
    let old_view = state
        .accept_speech("input-a", Cow::Borrowed("末尾の発話"), false)
        .unwrap();
    state.stop_requested = true;
    state.active_stream = Some(StreamOwner::new("prior stream".into()));
    let failure = state.fail_capture("input-a", "recognizer stopped").unwrap();
    assert_eq!(failure.text, "第一の確定行。第二の確定行。末尾の発話");
    assert_eq!(failure.message, "recognizer stopped");
    assert_eq!(failure.view.mode, CapsuleMode::Notice);
    assert!(failure.view.text.contains("recognizer stopped"));
    assert!(failure.view.is_current());
    assert!(!old_view.is_current());
    assert!(state.capture.id().is_none());
    assert!(state.active_stream.is_none());
    assert!(state.current_speech.is_empty());
    assert!(state.finals_accumulated.is_empty());
    assert!(!state.stop_requested);
    assert!(state.fail_capture("input-a", "duplicate error").is_none());
    assert!(
        !state.capture.finish("input-a"),
        "idle accepted the interrupted capture twice"
    );
}

#[test]
fn an_old_capture_error_does_not_change_the_current_capture() {
    let mut state = capturing("input-b");
    let view = state
        .accept_speech("input-b", Cow::Borrowed("new speech"), false)
        .unwrap();
    state.stop_requested = true;
    for id in ["input-a", ""] {
        assert!(state.fail_capture(id, "old error").is_none());
    }
    assert_eq!(state.capture.id(), Some("input-b"));
    assert_eq!(state.current_speech, "new speech");
    assert_eq!(state.mode, Some(CapsuleMode::Listening));
    assert!(state.stop_requested);
    assert!(view.is_current());
}

#[test]
fn an_empty_initialization_failure_keeps_the_error_without_a_storage_notice() {
    let mut state = capturing("input-a");
    state.current_speech = " \n\u{3000}".into();
    let failure = state.fail_capture("input-a", "model unavailable").unwrap();
    assert!(failure.text.is_empty());
    assert_eq!(failure.view.text, "model unavailable");
    assert_eq!(state.mode, Some(CapsuleMode::Notice));
}

#[test]
fn recovery_status_changes_only_its_own_notice_and_rejects_duplicate_completion() {
    let mut state = capturing("input-a");
    state.current_speech = "speech".into();
    let failure = state.fail_capture("input-a", "recognizer stopped").unwrap();
    let saved = state
        .complete_capture_recovery(&failure.view.lease, &failure.message, None)
        .unwrap();
    assert!(saved.text.contains("履歴に保存しました"));
    assert!(saved.text.contains("recognizer stopped"));
    assert_eq!(saved.mode, CapsuleMode::Notice);
    assert!(state
        .complete_capture_recovery(&failure.view.lease, &failure.message, Some("late error"))
        .is_none());
    let other = state.notice_view("another notice");
    assert!(state
        .complete_capture_recovery(&saved.lease, "old recognizer error", None)
        .is_none());
    assert!(other.is_current());
    state.close_view(None).unwrap();
    assert!(state
        .complete_capture_recovery(&other.lease, "closed", None)
        .is_none());
    assert!(state.mode.is_none());
}

#[test]
fn speech_consumption_reuses_owned_final_and_partial_buffers() {
    for partial in [false, true] {
        let mut state = SharedState::default();
        let text = " \u{3000}日本語の授業\n ".to_owned();
        let pointer = text.as_ptr();
        if partial {
            state.current_speech = text;
        } else {
            state.finals_accumulated = text;
        }
        let output = consume_all_speech(&mut state);
        assert_eq!(output, "日本語の授業");
        assert_eq!(
            output.as_ptr(),
            pointer,
            "consumption copied an owned message"
        );
        assert!(state.current_speech.is_empty());
        assert!(state.finals_accumulated.is_empty());
    }
}

#[test]
fn owned_consumption_preserves_cjk_separators_and_unicode_whitespace() {
    for (finals, partial, expected) in [
        ("日本語", "\u{3000}の授業 \n", "日本語の授業"),
        ("hello", " world\t", "hello world"),
        ("\nhello \n", "world", "hello \n world"),
        ("\u{3000}", "\n\t", ""),
    ] {
        let mut state = SharedState::default();
        state.finals_accumulated = finals.into();
        state.current_speech = partial.into();
        assert_eq!(consume_all_speech(&mut state), expected);
    }
}

#[test]
fn previous_conversation_tokens_and_terminal_events_cannot_touch_a_replacement() {
    let mut state = processing("b");
    assert!(!state.append_stream_token("a", &request(&state, "a"), "old token"));
    assert!(state
        .complete_stream("a", &request(&state, "a"), Some("old error"))
        .is_none());
    assert!(state
        .complete_stream("a", &request(&state, "a"), None)
        .is_none());
    assert!(state.owns_stream("b"));
    assert_eq!(state.mode, Some(CapsuleMode::Processing));
    assert!(state.result_accumulated.is_empty());
    assert!(state.append_stream_token("b", &request(&state, "b"), "current result"));
    let completed = state
        .complete_stream("b", &request(&state, "b"), None)
        .unwrap();
    assert_eq!(completed.view.text, "current result");
    assert_eq!(completed.view.mode, CapsuleMode::Result);
    assert!(completed.view.is_current());
}

#[test]
fn terminal_event_consumes_only_its_request_and_runs_once() {
    let mut state = processing("a");
    state.append_stream_token("a", &request(&state, "a"), " answer ");
    let completed = state
        .complete_stream("a", &request(&state, "a"), None)
        .unwrap();
    assert!(state
        .complete_stream("a", &request(&state, "a"), None)
        .is_none());
    assert!(state
        .complete_stream("a", &request(&state, "a"), Some("late error"))
        .is_none());
    assert!(!state.append_stream_token("a", &request(&state, "a"), "late token"));
    state.active_stream = Some(StreamOwner::new("b".into()));
    let replacement = state.prepare_view(CapsuleMode::Processing, String::new());
    assert!(state.owns_stream("b"));
    assert!(!completed.view.is_current());
    assert!(replacement.is_current());
}

#[test]
fn speech_callbacks_must_prepare_ui_updates_before_releasing_the_state_lock() {
    let mut state = SharedState::default();
    let delayed = state.prepare_view(CapsuleMode::Listening, "old speech".into());
    let latest = state.prepare_view(CapsuleMode::Processing, String::new());
    let mailbox = LatestUiMailbox::default();
    let ticket = mailbox.push(latest).unwrap();
    assert!(mailbox.push(delayed).is_none());
    assert_eq!(mailbox.take(ticket).unwrap().mode, CapsuleMode::Processing);
    state.prepare_view(CapsuleMode::Listening, "new capture".into());
    assert!(state.current_speech.is_empty());
}

#[test]
fn one_thousand_speech_updates_share_one_dispatch_and_keep_the_initial_view_epoch() {
    let mut state = SharedState::default();
    let mailbox = LatestUiMailbox::default();
    let mut tickets = Vec::new();
    let first_epoch = state
        .prepare_view(CapsuleMode::Listening, "prompt".into())
        .epoch;
    for i in 1..=1000 {
        state.current_speech = format!("speech {i}");
        let view = state.listening_view().unwrap();
        assert_eq!(view.epoch, first_epoch);
        if let Some(ticket) = mailbox.push(view) {
            tickets.push(ticket);
        }
    }
    assert_eq!(tickets.len(), 1);
    assert_eq!(mailbox.take(tickets[0]).unwrap().text, "speech 1000");
    // A fresh capture can remain in Listening while replacing its old window state.
    state.invalidate_view();
    let fresh = state.prepare_view(CapsuleMode::Listening, "fresh prompt".into());
    assert_ne!(fresh.epoch, first_epoch);
}

#[test]
fn closing_invalidates_queued_updates_without_erasing_a_later_capture() {
    let mut state = processing("a");
    let old = state.prepare_view(CapsuleMode::Processing, String::new());
    let mailbox = LatestUiMailbox::default();
    let old_ticket = mailbox.push(old).unwrap();
    mailbox.clear();
    state.close_view(None).unwrap();
    assert!(state.active_stream.is_none());
    let fresh = state.prepare_view(CapsuleMode::Listening, "new capture".into());
    let lease = fresh.lease.clone();
    let new_ticket = mailbox.push(fresh).unwrap();
    assert!(mailbox.take(old_ticket).is_none());
    mailbox.cancel(old_ticket);
    assert_eq!(mailbox.take(new_ticket).unwrap().text, "new capture");
    assert!(lease.is_current());
    assert_eq!(state.mode, Some(CapsuleMode::Listening));
}

#[test]
fn stopping_freezes_display_but_retains_all_final_speech_for_submission() {
    let mut state = SharedState::default();
    state.prepare_view(CapsuleMode::Listening, "prompt".into());
    state.stop_requested = true;
    append_final_segment(&mut state, "first");
    assert!(state.listening_view().is_none());
    append_final_segment(&mut state, "second");
    state.current_speech = "tail".into();
    assert_eq!(consume_all_speech(&mut state), "first second tail");
    assert!(state.finals_accumulated.is_empty());
    assert!(state.current_speech.is_empty());
}

#[test]
fn empty_and_error_results_change_to_notice_without_an_empty_result_view() {
    let mut empty = processing("a");
    let done = empty
        .complete_stream("a", &request(&empty, "a"), None)
        .unwrap();
    assert_eq!(done.view.mode, CapsuleMode::Notice);
    assert_eq!(done.view.text, "応答を取得できませんでした");
    let mut error = processing("b");
    error.append_stream_token("b", &request(&error, "b"), "unfinished");
    let failed = error
        .complete_stream("b", &request(&error, "b"), Some("request failed"))
        .unwrap();
    assert_eq!(failed.view.mode, CapsuleMode::Notice);
    assert_eq!(failed.view.text, "request failed");
    assert_eq!(error.result_accumulated, "request failed");
}

#[test]
fn cancelled_and_completed_requests_reject_delayed_terminal_events() {
    let mut state = SharedState::default();
    let cancelled = state.begin_stream("a".into()).owner;
    state.cancel_stream();
    assert!(state
        .complete_stream("a", cancelled.request_id(), None)
        .is_none());
    let completed = state.begin_stream("b".into()).owner;
    state
        .complete_stream("b", completed.request_id(), None)
        .unwrap();
    assert!(state
        .complete_stream("b", completed.request_id(), Some("late error"))
        .is_none());
    let current = state.begin_stream("b".into()).owner;
    assert_ne!(completed.request_id(), current.request_id());
    assert!(state
        .complete_stream("b", completed.request_id(), None)
        .is_none());
    assert!(state.owns_stream("b"));
}

#[test]
fn an_old_autoclose_lease_cannot_erase_another_notice_or_capture() {
    let mut state = SharedState::default();
    let old_notice = state.notice_view("old");
    let next_notice = state.notice_view("next");
    assert_ne!(old_notice.epoch, next_notice.epoch);
    assert!(state.close_view(Some(&old_notice.lease)).is_none());
    assert!(next_notice.is_current());
    let capture = state.prepare_view(CapsuleMode::Listening, "new capture".into());
    assert!(state.close_view(Some(&next_notice.lease)).is_none());
    assert!(capture.is_current());
    assert_eq!(state.mode, Some(CapsuleMode::Listening));
}

#[test]
fn concurrent_previous_and_current_requests_preserve_only_the_current_result() {
    let mut shared = SharedState::default();
    let old = shared.begin_stream("same conversation".into()).owner;
    let current = shared.begin_stream("same conversation".into()).owner;
    let state = Arc::new(Mutex::new(shared));
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let workers: Vec<_> = (0..8)
        .map(|index| {
            let state = state.clone();
            let barrier = barrier.clone();
            let owner = if index % 2 == 0 {
                old.clone()
            } else {
                current.clone()
            };
            std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..200 {
                    let accepted = state.lock().unwrap().append_stream_token(
                        owner.conversation_id(),
                        owner.request_id(),
                        "字",
                    );
                    assert_eq!(accepted, index % 2 != 0);
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    let mut state = state.lock().unwrap();
    let done = state
        .complete_stream(current.conversation_id(), current.request_id(), None)
        .unwrap();
    assert_eq!(done.view.text, "字".repeat(800));
}

#[test]
fn delayed_physical_close_cannot_replace_a_reopened_view_or_its_text() {
    let mut state = SharedState::default();
    state.capture.begin("a".into());
    state.prepare_view(CapsuleMode::Listening, "old speech".into());
    state.close_view(None).unwrap();
    let close = NativeViewRequest::Close {
        lease: state.view_lease(),
        immediate: false,
    };
    let queue = LatestUiMailbox::default();
    let old_ticket = queue.push(close).unwrap();
    state.capture.begin("b".into());
    let reopened = state.prepare_view(CapsuleMode::Listening, "new speech".into());
    assert!(queue.push(NativeViewRequest::Update(reopened)).is_none());
    // The existing dispatcher consumes the replacement; old close cannot hide it.
    let NativeViewRequest::Update(current) = queue.take(old_ticket).unwrap() else {
        panic!("old close survived");
    };
    assert_eq!(current.text, "new speech");
    assert_eq!(state.capture.id(), Some("b"));
    let old_close = state.view_lease();
    state.notice_view("new notice");
    assert!(state.close_view(Some(&old_close)).is_none());
    assert_eq!(state.mode, Some(CapsuleMode::Notice));
}

#[test]
fn thousand_pending_speech_updates_can_be_closed_without_resurrecting_the_view() {
    let mut state = SharedState::default();
    let queue = LatestUiMailbox::default();
    let ticket = queue
        .push(NativeViewRequest::Update(
            state.prepare_view(CapsuleMode::Listening, "prompt".into()),
        ))
        .unwrap();
    for i in 0..1000 {
        assert!(queue
            .push(NativeViewRequest::Update(
                state.prepare_view(CapsuleMode::Listening, format!("speech {i}"))
            ))
            .is_none());
    }
    state.close_view(None).unwrap();
    assert!(queue
        .push(NativeViewRequest::Close {
            lease: state.view_lease(),
            immediate: true
        })
        .is_none());
    let NativeViewRequest::Close { immediate, lease } = queue.take(ticket).unwrap() else {
        panic!("stale speech survived close");
    };
    assert!(immediate);
    assert!(lease.is_current());
    assert!(queue.take(ticket).is_none());
    assert_eq!(state.mode, None);
}
