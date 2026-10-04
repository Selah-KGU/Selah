//! KWIC portal commands.

#[path = "kwic_commands/detail.rs"]
mod detail;
#[path = "kwic_commands/home.rs"]
mod home;
#[path = "kwic_commands/http.rs"]
mod http;
#[path = "kwic_commands/open.rs"]
mod open;
#[path = "kwic_commands/parse.rs"]
mod parse;
#[path = "kwic_commands/subportal.rs"]
mod subportal;
#[path = "kwic_commands/types.rs"]
mod types;

pub use detail::*;
pub use home::*;
pub(in crate::kwic_commands) use http::*;
pub use open::*;
#[cfg(test)]
pub(in crate::kwic_commands) use parse::parse_information_list_items;
pub(in crate::kwic_commands) use parse::{
    compact_inline_images, extract_csrf_token, merge_information_list_sections,
    parse_cabinet_reference, parse_detail_html, parse_portal_home, parse_subportal,
};
pub use subportal::*;
pub use types::*;

#[cfg(test)]
#[path = "kwic_commands/tests.rs"]
mod tests;
