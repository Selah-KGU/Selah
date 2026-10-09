use super::*;

fn tab(id: usize) -> DocumentTab {
    DocumentTab {
        id: format!("tab-{id}"),
        label: format!("label-{id}"),
        target: format!("target-{id}"),
        key: Some(format!("key-{id}")),
        title: format!("Page {id}"),
        url: format!("https://page-{id}.example/"),
        kind: "detail".into(),
        controls: (0..8)
            .map(|i| DocumentTabControl {
                id: format!("control-{i}"),
                label: format!("Control {i}"),
                action: "detail.refresh".into(),
                icon: Some("arrow.clockwise".into()),
                value: None,
                group: Some("view".into()),
                primary: i == 0,
                active: false,
                disabled: false,
                tone: None,
                payload: Some(serde_json::json!({ "index": i, "query": "日本語検索", "options": ["a", "b", "c"] })),
                indicator: true,
                indicator_on: true,
            })
            .collect(),
        reopen: Some(serde_json::json!({"type": "detail", "params": format!("page-{id}")})),
        loading: id % 2 == 0,
        child: Some(ChildPane {
            target: format!("child-{id}"),
            label: format!("child-label-{id}"),
            is_board: true,
            child: Some(Box::new(ChildPane {
                target: format!("grandchild-{id}"),
                label: format!("grandchild-label-{id}"),
                is_board: false,
                child: None,
            })),
        }),
        split_weights: vec![0.2, 0.3, 0.5],
        active_split_drag: Some(1),
    }
}

fn window(count: usize) -> DocumentWindowState {
    DocumentWindowState {
        tabs: (0..count).map(tab).collect(),
        active: count.checked_sub(1).map(|i| format!("tab-{i}")),
    }
}

// Frozen previous read paths, retained only as byte/value equivalence and
// benchmark baselines. They intentionally clone the whole window first.
fn old_active(state: &DocumentWindowState) -> Option<DocumentTab> {
    let state = state.clone();
    let active = state.active.as_deref()?;
    state.tabs.iter().find(|tab| tab.id == active).cloned()
}
fn old_infos(state: &DocumentWindowState) -> Vec<DocumentTabInfo> {
    tab_infos(&state.clone())
}

#[test]
fn selected_tab_keeps_the_clicked_origin_after_activation_changes() {
    let mut state = window(3);
    assert_eq!(selected_tab(&state, None).unwrap().id, "tab-2");
    assert_eq!(
        selected_tab(&state, Some("tab-0")).unwrap().target,
        "target-0"
    );
    state.active = Some("tab-1".into());
    assert_eq!(
        selected_tab(&state, Some("tab-0")).unwrap().target,
        "target-0"
    );
    assert_eq!(selected_tab(&state, None).unwrap().target, "target-1");
    state.tabs.remove(0);
    assert!(selected_tab(&state, Some("tab-0")).is_none());
    assert!(selected_tab(&state, Some("another-owner-tab")).is_none());
    assert!(selected_tab(&state, Some("")).is_none());
}

#[test]
fn explicit_tab_does_not_require_an_active_tab_and_legacy_missing_active_stays_missing() {
    let mut state = window(3);
    state.active = None;
    assert!(selected_tab(&state, None).is_none());
    assert_eq!(selected_tab(&state, Some("tab-1")).unwrap().id, "tab-1");
    state.active = Some("closed".into());
    assert!(selected_tab(&state, None).is_none());
    assert!(selected_tab(&DocumentWindowState::default(), Some("tab-1")).is_none());
}

#[test]
fn tab_list_projection_preserves_full_json_order_flags_controls_and_split_ratios() {
    for count in [0, 1, 4, 32] {
        let state = window(count);
        let before = serde_json::to_vec(&old_infos(&state)).unwrap();
        let after = serde_json::to_vec(&tab_infos(&state)).unwrap();
        assert_eq!(after, before);
        assert_eq!(
            tab_infos(&state).iter().filter(|tab| tab.active).count(),
            usize::from(count > 0)
        );
        let selected = selected_tab(&state, None).cloned();
        assert_eq!(format!("{selected:?}"), format!("{:?}", old_active(&state)));
    }
}

#[test]
fn a_returned_snapshot_is_independent_of_later_state_changes() {
    let mut state = window(3);
    let selected = selected_tab(&state, Some("tab-1")).cloned().unwrap();
    let infos = tab_infos(&state);
    state.tabs[1].controls[0].payload = None;
    state.tabs[1].title = "changed".into();
    state.tabs[1].child = None;
    state.tabs.clear();
    assert_eq!(selected.title, "Page 1");
    assert!(selected.controls[0].payload.is_some());
    assert!(selected.child.unwrap().child.is_some());
    assert_eq!(infos[1].title, "Page 1");
    assert!(infos[1].controls[0].payload.is_some());
}

