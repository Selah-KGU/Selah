use super::super::tests::{expected_saved, json_body, saved_fixture, session};
use super::*;
use serde_json::{json, Value};
use std::sync::Mutex;
#[path = "../../ai_output/reconcile/fixtures.rs"]
mod boards;

fn expand(mut wire: Value) -> Value {
    let object = wire.as_object_mut().unwrap();
    assert_eq!(object.remove("whiteboard_table_version"), Some(json!(1)));
    let boards = object.remove("whiteboards").unwrap();
    for chunk in object["summaries"].as_array_mut().unwrap() {
        let chunk = chunk.as_object_mut().unwrap();
        if let Some(reference) = chunk.remove("whiteboard_ref") {
            chunk.insert(
                "whiteboard".into(),
                boards[reference.as_u64().unwrap() as usize].clone(),
            );
        }
    }
    wire
}
fn history(count: usize, board: SharedWhiteboard) -> LiveSummaryChunks {
    Arc::new(
        (0..count)
            .map(|index| {
                LiveSummaryChunk {
                    title: format!("区間 {index}"),
                    range_label: "10:00-10:05".into(),
                    body: format!("# 全文 {index} 👩🏽‍💻\n引用\""),
                    line_count: index,
                    terms: vec![],
                    whiteboard: Some(board.clone()),
                }
                .into()
            })
            .collect(),
    )
}

#[test]
fn compact_snapshots_restore_every_legacy_byte_for_128_mixed_histories() {
    for seed in 0..32 {
        for lines in [0, 1, 120, 121] {
            let mut source = session(lines);
            let shared: SharedWhiteboard = boards::board(12, seed).into();
            let empty: SharedWhiteboard = boards::board(0, seed).into();
            let mut chunks = vec![];
            for index in 0..24 {
                let board = match index % 5 {
                    0 => None,
                    1 | 2 => Some(shared.clone()),
                    3 => Some(empty.clone()),
                    _ => Some(Arc::new(shared.as_ref().clone())),
                };
                let mut chunk = source.summaries[0].as_ref().clone();
                chunk.whiteboard = board;
                chunk.title = format!("{seed}/{index}");
                chunks.push(chunk.into());
            }
            source.summaries = Arc::new(chunks);
            let mut full = source.completed_snapshot();
            full.update_revision = seed as u64 + 1;
            full.finish_phase = Some(LiveFinishPhase::SavingFinal);
            full.finish_revision = seed as u64;
            let page = LiveSurfaceSnapshot::from(full);
            let old = serde_json::to_vec(&page).unwrap();
            let compact = CompactSurfaceSnapshot::from(page);
            assert_eq!(compact.boards.len(), 2);
            assert!(Arc::ptr_eq(&compact.boards[0], &shared));
            assert!(Arc::ptr_eq(&compact.boards[1], &empty));
            let wire = serde_json::to_value(&compact).unwrap();
            assert!(wire["summaries"][0].get("whiteboard_ref").is_none());
            assert_eq!(wire["summaries"][1]["whiteboard_ref"], 0);
            assert_eq!(wire["summaries"][2]["whiteboard_ref"], 0);
            let restored: LiveSurfaceSnapshot = serde_json::from_value(expand(wire)).unwrap();
            assert_eq!(
                serde_json::to_vec(&restored).unwrap(),
                old,
                "{seed}/{lines}"
            );
        }
    }
}

#[test]
fn identical_cache_boards_merge_but_changes_to_any_board_field_keep_separate_versions() {
    let original: SharedWhiteboard = boards::board(12, 1).into();
    let value = serde_json::to_value(&original).unwrap();
    let mut changes = vec![];
    for field in ["title", "layout", "normalized_by"] {
        let mut changed = value.clone();
        changed[field] = json!(format!("changed {field}"));
        changes.push(changed);
    }
    let mut schema = value.clone();
    schema["schema_version"] = json!(2);
    changes.push(schema);
    for field in [
        "id",
        "label",
        "detail",
        "node_type",
        "kind",
        "role",
        "parent_id",
        "source_type",
        "source_excerpt",
        "external_source",
    ] {
        let mut changed = value.clone();
        changed["nodes"][0][field] = json!(format!("changed {field}"));
        changes.push(changed);
    }
    for field in ["from", "to", "label"] {
        let mut changed = value.clone();
        changed["edges"][0][field] = json!(format!("changed {field}"));
        changes.push(changed);
    }
    let mut order = value.clone();
    order["nodes"].as_array_mut().unwrap().reverse();
    changes.push(order);
    let mut edge_order = value.clone();
    edge_order["edges"].as_array_mut().unwrap().reverse();
    changes.push(edge_order);
    for changed in changes {
        let second: SharedWhiteboard = serde_json::from_value::<LiveWhiteboard>(changed)
            .unwrap()
            .into();
        let mut source = session(0);
        let mut chunks = history(2, original.clone()).as_ref().clone();
        Arc::make_mut(&mut chunks[1]).whiteboard = Some(second);
        source.summaries = Arc::new(chunks);
        let page = LiveSurfaceSnapshot::from(source.snapshot());
        let old = serde_json::to_value(&page).unwrap();
        let compact = CompactSurfaceSnapshot::from(page);
        assert_eq!(compact.boards.len(), 2);
        assert_eq!(expand(serde_json::to_value(&compact).unwrap()), old);
    }
}

