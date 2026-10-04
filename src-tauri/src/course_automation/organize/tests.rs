use super::super::CourseAutomationStatus;
use super::apply::{apply_groups, prune_empty_dirs};
use super::plan::confident_plan;
use super::theme::{sanitize_component, theme_of};
use super::types::{OrganizeCandidate, PlannedGroup};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

#[test]
fn session_marker_groups_across_kinds() {
    let (key_a, label_a) = theme_of("第3回 レポート課題", "", "report3.pdf", "report");
    let (key_b, _) = theme_of("講義スライド", "第3回の内容です", "slides.pdf", "material");
    assert_eq!(key_a, key_b);
    assert_eq!(label_a, "第03回");
}

#[test]
fn fullwidth_and_halfwidth_session_markers_merge() {
    let (key_full, label_full) = theme_of("第１０回 資料", "", "a.pdf", "material");
    let (key_half, label_half) = theme_of("第10回 解答", "", "b.pdf", "material");
    assert_eq!(key_full, key_half);
    assert_eq!(label_full, "第10回");
    assert_eq!(label_half, "第10回");
}

#[test]
fn topic_marker_groups_reports() {
    let (key, label) = theme_of("個人レポートの提出について", "", "report.pdf", "material");
    assert_eq!(key, "topic:レポート");
    assert_eq!(label, "レポート");
}

#[test]
fn english_and_japanese_topic_merge_to_one_folder() {
    let (key_en, label_en) = theme_of("Final Report guidelines", "", "report.pdf", "material");
    let (key_ja, label_ja) = theme_of("個人レポートについて", "", "doc.pdf", "material");
    assert_eq!(key_en, key_ja);
    assert_eq!(label_en, "レポート");
    assert_eq!(label_ja, "レポート");
}

#[test]
fn falls_back_to_kind_folder() {
    let (key, label) = theme_of("配布スライド", "", "slide.pdf", "material");
    assert_eq!(key, "kind:material");
    assert_eq!(label, "教材");
}

#[test]
fn sanitize_strips_separators() {
    assert_eq!(sanitize_component("第3回/レポート"), "第3回_レポート");
    assert!(!sanitize_component("..").contains('.'));
}

fn candidate(path: &str, kind: &str) -> OrganizeCandidate {
    let filename = Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_string();
    OrganizeCandidate {
        path: path.to_string(),
        filename: filename.clone(),
        title: filename,
        summary: String::new(),
        kind: kind.to_string(),
    }
}

#[test]
fn confident_plan_splits_marker_files_from_ambiguous() {
    let mut candidates: BTreeMap<String, OrganizeCandidate> = BTreeMap::new();
    candidates.insert("a".into(), candidate("/c/第03回資料.pdf", "material")); // session marker
    candidates.insert("b".into(), candidate("/c/レポート課題.pdf", "material")); // topic marker
    candidates.insert("c".into(), candidate("/c/20260526_live.md", "ライブノート")); // date only
    candidates.insert("d".into(), candidate("/c/slides.pdf", "material")); // no marker

    let (plan, ambiguous) = confident_plan(&candidates);

    let placed: HashSet<String> = plan.iter().flat_map(|g| g.doc_ids.clone()).collect();
    assert!(placed.contains("a") && placed.contains("b"));
    // Only the markerless files need the AI.
    assert_eq!(ambiguous.len(), 2);
    assert!(ambiguous.contains(&"c".to_string()) && ambiguous.contains(&"d".to_string()));
}

