//! Provider abstraction for the Selah agent.
//!
//! Two concrete variants:
//!   - **Local (macOS)**: runs Apple's on-device Foundation Model when Apple
//!     Intelligence is available. Windows stays cloud-only.
//!   - **Remote**: calls any OpenAI-compatible or Gemini API (SSE streaming).
//!
//! The agent pipeline (`agent.rs`) talks only to the `AgentProvider` enum,
//! so switching between local and remote is transparent.

mod cancel;
mod error;
mod function_call;
mod gemini;
mod http;
mod messages;
mod openai;
mod provider;
mod stream;

#[cfg(test)]
mod tests;

pub use cancel::cancel_remote;
pub use provider::AgentProvider;

pub(in crate::agent_provider) use cancel::{
    clear_remote_cancel, is_remote_cancelled, plan_gen_id, CANCELLED_MSG, TURN_CANCEL,
};
pub(in crate::agent_provider) use error::*;
pub(in crate::agent_provider) use function_call::*;
pub(in crate::agent_provider) use gemini::*;
pub(in crate::agent_provider) use http::*;
pub(in crate::agent_provider) use messages::*;
pub(in crate::agent_provider) use openai::*;
pub(in crate::agent_provider) use stream::*;
