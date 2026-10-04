//! On-device Apple Intelligence via the Swift Foundation Models bridge.
//!
//! The bridge is a dylib built by `build.rs` and loaded only when this process
//! actually runs inference. The app binary itself stays free of a hard
//! FoundationModels / Swift runtime dependency so macOS 11–25 can still launch.

#![cfg_attr(not(target_os = "macos"), allow(dead_code, unused_imports))]

use crate::ai::ChatMessage;
use crate::local_ai_support;
use serde::Deserialize;
use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, OnceLock};

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

#[cfg(target_os = "macos")]
const RTLD_NOW: i32 = 2;
#[cfg(target_os = "macos")]
const RTLD_LOCAL: i32 = 4;

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

#[cfg(target_os = "macos")]
static CANCEL_FLAGS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));
#[cfg(target_os = "macos")]
static INFERENCE_LOCK: Mutex<()> = Mutex::new(());

pub fn unload_model() {}

#[cfg(target_os = "macos")]
pub fn cancel_inference(gen_id: &str) {
    if gen_id.is_empty() {
        return;
    }
    if let Ok(mut set) = CANCEL_FLAGS.lock() {
        set.insert(gen_id.to_string());
    }
    if let Some(api) = loaded_api() {
        if let Ok(c_id) = CString::new(gen_id) {
            unsafe { (api.cancel)(c_id.as_ptr()) };
        }
    }
}

#[cfg(target_os = "macos")]
pub fn clear_inference_cancel(gen_id: &str) {
    if gen_id.is_empty() {
        return;
    }
    if let Ok(mut set) = CANCEL_FLAGS.lock() {
        set.remove(gen_id);
    }
    if let Some(api) = loaded_api() {
        if let Ok(c_id) = CString::new(gen_id) {
            unsafe { (api.clear_cancel)(c_id.as_ptr()) };
        }
    }
}

