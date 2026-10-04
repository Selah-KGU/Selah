use super::*;

#[test]
fn scan_attributes_theme_subfolders_to_their_course() {
    // base/<course>/<theme>/<file> — the file belongs to <course>, not <theme>.
    let base = std::env::temp_dir().join(format!("scan-course-{}", uuid::Uuid::new_v4()));
    let course = base.join("政治学基礎");
    let theme = course.join("第01回");
    std::fs::create_dir_all(&theme).unwrap();
    let flat = course.join("シラバス.pdf");
    let nested = theme.join("資料.pdf");
    std::fs::write(&flat, b"a").unwrap();
    std::fs::write(&nested, b"b").unwrap();

    let known = std::collections::HashSet::new();
    let mut discovered = Vec::new();
    scan_dir_recursive(&base, "", &known, &mut discovered, 0);

    let by_name = |n: &str| {
        discovered
            .iter()
            .find(|r| r.filename == n)
            .map(|r| r.course_name.clone())
    };
    // Both files attribute to the course, regardless of theme nesting.
    assert_eq!(by_name("シラバス.pdf").as_deref(), Some("政治学基礎"));
    assert_eq!(by_name("資料.pdf").as_deref(), Some("政治学基礎"));

    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn simplify_strips_faculty_path_and_course_code() {
    // The faculty-path + 8-digit course-code prefix must not leak into the
    // course label (e.g. notification titles).
    let s = simplify_course_name("国際学部/International Studies 34001001 キリスト教学A　１");
    assert!(!s.contains("34001001"), "got: {s}");
    assert!(!s.contains("International Studies"), "got: {s}");
    assert!(s.starts_with("キリスト教学A"), "got: {s}");
}

#[test]
fn theme_subfolder_extracts_in_course_path() {
    let base = std::path::Path::new("/Users/x/Selah");
    // base/<course>/<theme>/<file> → theme
    assert_eq!(
        theme_subfolder("/Users/x/Selah/政治学基礎/第01回/資料.pdf", base),
        "第01回"
    );
    // base/<course>/<file> → no theme (course root)
    assert_eq!(
        theme_subfolder("/Users/x/Selah/政治学基礎/資料.pdf", base),
        ""
    );
    // outside base → empty
    assert_eq!(theme_subfolder("/elsewhere/資料.pdf", base), "");
}
