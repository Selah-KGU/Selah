use super::*;
use crate::live::support::{current_notification, current_update};
use crate::live::tests::transcript::recording;
use crate::live::{LiveState, SharedWhiteboard};
use serde_json::{json, Value};
use std::sync::Arc;
#[path = "../ai_output/reconcile/fixtures.rs"]
mod boards;

fn chunk(board: Option<SharedWhiteboard>, index: usize) -> SharedSummaryChunk {
    LiveSummaryChunk {
        title: format!("区間 {index} 🙂"),
        range_label: "10:00-10:05".into(),
        body: format!("# 本文 {index}\n中文・日本語 👩🏽‍💻\n\"引用\""),
        line_count: index,
        terms: vec![super::super::LiveTermExplanation {
            term: "講義".into(),
            explanation: "用語の完全な説明".into(),
            source_excerpt: "原文の出典".into(),
            external_source: "https://example.invalid".into(),
        }],
        whiteboard: board,
    }
    .into()
}
fn expand(mut wire: Value, history: &[SharedSummaryChunk]) -> Value {
    assert_eq!(
        wire.as_object_mut()
            .unwrap()
            .remove("whiteboard_delta_version"),
        Some(json!(1))
    );
    if let Some(chunk) = wire.get_mut("latest_summary") {
        if let Some(index) = chunk
            .as_object_mut()
            .unwrap()
            .remove("whiteboard_from_summary")
        {
            assert!(chunk.get("whiteboard").is_none());
            let board = history[index.as_u64().unwrap() as usize]
                .whiteboard
                .as_ref()
                .unwrap();
            chunk["whiteboard"] = serde_json::to_value(board).unwrap();
        }
    }
    wire
}

#[test]
fn actual_captures_restore_all_fields_for_256_shared_new_empty_and_missing_boards() {
    let mut wire_fixtures = vec![];
    for seed in 0..32 {
        for variant in 0..8 {
            let state = LiveState::new();
            let mut source = recording();
            let board: SharedWhiteboard = boards::board(3, seed).into();
            let empty: SharedWhiteboard = boards::board(0, seed).into();
            let chunks = match variant {
                0 => vec![chunk(Some(board.clone()), 0), chunk(Some(board.clone()), 1)],
                1 => vec![
                    chunk(Some(board.clone()), 0),
                    chunk(None, 1),
                    chunk(Some(board.clone()), 2),
                ],
                2 => vec![
                    chunk(Some(board.clone()), 0),
                    chunk(Some(empty.clone()), 1),
                    chunk(Some(empty), 2),
                ],
                3 => vec![
                    chunk(Some(board.clone()), 0),
                    chunk(Some(Arc::new(board.as_ref().clone())), 1),
                ],
                4 => vec![
                    chunk(Some(board), 0),
                    chunk(Some(boards::board(4, seed + 1).into()), 1),
                ],
                5 => vec![chunk(Some(board), 0), chunk(None, 1)],
                6 => vec![chunk(Some(board), 0)],
                _ => vec![],
            };
            source.summaries = Arc::new(chunks);
            let history = source.summaries.clone();
            *state.session.lock().unwrap() = Some(source);
            let notification = current_notification(&state, true).unwrap();
            let wire = serde_json::to_value(&notification).unwrap();
            let mut legacy = serde_json::to_value(current_update(&state, true).unwrap()).unwrap();
            assert!(
                legacy["update_revision"].as_u64().unwrap()
                    > wire["update_revision"].as_u64().unwrap()
            );
            // These are two ordered captures of the identical source state.
            legacy["update_revision"] = wire["update_revision"].clone();
            assert_eq!(expand(wire.clone(), &history), legacy);
            let reference = wire["latest_summary"].get("whiteboard_from_summary");
            if variant <= 2 {
                assert_eq!(reference, Some(&json!(if variant == 2 { 1 } else { 0 })));
                assert!(wire["latest_summary"].get("whiteboard").is_none());
            } else {
                assert!(reference.is_none());
            }
            let status: crate::live::LiveSessionStatus =
                serde_json::from_value(wire.clone()).unwrap();
            assert!(status.active);
            assert_eq!(status.session_id.as_deref(), Some("recording-test"));
            assert_eq!(
                status.update_revision,
                wire["update_revision"].as_u64().unwrap()
            );
            // Keep eight real native wires for cross-language receiver checks.
            if seed == 0 {
                let mut snapshot =
                    serde_json::to_value(crate::live::surface::current(&state)).unwrap();
                snapshot["update_revision"] = json!(0);
                snapshot["summaries"].as_array_mut().unwrap().pop();
                wire_fixtures.push(json!({ "variant": variant, "current": snapshot, "notification": wire, "legacy_update": legacy }));
            }
        }
    }
    if let Ok(path) = std::env::var("SELAH_NOTIFICATION_FIXTURE_PATH") {
        std::fs::write(path, serde_json::to_vec_pretty(&wire_fixtures).unwrap()).unwrap();
    }
}

