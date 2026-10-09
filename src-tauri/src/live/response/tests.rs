use super::super::{
    tests::transcript::recording, LiveSessionSnapshot, LiveSummaryChunk, LiveTranscriptLine,
};
use super::*;
use std::sync::{Arc, Mutex};
use tauri::ipc::IpcResponse;

#[tokio::test(flavor = "current_thread")]
async fn current_response_keeps_full_long_recording_and_existing_snapshot_fields() {
    let state = LiveState::new();
    let mut session = recording();
    for index in 0..10_000 {
        session.append_line(LiveTranscriptLine {
            text: format!("{index}: 日本語の完全な発話\n\"quoted\" 👩🏽‍💻"),
            at: "23:59:59".into(),
        });
    }
    session.summaries = Arc::new(vec![LiveSummaryChunk {
        title: "complete chunk".into(),
        range_label: "23:50-23:59".into(),
        body: "# 全文\n".repeat(2048),
        line_count: 10_000,
        terms: Vec::new(),
        whiteboard: None,
    }
    .into()]);
    *state.session.lock().unwrap() = Some(session);
    let mut expected = serde_json::to_value(current_snapshot(&state)).unwrap();
    let response = current(state.clone()).await.unwrap();
    let body = response.body().unwrap();
    let parsed = body.deserialize::<serde_json::Value>().unwrap();
    assert_eq!(
        parsed["update_revision"].as_u64().unwrap(),
        expected["update_revision"].as_u64().unwrap() + 1
    );
    expected["update_revision"] = parsed["update_revision"].clone();
    assert_eq!(parsed, expected);
    assert_eq!(parsed["transcript_lines"].as_array().unwrap().len(), 10_000);
    assert_eq!(parsed["pending_lines"].as_array().unwrap().len(), 10_000);
    let decoded: LiveSessionSnapshot = serde_json::from_value(parsed).unwrap();
    assert_eq!(
        decoded.transcript_lines[9_999].text,
        "9999: 日本語の完全な発話\n\"quoted\" 👩🏽‍💻"
    );
    assert_eq!(decoded.summaries[0].body, "# 全文\n".repeat(2048));
}

