use super::*;
use crate::live::{commands::peek_snapshot, tests::transcript::recording};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::ipc::{InvokeResponseBody, IpcResponse};

pub(super) fn session(lines: usize) -> LiveSession {
    let mut session = recording();
    for i in 0..lines {
        session.append_line(LiveTranscriptLine {
            text: format!("{i}: 日本語の発話\n\"quoted\" 👩🏽‍💻"),
            at: "10:00:00".into(),
        });
    }
    session.summaries = Arc::new(vec![serde_json::from_value::<LiveSummaryChunk>(json!({
        "title":"段落", "range_label":"10:00-10:05", "body":"# 全文 🌕\n\n詳細", "line_count":lines,
        "terms":[{"term":"用語","explanation":"詳細","source_excerpt":"原文 🌕","external_source":""}],
        "whiteboard":{"title":"板書", "nodes":[{"id":"n1","label":"🌕","detail":"全詳細","parent_id":""}],
            "edges":[],"schema_version":1,"normalized_by":"backend"}
    })).unwrap().into()]);
    session
}
pub(super) fn expected(full: &LiveSessionSnapshot) -> Value {
    let mut value = serde_json::to_value(full).unwrap();
    let object = value.as_object_mut().unwrap();
    let lines = object.remove("transcript_lines").unwrap();
    let pending = object.remove("pending_lines").unwrap();
    let lines = lines.as_array().unwrap();
    object.insert("transcript_line_count".into(), json!(lines.len()));
    object.insert(
        "pending_from_line".into(),
        json!(lines.len() - pending.as_array().unwrap().len()),
    );
    object.insert(
        "visible_lines".into(),
        json!(&lines[lines.len().saturating_sub(120)..]),
    );
    value
}
pub(super) fn json_body(response: tauri::ipc::Response) -> String {
    match response.body().unwrap() {
        InvokeResponseBody::Json(text) => text,
        InvokeResponseBody::Raw(_) => panic!("page reply must be JSON"),
    }
}

#[test]
fn projection_keeps_all_metadata_and_summary_details_and_complete_last_120_lines() {
    for count in [0, 1, 119, 120, 121, 10_000] {
        let mut session = session(count);
        session.pending_lines =
            Arc::new(session.transcript_lines[count.saturating_sub(7)..].to_vec());
        session.finish_phase = Some(LiveFinishPhase::SavingFinal);
        session.finish_revision = 5;
        session.flush_in_flight = true;
        let mut full = session.snapshot();
        full.update_revision = 42;
        let raw = expected(&full);
        let index = Arc::downgrade(&full.transcript_lines);
        let pending = Arc::downgrade(&full.pending_lines);
        let view = LiveSurfaceSnapshot::from(full);
        assert_eq!(serde_json::to_value(&view).unwrap(), raw);
        assert_eq!(view.visible_lines.len(), count.min(120));
        assert!(Arc::ptr_eq(&view.summaries, &session.summaries));
        for (tail, original) in view
            .visible_lines
            .iter()
            .zip(&session.transcript_lines[count.saturating_sub(120)..])
        {
            assert!(Arc::ptr_eq(tail, original));
            assert_eq!(tail.text.as_ptr(), original.text.as_ptr());
        }
        assert_eq!(index.strong_count(), 1);
        assert_eq!(pending.strong_count(), 1);
    }
    let mut session = session(121);
    let long = "全文 👩🏽‍💻\n引用\"".repeat(20_000);
    session.append_line(LiveTranscriptLine {
        text: long.clone(),
        at: "10:00:01".into(),
    });
    let view = LiveSurfaceSnapshot::from(session.completed_snapshot());
    assert!(!view.active);
    assert_eq!(view.finish_phase, None);
    assert_eq!(view.next_summary_at_ms, None);
    assert_eq!(view.visible_lines.last().unwrap().text, long);
}

#[test]
fn captured_surface_does_not_retain_full_indexes_or_hidden_lines_after_replacement() {
    let state = LiveState::new();
    let session = session(10_000);
    let early = Arc::downgrade(&session.transcript_lines[0]);
    let last = Arc::downgrade(&session.transcript_lines[9_999]);
    let index = Arc::downgrade(&session.transcript_lines);
    *state.session.lock().unwrap() = Some(session);
    let view = current(&state);
    assert_eq!(index.strong_count(), 1);
    *state.session.lock().unwrap() = None;
    assert!(index.upgrade().is_none());
    assert!(early.upgrade().is_none());
    assert!(last.upgrade().is_some());
    assert_eq!(view.transcript_line_count, 10_000);
    let inactive = current(&state);
    assert!(!inactive.active);
    assert!(inactive.update_revision > view.update_revision);
    assert!(inactive.visible_lines.is_empty());
    drop(view);
    assert!(last.upgrade().is_none());
}