#[cfg(target_os = "macos")]
fn is_cancelled(gen_id: &str) -> bool {
    if gen_id.is_empty() {
        return false;
    }
    CANCEL_FLAGS
        .lock()
        .map(|set| set.contains(gen_id))
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
pub fn run_inference(req: InferenceRequest) -> Result<String, String> {
    run_local(&req, None)
}

#[cfg(target_os = "macos")]
pub fn run_inference_streaming<F: FnMut(&str, bool)>(
    req: InferenceRequest,
    mut on_chunk: F,
) -> Result<String, String> {
    if !req.gen_id.is_empty() {
        clear_inference_cancel(&req.gen_id);
    }
    let result = run_local(&req, Some(&mut on_chunk));
    if !req.gen_id.is_empty() {
        clear_inference_cancel(&req.gen_id);
    }
    result
}

#[cfg(target_os = "macos")]
fn run_local(
    req: &InferenceRequest,
    on_chunk: Option<&mut dyn FnMut(&str, bool)>,
) -> Result<String, String> {
    let _ = (&req.model_id, &req.file_name, req.think_budget_pct);
    local_ai_support::ensure_supported()?;
    if is_cancelled(&req.gen_id) {
        return Err(CANCELLED_MSG.into());
    }
    let _guard = INFERENCE_LOCK
        .lock()
        .map_err(|_| "推論ロックの取得に失敗しました".to_string())?;
    if is_cancelled(&req.gen_id) {
        return Err(CANCELLED_MSG.into());
    }

    let (instructions, prompt) = apple_request_parts(&req.messages, &req.prefill);
    let fitted = fit_apple_request(&instructions, &prompt, req.max_tokens);
    if fitted.trimmed {
        log::info!(
            "Apple Intelligence input trimmed to {} tokens; response cap {}",
            fitted.input_tokens,
            fitted.max_tokens
        );
    }
    let temperature = req.sampler.temperature;
    let request = serde_json::json!({
        "gen_id": req.gen_id,
        "instructions": fitted.instructions,
        "prompt": fitted.prompt,
        "temperature": temperature,
        "max_tokens": fitted.max_tokens,
        "greedy": temperature <= 0.0,
    });
    let request = serde_json::to_string(&request).map_err(|error| error.to_string())?;
    let api = library()?;
    let c_request =
        CString::new(request).map_err(|_| "推論リクエストに NUL が含まれています".to_string())?;
    let mut streamed = String::new();
    let response = if let Some(callback) = on_chunk {
        let mut wrapped = |text: &str, is_think: bool| {
            streamed.push_str(text);
            callback(text, is_think);
        };
        call_generate(&api, c_request.as_ptr(), Some(&mut wrapped))
    } else {
        call_generate(&api, c_request.as_ptr(), None)
    };
    let owned = BridgeString {
        ptr: response,
        free: api.free,
    };
    let raw = owned.as_str()?.to_string();
    match parse_generate_response(&raw) {
        Ok(text) => Ok(text),
        Err(error) => recover_context_limit(&error, &streamed),
    }
}

#[cfg(target_os = "macos")]
fn call_generate(
    api: &BridgeApi,
    request: *const c_char,
    on_chunk: Option<&mut dyn FnMut(&str, bool)>,
) -> *mut c_char {
    match on_chunk {
        Some(callback) => {
            let mut ctx = ChunkCtx { callback };
            unsafe {
                (api.generate)(
                    request,
                    Some(chunk_trampoline),
                    &mut ctx as *mut ChunkCtx as *mut c_void,
                )
            }
        }
        None => unsafe { (api.generate)(request, None, std::ptr::null_mut()) },
    }
}

#[cfg(target_os = "macos")]
pub fn query_availability() -> Result<BridgeStatus, String> {
    if local_ai_support::macos_major_version().unwrap_or(0) < 26 {
        return Ok(BridgeStatus {
            supported: false,
            reason: local_ai_support::MACOS_TOO_OLD_MESSAGE.into(),
            permanent: true,
            model: String::new(),
            context_size: 0,
        });
    }
    let api = library()?;
    let owned = BridgeString {
        ptr: unsafe { (api.availability)() },
        free: api.free,
    };
    let raw = owned.as_str()?.to_string();
    let parsed: AvailabilityJson = serde_json::from_str(&raw)
        .map_err(|error| format!("可用性 JSON を読み取れません: {error}"))?;
    Ok(BridgeStatus {
        supported: parsed.supported,
        reason: parsed.reason,
        permanent: parsed.permanent,
        model: parsed.model,
        context_size: parsed.context_size,
    })
}

/// Apple Intelligence counts about one token per CJK character and ~3 ASCII characters.
pub(crate) fn estimate_apple_tokens(text: &str) -> usize {
    let mut ascii = 0usize;
    let mut other = 0usize;
    for ch in text.chars() {
        if ch.is_ascii() {
            ascii += 1;
        } else {
            other += 1;
        }
    }
    other + ascii.div_ceil(3) + 1
}

/// requested == 0 means "use the room left in the window", never unlimited.
pub(crate) fn apple_response_limit(requested: u32, input_tokens: usize) -> u32 {
    let room = APPLE_CONTEXT_WINDOW_TOKENS
        .saturating_sub(APPLE_CONTEXT_OVERHEAD_TOKENS)
        .saturating_sub(input_tokens)
        .clamp(
            192,
            APPLE_CONTEXT_WINDOW_TOKENS - APPLE_CONTEXT_OVERHEAD_TOKENS,
        ) as u32;
    if requested == 0 {
        room
    } else {
        requested.min(room).max(32)
    }
}

pub(crate) struct FittedAppleRequest {
    pub instructions: String,
    pub prompt: String,
    pub max_tokens: u32,
    pub input_tokens: usize,
    pub trimmed: bool,
}

/// Fit instructions and the conversation into the Apple window before the bridge runs.
/// Every local caller, including timetable and Live, goes through this.
pub(crate) fn fit_apple_request(
    instructions: &str,
    prompt: &str,
    requested_max_tokens: u32,
) -> FittedAppleRequest {
    let budget = prompt_budget_for(&instructions, &prompt);
    let mut instructions = instructions.trim().to_string();
    let mut prompt = prompt.trim().to_string();
    if prompt.is_empty() {
        prompt = "応答してください。".to_string();
    }
    let before = estimate_apple_tokens(&instructions) + estimate_apple_tokens(&prompt);
    if before > budget {
        let prompt_floor = 512usize.min(budget.saturating_sub(256));
        let prompt_budget = (budget / 2).clamp(prompt_floor, budget.saturating_sub(256));
        prompt = trim_apple_turns(&prompt, prompt_budget);
        let instruction_budget = budget
            .saturating_sub(estimate_apple_tokens(&prompt))
            .max(128);
        if estimate_apple_tokens(&instructions) > instruction_budget {
            instructions = trim_apple_text(&instructions, instruction_budget);
        }
        let used = estimate_apple_tokens(&instructions) + estimate_apple_tokens(&prompt);
        if used > budget {
            let tighter = budget
                .saturating_sub(estimate_apple_tokens(&instructions))
                .max(64);
            prompt = trim_apple_turns(&prompt, tighter);
        }
    }
    let mut input_tokens = estimate_apple_tokens(&instructions) + estimate_apple_tokens(&prompt);
    if input_tokens > budget {
        prompt = trim_apple_turns(&prompt, budget / 2);
        let instruction_budget = budget.saturating_sub(estimate_apple_tokens(&prompt));
        instructions = trim_apple_text(&instructions, instruction_budget);
        input_tokens = estimate_apple_tokens(&instructions) + estimate_apple_tokens(&prompt);
    }
    FittedAppleRequest {
        instructions,
        prompt,
        max_tokens: apple_response_limit(requested_max_tokens, input_tokens),
        input_tokens,
        trimmed: before > budget,
    }
}

fn json_reply_requested(instructions: &str, prompt: &str) -> bool {
    instructions.contains("JSONのみ")
        || instructions.contains("JSONだけ")
        || instructions.contains("Output one JSON")
        || instructions.contains("出力は必ず")
        || prompt.contains("JSONのみ")
        || prompt.contains("JSONだけ")
}

fn prompt_budget_for(instructions: &str, prompt: &str) -> usize {
    let reserve = if json_reply_requested(instructions, prompt) {
        APPLE_JSON_RESPONSE_RESERVE_TOKENS
    } else {
        APPLE_RESPONSE_RESERVE_TOKENS
    };
    APPLE_CONTEXT_WINDOW_TOKENS
        .saturating_sub(APPLE_CONTEXT_OVERHEAD_TOKENS)
        .saturating_sub(reserve)
        .max(512)
}

/// Shrink a JSON value without cutting through an object, array, or string.
/// Arrays lose whole items. Long strings are shortened inside the quotes.
pub(crate) fn compact_json_value(value: &serde_json::Value, budget: usize) -> serde_json::Value {
    if budget < 2 {
        return serde_json::Value::Null;
    }
    let raw = serde_json::to_string(value).unwrap_or_else(|_| "null".into());
    if estimate_apple_tokens(&raw) <= budget {
        return value.clone();
    }
    match value {
        serde_json::Value::String(text) => {
            serde_json::Value::String(truncate_json_string(text, budget))
        }
        serde_json::Value::Array(items) => compact_json_array(items, budget),
        serde_json::Value::Object(map) => compact_json_object(map, budget),
        other => {
            if estimate_apple_tokens(&raw) <= budget {
                other.clone()
            } else {
                serde_json::Value::Null
            }
        }
    }
}

fn compact_json_array(items: &[serde_json::Value], budget: usize) -> serde_json::Value {
    let mut kept = Vec::new();
    for item in items {
        let used = serde_json::to_string(&serde_json::Value::Array(kept.clone()))
            .map(|text| estimate_apple_tokens(&text))
            .unwrap_or(2);
        let remaining = budget.saturating_sub(used).saturating_sub(1);
        if remaining < 2 {
            break;
        }
        let shrunk = compact_json_value(item, remaining);
        let mut trial = kept.clone();
        trial.push(shrunk.clone());
        let rendered =
            serde_json::to_string(&serde_json::Value::Array(trial)).unwrap_or_else(|_| "[]".into());
        if estimate_apple_tokens(&rendered) > budget {
            break;
        }
        kept.push(shrunk);
    }
    serde_json::Value::Array(kept)
}

fn compact_json_object(
    map: &serde_json::Map<String, serde_json::Value>,
    budget: usize,
) -> serde_json::Value {
    // Keep smaller fields first so a large array is shortened instead of deleted.
    let mut entries: Vec<(&String, &serde_json::Value)> = map.iter().collect();
    entries.sort_by_key(|(_, value)| {
        serde_json::to_string(value)
            .map(|text| text.len())
            .unwrap_or(0)
    });
    let mut kept = serde_json::Map::new();
    for (key, value) in entries {
        let used = serde_json::to_string(&serde_json::Value::Object(kept.clone()))
            .map(|text| estimate_apple_tokens(&text))
            .unwrap_or(2);
        let key_cost = estimate_apple_tokens(key) + 3;
        let remaining = budget.saturating_sub(used).saturating_sub(key_cost);
        if remaining < 2 {
            continue;
        }
        let shrunk = compact_json_value(value, remaining);
        let mut trial = kept.clone();
        trial.insert(key.clone(), shrunk.clone());
        let rendered = serde_json::to_string(&serde_json::Value::Object(trial))
            .unwrap_or_else(|_| "{}".into());
        if estimate_apple_tokens(&rendered) > budget {
            continue;
        }
        kept.insert(key.clone(), shrunk);
    }
    serde_json::Value::Object(kept)
}
fn truncate_json_string(text: &str, budget: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut low = 0usize;
    let mut high = chars.len();
    while low < high {
        let mid = (low + high + 1) / 2;
        let candidate: String = chars[..mid].iter().collect();
        let rendered = serde_json::to_string(&serde_json::Value::String(candidate))
            .unwrap_or_else(|_| "\"\"".into());
        if estimate_apple_tokens(&rendered) <= budget {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    let mut out: String = chars[..low].iter().collect();
    if low < chars.len()
        && estimate_apple_tokens(
            &serde_json::to_string(&serde_json::Value::String(format!("{out}…")))
                .unwrap_or_default(),
        ) <= budget
    {
        out.push('…');
    }
    out
}

fn outermost_brace_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut in_string = false;
    let mut escape = false;
    let mut index = 0usize;
    while index < text.len() {
        let ch = text[index..].chars().next().unwrap_or('\0');
        let len = ch.len_utf8();
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            index += len;
            continue;
        }
        if ch == '"' {
            in_string = true;
            index += len;
            continue;
        }
        if ch == '{' || ch == '[' {
            if let Some(end) = matching_close(text, index) {
                spans.push((index, end));
                index = end;
                continue;
            }
        }
        index += len;
    }
    spans
}

fn matching_close(text: &str, open_byte: usize) -> Option<usize> {
    let opener = text[open_byte..].chars().next()?;
    let closer = if opener == '{' { '}' } else { ']' };
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    let mut index = open_byte;
    while index < text.len() {
        let ch = text[index..].chars().next()?;
        let len = ch.len_utf8();
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
        } else {
            match ch {
                '"' => in_string = true,
                '{' | '[' => depth += 1,
                '}' | ']' => {
                    depth -= 1;
                    if depth == 0 {
                        return (ch == closer).then_some(index + len);
                    }
                }
                _ => {}
            }
        }
        index += len;
    }
    None
}

