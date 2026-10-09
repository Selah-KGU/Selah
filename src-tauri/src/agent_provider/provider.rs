//! Resolved local or remote inference provider.

use super::*;
use crate::agent_error::AgentError;
use crate::ai::{AiConfig, ChatMessage};
#[cfg(target_os = "macos")]
use crate::local_ai;

// ─────────────────────── Provider enum ───────────────────────

/// Resolved provider ready to run inference.
pub enum AgentProvider {
    #[cfg(target_os = "macos")]
    Local {
        model_id: String,
        file_name: String,
    },
    Remote {
        config: AiConfig,
    },
}

impl AgentProvider {
    /// Resolve the correct provider from the user's current AiConfig.
    pub fn resolve() -> Result<Self, AgentError> {
        let cfg = crate::ai::load_ai_config();
        if !cfg.ai_enabled {
            return Err(AgentError::config(
                "AI機能が無効になっています。設定画面で有効にしてください。",
            ));
        }
        match cfg.provider.as_str() {
            #[cfg(target_os = "macos")]
            "local" => {
                crate::local_ai_support::ensure_supported().map_err(AgentError::config)?;
                Self::resolve_local(&cfg)
            }
            #[cfg(not(target_os = "macos"))]
            "local" => Err(AgentError::config(
                crate::local_ai_support::unsupported_message(),
            )),
            // OpenRouter and DeepSeek are OpenAI-compatible, so they route through the OpenAI path.
            "openai" | "openrouter" | "deepseek" | "gemini" => Ok(Self::Remote { config: cfg }),
            other => Err(AgentError::config(format!("不明なプロバイダー: {}", other))),
        }
    }

    #[cfg(target_os = "macos")]
    fn resolve_local(cfg: &AiConfig) -> Result<Self, AgentError> {
        let _ = cfg;
        Ok(Self::Local {
            model_id: local_ai::APPLE_INTELLIGENCE_MODEL_ID.into(),
            file_name: String::new(),
        })
    }

    /// Non-streaming inference used for Phase 1 (planning).
    /// A scoped Agent request resolves its own immutable generation identity.
    pub async fn plan(
        &self,
        messages: Vec<ChatMessage>,
        max_tokens: u32,
        temperature: f32,
        prefill: &str,
        think_budget_pct: u32,
        gen_id: &str,
    ) -> Result<String, AgentError> {
        #[cfg(not(target_os = "macos"))]
        let _ = (prefill, think_budget_pct);

        let owner = crate::agent_turn_scope::current(gen_id);
        let generation = owner.as_ref().map(|turn| turn.generation().to_owned());
        let gen_id = generation.as_deref().unwrap_or(gen_id);
        if Self::is_cancelled(gen_id) {
            return Err(AgentError::Cancelled);
        }
        let result = crate::agent_turn_scope::until_cancelled(owner.as_deref(), async {
            match self {
                #[cfg(target_os = "macos")]
                Self::Local {
                    model_id,
                    file_name,
                } => {
                    let model_id = model_id.clone();
                    let file_name = file_name.clone();
                    let prefill = prefill.to_string();
                    let gen_id = plan_gen_id(gen_id);
                    let keep_alive = owner.clone();
                    tokio::task::spawn_blocking(move || {
                        let _keep_alive = keep_alive;
                        local_ai::run_inference(local_ai::InferenceRequest {
                            model_id,
                            file_name,
                            messages,
                            sampler: local_ai::SamplerConfig::deterministic(temperature),
                            max_tokens,
                            prefill,
                            gen_id,
                            think_budget_pct,
                        })
                    })
                    .await
                    .map_err(AgentError::task)?
                    .map_err(agent_error_from_model)
                }
                Self::Remote { config } => {
                    let plan_id = plan_gen_id(gen_id);
                    clear_remote_cancel(&plan_id);
                    let result = remote_chat_completion(
                        config,
                        messages,
                        max_tokens,
                        temperature,
                        &plan_id,
                        true,
                    )
                    .await;
                    let cancelled = is_remote_cancelled(&plan_id);
                    clear_remote_cancel(&plan_id);
                    if cancelled {
                        return Err(AgentError::Cancelled);
                    }
                    result.map_err(agent_error_from_model)
                }
            }
        })
        .await;
        if owner.as_ref().is_some_and(|turn| turn.cancelled()) {
            return Err(AgentError::Cancelled);
        }
        result
    }