#[test]
fn course_preview_uses_the_same_storage_gate_and_full_cache_projection() {
    let state = LiveState::new();
    let session = session(10_000);
    let course = session.course.clone();
    let cache = cache::LiveDayCache {
        date: "2026-10-08".into(),
        course_name: course.course_name.clone(),
        started_at: "2026-10-08 10:00:00".into(),
        transcript_lines: session.transcript_lines.as_ref().clone(),
        summaries: session.summaries.as_ref().clone(),
    };
    let full = peek_snapshot(&state, course.clone(), |_| {
        assert!(state.persistence.gate.try_lock().is_err());
        Some(cache)
    });
    let raw = expected(&full);
    let view = LiveSurfaceSnapshot::from(full);
    assert_eq!(serde_json::to_value(&view).unwrap(), raw);
    assert!(!view.active);
    assert!(view.session_id.is_none());
    assert_eq!(view.pending_from_line, 10_000);
    assert_eq!(view.update_revision, 0);
    let empty = LiveSurfaceSnapshot::from(peek_snapshot(&state, course, |_| None));
    assert_eq!(empty.transcript_line_count, 0);
    assert!(empty.course.is_none());
    assert!(empty.visible_lines.is_empty());
}

struct Paused {
    state: LiveState,
    view: LiveSurfaceSnapshot,
    entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
}
impl Serialize for Paused {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        assert!(self.state.session.try_lock().is_ok());
        assert!(self.state.persistence.gate.try_lock().is_ok());
        self.entered
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(())
            .unwrap();
        self.release
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        self.view.serialize(serializer)
    }
}
#[tokio::test(flavor = "current_thread")]
async fn encoding_runs_on_a_worker_without_locks_or_a_retained_history_index() {
    let state = LiveState::new();
    *state.session.lock().unwrap() = Some(session(10_000));
    let worker = state.clone();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let task = tokio::spawn(response::work("surface fixture", move || {
        Ok(Paused {
            view: current(&worker),
            state: worker,
            entered: Mutex::new(Some(entered)),
            release: Mutex::new(Some(released)),
        })
    }));
    started.await.unwrap();
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    {
        let guard = state.session.lock().unwrap();
        let session = guard.as_ref().unwrap();
        assert_eq!(Arc::strong_count(&session.transcript_lines), 1);
        assert_eq!(Arc::strong_count(&session.pending_lines), 1);
    }
    state
        .append_line_for_session(
            Some("recording-test"),
            LiveTranscriptLine {
                text: "new tail".into(),
                at: "10:00:01".into(),
            },
        )
        .unwrap()
        .unwrap();
    *state.session.lock().unwrap() = Some(session(1));
    release.send(()).unwrap();
    let parsed: LiveSurfaceSnapshot =
        serde_json::from_str(&json_body(task.await.unwrap().unwrap())).unwrap();
    assert_eq!(parsed.transcript_line_count, 10_000);
    assert_eq!(parsed.visible_lines.len(), 120);
    assert_eq!(
        parsed.visible_lines.last().unwrap().text,
        "9999: 日本語の発話\n\"quoted\" 👩🏽‍💻"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn actual_worker_replies_reduce_transcript_bytes_and_leave_the_complete_record_intact() {
    let state = LiveState::new();
    *state.session.lock().unwrap() = Some(session(10_000));
    let full = json_body(response::current(state.clone()).await.unwrap());
    let thin_state = state.clone();
    let page = json_body(
        response::work("Live状態取得処理失敗", move || {
            Ok(current(&thin_state))
        })
        .await
        .unwrap(),
    );
    let full_parsed: LiveSessionSnapshot = serde_json::from_str(&full).unwrap();
    let parsed: LiveSurfaceSnapshot = serde_json::from_str(&page).unwrap();
    let mut raw = expected(&full_parsed);
    raw["update_revision"] = json!(parsed.update_revision);
    assert_eq!(serde_json::from_str::<Value>(&page).unwrap(), raw);
    assert_eq!(parsed.update_revision, full_parsed.update_revision + 1);
    assert!(page.len() * 20 < full.len());
    println!("10000-line actual worker replies: full {} bytes -> page {} bytes; all summaries and complete last 120 lines retained", full.len(), page.len());
    let guard = state.session.lock().unwrap();
    let session = guard.as_ref().unwrap();
    assert_eq!(session.transcript_lines.len(), 10_000);
    assert_eq!(session.pending_lines.len(), 10_000);
    assert_eq!(
        session.transcript_lines[0].text,
        "0: 日本語の発話\n\"quoted\" 👩🏽‍💻"
    );
}

pub(super) fn saved_fixture(count: usize) -> LiveSaveResult {
    let session = session(count);
    let markdown = build_markdown(
        &session.course,
        session.started_at,
        session.started_at + chrono::Duration::minutes(30),
        "### 全体要約\n\n- 中文 👩🏽‍💻\n- **完全な要約**\n\n### 今回の論点\n- 別の段落",
        &session.summaries,
        &session.transcript_lines,
    );
    LiveSaveResult {
        saved: true,
        path: "/fixture/授業.md".into(),
        markdown,
        snapshot: session.completed_snapshot(),
        suggested_todos: vec![serde_json::from_value(json!({
            "title":"完全な TODO", "course_name":"授業", "content_type":"assignment",
            "deadline":"2026-10-09", "note":"全文 🌕", "source_excerpt":"原文",
            "day":1, "period":3
        }))
        .unwrap()],
        todos_pending: true,
    }
}

pub(super) fn expected_saved(result: &LiveSaveResult) -> Value {
    let mut raw = serde_json::to_value(result).unwrap();
    let object = raw.as_object_mut().unwrap();
    object.remove("markdown");
    object.insert(
        "summary_markdown".into(),
        json!("- 中文 👩🏽‍💻\n- **完全な要約**"),
    );
    object.insert("snapshot".into(), expected(&result.snapshot));
    raw
}

#[test]
fn saved_summary_matches_the_original_ecmascript_section_and_whitespace_boundaries() {
    // Both runtimes consume the same independently specified expectations.
    let cases: Vec<Value> = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/live-save-summary.json"
    ))
    .unwrap();
    for case in cases {
        assert_eq!(
            summary_markdown(case["markdown"].as_str().unwrap()),
            case["summary"].as_str().unwrap(),
            "{}",
            case["name"]
        );
    }
}

