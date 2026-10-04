//! Local-only agent loop (Selah persona).
//!
//! Two-phase design:
//!   Phase 1 — Planning: asks the model to pick tools (JSON, non-streaming).
//!   Phase 2 — Answering: streams the final reply with persona + tool results.
//!
//! The on-device Apple model is unreliable at multi-turn ReAct, so we constrain
//! it to a single planning step per turn.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use tauri::{AppHandle, Emitter, Manager};

use crate::agent_error::AgentError;
use crate::agent_prompts;
use crate::agent_provider::AgentProvider;
use crate::agent_pseudo_call::{
    find_start as find_pseudo_tool_call_start, has_any as has_any_pseudo_tool_call,
    parse_any_raw as parse_any_raw_tool_call, parse_leading as parse_visible_tool_call,
    RawToolCall, ToolCall,
};
use crate::agent_text;
use crate::agent_tools;
use crate::ai::{ChatMessage, ImagePart};
use crate::db::Database;

mod answer;
mod browser_click;
mod config;
mod execute;
mod heuristic;
mod plan;
mod plan_finalize;
mod plan_messages;
mod plan_summary;
mod skip_tools;
mod stream;
mod text;
mod turn;
pub(in crate::agent) use answer::*;
pub(in crate::agent) use browser_click::*;
pub(in crate::agent) use config::*;
pub(in crate::agent) use execute::*;
pub(in crate::agent) use heuristic::*;
pub(in crate::agent) use plan::*;
pub(in crate::agent) use plan_finalize::*;
pub(in crate::agent) use plan_messages::*;
pub(in crate::agent) use plan_summary::*;
pub(in crate::agent) use skip_tools::*;
pub(in crate::agent) use stream::*;
pub(in crate::agent) use text::*;
pub use turn::*;

#[cfg(test)]
mod tests;
