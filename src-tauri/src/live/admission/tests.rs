use super::super::{
    tests::transcript::recording, LiveFinishPhase, LiveSessionSnapshot, LiveTranscriptLine,
};
use super::*;
use tauri::ipc::IpcResponse;

#[test]
fn manual_append_borrows_full_text_and_rejects_bad_fields_before_acceptance() {
    let value =
        serde_json::json!({"text": "\n日本語 \"quotes\" 👩🏽‍💻　".repeat(2048), "unused": [1,2,3]});
    let body = InvokeBody::Json(value);
    let text = append_text(&body).unwrap();
    let InvokeBody::Json(value) = &body else {
        unreachable!()
    };
    assert_eq!(text.as_ptr(), value["text"].as_str().unwrap().as_ptr());
    assert_eq!(text, "\n日本語 \"quotes\" 👩🏽‍💻　".repeat(2048));
    for value in [
        serde_json::json!({}),
        serde_json::json!({"text":null}),
        serde_json::json!({"text":1}),
        serde_json::json!({"text":["line"]}),
        serde_json::json!({"text":false}),
    ] {
        assert!(append_text(&InvokeBody::Json(value)).is_err());
    }
    assert!(append_text(&InvokeBody::Raw(vec![])).is_err());
    assert_eq!(
        append_text(&InvokeBody::Json(serde_json::json!({"text":""}))).unwrap(),
        ""
    );
}

#[tokio::test(flavor = "current_thread")]
async fn accepted_manual_append_is_committed_before_first_poll_and_reply_keeps_its_recording() {
    let state = LiveState::new();
    *state.session.lock().unwrap() = Some(recording());
    let reply = append_response(&state, || {
        state
            .append_line_for_session(
                None,
                LiveTranscriptLine {
                    text: "accepted full line\n\"quoted\" 🌕".into(),
                    at: "10:00:00".into(),
                },
            )
            .map(|update| update.is_some())
    });
    assert_eq!(current_snapshot(&state).transcript_lines.len(), 1);
    assert_eq!(
        current_snapshot(&state).transcript_lines[0].text,
        "accepted full line\n\"quoted\" 🌕"
    );
    let mut replacement = recording();
    replacement.session_id = "next recording".into();
    *state.session.lock().unwrap() = Some(replacement);
    let decoded = reply
        .await
        .unwrap()
        .body()
        .unwrap()
        .deserialize::<LiveSessionSnapshot>()
        .unwrap();
    assert_eq!(decoded.session_id.as_deref(), Some("recording-test"));
    assert_eq!(
        decoded.transcript_lines[0].text,
        "accepted full line\n\"quoted\" 🌕"
    );
    assert!(current_snapshot(&state).transcript_lines.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn dropped_response_cannot_revoke_text_and_finish_or_missing_session_errors_are_preserved() {
    let state = LiveState::new();
    *state.session.lock().unwrap() = Some(recording());
    drop(append_response(&state, || {
        state
            .append_line_for_session(
                None,
                LiveTranscriptLine {
                    text: "accepted".into(),
                    at: "10:00:00".into(),
                },
            )
            .map(|update| update.is_some())
    }));
    assert_eq!(
        current_snapshot(&state).transcript_lines[0].text,
        "accepted"
    );
    state.session.lock().unwrap().as_mut().unwrap().finish_phase =
        Some(LiveFinishPhase::Summarizing);
    let reply = append_response(&state, || {
        state
            .append_line_for_session(
                None,
                LiveTranscriptLine {
                    text: "rejected".into(),
                    at: "10:00:01".into(),
                },
            )
            .map(|update| update.is_some())
    });
    assert_eq!(reply.await.err().unwrap(), "Liveセッションを保存中です");
    assert_eq!(current_snapshot(&state).transcript_lines.len(), 1);
    *state.session.lock().unwrap() = None;
    let reply = append_response(&state, || {
        state
            .append_line_for_session(
                None,
                LiveTranscriptLine {
                    text: "missing".into(),
                    at: "10:00:02".into(),
                },
            )
            .map(|update| update.is_some())
    });
    assert_eq!(
        reply.await.err().unwrap(),
        "Liveセッションが開始されていません"
    );
}