#[test]
fn captures_and_event_clones_keep_no_old_chunks_speech_or_summary_index() {
    let state = LiveState::new();
    let mut source = recording();
    let board: SharedWhiteboard = boards::board(512, 7).into();
    let board_weak = Arc::downgrade(&board);
    let old = chunk(Some(board.clone()), 0);
    let old_weak = Arc::downgrade(&old);
    source.summaries = Arc::new(vec![old, chunk(None, 1), chunk(Some(board), 2)]);
    let history_weak = Arc::downgrade(&source.summaries);
    let speech_weak = Arc::downgrade(&source.transcript_lines);
    let pending_weak = Arc::downgrade(&source.pending_lines);
    *state.session.lock().unwrap() = Some(source);
    let notification = current_notification(&state, true).unwrap();
    let cloned = notification.clone();
    let wire = serde_json::to_vec(&notification).unwrap();
    assert!(state.session.try_lock().is_ok());
    *state.session.lock().unwrap() = None;
    assert!(old_weak.upgrade().is_none());
    assert!(history_weak.upgrade().is_none());
    assert!(speech_weak.upgrade().is_none());
    assert!(pending_weak.upgrade().is_none());
    assert!(board_weak.upgrade().is_some());
    assert_eq!(serde_json::to_vec(&cloned).unwrap(), wire);
    drop(notification);
    drop(cloned);
    assert!(board_weak.upgrade().is_none());
}

#[test]
fn metadata_and_inactive_updates_never_reference_a_board_or_keep_a_chunk() {
    let state = LiveState::new();
    let inactive = serde_json::to_value(current_notification(&state, true).unwrap()).unwrap();
    assert_eq!(inactive["active"], false);
    assert!(inactive.get("latest_summary").is_none());
    let mut source = recording();
    source.summaries = Arc::new(vec![chunk(Some(boards::board(512, 2).into()), 0)]);
    *state.session.lock().unwrap() = Some(source);
    let notification = current_notification(&state, false).unwrap();
    assert!(notification.update.latest_summary.is_none());
    let wire = serde_json::to_value(notification).unwrap();
    assert!(wire.get("latest_summary").is_none());
    assert!(wire.get("whiteboard_from_summary").is_none());
    assert_eq!(wire["summary_count"], 1);
}

#[test]
fn carried_notification_bytes_do_not_grow_with_board_contents() {
    let mut sizes = vec![];
    for count in [3, 96, 512, 4096] {
        let state = LiveState::new();
        let mut source = recording();
        let board: SharedWhiteboard = boards::board(count, 4).into();
        source.summaries = Arc::new(vec![chunk(Some(board.clone()), 0), chunk(Some(board), 1)]);
        *state.session.lock().unwrap() = Some(source);
        let old = serde_json::to_vec(&current_update(&state, true).unwrap()).unwrap();
        let new = serde_json::to_vec(&current_notification(&state, true).unwrap()).unwrap();
        sizes.push(new.len());
        assert!(new.len() < 1000);
        assert!(new.len() < old.len());
        eprintln!(
            "LIVE carried notification, {count} nodes: {} -> {} bytes",
            old.len(),
            new.len()
        );
    }
    assert!(sizes.iter().all(|size| *size == sizes[0]));
}