fn compact_embedded_json(text: &str, span_budget: usize) -> String {
    let spans = outermost_brace_spans(text);
    if spans.is_empty() {
        return text.to_string();
    }
    let mut out = String::new();
    let mut cursor = 0usize;
    for (start, end) in spans {
        out.push_str(&text[cursor..start]);
        let span = &text[start..end];
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(span) {
            if estimate_apple_tokens(span) > span_budget {
                let compact = compact_json_value(&value, span_budget.max(16));
                out.push_str(&serde_json::to_string(&compact).unwrap_or_else(|_| "{}".into()));
            } else {
                out.push_str(span);
            }
        } else {
            out.push_str(span);
        }
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    out
}

fn split_atoms(text: &str) -> Vec<String> {
    let spans = outermost_brace_spans(text);
    let mut atoms = Vec::new();
    let mut cursor = 0usize;
    for (start, end) in spans {
        if start > cursor {
            push_prose_atoms(&mut atoms, &text[cursor..start]);
        }
        atoms.push(text[start..end].to_string());
        cursor = end;
    }
    if cursor < text.len() {
        push_prose_atoms(&mut atoms, &text[cursor..]);
    }
    atoms.retain(|atom| !atom.trim().is_empty());
    atoms
}

fn push_prose_atoms(out: &mut Vec<String>, prose: &str) {
    if prose.is_empty() {
        return;
    }
    let parts: Vec<&str> = prose.split("\n\n").collect();
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if index == 0 {
            out.push((*part).to_string());
        } else {
            out.push(format!("\n\n{part}"));
        }
    }
}

