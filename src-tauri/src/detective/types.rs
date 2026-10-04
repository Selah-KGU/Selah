//! All Detective domain types + AI draft (deserialize) shapes + builders.

#[path = "types/campaign.rs"]
mod campaign;
#[path = "types/case.rs"]
mod case;
#[path = "types/context.rs"]
mod context;
#[path = "types/drafts.rs"]
mod drafts;

pub use campaign::*;
pub use case::*;
pub use context::*;
pub(crate) use drafts::*;
