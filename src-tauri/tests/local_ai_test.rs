#![cfg(target_os = "macos")]

use app_lib::ai::ChatMessage;
use app_lib::local_ai::{
    run_inference, run_inference_streaming, InferenceRequest, SamplerConfig, CANCELLED_MSG,
};
use app_lib::local_ai_support;

#[test]
fn apple_intelligence_answers_when_available() {
    let support = local_ai_support::current();
    if !support.supported {
        eprintln!("skip apple intelligence smoke: {}", support.reason);
        return;
    }
    let text = run_inference(InferenceRequest {
        model_id: local_ai_support::APPLE_INTELLIGENCE_MODEL_ID.into(),
        file_name: String::new(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: "Reply with exactly OK.".into(),
            images: Vec::new(),
        }],
        sampler: SamplerConfig::deterministic(0.0),
        max_tokens: 16,
        prefill: String::new(),
        gen_id: "apple-intelligence-smoke".into(),
        think_budget_pct: 0,
    })
    .expect("apple intelligence request");
    assert!(!text.trim().is_empty(), "empty response");
    assert_ne!(text, CANCELLED_MSG);

    let mut streamed = String::new();
    let streamed_full = run_inference_streaming(
        InferenceRequest {
            model_id: local_ai_support::APPLE_INTELLIGENCE_MODEL_ID.into(),
            file_name: String::new(),
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: "Reply with exactly OK.".into(),
                    images: Vec::new(),
                },
                ChatMessage {
                    role: "user".into(),
                    content: "OK".into(),
                    images: Vec::new(),
                },
            ],
            sampler: SamplerConfig::deterministic(0.0),
            max_tokens: 16,
            prefill: String::new(),
            gen_id: "apple-intelligence-stream".into(),
            think_budget_pct: 0,
        },
        |chunk, is_think| {
            assert!(!is_think);
            streamed.push_str(chunk);
        },
    )
    .expect("streaming apple intelligence request");
    assert_eq!(streamed, streamed_full);
    assert!(!streamed_full.trim().is_empty());
}