    /// Streaming inference used for Phase 2 (answering).
    /// `on_chunk(text, is_think)` is called for each token/chunk.
    pub async fn answer<F>(
        &self,
        messages: Vec<ChatMessage>,
        gen_id: &str,
        think_budget_pct: u32,
        on_chunk: F,
    ) -> Result<String, AgentError>
    where
        F: FnMut(&str, bool) + Send + 'static,
    {
        let owner = crate::agent_turn_scope::current(gen_id);
        let generation = owner.as_ref().map(|turn| turn.generation().to_owned());
        let gen_id = generation.as_deref().unwrap_or(gen_id);
        if Self::is_cancelled(gen_id) {
            return Err(AgentError::Cancelled);
        }
        let callback_owner = owner.clone();
        let mut on_chunk = on_chunk;
        let on_chunk = move |text: &str, is_think: bool| {
            // Streaming adapters may keep running after the async waiter is
            // dropped. Retain and check the actual request on their thread.
            if callback_owner.as_ref().is_none_or(|turn| !turn.cancelled()) {
                on_chunk(text, is_think);
            }
        };
        let result = crate::agent_turn_scope::until_cancelled(owner.as_deref(), async {
            match self {
                #[cfg(target_os = "macos")]
                Self::Local {
                    model_id,
                    file_name,
                } => {
                    let model_id = model_id.clone();
                    let file_name = file_name.clone();
                    let gen_id = gen_id.to_string();
                    let keep_alive = owner.clone();
                    tokio::task::spawn_blocking(move || {
                        let _keep_alive = keep_alive;
                        local_ai::run_inference_streaming(
                            local_ai::InferenceRequest {
                                model_id,
                                file_name,
                                messages,
                                sampler: local_ai::SamplerConfig::default(),
                                max_tokens: 0,
                                prefill: String::new(),
                                gen_id,
                                think_budget_pct,
                            },
                            on_chunk,
                        )
                    })
                    .await
                    .map_err(AgentError::task)?
                    .map_err(agent_error_from_model)
                }
                Self::Remote { config } => {
                    let gen_id = gen_id.to_string();
                    remote_stream_answer(config, messages, &gen_id, on_chunk, think_budget_pct)
                        .await
                        .map_err(|e| {
                            if e == CANCELLED_MSG {
                                AgentError::Cancelled
                            } else {
                                AgentError::model(e)
                            }
                        })
                }
            }
        })
        .await;
        if owner.as_ref().is_some_and(|turn| turn.cancelled()) {
            return Err(AgentError::Cancelled);
        }
        result
    }

    /// Cancel any ongoing inference for `gen_id`.
    /// Cancels both the answer-phase id and the synthesised plan-phase id.
    pub fn cancel(gen_id: &str) {
        if let Ok(mut set) = TURN_CANCEL.lock() {
            set.insert(gen_id.to_string());
        }
        #[cfg(target_os = "macos")]
        local_ai::cancel_inference(gen_id);
        cancel_remote(gen_id);
        let plan_id = plan_gen_id(gen_id);
        if !plan_id.is_empty() {
            #[cfg(target_os = "macos")]
            local_ai::cancel_inference(&plan_id);
            cancel_remote(&plan_id);
        }
    }

    pub fn clear_cancel(gen_id: &str) {
        if let Ok(mut set) = TURN_CANCEL.lock() {
            set.remove(gen_id);
        }
        #[cfg(target_os = "macos")]
        local_ai::clear_inference_cancel(gen_id);
        clear_remote_cancel(gen_id);
        let plan_id = plan_gen_id(gen_id);
        if !plan_id.is_empty() {
            #[cfg(target_os = "macos")]
            local_ai::clear_inference_cancel(&plan_id);
            clear_remote_cancel(&plan_id);
        }
    }

    pub fn is_cancelled(gen_id: &str) -> bool {
        if let Some(turn) = crate::agent_turn_scope::current(gen_id) {
            return turn.cancelled();
        }
        if crate::agent_turn_scope::generation_cancelled(gen_id) {
            return true;
        }
        TURN_CANCEL
            .lock()
            .map(|set| set.contains(gen_id))
            .unwrap_or(false)
    }

    /// Whether the provider honours assistant prefill (used for Phase 1 JSON).
    /// Apple Intelligence and the cloud APIs both need a full object, not a prefix.
    pub fn supports_prefill(&self) -> bool {
        let _ = self;
        false
    }

    /// Whether to attempt sending images to this provider. On-device Apple
    /// Intelligence is text-only, so never. For Remote we always TRY (we don't pre-judge any
    /// model — including DeepSeek): vision models receive the image, and if a
    /// text-only endpoint rejects the `image_url` part the request layer falls
    /// back to text-only for that one call instead of failing.
    pub fn supports_vision(&self) -> bool {
        matches!(self, Self::Remote { .. })
    }

    pub fn is_local(&self) -> bool {
        match self {
            #[cfg(target_os = "macos")]
            Self::Local { .. } => true,
            Self::Remote { .. } => false,
        }
    }
}