fn is_brace_atom(atom: &str) -> bool {
    let trimmed = atom.trim_start();
    trimmed.starts_with('{') || trimmed.starts_with('[')
}

fn fit_atom(atom: &str, budget: usize, keep_tail: bool) -> Option<String> {
    if budget < 2 || atom.is_empty() {
        return None;
    }
    if estimate_apple_tokens(atom) <= budget {
        return Some(atom.to_string());
    }
    if is_brace_atom(atom) {
        let trimmed = atom.trim();
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            let compact = compact_json_value(&value, budget);
            let rendered = serde_json::to_string(&compact).unwrap_or_else(|_| "{}".into());
            if estimate_apple_tokens(&rendered) <= budget {
                return Some(rendered);
            }
        }
        return None;
    }
    if atom.contains('{') || atom.contains('[') {
        return None;
    }
    let sliced = slice_prose(atom, budget, keep_tail);
    if sliced.trim().is_empty() {
        None
    } else {
        Some(sliced)
    }
}

fn slice_prose(text: &str, budget: usize, keep_tail: bool) -> String {
    if estimate_apple_tokens(text) <= budget {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut low = 0usize;
    let mut high = chars.len();
    while low < high {
        let mid = (low + high + 1) / 2;
        let slice: String = if keep_tail {
            chars[chars.len() - mid..].iter().collect()
        } else {
            chars[..mid].iter().collect()
        };
        if estimate_apple_tokens(&slice) <= budget {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    if keep_tail {
        chars[chars.len() - low..].iter().collect()
    } else {
        chars[..low].iter().collect()
    }
}

fn join_atoms(atoms: &[String]) -> String {
    let mut out = String::new();
    let mut omitted = false;
    for atom in atoms {
        if atom.is_empty() {
            omitted = true;
            continue;
        }
        if omitted && !out.is_empty() && !out.ends_with('…') {
            out.push_str("\n…\n");
        }
        omitted = false;
        out.push_str(atom);
    }
    out
}

fn enforce_atom_budget(mut atoms: Vec<String>, budget: usize, drop_front: bool) -> String {
    while estimate_apple_tokens(&join_atoms(&atoms)) > budget && !atoms.is_empty() {
        if drop_front {
            atoms.remove(0);
        } else {
            atoms.pop();
        }
    }
    join_atoms(&atoms)
}

fn atom_room(budget: usize, used: usize) -> usize {
    let separator = usize::from(used > 0);
    budget.saturating_sub(used).saturating_sub(separator)
}

fn select_side(atoms: &[String], budget: usize, keep_tail: bool) -> String {
    let mut chosen = Vec::new();
    let mut used = 0usize;
    let indexes: Vec<usize> = if keep_tail {
        (0..atoms.len()).rev().collect()
    } else {
        (0..atoms.len()).collect()
    };
    for index in indexes {
        let separator = usize::from(used > 0);
        let remaining = atom_room(budget, used);
        let Some(fitted) = fit_atom(&atoms[index], remaining, keep_tail) else {
            continue;
        };
        let cost = estimate_apple_tokens(&fitted) + separator;
        if used + cost > budget {
            continue;
        }
        used += cost;
        if keep_tail {
            chosen.insert(0, fitted);
        } else {
            chosen.push(fitted);
        }
    }
    enforce_atom_budget(chosen, budget, keep_tail)
}

fn select_head_and_tail(atoms: &[String], budget: usize) -> String {
    let mut chosen: Vec<Option<String>> = vec![None; atoms.len()];
    let mut used = 0usize;
    for (index, atom) in atoms.iter().enumerate() {
        if !is_brace_atom(atom) {
            continue;
        }
        let separator = usize::from(used > 0);
        let remaining = atom_room(budget, used);
        let Some(fitted) = fit_atom(atom, remaining, false) else {
            continue;
        };
        let cost = estimate_apple_tokens(&fitted) + separator;
        if used + cost > budget {
            continue;
        }
        used += cost;
        chosen[index] = Some(fitted);
    }
    let prose_budget = budget.saturating_sub(used);
    let head_limit = prose_budget / 3;
    let mut prose_used = 0usize;
    for (index, atom) in atoms.iter().enumerate() {
        if chosen[index].is_some() || is_brace_atom(atom) {
            continue;
        }
        let separator = usize::from(prose_used > 0);
        let remaining = atom_room(head_limit, prose_used);
        let Some(fitted) = fit_atom(atom, remaining, false) else {
            break;
        };
        let cost = estimate_apple_tokens(&fitted) + separator;
        if prose_used + cost > head_limit {
            break;
        }
        prose_used += cost;
        chosen[index] = Some(fitted);
    }
    for (index, atom) in atoms.iter().enumerate().rev() {
        if chosen[index].is_some() || is_brace_atom(atom) {
            continue;
        }
        let separator = usize::from(prose_used > 0);
        let remaining = atom_room(prose_budget, prose_used);
        let Some(fitted) = fit_atom(atom, remaining, true) else {
            continue;
        };
        let cost = estimate_apple_tokens(&fitted) + separator;
        if prose_used + cost > prose_budget {
            continue;
        }
        prose_used += cost;
        chosen[index] = Some(fitted);
    }
    let kept: Vec<String> = chosen.into_iter().flatten().collect();
    enforce_atom_budget(kept, budget, false)
}

/// Keep rules, JSON blocks, and the latest material. JSON is never cut mid-value.
pub(crate) fn trim_apple_text(text: &str, budget: usize) -> String {
    trim_structured(text, budget, false)
}

fn trim_apple_turns(prompt: &str, budget: usize) -> String {
    trim_structured(prompt, budget, true)
}

fn trim_structured(text: &str, budget: usize, keep_tail: bool) -> String {
    if budget == 0 || text.is_empty() {
        return String::new();
    }
    let compacted = compact_embedded_json(text, budget.max(32));
    if estimate_apple_tokens(&compacted) <= budget {
        return compacted;
    }
    let atoms = split_atoms(&compacted);
    let selected = if keep_tail {
        select_side(&atoms, budget, true)
    } else {
        select_head_and_tail(&atoms, budget)
    };
    if selected.is_empty() && !compacted.contains('{') && !compacted.contains('[') {
        return slice_prose(&compacted, budget, keep_tail);
    }
    selected
}

pub(crate) fn apple_request_parts(messages: &[ChatMessage], prefill: &str) -> (String, String) {
    let mut instructions = Vec::new();
    let mut turns = Vec::new();
    for message in messages {
        let content = message.content.trim();
        if content.is_empty() {
            continue;
        }
        match message.role.as_str() {
            "system" | "developer" => instructions.push(content.to_string()),
            "assistant" => turns.push(format!("Assistant:\n{content}")),
            _ => turns.push(format!("User:\n{content}")),
        }
    }
    let prefill = prefill.trim();
    if !prefill.is_empty() {
        instructions.push(
            "応答は下書きの続きだけを出力し、下書き自体は繰り返さないでください。".to_string(),
        );
        turns.push(format!("Assistant:\n{prefill}"));
    }
    (instructions.join("\n\n"), turns.join("\n\n"))
}

#[cfg(target_os = "macos")]
struct ChunkCtx<'a> {
    callback: &'a mut dyn FnMut(&str, bool),
}

#[cfg(target_os = "macos")]
unsafe extern "C" fn chunk_trampoline(chunk: *const c_char, is_final: c_int, ctx: *mut c_void) {
    if chunk.is_null() || ctx.is_null() || is_final != 0 {
        return;
    }
    let text = unsafe { CStr::from_ptr(chunk) }.to_string_lossy();
    if text.is_empty() {
        return;
    }
    let ctx = unsafe { &mut *(ctx as *mut ChunkCtx) };
    (ctx.callback)(&text, false);
}

#[cfg(target_os = "macos")]
#[derive(Deserialize)]
struct AvailabilityJson {
    supported: bool,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    permanent: bool,
    #[serde(default)]
    model: String,
    #[serde(default)]
    context_size: i64,
}

#[cfg(target_os = "macos")]
#[derive(Deserialize)]
struct GenerateJson {
    ok: bool,
    #[serde(default)]
    text: String,
    #[serde(default)]
    error: String,
    #[serde(default)]
    cancelled: bool,
}

pub(crate) fn is_apple_context_limit(message: &str) -> bool {
    message.contains("コンテキスト上限")
}

/// Apple Intelligence throws once the window is full, even after it has already
/// streamed a usable reply. Keep that reply instead of failing the whole send.
pub(crate) fn recover_context_limit(error: &str, partial: &str) -> Result<String, String> {
    let partial = partial.trim();
    if is_apple_context_limit(error) && !partial.is_empty() {
        Ok(partial.to_string())
    } else {
        Err(error.to_string())
    }
}

#[cfg(target_os = "macos")]
fn parse_generate_response(raw: &str) -> Result<String, String> {
    let parsed: GenerateJson = serde_json::from_str(raw)
        .map_err(|error| format!("推論結果を読み取れません: {error}: {raw}"))?;
    if parsed.cancelled || parsed.error == CANCELLED_MSG {
        return Err(CANCELLED_MSG.into());
    }
    if parsed.ok {
        return Ok(parsed.text);
    }
    if parsed.error.is_empty() {
        Err("Apple Intelligence の推論に失敗しました".into())
    } else {
        Err(parsed.error)
    }
}

#[cfg(target_os = "macos")]
type FreeFn = unsafe extern "C" fn(*mut c_char);
#[cfg(target_os = "macos")]
type AvailabilityFn = unsafe extern "C" fn() -> *mut c_char;
#[cfg(target_os = "macos")]
type ChunkFn = unsafe extern "C" fn(*const c_char, c_int, *mut c_void);
#[cfg(target_os = "macos")]
type GenerateFn = unsafe extern "C" fn(*const c_char, Option<ChunkFn>, *mut c_void) -> *mut c_char;
#[cfg(target_os = "macos")]
type CancelFn = unsafe extern "C" fn(*const c_char);

#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
struct BridgeApi {
    free: FreeFn,
    availability: AvailabilityFn,
    generate: GenerateFn,
    cancel: CancelFn,
    clear_cancel: CancelFn,
}

#[cfg(target_os = "macos")]
struct BridgeString {
    ptr: *mut c_char,
    free: FreeFn,
}

#[cfg(target_os = "macos")]
impl BridgeString {
    fn as_str(&self) -> Result<&str, String> {
        if self.ptr.is_null() {
            return Err("Apple Intelligence ブリッジが空の応答を返しました".into());
        }
        unsafe { CStr::from_ptr(self.ptr) }
            .to_str()
            .map_err(|_| "Apple Intelligence ブリッジの応答が UTF-8 ではありません".into())
    }
}

#[cfg(target_os = "macos")]
impl Drop for BridgeString {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { (self.free)(self.ptr) };
            self.ptr = std::ptr::null_mut();
        }
    }
}

