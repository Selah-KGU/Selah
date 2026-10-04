use crate::client;
use crate::config;
use crate::luna_client;
use crate::luna_parser;
use crate::LunaState;
use chrono::TimeZone;
use std::sync::LazyLock;
use tauri::State;

#[path = "luna_commands/attendance.rs"]
mod attendance;
#[path = "luna_commands/course.rs"]
mod course;
#[path = "luna_commands/detail.rs"]
mod detail;
#[path = "luna_commands/discussion.rs"]
mod discussion;
#[path = "luna_commands/downloads.rs"]
mod downloads;
#[path = "luna_commands/forum.rs"]
mod forum;
#[path = "luna_commands/html_form.rs"]
mod html_form;
#[path = "luna_commands/http.rs"]
mod http;
#[path = "luna_commands/inquiry.rs"]
mod inquiry;
#[path = "luna_commands/navigation.rs"]
mod navigation;
#[path = "luna_commands/report.rs"]
mod report;
#[path = "luna_commands/session.rs"]
mod session;
#[path = "luna_commands/support.rs"]
mod support;
#[path = "luna_commands/survey.rs"]
mod survey;
#[cfg(test)]
#[path = "luna_commands/tests.rs"]
mod tests;

pub use attendance::*;
pub(in crate::luna_commands) use detail::looks_like_luna_home_redirect;
pub use detail::*;
pub use discussion::*;
pub use downloads::*;
pub(in crate::luna_commands) use html_form::*;
pub use navigation::*;
pub use report::*;
pub use survey::*;
#[cfg(test)]
pub(in crate::luna_commands) use survey::{
    detect_survey_submit_error, normalize_luna_relative_path, survey_answer_field_name,
    survey_answer_payload, survey_answer_values,
};

pub use course::*;
pub use forum::LunaDiscussionUploadFile;
use forum::{
    append_forum_file_fields, decode_forum_upload_files, has_forum_file_upload_support,
    upload_forum_files,
};
use http::*;
pub use inquiry::*;
pub use session::*;
use support::{html_escape, is_safe_param, luna_fetch_cached};

const LUNA_DETAIL_CACHE_VERSION: &str = "v3";
const LUNA_REPORT_DETAIL_CACHE_VERSION: &str = "v2";
const LUNA_ANNOUNCEMENT_CACHE_VERSION: &str = "v3";
const LUNA_DETAIL_RETRY_ATTEMPTS: usize = 3;
const LUNA_FORUM_FILE_MAX_BYTES: usize = 100 * 1024 * 1024;

// ── Cached selectors (compiled once, reused across all calls) ──
macro_rules! sel {
    ($name:ident, $s:expr) => {
        static $name: LazyLock<scraper::Selector> =
            LazyLock::new(|| scraper::Selector::parse($s).expect(concat!("bad selector: ", $s)));
    };
}
sel!(SEL_META_REFRESH, "meta[http-equiv='refresh']");
sel!(SEL_IFRAME_SRC, "iframe[src]");
sel!(SEL_SCRIPT, "script");
sel!(SEL_A_HREF, "a[href]");
sel!(SEL_BODY, "body");
sel!(SEL_FORM, "form");
sel!(SEL_REPORT_FORM, "form#reportSubmissionForm");
sel!(SEL_HIDDEN_INPUT, "input[type='hidden']");
sel!(SEL_UPDATE_INFO_LIST, ".update-info-list");
sel!(SEL_DETAIL_VERT, ".contents-detail.contents-vertical");
sel!(
    SEL_DETAIL_TITLE,
    "#osiraseTitle, .block-title-txt, .contents-title-txt"
);
sel!(
    SEL_THREAD_POST_MARKER,
    ".thread-post-area, #threadPostListArea, .postContentsText"
);