#[test]
fn filing_is_root_relative_and_never_nests() {
    let root = std::env::temp_dir().join(format!("organize-flat-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let a = root.join("第01回資料.pdf");
    let b = root.join("第01回座席表.pdf");
    std::fs::write(&a, b"a").unwrap();
    std::fs::write(&b, b"b").unwrap();

    let mut candidates: BTreeMap<String, OrganizeCandidate> = BTreeMap::new();
    candidates.insert("id_a".into(), candidate(a.to_str().unwrap(), "material"));
    candidates.insert("id_b".into(), candidate(b.to_str().unwrap(), "material"));
    let planned = vec![PlannedGroup {
        label: "第01回".into(),
        kind: "material".into(),
        doc_ids: vec!["id_a".into(), "id_b".into()],
    }];
    let mut status = CourseAutomationStatus::default();

    // First filing: both move into <root>/第01回/.
    let moved = apply_groups(&mut status, &candidates, &planned, &root);
    assert_eq!(moved, 2);
    assert!(root.join("第01回").join("第01回資料.pdf").is_file());
    assert!(root.join("第01回").join("第01回座席表.pdf").is_file());

    // Re-run with the files at their new (filed) locations: idempotent, no
    // second level of 第01回/第01回.
    let mut candidates2: BTreeMap<String, OrganizeCandidate> = BTreeMap::new();
    candidates2.insert(
        "id_a".into(),
        candidate(
            root.join("第01回").join("第01回資料.pdf").to_str().unwrap(),
            "material",
        ),
    );
    candidates2.insert(
        "id_b".into(),
        candidate(
            root.join("第01回")
                .join("第01回座席表.pdf")
                .to_str()
                .unwrap(),
            "material",
        ),
    );
    let moved2 = apply_groups(&mut status, &candidates2, &planned, &root);
    assert_eq!(moved2, 0);
    assert!(!root.join("第01回").join("第01回").exists());

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn lone_file_is_still_filed_into_its_theme_folder() {
    let root = std::env::temp_dir().join(format!("organize-solo-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let solo = root.join("第07回資料.pdf");
    std::fs::write(&solo, b"x").unwrap();

    let mut candidates: BTreeMap<String, OrganizeCandidate> = BTreeMap::new();
    candidates.insert(
        "id_solo".into(),
        candidate(solo.to_str().unwrap(), "material"),
    );
    let planned = vec![PlannedGroup {
        label: "第07回".into(),
        kind: "material".into(),
        doc_ids: vec!["id_solo".into()],
    }];
    let mut status = CourseAutomationStatus::default();

    let moved = apply_groups(&mut status, &candidates, &planned, &root);
    assert_eq!(moved, 1);
    assert!(root.join("第07回").join("第07回資料.pdf").is_file());
    // Nothing left loose at the root.
    assert!(!root.join("第07回資料.pdf").exists());

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn filing_flattens_existing_nesting() {
    let root = std::env::temp_dir().join(format!("organize-heal-{}", uuid::Uuid::new_v4()));
    // Simulate the old bug: files buried several 第01回 levels deep.
    let nested = root.join("第01回").join("第01回").join("第01回");
    std::fs::create_dir_all(&nested).unwrap();
    let a = nested.join("資料.pdf");
    let b = nested.join("座席表.pdf");
    std::fs::write(&a, b"a").unwrap();
    std::fs::write(&b, b"b").unwrap();

    let mut candidates: BTreeMap<String, OrganizeCandidate> = BTreeMap::new();
    candidates.insert("id_a".into(), candidate(a.to_str().unwrap(), "material"));
    candidates.insert("id_b".into(), candidate(b.to_str().unwrap(), "material"));
    let planned = vec![PlannedGroup {
        label: "第01回".into(),
        kind: "material".into(),
        doc_ids: vec!["id_a".into(), "id_b".into()],
    }];
    let mut status = CourseAutomationStatus::default();

    let moved = apply_groups(&mut status, &candidates, &planned, &root);
    assert_eq!(moved, 2);
    // Pulled back up to the single-level location.
    assert!(root.join("第01回").join("資料.pdf").is_file());
    assert!(root.join("第01回").join("座席表.pdf").is_file());
    // The vacated nested chain 第01回/第01回/第01回 is swept away.
    assert!(!root.join("第01回").join("第01回").exists());

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn pruning_clears_vacated_and_junk_only_folders() {
    let root = std::env::temp_dir().join(format!("organize-prune-{}", uuid::Uuid::new_v4()));
    // An empty theme folder, one with only a .DS_Store, and a nested empty
    // chain — all should be removed. A folder with a real file must survive.
    std::fs::create_dir_all(root.join("空")).unwrap();
    std::fs::create_dir_all(root.join("ゴミ")).unwrap();
    std::fs::write(root.join("ゴミ").join(".DS_Store"), b"x").unwrap();
    std::fs::create_dir_all(root.join("深").join("層").join("空")).unwrap();
    std::fs::create_dir_all(root.join("保持")).unwrap();
    std::fs::write(root.join("保持").join("資料.pdf"), b"x").unwrap();

    prune_empty_dirs(&root);

    assert!(!root.join("空").exists());
    assert!(!root.join("ゴミ").exists());
    assert!(!root.join("深").exists());
    assert!(root.join("保持").join("資料.pdf").is_file());
    // Root itself is never removed.
    assert!(root.is_dir());

    std::fs::remove_dir_all(&root).ok();
}
