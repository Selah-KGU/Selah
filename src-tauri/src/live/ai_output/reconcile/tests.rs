use super::*;
use crate::live::{LiveSummaryChunk, SharedSummaryChunk};
use std::sync::Arc;
#[path = "before.rs"]
mod before;
#[path = "fixtures.rs"]
mod fixtures;

fn summary(board: Option<SharedWhiteboard>) -> SharedSummaryChunk {
    LiveSummaryChunk {
        title: "区間".into(),
        range_label: "10:00-10:10".into(),
        body: "本文の全文".into(),
        line_count: 1,
        terms: vec![],
        whiteboard: board,
    }
    .into()
}

#[test]
fn shared_selection_matches_owned_predecessor_for_3136_legacy_and_model_pairs() {
    for seed in 0..64 {
        for count in [0, 1, 3, 4, 6, 12, 64] {
            let previous: SharedWhiteboard = fixtures::board(count, seed).into();
            let original = serde_json::to_vec(&previous).unwrap();
            for mode in 0..7 {
                let mut current = fixtures::board(
                    match mode {
                        2 => 3,
                        4 => count + 1,
                        _ => count,
                    },
                    seed,
                );
                match mode {
                    3 => {
                        for node in &mut current.nodes {
                            node.id = format!("new-{}", node.id);
                        }
                    }
                    5 => {
                        for node in &mut current.nodes {
                            node.role = "branch".into();
                            node.node_type = "term".into();
                        }
                    }
                    6 => current.edges.clear(),
                    _ => {}
                }
                let model = (mode != 0).then_some(current);
                let expected = before::reconcile_whiteboard(Some(&previous), model.clone());
                let actual = reconcile_whiteboard(Some(&previous), model);
                assert_eq!(
                    serde_json::to_vec(&actual).unwrap(),
                    serde_json::to_vec(&expected).unwrap(),
                    "{seed}/{count}/{mode}"
                );
                assert_eq!(serde_json::to_vec(&previous).unwrap(), original);
            }
        }
    }
}

#[test]
fn carry_forward_reuses_all_buffers_and_has_no_hidden_retainer() {
    let previous: SharedWhiteboard = fixtures::board(512, 1).into();
    let weak = Arc::downgrade(&previous);
    let original = serde_json::to_vec(&previous).unwrap();
    let mut history = vec![];
    for index in 0..64 {
        let current = (index % 2 == 1).then(|| fixtures::board(1, 1));
        let board = reconcile_whiteboard(Some(&previous), current).unwrap();
        assert!(Arc::ptr_eq(&board, &previous));
        assert_eq!(board.nodes.as_ptr(), previous.nodes.as_ptr());
        assert_eq!(
            board.nodes[0].detail.as_ptr(),
            previous.nodes[0].detail.as_ptr()
        );
        assert_eq!(board.edges.as_ptr(), previous.edges.as_ptr());
        history.push(summary(Some(board)));
    }
    assert_eq!(Arc::strong_count(&previous), 65);
    let captured = history.clone();
    drop(previous);
    drop(history);
    assert_eq!(
        serde_json::to_vec(captured[0].whiteboard.as_ref().unwrap()).unwrap(),
        original
    );
    assert!(weak.upgrade().is_some());
    drop(captured);
    assert!(weak.upgrade().is_none());
}

#[test]
fn accepted_owned_model_moves_buffers_once_and_keeps_old_version_immutable() {
    let previous: SharedWhiteboard = fixtures::board(64, 1).into();
    let old_bytes = serde_json::to_vec(&previous).unwrap();
    let mut model = fixtures::board(65, 1);
    for node in &mut model.nodes {
        node.id = format!("rewritten-{}", node.id);
    }
    model.edges.clear();
    let expected = serde_json::to_vec(&model).unwrap();
    let nodes = model.nodes.as_ptr();
    let detail = model.nodes[0].detail.as_ptr();
    let output = reconcile_whiteboard(Some(&previous), Some(model)).unwrap();
    assert!(!Arc::ptr_eq(&output, &previous));
    assert_eq!(output.nodes.as_ptr(), nodes);
    assert_eq!(output.nodes[0].detail.as_ptr(), detail);
    assert_eq!(Arc::strong_count(&output), 1);
    assert_eq!(serde_json::to_vec(&output).unwrap(), expected);
    assert_eq!(serde_json::to_vec(&previous).unwrap(), old_bytes);
    let fresh = reconcile_whiteboard(None, Some(fixtures::board(1, 1))).unwrap();
    assert_eq!(fresh.nodes.len(), 1);
    assert!(reconcile_whiteboard(None, None).is_none());
}

#[test]
fn shared_boards_serialize_complete_legacy_objects_and_empty_latest_stays_authoritative() {
    let board: SharedWhiteboard = fixtures::board(12, 1).into();
    let history = vec![
        summary(Some(board.clone())),
        summary(None),
        summary(Some(board.clone())),
    ];
    let json = serde_json::to_vec(&history).unwrap();
    let value = serde_json::to_value(&history).unwrap();
    assert_eq!(
        value[0]["whiteboard"],
        serde_json::to_value(board.as_ref()).unwrap()
    );
    assert_eq!(value[0]["whiteboard"], value[2]["whiteboard"]);
    assert!(value[1].get("whiteboard").is_none());
    let restored: Vec<SharedSummaryChunk> = serde_json::from_slice(&json).unwrap();
    assert_eq!(serde_json::to_vec(&restored).unwrap(), json);
    assert_eq!(restored[0].whiteboard.as_ref().unwrap().nodes.len(), 12);
    // Wire objects carry full content rather than sharing IDs. No interning is
    // assumed when reading an older cache with repeated complete objects.
    assert!(!Arc::ptr_eq(
        restored[0].whiteboard.as_ref().unwrap(),
        restored[2].whiteboard.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(
        super::super::context::latest_shared_whiteboard(&history).unwrap(),
        &board
    ));
    let empty: SharedWhiteboard = fixtures::board(0, 1).into();
    let current = vec![
        history[0].clone(),
        summary(Some(empty.clone())),
        summary(None),
    ];
    assert!(Arc::ptr_eq(
        super::super::context::latest_shared_whiteboard(&current).unwrap(),
        &empty
    ));
}

#[test]
fn fixed_guards_preserve_shrink_churn_cross_edges_and_allow_growing_rewrites() {
    let previous: SharedWhiteboard = fixtures::board(12, 1).into();
    assert!(Arc::ptr_eq(
        &reconcile_whiteboard(Some(&previous), Some(fixtures::board(3, 1))).unwrap(),
        &previous
    ));
    for mode in 0..3 {
        let mut model = fixtures::board(12, 1);
        match mode {
            0 => {
                for node in &mut model.nodes {
                    if node.role == "main" {
                        node.id = format!("new-{}", node.id);
                    }
                }
            }
            1 => {
                for node in &mut model.nodes {
                    if node.role != "main" {
                        node.id = format!("new-{}", node.id);
                    }
                }
            }
            _ => model.edges.clear(),
        }
        assert!(Arc::ptr_eq(
            &reconcile_whiteboard(Some(&previous), Some(model)).unwrap(),
            &previous
        ));
    }
    let mut growing = fixtures::board(13, 1);
    for node in &mut growing.nodes {
        node.id = format!("all-new-{}", node.id);
    }
    growing.edges.clear();
    assert!(!Arc::ptr_eq(
        &reconcile_whiteboard(Some(&previous), Some(growing)).unwrap(),
        &previous
    ));
}
