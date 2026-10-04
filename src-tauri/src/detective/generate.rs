//! The three-pass AI generation pipeline + campaign bible + knowledge extraction + AI input building.

#[path = "generate/campaign.rs"]
mod campaign;
#[path = "generate/chapter.rs"]
mod chapter;
#[path = "generate/input.rs"]
mod input;
#[path = "generate/knowledge.rs"]
mod knowledge;

pub(crate) use campaign::*;
pub(crate) use chapter::*;
pub(crate) use input::*;
pub(crate) use knowledge::*;
