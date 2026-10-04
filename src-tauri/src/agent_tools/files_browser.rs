use super::*;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

#[path = "files_browser/browser.rs"]
mod browser;
#[path = "files_browser/course_material.rs"]
mod course_material;
#[path = "files_browser/local_files.rs"]
mod local_files;
#[path = "files_browser/luna_attachment.rs"]
mod luna_attachment;
#[path = "files_browser/luna_detail.rs"]
mod luna_detail;
#[path = "files_browser/office_text.rs"]
mod office_text;
#[path = "files_browser/text.rs"]
mod text;
#[path = "files_browser/url_download.rs"]
mod url_download;

#[cfg(test)]
#[path = "files_browser/tests.rs"]
mod tests;

pub(super) use browser::*;
pub(super) use course_material::*;
pub(super) use local_files::*;
pub(super) use luna_attachment::*;
pub(super) use luna_detail::*;
pub(super) use office_text::*;
pub(super) use text::*;
pub(super) use url_download::*;