#[test]
fn compact_capture_keeps_only_visible_speech_and_releases_shared_boards_with_last_owner() {
    let state = LiveState::new();
    let mut source = session(10_000);
    let board: SharedWhiteboard = boards::board(96, 1).into();
    let weak = Arc::downgrade(&board);
    let hidden = Arc::downgrade(&source.transcript_lines[0]);
    let last = Arc::downgrade(source.transcript_lines.last().unwrap());
    source.summaries = history(32, board.clone());
    drop(board);
    *state.session.lock().unwrap() = Some(source);
    let compact = CompactSurfaceSnapshot::from(current(&state));
    *state.session.lock().unwrap() = None;
    assert!(hidden.upgrade().is_none());
    assert!(last.upgrade().is_some());
    assert_eq!(compact.boards.len(), 1);
    let json = serde_json::to_vec(&compact).unwrap();
    drop(compact);
    assert!(weak.upgrade().is_none());
    assert!(last.upgrade().is_none());
    let value: Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(value["summaries"].as_array().unwrap().len(), 32);
    assert_eq!(
        value["whiteboards"][0]["nodes"].as_array().unwrap().len(),
        96
    );
}

#[tokio::test(flavor = "current_thread")]
async fn compact_save_worker_emits_one_identical_compact_reply_and_preserves_complete_legacy_view()
{
    let mut result = saved_fixture(10_000);
    result.snapshot.summaries = history(32, boards::board(96, 1).into());
    let expected = expected_saved(&result);
    let original = serde_json::to_vec(&result).unwrap();
    let legacy = serde_json::to_vec(&LiveSurfaceSaveResult::from(result.clone())).unwrap();
    let hidden = Arc::downgrade(&result.snapshot.transcript_lines[0]);
    let publish = Arc::new(Mutex::new(vec![]));
    let events = publish.clone();
    let caller = std::thread::current().id();
    let raw = json_body(
        response::saved(
            result.clone(),
            response::FinishReply::CompactSurface,
            move |name, json| {
                assert_ne!(std::thread::current().id(), caller);
                events.lock().unwrap().push((name, json));
            },
        )
        .await
        .unwrap(),
    );
    let value: Value = serde_json::from_str(&raw).unwrap();
    let mut restored = value.clone();
    restored["snapshot"] = expand(restored["snapshot"].take());
    assert_eq!(restored, expected);
    assert_eq!(serde_json::to_vec(&result).unwrap(), original);
    assert_eq!(
        publish.lock().unwrap().as_slice(),
        [("live-surface-compact-saved", raw.clone())]
    );
    assert_eq!(
        value["snapshot"]["whiteboards"].as_array().unwrap().len(),
        1
    );
    assert!(raw.len() * 20 < legacy.len());
    println!("32 shared 96-node boards, 10000 speech lines: legacy page {} bytes -> compact page {} bytes; full saved projection and every board field restored",legacy.len(),raw.len());
    drop(result);
    assert!(hidden.upgrade().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn empty_compact_save_has_no_event_and_still_expands_to_the_legacy_empty_page() {
    let result = LiveSaveResult {
        saved: false,
        path: String::new(),
        markdown: String::new(),
        snapshot: empty_snapshot(),
        suggested_todos: vec![],
        todos_pending: false,
    };
    let mut expected = expected_saved(&result);
    expected["summary_markdown"] = json!("");
    let raw = json_body(
        response::saved(result, response::FinishReply::CompactSurface, |_, _| {
            panic!("empty save emitted event")
        })
        .await
        .unwrap(),
    );
    let mut value: Value = serde_json::from_str(&raw).unwrap();
    value["snapshot"] = expand(value["snapshot"].take());
    assert_eq!(value, expected);
}
