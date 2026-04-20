//! Structured error type for the agent subsystem.
//!
//! Replaces raw `String` errors with a typed enum so callers can match on
//! error kind without string parsing.

use std::fmt;

#[derive(Debug)]
pub enum AgentError {
    /// AI feature is disabled or misconfigured.
    ConfigError(String),
    /// Model file missing, load failed, or inference-level fault.
    ModelError(String),
    /// Tool dispatch or execution failure.
    ToolError { tool: String, detail: String },
    /// Context window exceeded, tokenization failure.
    ContextError(String),
    /// Database persistence failure.
    DbError(String),
    /// Inference was cancelled by the user.
    Cancelled,
    /// Tokio task join error.
    TaskError(String),
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConfigError(s) => write!(f, "{}", s),
            Self::ModelError(s) => write!(f, "{}", s),
            Self::ToolError { tool, detail } => write!(f, "[{}] {}", tool, detail),
            Self::ContextError(s) => write!(f, "{}", s),
            Self::DbError(s) => write!(f, "{}", s),
            Self::Cancelled => write!(f, "推論はキャンセルされました"),
            Self::TaskError(s) => write!(f, "タスク実行エラー: {}", s),
        }
    }
}

impl std::error::Error for AgentError {}

// ── Convenience conversions ──

impl From<AgentError> for String {
    fn from(e: AgentError) -> String {
        e.to_string()
    }
}

impl AgentError {
    pub fn config(s: impl Into<String>) -> Self {
        Self::ConfigError(s.into())
    }

    pub fn model(s: impl Into<String>) -> Self {
        Self::ModelError(s.into())
    }

    pub fn tool(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::ToolError {
            tool: name.into(),
            detail: detail.into(),
        }
    }

    pub fn context(s: impl Into<String>) -> Self {
        Self::ContextError(s.into())
    }

    pub fn db(s: impl Into<String>) -> Self {
        Self::DbError(s.into())
    }

    pub fn task(e: impl fmt::Display) -> Self {
        Self::TaskError(e.to_string())
    }
}
