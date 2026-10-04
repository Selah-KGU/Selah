//! Every AI prompt builder (outline / draft / editor / bible / finale) + section renderers.

#[path = "prompts/campaign.rs"]
mod campaign;
#[path = "prompts/case.rs"]
mod case_prompt;
#[path = "prompts/outline.rs"]
mod outline;
#[path = "prompts/sections.rs"]
mod sections;

pub(crate) use campaign::*;
pub(crate) use case_prompt::*;
pub(crate) use outline::*;
