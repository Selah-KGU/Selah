//! On-device Apple Intelligence via the Swift Foundation Models bridge.
//!
//! The bridge is a dylib built by `build.rs` and loaded only when this process
//! actually runs inference. The app binary itself stays free of a hard
//! FoundationModels / Swift runtime dependency so macOS 11–25 can still launch.

#![cfg_attr(not(target_os = "macos"), allow(dead_code, unused_imports))]

use crate::local_ai_support;

pub const APPLE_INTELLIGENCE_MODEL_ID: &str = local_ai_support::APPLE_INTELLIGENCE_MODEL_ID;
pub const CANCELLED_MSG: &str = "推論はキャンセルされました";

/// On-device Foundation Models window. Instructions, prompt, and the reply share it.
/// This replaces the old llama N_CTX = 65536 cap, which Apple Intelligence does not inherit.
pub const APPLE_CONTEXT_WINDOW_TOKENS: usize = 4096;
pub const APPLE_CONTEXT_OVERHEAD_TOKENS: usize = 160;
/// Room kept for a reply so generation can stop inside the window instead of throwing.
pub const APPLE_RESPONSE_RESERVE_TOKENS: usize = 768;
pub const APPLE_PROMPT_TOKEN_BUDGET: usize =
    APPLE_CONTEXT_WINDOW_TOKENS - APPLE_CONTEXT_OVERHEAD_TOKENS - APPLE_RESPONSE_RESERVE_TOKENS;
/// Plan output cap. Answer calls may use more when the prompt leaves more room.
pub const APPLE_MAX_RESPONSE_TOKENS: u32 = APPLE_RESPONSE_RESERVE_TOKENS as u32;
/// Extra reply room when the caller must return one JSON object.
pub const APPLE_JSON_RESPONSE_RESERVE_TOKENS: usize = 1280;

#[path = "local_ai/bridge.rs"]
mod bridge;
#[path = "local_ai/budget.rs"]
mod budget;
#[path = "local_ai/types.rs"]
mod types;

pub use bridge::unload_model;
#[cfg(target_os = "macos")]
pub use bridge::{
    cancel_inference, clear_inference_cancel, query_availability, run_inference,
    run_inference_streaming,
};
pub(crate) use budget::{compact_json_value, estimate_apple_tokens, trim_apple_text};
pub use types::*;