#[cfg(target_os = "macos")]
fn loaded_api() -> Option<BridgeApi> {
    library().ok()
}

#[cfg(target_os = "macos")]
fn library() -> Result<BridgeApi, String> {
    static LOADED: OnceLock<Result<BridgeApi, String>> = OnceLock::new();
    LOADED.get_or_init(load_library).clone()
}

#[cfg(target_os = "macos")]
fn load_library() -> Result<BridgeApi, String> {
    let path = library_path()?;
    let c_path = CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| "Apple Intelligence ブリッジのパスが不正です".to_string())?;
    let handle = unsafe { dlopen(c_path.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
    if handle.is_null() {
        return Err(format!(
            "Apple Intelligence ブリッジを読み込めませんでした: {}",
            last_dlerror()
        ));
    }
    unsafe {
        Ok(BridgeApi {
            free: transmute_symbol(handle, "selah_apple_ai_free")?,
            availability: transmute_symbol(handle, "selah_apple_ai_availability_json")?,
            generate: transmute_symbol(handle, "selah_apple_ai_generate")?,
            cancel: transmute_symbol(handle, "selah_apple_ai_cancel")?,
            clear_cancel: transmute_symbol(handle, "selah_apple_ai_clear_cancel")?,
        })
    }
}

#[cfg(target_os = "macos")]
fn library_path() -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Ok(path) = std::env::var("SELAH_APPLE_AI_LIB") {
        candidates.push(PathBuf::from(path));
    }
    candidates.push(PathBuf::from(env!("SELAH_APPLE_AI_LIB")));
    candidates.push(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/lib/libselah_apple_ai.dylib"
    )));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join("../Frameworks/libselah_apple_ai.dylib"));
            candidates.push(parent.join("libselah_apple_ai.dylib"));
        }
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| "Apple Intelligence ブリッジが見つかりません".to_string())
}

