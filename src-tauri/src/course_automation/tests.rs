use super::*;

fn luna_todo(
    course_name: &str,
    content_name: &str,
    deadline: &str,
) -> crate::luna_parser::LunaTodoItem {
    crate::luna_parser::LunaTodoItem {
        course_name: course_name.into(),
        content_type: "課題".into(),
        content_name: content_name.into(),
        url: "/lms/todo".into(),
        deadline: deadline.into(),
        status: "未提出".into(),
        feedback: String::new(),
    }
}

fn existing_todo(title: &str) -> context::ExistingCourseTodo {
    context::ExistingCourseTodo {
        title: title.into(),
        content_type: "課題".into(),
        deadline: "2026-07-03".into(),
        status: "未提出".into(),
        source: "LUNA".into(),
    }
}

#[path = "tests/delta.rs"]
mod delta;
#[path = "tests/downloads.rs"]
mod downloads;
#[path = "tests/ledger.rs"]
mod ledger;
#[path = "tests/run_status.rs"]
mod run_status;
