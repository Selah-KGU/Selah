use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

#[path = "luna_parser/course.rs"]
mod course;
#[path = "luna_parser/detail.rs"]
mod detail;
#[path = "luna_parser/inquiry.rs"]
mod inquiry;
#[path = "luna_parser/overview.rs"]
mod overview;

pub use course::*;
#[cfg(test)]
use course::{extract_quill_delta_html, extract_quill_delta_text};
pub(crate) use detail::is_blacklisted_system_notice_text;
#[allow(unused_imports)]
use detail::{classify_link, extract_named_quill_text, extract_quill_rich_html};
#[allow(unused_imports)]
pub use detail::{
    parse_luna_announcement_detail, parse_luna_detail_page, LunaAttachment, LunaDetailPage,
    LunaDetailSection,
};
#[allow(unused_imports)]
pub use inquiry::LunaInquiryPost;
pub use inquiry::{parse_luna_inquiry_detail, LunaInquiryDetail};
pub use overview::*;

/// Five content lists returned from the contents page (materials, reports, examinations, discussions, surveys)
pub type ContentsPageResult = (
    Vec<LunaContentItem>,
    Vec<LunaContentItem>,
    Vec<LunaContentItem>,
    Vec<LunaContentItem>,
    Vec<LunaContentItem>,
);

#[path = "luna_parser/helpers.rs"]
mod helpers;
#[path = "luna_parser/selectors.rs"]
mod selectors;
#[cfg(test)]
#[path = "luna_parser/tests.rs"]
mod tests;

use helpers::*;
use selectors::*;