#[test]
#[ignore = "manual read-path microbenchmark; excludes native IPC and UI"]
fn benchmark_tab_state_reads() {
    use std::hint::black_box;
    use std::time::Instant;
    fn compare(
        state: &DocumentWindowState,
        before: impl Fn(&DocumentWindowState),
        after: impl Fn(&DocumentWindowState),
    ) -> (f64, f64) {
        let measure = |read: &dyn Fn(&DocumentWindowState)| {
            let start = Instant::now();
            for _ in 0..1000 {
                read(black_box(state));
            }
            start.elapsed().as_secs_f64() * 1e3
        };
        let mut old = Vec::new();
        let mut new = Vec::new();
        for trial in 0..9 {
            if trial % 2 == 0 {
                old.push(measure(&before));
                new.push(measure(&after));
            } else {
                new.push(measure(&after));
                old.push(measure(&before));
            }
        }
        old.sort_by(f64::total_cmp);
        new.sort_by(f64::total_cmp);
        (old[4], new[4])
    }
    for count in [1, 4, 32] {
        let state = window(count);
        // Warm both paths; values above must remain exactly equivalent.
        for _ in 0..100 {
            black_box(old_active(&state));
            black_box(selected_tab(&state, None).cloned());
            black_box(old_infos(&state));
            black_box(tab_infos(&state));
        }
        let (old, new) = compare(
            &state,
            |s| {
                black_box(old_active(s));
            },
            |s| {
                black_box(selected_tab(s, None).cloned());
            },
        );
        let (old_list, new_list) = compare(
            &state,
            |s| {
                black_box(old_infos(s));
            },
            |s| {
                black_box(tab_infos(s));
            },
        );
        println!("tabs={count}: active {old:.3} -> {new:.3} us/read; list {old_list:.3} -> {new_list:.3} us/read");
    }
}

#[test]
fn unchanged_controls_do_not_request_notifications_but_changes_and_cleanup_do() {
    let mut state = window(4);
    let controls = state.tabs[0].controls.clone();
    for _ in 0..1000 {
        assert!(!update_controls(Some(&mut state), Some("target-0"), controls.clone()).unwrap());
    }
    let mut changed = controls.clone();
    changed[0].disabled = true;
    assert!(update_controls(Some(&mut state), Some("target-0"), changed.clone()).unwrap());
    assert_eq!(state.tabs[0].controls, changed);
    assert!(!update_controls(Some(&mut state), Some("target-0"), changed).unwrap());
    assert!(update_controls(Some(&mut state), Some("target-0"), vec![]).unwrap());
    assert!(!update_controls(Some(&mut state), Some("target-0"), vec![]).unwrap());
    assert_eq!(state.tabs[1].controls, controls);
}

#[test]
fn every_control_field_and_order_change_remains_observable() {
    let mut state = window(1);
    let original = state.tabs[0].controls.clone();
    let changes: Vec<Box<dyn Fn(&mut DocumentTabControl)>> = vec![
        Box::new(|c| c.id.push('!')),
        Box::new(|c| c.label.push('!')),
        Box::new(|c| c.action.push('!')),
        Box::new(|c| c.icon = None),
        Box::new(|c| c.value = Some("new".into())),
        Box::new(|c| c.group = None),
        Box::new(|c| c.primary = !c.primary),
        Box::new(|c| c.active = !c.active),
        Box::new(|c| c.disabled = !c.disabled),
        Box::new(|c| c.tone = Some("danger".into())),
        Box::new(|c| c.payload.as_mut().unwrap()["query"] = serde_json::json!("changed")),
        Box::new(|c| c.indicator = !c.indicator),
        Box::new(|c| c.indicator_on = !c.indicator_on),
    ];
    for change in changes {
        let mut next = original.clone();
        change(&mut next[0]);
        assert!(update_controls(Some(&mut state), None, next.clone()).unwrap());
        assert_eq!(state.tabs[0].controls, next);
        assert!(update_controls(Some(&mut state), None, original.clone()).unwrap());
    }
    let mut reordered = original.clone();
    reordered.reverse();
    assert!(update_controls(Some(&mut state), None, reordered.clone()).unwrap());
    assert_eq!(state.tabs[0].controls, reordered);
}

#[test]
fn control_targets_preserve_id_label_target_aliases_and_legacy_active_resolution() {
    for alias in ["tab-0", "label-0", "target-0"] {
        let mut state = window(3);
        assert!(update_controls(Some(&mut state), Some(alias), vec![]).unwrap());
        assert!(state.tabs[0].controls.is_empty());
        assert!(!state.tabs[2].controls.is_empty());
    }
    let mut state = window(3);
    assert!(update_controls(Some(&mut state), None, vec![]).unwrap());
    assert!(state.tabs[2].controls.is_empty());
    assert!(!state.tabs[0].controls.is_empty());
    state.active = None;
    assert_eq!(
        update_controls(Some(&mut state), None, vec![]).unwrap_err(),
        "アクティブなタブがありません"
    );
    assert!(update_controls(Some(&mut state), Some("target-0"), vec![]).unwrap());
}

#[test]
fn missing_control_targets_preserve_errors_and_leave_state_untouched() {
    let mut state = window(2);
    let before = serde_json::to_vec(&tab_infos(&state)).unwrap();
    assert_eq!(
        update_controls(Some(&mut state), Some("missing"), vec![]).unwrap_err(),
        "タブが見つかりません: missing"
    );
    assert_eq!(serde_json::to_vec(&tab_infos(&state)).unwrap(), before);
    assert_eq!(
        update_controls(None, Some("missing"), vec![]).unwrap_err(),
        "タブが見つかりません: missing"
    );
    assert_eq!(
        update_controls(None, None, vec![]).unwrap_err(),
        "アクティブなタブがありません"
    );
}