#[cfg(target_os = "macos")]
unsafe fn transmute_symbol<T>(handle: *mut c_void, name: &str) -> Result<T, String> {
    let c_name = CString::new(name).map_err(|_| format!("symbol {name} is invalid"))?;
    let symbol = unsafe { dlsym(handle, c_name.as_ptr()) };
    if symbol.is_null() {
        return Err(format!(
            "Apple Intelligence ブリッジに {name} がありません: {}",
            last_dlerror()
        ));
    }
    Ok(unsafe { std::mem::transmute_copy(&symbol) })
}

#[cfg(target_os = "macos")]
fn last_dlerror() -> String {
    let ptr = unsafe { dlerror() };
    if ptr.is_null() {
        return "unknown dlopen error".into();
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(target_os = "macos")]
extern "C" {
    fn dlopen(path: *const c_char, flags: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: role.into(),
            content: content.into(),
            images: Vec::new(),
        }
    }

    #[test]
    fn system_messages_become_instructions_and_turns_keep_order() {
        let (instructions, prompt) = apple_request_parts(
            &[
                message("system", "関学生向けに短く答える"),
                message("user", "次の授業は？"),
                message("assistant", "3限です"),
                message("user", "教室は？"),
            ],
            "",
        );
        assert_eq!(instructions, "関学生向けに短く答える");
        assert_eq!(
            prompt,
            "User:\n次の授業は？\n\nAssistant:\n3限です\n\nUser:\n教室は？"
        );
    }

    #[test]
    fn context_limit_keeps_partial_output() {
        let partial = "今日の3限はアルゴリズムです。";
        let recovered = recover_context_limit(
            "入力が Apple Intelligence のコンテキスト上限を超えています。",
            partial,
        )
        .expect("partial reply");
        assert_eq!(recovered, partial);
        assert!(recover_context_limit(
            "入力が Apple Intelligence のコンテキスト上限を超えています。",
            "   "
        )
        .is_err());
        assert!(recover_context_limit("Apple Intelligence の推論に失敗しました", partial).is_err());
    }

    #[test]
    fn prefill_is_appended_as_an_assistant_draft() {
        let (instructions, prompt) =
            apple_request_parts(&[message("user", "JSONで")], "{\"tools\":[");
        assert!(instructions.contains("下書き"));
        assert!(prompt.ends_with("Assistant:\n{\"tools\":["));
    }

    #[test]
    fn apple_limit_stops_inside_the_window() {
        let short = fit_apple_request("指示", "User:\n天気", 0);
        assert!(!short.trimmed);
        assert!(short.max_tokens > 0);
        assert!(short.max_tokens < 8_192);

        let oversized_request = fit_apple_request("指示", "User:\n天気", 8_192);
        assert!(
            oversized_request.max_tokens
                <= (APPLE_CONTEXT_WINDOW_TOKENS - APPLE_CONTEXT_OVERHEAD_TOKENS) as u32
        );
        assert!(oversized_request.max_tokens < 8_192);

        let instructions = format!("RULES\n{}", "指示".repeat(8_000));
        let prompt = format!("User:\n{}\n\nUser:\n最終質問です", "古い".repeat(8_000));
        let fitted = fit_apple_request(&instructions, &prompt, 0);
        assert!(fitted.trimmed);
        assert!(fitted.prompt.contains("最終質問です"));
        assert!(fitted.input_tokens <= APPLE_PROMPT_TOKEN_BUDGET);
        assert!(fitted.max_tokens >= 192);
        assert!(fitted.instructions.contains("RULES") || fitted.instructions.contains("指示"));
    }

    #[test]
    fn json_is_compacted_instead_of_cut() {
        let value = serde_json::json!({
            "items": (0..20).map(|index| serde_json::json!({
                "id": index,
                "title": "課題",
                "body": "あ".repeat(80)
            })).collect::<Vec<_>>()
        });
        let compact = compact_json_value(&value, 80);
        let rendered = serde_json::to_string(&compact).expect("json");
        assert!(serde_json::from_str::<serde_json::Value>(&rendered).is_ok());
        assert!(estimate_apple_tokens(&rendered) <= 80);
    }

    #[test]
    fn schema_block_is_kept_whole() {
        let schema = r#"{
  "current_week": [{"day": 1, "course_name": "科目"}]
}"#;
        let prose = "説明".repeat(3000);
        let text = format!("{prose}\n\n出力形式:\n{schema}\n\n{prose}");
        let trimmed = trim_apple_text(&text, 500);
        assert!(super::braces_balanced(&trimmed), "{trimmed}");
        assert!(trimmed.contains("current_week"));
        let start = trimmed.find('{').expect("schema");
        let end = super::matching_close(&trimmed, start).expect("close");
        let block = &trimmed[start..end];
        assert!(
            serde_json::from_str::<serde_json::Value>(block).is_ok(),
            "{block}"
        );
    }

    #[test]
    fn invalid_schema_block_is_kept_whole() {
        let schema = "{\n  \"next_week\": [同じ形式],\n  \"weekly_summary\": \"文\"\n}";
        let prose = "説明".repeat(3000);
        let text = format!("ルール\n\n{prose}\n\n出力形式:\n{schema}\n\n末尾の条件");
        let trimmed = trim_apple_text(&text, 500);
        assert!(super::braces_balanced(&trimmed), "{trimmed}");
        assert!(trimmed.contains("同じ形式"), "{trimmed}");
        assert!(trimmed.contains("weekly_summary"), "{trimmed}");
        assert!(trimmed.contains("末尾の条件"), "{trimmed}");
    }

    #[test]
    fn oversized_invalid_schema_is_dropped_not_sliced() {
        let schema = format!(
            "{{\"next_week\": [同じ形式], \"note\": \"{}\"}}",
            "科目".repeat(800)
        );
        let text = format!("先頭\n\n{schema}\n\n末尾の質問");
        let trimmed = trim_apple_text(&text, 40);
        assert!(super::braces_balanced(&trimmed), "{trimmed}");
        assert!(!trimmed.contains("同じ形式"), "{trimmed}");
        assert!(trimmed.contains("末尾の質問") || trimmed.contains("先頭"));
    }
}

#[cfg(test)]
fn braces_balanced(text: &str) -> bool {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for ch in text.chars() {
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' | '[' => depth += 1,
            '}' | ']' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    !in_string && depth == 0
}