struct PausedSnapshot {
    state: LiveState,
    snapshot: LiveSessionSnapshot,
    entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
}
impl Serialize for PausedSnapshot {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        assert!(
            self.state.session.try_lock().is_ok(),
            "serialization retained LIVE state lock"
        );
        assert!(
            self.state.persistence.gate.try_lock().is_ok(),
            "serialization retained storage lock"
        );
        self.entered
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(())
            .unwrap();
        self.release.lock().unwrap().take().unwrap().recv().unwrap();
        self.snapshot.serialize(serializer)
    }
}
#[tokio::test(flavor = "current_thread")]
async fn paused_encoding_releases_both_locks_and_keeps_its_snapshot_across_append_and_replacement()
{
    let state = LiveState::new();
    let mut session = recording();
    session.append_line(LiveTranscriptLine {
        text: "original full line 🌕".into(),
        at: "10:00:00".into(),
    });
    *state.session.lock().unwrap() = Some(session);
    let working = state.clone();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let task = tokio::spawn(work("snapshot failed", move || {
        // The preview path may hold storage while obtaining the snapshot. That
        // guard must be gone when the common worker begins serialization.
        let _storage = working.persistence.gate.lock().unwrap();
        Ok(PausedSnapshot {
            snapshot: current_snapshot(&working),
            state: working.clone(),
            entered: Mutex::new(Some(entered)),
            release: Mutex::new(Some(released)),
        })
    }));
    started.await.unwrap();
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    let tail = state
        .append_line_for_session(
            Some("recording-test"),
            LiveTranscriptLine {
                text: "new full tail 👩🏽‍💻".into(),
                at: "10:00:01".into(),
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(tail.line_count, 2);
    let mut next = recording();
    next.session_id = "replacement".into();
    next.append_line(LiveTranscriptLine {
        text: "another recording".into(),
        at: "11:00:00".into(),
    });
    *state.session.lock().unwrap() = Some(next);
    release.send(()).unwrap();
    let decoded = task
        .await
        .unwrap()
        .unwrap()
        .body()
        .unwrap()
        .deserialize::<LiveSessionSnapshot>()
        .unwrap();
    assert_eq!(decoded.session_id.as_deref(), Some("recording-test"));
    assert_eq!(decoded.transcript_lines.len(), 1);
    assert_eq!(decoded.transcript_lines[0].text, "original full line 🌕");
    assert_eq!(
        current_snapshot(&state).session_id.as_deref(),
        Some("replacement")
    );
    assert_eq!(
        current_snapshot(&state).transcript_lines[0].text,
        "another recording"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn saved_event_and_reply_preserve_full_markdown_snapshot_and_todo_flags() {
    let mut session = recording();
    session.append_line(LiveTranscriptLine {
        text: "完全な記録\n\"quotes\" 👩🏽‍💻".repeat(1024),
        at: "10:00:00".into(),
    });
    let result = LiveSaveResult {
        saved: true,
        path: "/temporary/記録.md".into(),
        markdown: "# 全文\n\"引用\" 🌕".repeat(2048),
        snapshot: session.completed_snapshot(),
        suggested_todos: Vec::new(),
        todos_pending: true,
    };
    let expected = serde_json::to_value(&result).unwrap();
    let published = Arc::new(Mutex::new(None));
    let output = published.clone();
    let caller = std::thread::current().id();
    let response = saved(result, FinishReply::Full, move |name, json| {
        assert_ne!(std::thread::current().id(), caller);
        if name == "live-session-saved" {
            *output.lock().unwrap() = Some(json);
        }
    })
    .await
    .unwrap();
    let reply = response
        .body()
        .unwrap()
        .deserialize::<serde_json::Value>()
        .unwrap();
    let event: serde_json::Value =
        serde_json::from_str(published.lock().unwrap().as_ref().unwrap()).unwrap();
    assert_eq!(reply, expected);
    assert_eq!(event, reply);
    let decoded: LiveSaveResult = serde_json::from_value(event).unwrap();
    assert!(decoded.todos_pending);
    assert!(!decoded.snapshot.active);
    assert_eq!(
        decoded.snapshot.transcript_lines[0].text,
        "完全な記録\n\"quotes\" 👩🏽‍💻".repeat(1024)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn inactive_and_empty_save_replies_remain_objects_with_the_existing_null_and_array_fields() {
    let state = LiveState::new();
    let inactive = current(state)
        .await
        .unwrap()
        .body()
        .unwrap()
        .deserialize::<serde_json::Value>()
        .unwrap();
    // A state read reserves a revision even without an active session; the
    // static empty-cache snapshot has no such read and starts at zero.
    let mut expected_inactive = super::super::empty_snapshot();
    expected_inactive.update_revision = 1;
    assert_eq!(inactive, serde_json::to_value(expected_inactive).unwrap());
    let result = LiveSaveResult {
        saved: false,
        path: String::new(),
        markdown: String::new(),
        snapshot: super::super::empty_snapshot(),
        suggested_todos: Vec::new(),
        todos_pending: false,
    };
    let expected = serde_json::to_value(&result).unwrap();
    let empty = encode(result)
        .await
        .unwrap()
        .body()
        .unwrap()
        .deserialize::<serde_json::Value>()
        .unwrap();
    assert_eq!(empty, expected);
}

#[tokio::test(flavor = "current_thread")]
async fn overall_summary_reply_remains_the_same_json_string_with_full_markdown() {
    let text = "# 全体要約\n\"quoted\" \\ 日本語 👩🏽‍💻".repeat(4096);
    let decoded = encode(text.clone())
        .await
        .unwrap()
        .body()
        .unwrap()
        .deserialize::<String>()
        .unwrap();
    assert_eq!(decoded, text);
}
