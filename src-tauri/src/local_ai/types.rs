use crate::ai::ChatMessage;

#[derive(Debug, Clone)]
pub struct SamplerConfig {
    pub temperature: f32,
    pub top_k: i32,
    pub top_p: f32,
    pub presence_penalty: f32,
    pub penalty_last_n: i32,
}

impl Default for SamplerConfig {
    fn default() -> Self {
        Self {
            temperature: 0.7,
            top_k: 20,
            top_p: 0.95,
            presence_penalty: 1.5,
            penalty_last_n: 256,
        }
    }
}

impl SamplerConfig {
    pub fn deterministic(temperature: f32) -> Self {
        Self {
            temperature,
            ..Default::default()
        }
    }
}

/// Parameters kept so existing callers do not need a wide rewrite.
/// `file_name` and `think_budget_pct` are ignored: Apple Intelligence has one
/// system model and does not emit `<think>` tags.
pub struct InferenceRequest {
    pub model_id: String,
    pub file_name: String,
    pub messages: Vec<ChatMessage>,
    pub sampler: SamplerConfig,
    pub max_tokens: u32,
    pub prefill: String,
    pub gen_id: String,
    pub think_budget_pct: u32,
}

#[derive(Debug, Clone)]
pub struct BridgeStatus {
    pub supported: bool,
    pub reason: String,
    pub permanent: bool,
    pub model: String,
    pub context_size: i64,
}
