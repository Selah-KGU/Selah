#[allow(unused_imports)]
use super::*;

#[path = "academic/course_match.rs"]
mod course_match;
#[path = "academic/notices.rs"]
mod notices;
#[path = "academic/schedule.rs"]
mod schedule;

pub(super) use course_match::{get_course_context, normalize_text, search_courses};
pub(super) use notices::{
    get_course_detail, get_notification_detail, list_luna_announcements, list_luna_todos,
    list_recent_notifications,
};
pub(super) use schedule::{list_today_classes, list_week_classes};
