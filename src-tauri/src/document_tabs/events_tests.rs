use super::*;
use std::sync::Arc;
use tauri::Listener;

fn tabs(count: usize) -> Vec<DocumentTabInfo> {
    (0..count)
        .map(|i| DocumentTabInfo {
            id: format!("id-{i}"),
            label: format!("label-{i}"),
            target: format!("page-{i}"),
            title: format!("授業 {i}"),
            url: String::new(),
            kind: "detail".into(),
            active: i == 0,
            loading: false,
            controls: vec![],
            split_ratios: vec![],
            reopen: None,
        })
        .collect()
}

#[test]
fn global_listener_receives_one_full_snapshot_for_any_tab_count() {
    for count in [0, 1, 4, 32] {
        let app = tauri::test::mock_app();
        let received = Arc::new(Mutex::new(Vec::new()));
        let captured = received.clone();
        app.listen_any("document-tabs-changed", move |event| {
            captured.lock().unwrap().push(event.payload().to_string());
        });
        let input = tabs(count);
        let expected = serde_json::json!({"owner": "document-tabs", "tabs": input});
        emit_tab_snapshot(app.handle(), "document-tabs", input);
        let values = received.lock().unwrap();
        assert_eq!(values.len(), 1, "count={count}");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&values[0]).unwrap(),
            expected
        );
    }
}

// Exact previous per-label emission, frozen as a test-only baseline. Tauri's
// real global listener sees each emit_to, even for labels with no webview.
fn old_emit<R: tauri::Runtime>(app: &tauri::AppHandle<R>, tabs: &[DocumentTabInfo]) {
    let payload = serde_json::json!({ "owner": "document-tabs", "tabs": tabs });
    let mut labels = vec![TAB_STRIP_LABEL.to_string(), AGENT_PANEL_LABEL.to_string()];
    for tab in tabs {
        for index in 0..MAX_SPLIT_DIVIDERS {
            labels.push(split_divider_target(&tab.target, index));
        }
    }
    for label in labels {
        app.emit_to(
            tauri::EventTarget::AnyLabel { label },
            "document-tabs-changed",
            payload.clone(),
        )
        .unwrap();
    }
}

#[test]
fn old_global_fanout_is_two_plus_two_per_tab_while_new_delivery_is_once() {
    let app = tauri::test::mock_app();
    let counts = Arc::new(Mutex::new(0));
    let captured = counts.clone();
    app.listen_any("document-tabs-changed", move |_| {
        *captured.lock().unwrap() += 1
    });
    for count in [0, 1, 4, 32] {
        let tabs = tabs(count);
        *counts.lock().unwrap() = 0;
        old_emit(app.handle(), &tabs);
        let before = *counts.lock().unwrap();
        assert_eq!(before, 2 + 2 * count);
        *counts.lock().unwrap() = 0;
        emit_tab_snapshot(app.handle(), "document-tabs", tabs);
        assert_eq!(*counts.lock().unwrap(), 1);
        println!("tabs={count}: global callbacks {before} -> 1");
    }
}

#[test]
fn label_subscriptions_keep_the_same_receivers_and_unrelated_labels_are_excluded() {
    let app = tauri::test::mock_app();
    let received = Arc::new(Mutex::new(HashMap::<String, usize>::new()));
    let input = vec![DocumentTabInfo {
        id: "tab".into(),
        label: "label".into(),
        target: "page".into(),
        title: "授業".into(),
        url: String::new(),
        kind: "detail".into(),
        active: true,
        loading: false,
        controls: vec![],
        split_ratios: vec![],
        reopen: None,
    }];
    let wanted = snapshot_targets(&input);
    let mut windows = Vec::new();
    for label in wanted.iter().chain(["unrelated".to_string()].iter()) {
        let window = tauri::WindowBuilder::new(&app, label).build().unwrap();
        let name = label.clone();
        let captured = received.clone();
        window.listen("document-tabs-changed", move |_| {
            *captured.lock().unwrap().entry(name.clone()).or_default() += 1
        });
        windows.push(window);
    }
    let captured = received.clone();
    app.listen("document-tabs-changed", move |_| {
        *captured
            .lock()
            .unwrap()
            .entry("app-only".into())
            .or_default() += 1
    });
    emit_tab_snapshot(app.handle(), "document-tabs", input.clone());
    assert_eq!(received.lock().unwrap().len(), wanted.len());
    for label in &wanted {
        assert_eq!(received.lock().unwrap().get(label), Some(&1));
    }
    received.lock().unwrap().clear();
    old_emit(app.handle(), &input);
    assert_eq!(received.lock().unwrap().len(), wanted.len());
    for label in &wanted {
        assert_eq!(received.lock().unwrap().get(label), Some(&1));
    }
    // Pure target-kind checks use the same production filter. Tauri's Any
    // bypass is separately covered by the real mock-runtime global tests.
    for label in wanted {
        for target in [
            tauri::EventTarget::AnyLabel {
                label: label.clone(),
            },
            tauri::EventTarget::Window {
                label: label.clone(),
            },
            tauri::EventTarget::Webview {
                label: label.clone(),
            },
            tauri::EventTarget::WebviewWindow { label },
        ] {
            assert!(matches_target(&snapshot_targets(&input), &target));
        }
    }
    assert!(!matches_target(
        &snapshot_targets(&input),
        &tauri::EventTarget::App
    ));
    assert!(!matches_target(
        &snapshot_targets(&input),
        &tauri::EventTarget::AnyLabel {
            label: "unrelated".into()
        }
    ));
}
