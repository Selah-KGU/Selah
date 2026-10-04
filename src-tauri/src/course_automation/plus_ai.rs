//! SenseA requests to the configured agent provider.
//!
//! One document, one course summary, and one summary batch all go through
//! the same JSON request helper. Status persistence stays in the parent.

use super::*;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct DocumentAgentOutput {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    findings: Vec<String>,
    #[serde(default)]
    seat_evidence: Vec<String>,
    #[serde(default)]
    print_instruction: String,
    #[serde(default)]
    trigger_decision: String,
    #[serde(default)]
    observation_context: String,
}

pub(super) async fn analyze_document_with_agent(
    luna_id: &str,
    course_name: &str,
    document: &AnalysisDocument,
    student: &Value,
) -> Result<(DocumentAnalysis, AiUsageEstimate), String> {
    let provider = AgentProvider::resolve().map_err(|error| error.to_string())?;
    let input = context::IndividualInput {
        course_id: luna_id,
        course_name,
        student,
        document: document.into(),
    };
    let label = format!("「{}」の個別まとめ", document_label(document));
    let response: PlusJsonResponse<DocumentAgentOutput> = request_plus_json(
        &provider,
        context::INDIVIDUAL_SYSTEM_PROMPT,
        serde_json::to_string(&input).map_err(|error| error.to_string())?,
        document.images.clone(),
        PLUS_DOCUMENT_MAX_TOKENS,
        10,
        &format!("course-automation-{}-{}", luna_id, document_id(document)),
        &label,
    )
    .await?;
    let output = normalize_document_agent_output(response.value);
    Ok((
        DocumentAnalysis {
            id: document_id(document),
            fingerprint: document_fingerprint(document)?,
            source_fingerprint: document.source_fingerprint.clone(),
            kind: document.kind.clone(),
            title: document.title.clone(),
            filename: document.filename.clone(),
            path: document.path.clone(),
            status: "done".into(),
            content: String::new(),
            summary: output.summary,
            findings: output.findings,
            seat_evidence: output.seat_evidence,
            print_instruction: output.print_instruction,
            trigger_decision: normalize_trigger_decision(&output.trigger_decision),
            observation_context: output.observation_context,
            error: String::new(),
        },
        response.usage,
    ))
}

pub(super) async fn summarize_with_agent(
    luna_id: &str,
    course_name: &str,
    new_or_changed_documents: &[DocumentAnalysis],
    student: &Value,
    existing_course_todos: &[context::ExistingCourseTodo],
    previous: &AgentCourseAnalysis,
) -> Result<AgentCourseAnalysis, String> {
    let provider = AgentProvider::resolve().map_err(|error| error.to_string())?;
    let configured_max_tokens = crate::ai::load_ai_config().max_tokens;
    let mut working_memory = previous.clone();
    for (batch_index, batch) in context::summary_batches(new_or_changed_documents)
        .into_iter()
        .enumerate()
    {
        let source_events = Vec::new();
        let (next_memory, _usage) = summarize_batch_with_agent(
            &provider,
            luna_id,
            course_name,
            &batch,
            &source_events,
            student,
            &working_memory,
            existing_course_todos,
            configured_max_tokens,
            batch_index,
        )
        .await?;
        working_memory = next_memory;
    }
    Ok(working_memory)
}

pub(super) async fn summarize_batch_with_agent(
    provider: &AgentProvider,
    luna_id: &str,
    course_name: &str,
    batch: &[&DocumentAnalysis],
    source_events: &[&SourceEvent],
    student: &Value,
    working_memory: &AgentCourseAnalysis,
    existing_course_todos: &[context::ExistingCourseTodo],
    configured_max_tokens: u32,
    batch_index: usize,
) -> Result<(AgentCourseAnalysis, AiUsageEstimate), String> {
    let prompt_memory = context::prompt_course_analysis(working_memory);
    let input = context::SummaryInput {
        current_local_time: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
        course_id: luna_id,
        course_name,
        student,
        existing_course_todos,
        previous_course_analysis: &prompt_memory,
        new_or_changed_documents: batch
            .iter()
            .copied()
            .map(context::CompactAnalysis::from)
            .collect(),
        source_events: source_events
            .iter()
            .copied()
            .map(context::CompactSourceEvent::from)
            .collect(),
    };
    let label = format!("最終まとめ batch {}", batch_index + 1);
    let gen_id = summary_generation_id(
        luna_id,
        batch_index,
        batch,
        source_events,
        &prompt_memory,
        existing_course_todos,
    );
    let response: PlusJsonResponse<AgentCourseAnalysis> = request_plus_json(
        provider,
        context::SUMMARY_SYSTEM_PROMPT,
        serde_json::to_string(&input).map_err(|error| error.to_string())?,
        Vec::new(),
        configured_max_tokens,
        20,
        &gen_id,
        &label,
    )
    .await?;
    Ok((
        normalize_course_analysis(
            response.value,
            &working_memory.archived_context,
            &chrono::Local::now().format("%Y-%m-%d").to_string(),
        ),
        response.usage,
    ))
}