#[test]
fn saved_projection_keeps_final_metadata_and_releases_archive_and_hidden_line_owners() {
    let result = saved_fixture(10_000);
    assert!(result.markdown.contains("0: 日本語の発話"));
    assert!(result.markdown.contains("9999: 日本語の発話"));
    let raw = expected_saved(&result);
    let index = Arc::downgrade(&result.snapshot.transcript_lines);
    let pending = Arc::downgrade(&result.snapshot.pending_lines);
    let first = Arc::downgrade(&result.snapshot.transcript_lines[0]);
    let tail = Arc::downgrade(&result.snapshot.transcript_lines[9_999]);
    let summaries = Arc::downgrade(&result.snapshot.summaries);
    let view = LiveSurfaceSaveResult::from(result);
    assert_eq!(serde_json::to_value(&view).unwrap(), raw);
    assert!(index.upgrade().is_none());
    assert!(pending.upgrade().is_none());
    assert!(first.upgrade().is_none());
    assert!(tail.upgrade().is_some());
    assert!(summaries.upgrade().is_some());
    assert_eq!(view.snapshot.visible_lines.len(), 120);
    assert_eq!(view.snapshot.transcript_line_count, 10_000);
    assert!(!view.snapshot.active);
    assert!(view.snapshot.finish_phase.is_none());
    assert_eq!(view.snapshot.session_id.as_deref(), Some("recording-test"));
    drop(view);
    assert!(tail.upgrade().is_none());
    assert!(summaries.upgrade().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn saved_worker_publishes_only_the_page_event_for_surface_saves_and_keeps_legacy_full_replies(
) {
    let result = saved_fixture(10_000);
    let full_expected = serde_json::to_value(&result).unwrap();
    let page_expected = expected_saved(&result);
    let caller = std::thread::current().id();
    let mut sizes = Vec::new();
    for reply in [response::FinishReply::Full, response::FinishReply::Surface] {
        let events = Arc::new(Mutex::new(Vec::new()));
        let publish = events.clone();
        let raw = json_body(
            response::saved(result.clone(), reply, move |name, json| {
                assert_ne!(std::thread::current().id(), caller);
                publish.lock().unwrap().push((name, json));
            })
            .await
            .unwrap(),
        );
        let events = events.lock().unwrap();
        let expected = match reply {
            response::FinishReply::CompactSurface => {
                unreachable!("compact reply tested separately")
            }
            response::FinishReply::Full => {
                assert_eq!(events.len(), 2);
                assert_eq!(events[1].0, "live-session-saved");
                assert_eq!(events[1].1, raw);
                &full_expected
            }
            response::FinishReply::Surface => {
                assert_eq!(
                    events.len(),
                    1,
                    "surface save published a full-record event"
                );
                assert_eq!(events[0].1, raw);
                &page_expected
            }
        };
        assert_eq!(serde_json::from_str::<Value>(&raw).unwrap(), *expected);
        assert_eq!(events[0].0, "live-surface-saved");
        assert_eq!(
            serde_json::from_str::<Value>(&events[0].1).unwrap(),
            page_expected
        );
        sizes.push(raw.len());
    }
    assert!(sizes[1] * 20 < sizes[0]);
    println!("10000-line actual save replies: full {} bytes -> page {} bytes; complete archive built, exact summary preview, all chunks and last 120 lines retained", sizes[0], sizes[1]);
    // The canonical save input, including the first/last transcript and full
    // Markdown, remains complete for disk persistence and background TODOs.
    assert_eq!(serde_json::to_value(&result).unwrap(), full_expected);
    for reply in [response::FinishReply::Full, response::FinishReply::Surface] {
        let empty = LiveSaveResult {
            saved: false,
            path: String::new(),
            markdown: String::new(),
            snapshot: empty_snapshot(),
            suggested_todos: Vec::new(),
            todos_pending: false,
        };
        let raw = json_body(
            response::saved(empty, reply, |_, _| {
                panic!("empty save emitted a saved event")
            })
            .await
            .unwrap(),
        );
        let value: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["saved"], false);
        assert_eq!(value["todos_pending"], false);
        match reply {
            response::FinishReply::CompactSurface => {
                unreachable!("compact reply tested separately")
            }
            response::FinishReply::Full => assert!(value["snapshot"]["transcript_lines"]
                .as_array()
                .unwrap()
                .is_empty()),
            response::FinishReply::Surface => {
                assert_eq!(value["summary_markdown"], "");
                assert_eq!(value["snapshot"]["transcript_line_count"], 0);
                assert!(value["snapshot"]["visible_lines"]
                    .as_array()
                    .unwrap()
                    .is_empty());
                assert!(value.get("markdown").is_none());
            }
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn saved_page_publication_does_not_retain_full_history_or_block_async_progress_and_new_recordings(
) {
    let state = LiveState::new();
    let result = saved_fixture(10_000);
    let first = Arc::downgrade(&result.snapshot.transcript_lines[0]);
    let index = Arc::downgrade(&result.snapshot.transcript_lines);
    let expected = expected_saved(&result);
    let worker_state = state.clone();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let mut entered = Some(entered);
    let task = tokio::spawn(response::saved(
        result,
        response::FinishReply::Surface,
        move |name, _| {
            assert_eq!(name, "live-surface-saved");
            assert!(worker_state.session.try_lock().is_ok());
            assert!(worker_state.persistence.gate.try_lock().is_ok());
            assert!(first.upgrade().is_none());
            assert!(index.upgrade().is_none());
            entered.take().unwrap().send(()).unwrap();
            released.recv_timeout(Duration::from_secs(5)).unwrap();
        },
    ));
    started.await.unwrap();
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    let mut replacement = session(1);
    replacement.session_id = "replacement".into();
    *state.session.lock().unwrap() = Some(replacement);
    state
        .append_line_for_session(
            Some("replacement"),
            LiveTranscriptLine {
                text: "new recording proceeds".into(),
                at: "11:00:01".into(),
            },
        )
        .unwrap()
        .unwrap();
    release.send(()).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&json_body(task.await.unwrap().unwrap())).unwrap(),
        expected
    );
    assert_eq!(current(&state).transcript_line_count, 2);
}
