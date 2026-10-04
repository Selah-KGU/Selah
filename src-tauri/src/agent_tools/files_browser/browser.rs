//! In-app browser and computer-control tools for the agent.

#[path = "browser/actions.rs"]
mod actions;
#[path = "browser/computer.rs"]
mod computer;
#[path = "browser/nav.rs"]
mod nav;
#[path = "browser/read.rs"]
mod read;
#[path = "browser/support.rs"]
mod support;

pub use actions::*;
pub use computer::*;
pub use nav::*;
pub use read::*;
