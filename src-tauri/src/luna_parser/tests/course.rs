use super::super::*;

#[test]
#[ignore] // requires local HTML dump file
fn test_parse_detail_attachments() {
    let html = std::fs::read_to_string("/tmp/luna_detail_lms_course_report_submission_idnumber=2026510010040201_reportId=225148.html")
        .expect("HTML dump file not found");
    let result = parse_luna_detail_page(&html);
    println!("Title: {}", result.title);
    println!("Sections: {}", result.sections.len());
    println!("Meta: {:?}", result.meta);
    println!("Attachments: {:?}", result.attachments);
    assert!(
        !result.attachments.is_empty(),
        "Should have at least one attachment"
    );
    let att = &result.attachments[0];
    assert!(!att.name.is_empty(), "Attachment name should not be empty");
    assert!(!att.url.is_empty(), "Attachment URL should not be empty");
    println!("PASS: name='{}', url='{}'", att.name, att.url);
}

#[test]
#[ignore] // requires local HTML dump file
fn test_parse_course_page() {
    let html = std::fs::read_to_string(
        "/tmp/luna_detail_lms_course_idnumber=2026510010040201#information.html",
    )
    .expect("Course HTML dump file not found");
    let result = parse_luna_course_contents(&html, "2026510010040201");
    println!("Course name: {}", result.course_name);
    println!("Semester: {}", result.semester);
    println!("Teachers: {}", result.teachers);
    println!("TA: {}", result.ta_info);
    println!("LA: {}", result.la_info);
    println!("Syllabus: {}", result.syllabus_url);
    println!("Menus: {}", result.menus.len());
    for m in &result.menus {
        println!("  {} ({})", m.name, m.module_type);
    }
    println!("Announcements: {}", result.announcements.len());
    for a in &result.announcements {
        println!(
            "  {} [{}~{}] new={}",
            a.title, a.start_date, a.end_date, a.is_new
        );
    }
    println!("Online tools: {}", result.online_tools.len());
    for t in &result.online_tools {
        println!("  {} -> {}", t.name, t.url);
    }
    assert!(
        !result.course_name.is_empty(),
        "Course name should not be empty"
    );
    assert!(!result.menus.is_empty(), "Should have menus");
}

#[test]
#[ignore] // requires local HTML dump file
fn test_parse_contents_page() {
    let html = std::fs::read_to_string("/tmp/luna_contents_2026510010040201.html")
        .expect("Contents HTML dump file not found");
    let (materials, reports, examinations, discussions, _surveys) = parse_luna_contents_page(&html);
    println!("Materials: {}", materials.len());
    for m in &materials {
        println!("  {} | {} | {}", m.title, m.period, m.status);
    }
    println!("Reports: {}", reports.len());
    for r in &reports {
        println!("  {} | {} | {}", r.title, r.period, r.status);
    }
    println!("Examinations: {}", examinations.len());
    for e in &examinations {
        println!("  {} | {} | {}", e.title, e.period, e.status);
    }
    println!("Discussions: {}", discussions.len());
    for d in &discussions {
        println!("  {} | {} | {}", d.title, d.period, d.status);
    }
    assert!(
        !materials.is_empty()
            || !reports.is_empty()
            || !examinations.is_empty()
            || !discussions.is_empty(),
        "Should have at least some content items"
    );
}
