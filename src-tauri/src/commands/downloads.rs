use super::load_download_config;

#[path = "downloads/duplicates.rs"]
mod duplicates;
#[path = "downloads/history.rs"]
mod history;
#[path = "downloads/history_store.rs"]
mod history_store;
#[path = "downloads/markdown.rs"]
mod markdown;
#[path = "downloads/migrate.rs"]
mod migrate;
#[path = "downloads/open.rs"]
mod open;
#[path = "downloads/paths.rs"]
mod paths;
#[path = "downloads/preview.rs"]
mod preview;
#[path = "downloads/scan.rs"]
mod scan;

#[cfg(test)]
#[path = "downloads/tests.rs"]
mod tests;

pub use duplicates::*;
pub use history::*;
pub use markdown::*;
pub use migrate::*;
pub use open::*;
pub use paths::*;
pub use preview::*;
pub use scan::*;

pub(in crate::commands::downloads) use history::{annotate_records, download_history_store};
pub(in crate::commands::downloads) use paths::{
    download_base, sanitize_path_component, theme_subfolder, validate_downloads_path,
};
pub(in crate::commands::downloads) use preview::is_markdown_ext;
#[cfg(test)]
pub(in crate::commands::downloads) use scan::scan_dir_recursive;