pub(super) fn summary_generation_id(
    luna_id: &str,
    batch_index: usize,
    batch: &[&DocumentAnalysis],
    source_events: &[&SourceEvent],
    working_memory: &AgentCourseAnalysis,
    existing_course_todos: &[context::ExistingCourseTodo],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"course-automation-summary");
    hasher.update([0]);
    hasher.update(luna_id.as_bytes());
    hasher.update([0]);
    hasher.update(batch_index.to_string().as_bytes());
    hasher.update([0]);
    for analysis in batch {
        hasher.update(b"document");
        hasher.update([0]);
        hasher.update(analysis.id.as_bytes());
        hasher.update([0]);
        hasher.update(analysis.fingerprint.as_bytes());
        hasher.update([0]);
    }
    for event in source_events {
        hasher.update(b"source-event");
        hasher.update([0]);
        hasher.update(event.id.as_bytes());
        hasher.update([0]);
    }
    if let Ok(memory_hash) = sha256_json(working_memory) {
        hasher.update(b"memory");
        hasher.update([0]);
        hasher.update(memory_hash.as_bytes());
    }
    if let Ok(todo_hash) = sha256_json(existing_course_todos) {
        hasher.update(b"existing-course-todos");
        hasher.update([0]);
        hasher.update(todo_hash.as_bytes());
    }
    let digest = format!("{:x}", hasher.finalize());
    format!(
        "course-automation-{}-summary-{}-{}",
        luna_id,
        batch_index,
        &digest[..16]
    )
}

pub(super) struct PlusJsonResponse<T> {
    pub(super) value: T,
    pub(super) usage: AiUsageEstimate,
}

pub(super) async fn request_plus_json<T: DeserializeOwned>(
    provider: &AgentProvider,
    instructions: &str,
    input: String,
    images: Vec<crate::ai::ImagePart>,
    max_tokens: u32,
    think_budget_pct: u32,
    gen_id: &str,
    label: &str,
) -> Result<PlusJsonResponse<T>, String> {
    context::log_request_size(label, instructions, &input);
    let prompt_tokens = context::estimate_tokens(instructions) + context::estimate_tokens(&input);
    let mut last_error = String::new();
    for attempt in 1..=PLUS_AI_ATTEMPTS {
        let request = provider.plan(
            vec![
                ChatMessage {
                    role: "system".into(),
                    content: instructions.to_string(),
                    images: Vec::new(),
                },
                ChatMessage {
                    role: "user".into(),
                    content: input.clone(),
                    images: images.clone(),
                },
            ],
            max_tokens,
            0.1,
            "",
            think_budget_pct,
            gen_id,
        );
        let response =
            match tokio::time::timeout(Duration::from_secs(PLUS_AI_TIMEOUT_SECS), request).await {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => {
                    last_error = error.to_string();
                    log::warn!(
                        "[course_automation] {} AI attempt {}/{} failed: {}",
                        label,
                        attempt,
                        PLUS_AI_ATTEMPTS,
                        last_error
                    );
                    if !plus_error_allows_retry(&last_error) {
                        return Err(last_error);
                    }
                    if attempt < PLUS_AI_ATTEMPTS {
                        tokio::time::sleep(Duration::from_millis(750)).await;
                    }
                    continue;
                }
                Err(_) => {
                    last_error = format!(
                        "AI リクエストが {} 秒でタイムアウトしました",
                        PLUS_AI_TIMEOUT_SECS
                    );
                    log::warn!(
                        "[course_automation] {} AI attempt {}/{} timed out",
                        label,
                        attempt,
                        PLUS_AI_ATTEMPTS
                    );
                    return Err(last_error);
                }
            };
        let response_tokens = context::estimate_tokens(&response);
        log::info!(
            "[course_automation] {} response size: {} tokens",
            label,
            response_tokens
        );
        let Some(json_text) = extract_json_object(&response) else {
            last_error = format!("{}の結果に JSON がありません", label);
            log::warn!(
                "[course_automation] {} AI attempt {}/{} returned no JSON",
                label,
                attempt,
                PLUS_AI_ATTEMPTS
            );
            if attempt < PLUS_AI_ATTEMPTS {
                tokio::time::sleep(Duration::from_millis(750)).await;
            }
            continue;
        };
        match serde_json::from_str(json_text) {
            Ok(output) => {
                return Ok(PlusJsonResponse {
                    value: output,
                    usage: AiUsageEstimate {
                        at: epoch_secs(),
                        label: label.to_string(),
                        prompt_tokens,
                        response_tokens,
                        max_tokens,
                        attempts: attempt,
                    },
                })
            }
            Err(error) => {
                last_error = format!("{} JSON 解析失敗: {}", label, error);
                log::warn!(
                    "[course_automation] {} AI attempt {}/{} returned invalid JSON: {}",
                    label,
                    attempt,
                    PLUS_AI_ATTEMPTS,
                    error
                );
                if attempt < PLUS_AI_ATTEMPTS {
                    tokio::time::sleep(Duration::from_millis(750)).await;
                }
            }
        }
    }
    Err(last_error)
}

pub(super) fn plus_error_allows_retry(error: &str) -> bool {
    !error.contains("自動再試行しません") && !error.contains("途中で中断")
}

pub(super) fn normalize_trigger_decision(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "immediate" => "immediate".into(),
        "observe" => "observe".into(),
        _ => "routine".into(),
    }
}

fn normalize_document_agent_output(mut output: DocumentAgentOutput) -> DocumentAgentOutput {
    output.summary = truncate_chars(output.summary.trim(), 240);
    output.findings = normalize_short_list(output.findings, 3, 240);
    output.seat_evidence = normalize_short_list(output.seat_evidence, 6, 240);
    output.print_instruction = truncate_chars(output.print_instruction.trim(), 240);
    output.observation_context = truncate_chars(output.observation_context.trim(), 240);
    output.trigger_decision = normalize_trigger_decision(&output.trigger_decision);
    output
}
