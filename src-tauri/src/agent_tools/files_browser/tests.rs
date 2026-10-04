use super::*;
use std::io::Write;

fn material_file(display_name: &str, file_name: &str) -> crate::luna_parser::LunaMaterialFile {
    crate::luna_parser::LunaMaterialFile {
        display_name: display_name.to_string(),
        file_name: file_name.to_string(),
        object_name: "object".into(),
        resource_id: "resource".into(),
        material_id: "material".into(),
        file_type: "0".into(),
        end_date: String::new(),
        scan_status: "1".into(),
        link_type: "file".into(),
        external_url: String::new(),
    }
}

fn course_contents(
    files: Vec<crate::luna_parser::LunaMaterialFile>,
) -> crate::luna_parser::LunaCourseContents {
    crate::luna_parser::LunaCourseContents {
        course_name: "政治学基礎 ２".into(),
        semester: String::new(),
        teachers: String::new(),
        ta_info: String::new(),
        la_info: String::new(),
        syllabus_url: String::new(),
        grade_url: String::new(),
        menus: Vec::new(),
        announcements: Vec::new(),
        online_tools: Vec::new(),
        materials: vec![crate::luna_parser::LunaContentItem {
            title: "中間試験資料".into(),
            url: String::new(),
            period: String::new(),
            status: String::new(),
            item_type: "material".into(),
            description: String::new(),
            files,
        }],
        reports: Vec::new(),
        examinations: Vec::new(),
        discussions: Vec::new(),
        surveys: Vec::new(),
        attendances: Vec::new(),
    }
}

fn course_item(title: &str, url: &str, item_type: &str) -> crate::luna_parser::LunaContentItem {
    crate::luna_parser::LunaContentItem {
        title: title.to_string(),
        url: url.to_string(),
        period: String::new(),
        status: String::new(),
        item_type: item_type.to_string(),
        description: String::new(),
        files: Vec::new(),
    }
}

fn office_fixture(extension: &str, entries: &[(&str, &str)]) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "selah-office-fixture-{}.{}",
        uuid::Uuid::new_v4(),
        extension
    ));
    let file = File::create(&path).expect("create fixture");
    let mut archive = zip::ZipWriter::new(file);
    for (name, body) in entries {
        archive
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .expect("start fixture file");
        archive
            .write_all(body.as_bytes())
            .expect("write fixture file");
    }
    archive.finish().expect("finish fixture");
    path
}

#[test]
fn current_course_activity_sources_use_current_contents_only_and_dedup() {
    let mut contents = course_contents(Vec::new());
    contents.announcements = vec![
        crate::luna_parser::LunaCourseAnnouncement {
            title: "LUNA告知".into(),
            info_id: "A1".into(),
            start_date: String::new(),
            end_date: String::new(),
            is_new: true,
        },
        crate::luna_parser::LunaCourseAnnouncement {
            title: "重複告知".into(),
            info_id: "A1".into(),
            start_date: String::new(),
            end_date: String::new(),
            is_new: false,
        },
    ];
    contents.reports = vec![
        course_item(
            "提出課題",
            &format!(
                "{}{}",
                crate::config::LUNA_BASE,
                "/lms/course/report/detail?id=R1"
            ),
            "report",
        ),
        course_item("空URL課題", "", "report"),
    ];

    let sources = current_course_activity_sources("6072", &contents, &["announcement", "report"]);
    assert_eq!(
        sources,
        vec![
            (
                "announcement".to_string(),
                "LUNA告知".to_string(),
                "/lms/coursetop/information/listdetail?idnumber=6072&informationId=A1".to_string(),
            ),
            (
                "report".to_string(),
                "提出課題".to_string(),
                "/lms/course/report/detail?id=R1".to_string(),
            ),
        ]
    );

    let report_only = current_course_activity_sources("6072", &contents, &["report"]);
    assert_eq!(
        report_only,
        vec![(
            "report".to_string(),
            "提出課題".to_string(),
            "/lms/course/report/detail?id=R1".to_string(),
        )]
    );
}

#[test]
fn cached_activity_detail_requires_same_list_fingerprint_and_fresh_ttl() {
    let cached = crate::agent_tools::ReusableActivityDetail {
        list_fingerprint: "list-v1".into(),
        source_fingerprint: "detail-v1".into(),
        checked_at: 100,
    };

    assert!(cached_activity_detail_is_fresh(
        &cached, "list-v1", 250, 300
    ));
    assert!(!cached_activity_detail_is_fresh(
        &cached, "list-v2", 250, 300
    ));
    assert!(!cached_activity_detail_is_fresh(
        &cached, "list-v1", 500, 300
    ));
    assert!(!cached_activity_detail_is_fresh(&cached, "list-v1", 250, 0));
    assert!(cached_activity_detail_matches_source(&cached, "list-v1"));
    assert!(!cached_activity_detail_matches_source(&cached, "list-v2"));
}

#[test]
fn matches_course_material_by_file_name() {
    let contents = course_contents(vec![material_file(
        "試験要項",
        "2026年度春中間試験の実施要項.pdf",
    )]);
    let matched = match_material_file(&contents, "2026年度春中間試験の実施要項.pdf")
        .expect("material should match");
    assert_eq!(
        effective_material_filename(&matched.file),
        "2026年度春中間試験の実施要項.pdf"
    );
}

#[test]
fn matches_course_material_by_display_name_when_file_name_is_empty() {
    let contents = course_contents(vec![material_file("2026年度春中間試験の実施要項.pdf", "")]);
    let matched = match_material_file(&contents, "2026年度春中間試験の実施要項")
        .expect("display name should match");
    assert_eq!(
        effective_material_filename(&matched.file),
        "2026年度春中間試験の実施要項.pdf"
    );
}

#[test]
fn extracts_pptx_slide_text() {
    let path = office_fixture(
        "pptx",
        &[(
            "ppt/slides/slide1.xml",
            "<p:sld><a:p><a:r><a:t>印刷して持参</a:t></a:r></a:p></p:sld>",
        )],
    );
    let text = read_supported_download_file(&path).expect("extract pptx");
    let _ = std::fs::remove_file(path);
    assert!(text.contains("印刷して持参"));
}

#[test]
fn extracts_xlsx_shared_string_text() {
    let path = office_fixture(
        "xlsx",
        &[(
            "xl/sharedStrings.xml",
            "<sst><si><t>学籍番号 12345678 座席 A-12</t></si></sst>",
        )],
    );
    let text = read_supported_download_file(&path).expect("extract xlsx");
    let _ = std::fs::remove_file(path);
    assert!(text.contains("座席 A-12"));
}
