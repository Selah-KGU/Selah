use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, OnceLock};
use tauri::Emitter;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
#[allow(deprecated)] // token_to_str — token_to_piece requires encoding_rs::Decoder setup
use llama_cpp_2::model::{AddBos, LlamaModel, Special};
use llama_cpp_2::sampling::LlamaSampler;

// ============ Model catalog ============

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub size_label: String,
    pub param_size: String,
    pub file_name: String,
    pub download_url: String,
    pub file_size_mb: u64,
}

static MODEL_CATALOG: LazyLock<Vec<ModelInfo>> = LazyLock::new(|| {
    vec![
        ModelInfo {
            id: "qwen3.5-2b".into(),
            name: "標準".into(),
            size_label: "Qwen 3.5 2B".into(),
            param_size: "2B".into(),
            file_name: "Qwen3.5-2B-Q4_K_M.gguf".into(),
            download_url:
                "https://huggingface.co/unsloth/Qwen3.5-2B-GGUF/resolve/main/Qwen3.5-2B-Q4_K_M.gguf"
                    .into(),
            file_size_mb: 1280,
        },
        ModelInfo {
            id: "qwen3.5-4b".into(),
            name: "高品質".into(),
            size_label: "Qwen 3.5 4B".into(),
            param_size: "4B".into(),
            file_name: "Qwen3.5-4B-Q4_K_M.gguf".into(),
            download_url:
                "https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/main/Qwen3.5-4B-Q4_K_M.gguf"
                    .into(),
            file_size_mb: 2740,
        },
    ]
});

pub fn model_catalog() -> &'static [ModelInfo] {
    &MODEL_CATALOG
}

// ============ Model directory ============

fn models_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = crate::client::data_dir().join("models");
        let _ = std::fs::create_dir_all(&dir);
        dir
    })
}

pub fn model_path(file_name: &str) -> PathBuf {
    models_dir().join(file_name)
}

pub fn is_model_downloaded(file_name: &str) -> bool {
    let path = model_path(file_name);
    path.exists()
        && path
            .metadata()
            .map(|m| m.len() > 1_000_000)
            .unwrap_or(false)
}

// ============ Model download ============

static DOWNLOAD_CANCEL: Mutex<bool> = Mutex::new(false);

pub fn cancel_download() {
    if let Ok(mut flag) = DOWNLOAD_CANCEL.lock() {
        *flag = true;
    }
}

pub fn download_model(app: &tauri::AppHandle, model: &ModelInfo) -> Result<(), String> {
    // Reset cancel flag
    if let Ok(mut flag) = DOWNLOAD_CANCEL.lock() {
        *flag = false;
    }

    let url = &model.download_url;
    let dest = model_path(&model.file_name);
    let tmp = dest.with_extension("gguf.part");

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3600))
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    // Support resume: check if partial file exists
    let mut resume_from: u64 = 0;
    if tmp.exists() {
        resume_from = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
    }

    let mut req = client.get(url);
    if resume_from > 0 {
        req = req.header("Range", format!("bytes={}-", resume_from));
    }

    let resp = req
        .send()
        .map_err(|e| format!("ダウンロード開始失敗: {}", e))?;

    if !resp.status().is_success() && resp.status().as_u16() != 206 {
        return Err(format!("ダウンロードエラー ({})", resp.status()));
    }

    let total_size = if resp.status().as_u16() == 206 {
        // Partial content — get total from Content-Range header
        resp.headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.rsplit('/').next())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
    } else {
        resume_from = 0; // Server doesn't support range, start over
        resp.content_length().unwrap_or(0)
    };

    let file = if resume_from > 0 {
        std::fs::OpenOptions::new()
            .append(true)
            .open(&tmp)
            .map_err(|e| format!("ファイルオープン失敗: {}", e))?
    } else {
        std::fs::File::create(&tmp).map_err(|e| format!("ファイル作成失敗: {}", e))?
    };

    let mut writer = std::io::BufWriter::new(file);
    let mut downloaded = resume_from;
    let mut last_emit = std::time::Instant::now();

    let mut reader = resp;
    let mut buf = vec![0u8; 256 * 1024]; // 256KB chunks

    loop {
        // Check cancellation
        if let Ok(flag) = DOWNLOAD_CANCEL.lock() {
            if *flag {
                drop(writer);
                let _ = std::fs::remove_file(&tmp);
                return Err("cancelled".into());
            }
        }

        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("ダウンロード読み取りエラー: {}", e))?;

        if n == 0 {
            break;
        }

        writer
            .write_all(&buf[..n])
            .map_err(|e| format!("ファイル書き込みエラー: {}", e))?;

        downloaded += n as u64;

        // Emit progress every 200ms
        if last_emit.elapsed() > std::time::Duration::from_millis(200) {
            let _ = app.emit("model-download-progress", serde_json::json!({
                "downloaded": downloaded,
                "total": total_size,
                "percent": if total_size > 0 { (downloaded as f64 / total_size as f64 * 100.0) as u32 } else { 0 },
            }));
            last_emit = std::time::Instant::now();
        }
    }

    writer
        .flush()
        .map_err(|e| format!("ファイルフラッシュエラー: {}", e))?;
    drop(writer);

    // Rename .part -> final
    std::fs::rename(&tmp, &dest).map_err(|e| format!("ファイルリネームエラー: {}", e))?;

    // Final progress
    let _ = app.emit(
        "model-download-progress",
        serde_json::json!({
            "downloaded": downloaded,
            "total": total_size,
            "percent": 100,
            "done": true,
        }),
    );

    Ok(())
}

// ============ Local inference ============

/// Inference engine: backend initialized once, model hot-swapped on demand.
struct InferenceEngine {
    backend: LlamaBackend,
    model: Option<(LlamaModel, String)>, // (model, model_id)
}

// SAFETY: LlamaModel and LlamaBackend wrap C pointers accessed exclusively
// through the Mutex — no concurrent access is possible.
unsafe impl Send for InferenceEngine {}

static ENGINE: Mutex<Option<InferenceEngine>> = Mutex::new(None);

/// Unload the current model from memory (backend stays alive).
pub fn unload_model() {
    if let Ok(mut lock) = ENGINE.lock() {
        if let Some(engine) = lock.as_mut() {
            if let Some((_, id)) = engine.model.take() {
                log::debug!("[local_ai] Unloaded model: {}", id);
            }
        }
    }
}

// ── Cancellation ──

static CANCEL_FLAGS: LazyLock<Mutex<std::collections::HashSet<String>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

pub fn cancel_inference(gen_id: &str) {
    if let Ok(mut set) = CANCEL_FLAGS.lock() {
        set.insert(gen_id.to_string());
    }
}

fn is_cancelled(gen_id: &str) -> bool {
    CANCEL_FLAGS
        .lock()
        .map(|s| s.contains(gen_id))
        .unwrap_or(false)
}

fn clear_cancel(gen_id: &str) {
    if let Ok(mut set) = CANCEL_FLAGS.lock() {
        set.remove(gen_id);
    }
}

// ── Public API ──

/// Sampler configuration for local inference. Decoupled from the inference
/// pipeline so callers can tune parameters without touching the engine.
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
    /// Low-creativity config for deterministic planning output.
    pub fn deterministic(temperature: f32) -> Self {
        Self {
            temperature,
            ..Default::default()
        }
    }

    fn build_sampler(&self) -> LlamaSampler {
        LlamaSampler::chain_simple([
            LlamaSampler::penalties(self.penalty_last_n, 1.0, 0.0, self.presence_penalty),
            LlamaSampler::top_k(self.top_k),
            LlamaSampler::top_p(self.top_p, 1),
            LlamaSampler::temp(self.temperature),
            LlamaSampler::dist(rand::random::<u32>()),
        ])
    }
}

/// All parameters for a single local inference call, bundled for clarity.
pub struct InferenceRequest {
    pub model_id: String,
    pub file_name: String,
    pub messages: Vec<crate::ai::ChatMessage>,
    pub sampler: SamplerConfig,
    pub max_tokens: u32,
    pub prefill: String,
    pub gen_id: String,
    pub think_budget_pct: u32,
}

/// Non-streaming inference (planning, schedule generation, etc.).
pub fn run_inference(req: InferenceRequest) -> Result<String, String> {
    run_local_inference(&req, None::<fn(&str, bool)>)
}

/// Streaming inference with token-level callback.
pub fn run_inference_streaming<F: FnMut(&str, bool)>(
    req: InferenceRequest,
    on_chunk: F,
) -> Result<String, String> {
    if !req.gen_id.is_empty() {
        clear_cancel(&req.gen_id);
    }
    let result = run_local_inference(&req, Some(on_chunk));
    if !req.gen_id.is_empty() {
        clear_cancel(&req.gen_id);
    }
    result
}

// ── Core inference pipeline ──

const N_CTX: u32 = 65536;
const PREFILL_CHUNK: usize = 1024;

fn run_local_inference<F: FnMut(&str, bool)>(
    req: &InferenceRequest,
    mut on_token: Option<F>,
) -> Result<String, String> {
    let mut lock = ENGINE
        .lock()
        .map_err(|_| "エンジンロック取得失敗".to_string())?;
    let engine = ensure_engine(&mut lock)?;
    ensure_model(engine, &req.model_id, &req.file_name)?;
    let (model, _) = engine.model.as_ref().unwrap();

    // Tokenize ChatML prompt (with optional assistant prefill).
    let prompt = format_chatml(&req.messages, &req.prefill);
    let tokens = model
        .str_to_token(&prompt, AddBos::Always)
        .map_err(|e| format!("トークン化失敗: {}", e))?;
    let n_tokens = tokens.len();
    if n_tokens as u32 >= N_CTX {
        return Err(format!(
            "入力が長すぎます（{}トークン / 上限{}）",
            n_tokens, N_CTX
        ));
    }

    // Context + prefill.
    let ctx_params = LlamaContextParams::default().with_n_ctx(std::num::NonZeroU32::new(N_CTX));
    let mut ctx = model
        .new_context(&engine.backend, ctx_params)
        .map_err(|e| format!("コンテキスト作成失敗: {}", e))?;
    prefill(&mut ctx, &tokens)?;

    // Sampler from config.
    let mut sampler = req.sampler.build_sampler();

    // Budget.
    let remaining = (N_CTX as usize).saturating_sub(n_tokens);
    let max_gen = if req.max_tokens == 0 {
        remaining
    } else {
        (req.max_tokens as usize).min(remaining)
    };

    // Think-block tracking (token level).
    let think_open = model
        .str_to_token("<think>", AddBos::Never)
        .unwrap_or_default();
    let think_close = model
        .str_to_token("</think>\n", AddBos::Never)
        .unwrap_or_default();
    let stop_tokens = [
        model.str_to_token("<|im_end|>", AddBos::Never).ok(),
        model.str_to_token("<|endoftext|>", AddBos::Never).ok(),
    ];

    let pct = (req.think_budget_pct as usize).min(90);
    let think_budget = max_gen * pct / 100;
    let mut think_state = ThinkState::new(think_budget);

    // Stream state.
    let streaming = on_token.is_some();
    let mut stream = StreamState::default();

    let mut n_cur = n_tokens;
    let mut output_tokens = Vec::with_capacity(max_gen);
    let mut batch = LlamaBatch::new(PREFILL_CHUNK, 1);

    for _ in 0..max_gen {
        if streaming && !req.gen_id.is_empty() && is_cancelled(&req.gen_id) {
            break;
        }

        let token = sampler.sample(&ctx, -1);
        if model.is_eog_token(token) {
            break;
        }
        if is_stop_token(&stop_tokens, token) {
            break;
        }

        output_tokens.push(token);

        // Think-block tracking.
        think_state.update(&output_tokens, &think_open, &think_close);

        // Force-close think block when budget exhausted.
        if think_state.should_force_close(&think_close) {
            think_state.force_close();
            decode_single(&mut ctx, &mut batch, token, &mut n_cur)?;
            if streaming {
                emit_piece(model, token, &mut stream, on_token.as_mut());
            }

            for &close_tok in &think_close {
                output_tokens.push(close_tok);
                decode_single(&mut ctx, &mut batch, close_tok, &mut n_cur)?;
                if streaming {
                    emit_piece(model, close_tok, &mut stream, on_token.as_mut());
                }
            }
            continue;
        }

        decode_single(&mut ctx, &mut batch, token, &mut n_cur)?;
        if streaming {
            emit_piece(model, token, &mut stream, on_token.as_mut());
        }
    }

    if streaming {
        stream.flush(on_token.as_mut());
        Ok(stream.visible.trim().to_string())
    } else {
        let generated = detokenize_all(model, &output_tokens)?;
        if req.prefill.is_empty() {
            Ok(generated)
        } else {
            Ok(format!("{}{}", req.prefill, generated))
        }
    }
}

// ── Engine / Model management ──

fn ensure_engine(lock: &mut Option<InferenceEngine>) -> Result<&mut InferenceEngine, String> {
    if lock.is_none() {
        log::debug!("[local_ai] Initializing inference backend");
        let mut backend =
            LlamaBackend::init().map_err(|e| format!("バックエンド初期化失敗: {}", e))?;
        backend.void_logs();
        *lock = Some(InferenceEngine {
            backend,
            model: None,
        });
    }
    Ok(lock.as_mut().unwrap())
}

fn ensure_model(
    engine: &mut InferenceEngine,
    model_id: &str,
    file_name: &str,
) -> Result<(), String> {
    let need_load = match engine.model.as_ref() {
        Some((_, id)) if id == model_id => false,
        Some((_, id)) => {
            log::debug!("[local_ai] Switching model: {} -> {}", id, model_id);
            true
        }
        None => true,
    };
    if need_load {
        engine.model = None;
        let path = model_path(file_name);
        if !path.exists() {
            return Err(format!("モデルファイルが見つかりません: {}", file_name));
        }
        log::debug!("[local_ai] Loading model: {} from {:?}", model_id, path);
        let model =
            LlamaModel::load_from_file(&engine.backend, &path, &LlamaModelParams::default())
                .map_err(|e| format!("モデル読み込み失敗: {}", e))?;
        engine.model = Some((model, model_id.to_string()));
        log::info!("[local_ai] Model ready: {}", model_id);
    }
    Ok(())
}

// ── ChatML formatting ──

fn format_chatml(messages: &[crate::ai::ChatMessage], prefill: &str) -> String {
    let cap: usize = messages
        .iter()
        .map(|m| m.role.len() + m.content.len() + 30)
        .sum();
    let mut s = String::with_capacity(cap + 20 + prefill.len());
    for msg in messages {
        s.push_str("<|im_start|>");
        s.push_str(&msg.role);
        s.push('\n');
        s.push_str(&msg.content);
        s.push_str("<|im_end|>\n");
    }
    s.push_str("<|im_start|>assistant\n");
    if !prefill.is_empty() {
        s.push_str(prefill);
    }
    s
}

// ── Prefill ──

fn prefill(
    ctx: &mut llama_cpp_2::context::LlamaContext,
    tokens: &[llama_cpp_2::token::LlamaToken],
) -> Result<(), String> {
    let mut batch = LlamaBatch::new(PREFILL_CHUNK, 1);
    for (chunk_idx, chunk) in tokens.chunks(PREFILL_CHUNK).enumerate() {
        batch.clear();
        let start = chunk_idx * PREFILL_CHUNK;
        for (j, &tok) in chunk.iter().enumerate() {
            batch
                .add(tok, (start + j) as i32, &[0], j == chunk.len() - 1)
                .map_err(|_| "バッチ追加失敗".to_string())?;
        }
        ctx.decode(&mut batch)
            .map_err(|e| format!("プロンプトデコード失敗: {}", e))?;
    }
    Ok(())
}

// ── Decode helpers ──

fn decode_single(
    ctx: &mut llama_cpp_2::context::LlamaContext,
    batch: &mut LlamaBatch,
    token: llama_cpp_2::token::LlamaToken,
    n_cur: &mut usize,
) -> Result<(), String> {
    batch.clear();
    batch
        .add(token, *n_cur as i32, &[0], true)
        .map_err(|_| "バッチ追加失敗".to_string())?;
    ctx.decode(batch)
        .map_err(|e| format!("デコード失敗: {}", e))?;
    *n_cur += 1;
    Ok(())
}

fn is_stop_token(
    stop_tokens: &[Option<Vec<llama_cpp_2::token::LlamaToken>>; 2],
    token: llama_cpp_2::token::LlamaToken,
) -> bool {
    stop_tokens.iter().any(|st| {
        st.as_ref()
            .map(|toks| toks.len() == 1 && toks[0] == token)
            .unwrap_or(false)
    })
}

fn detokenize_all(
    model: &LlamaModel,
    tokens: &[llama_cpp_2::token::LlamaToken],
) -> Result<String, String> {
    #[allow(deprecated)]
    let output = tokens
        .iter()
        .map(|&t| model.token_to_str(t, Special::Tokenize))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("デトークン化失敗: {}", e))?
        .join("");
    Ok(output.trim().to_string())
}

// ── Think-block state machine (token level) ──

#[derive(Default)]
struct ThinkState {
    in_think: bool,
    token_count: usize,
    forced_close: bool,
    budget: usize,
}

impl ThinkState {
    fn new(budget: usize) -> Self {
        Self {
            budget,
            ..Default::default()
        }
    }

    fn update(
        &mut self,
        output: &[llama_cpp_2::token::LlamaToken],
        open: &[llama_cpp_2::token::LlamaToken],
        close: &[llama_cpp_2::token::LlamaToken],
    ) {
        if !self.in_think && !self.forced_close {
            if !open.is_empty()
                && output.len() >= open.len()
                && output[output.len() - open.len()..] == open[..]
            {
                self.in_think = true;
                self.token_count = 0;
            }
        } else if self.in_think {
            self.token_count += 1;
            if !close.is_empty()
                && output.len() >= close.len()
                && output[output.len() - close.len()..] == close[..]
            {
                self.in_think = false;
            }
        }
    }

    fn should_force_close(&self, close: &[llama_cpp_2::token::LlamaToken]) -> bool {
        !close.is_empty() && self.in_think && self.token_count >= self.budget
    }

    fn force_close(&mut self) {
        self.in_think = false;
        self.forced_close = true;
    }
}

// ── Stream state machine (string level) ──

#[derive(Default)]
struct StreamState {
    pending: String,
    in_think: bool,
    visible: String,
}

impl StreamState {
    fn flush<F: FnMut(&str, bool)>(&mut self, on_chunk: Option<&mut F>) {
        if self.pending.is_empty() {
            return;
        }
        if let Some(cb) = on_chunk {
            cb(&self.pending, self.in_think);
        }
        if !self.in_think {
            self.visible.push_str(&self.pending);
        }
        self.pending.clear();
    }
}

const GUARD_WINDOW: usize = 7; // shorter than "<think>" / "</think>"

fn emit_piece<F: FnMut(&str, bool)>(
    model: &LlamaModel,
    token: llama_cpp_2::token::LlamaToken,
    stream: &mut StreamState,
    on_chunk: Option<&mut F>,
) {
    #[allow(deprecated)]
    let piece = model
        .token_to_str(token, Special::Tokenize)
        .unwrap_or_default();
    process_stream_piece(&piece, stream, on_chunk);
}

fn process_stream_piece<F: FnMut(&str, bool)>(
    piece: &str,
    stream: &mut StreamState,
    mut on_chunk: Option<&mut F>,
) {
    stream.pending.push_str(piece);
    loop {
        if stream.in_think {
            if let Some(pos) = stream.pending.find("</think>") {
                if pos > 0 {
                    let to_emit: String = stream.pending.drain(..pos).collect();
                    if let Some(cb) = on_chunk.as_deref_mut() {
                        cb(&to_emit, true);
                    }
                }
                stream.pending.drain(..("</think>".len()));
                stream.in_think = false;
                continue;
            }
            emit_safe(
                &mut stream.pending,
                true,
                &mut stream.visible,
                &mut on_chunk,
            );
            break;
        } else {
            if let Some(pos) = stream.pending.find("<think>") {
                if pos > 0 {
                    let to_emit: String = stream.pending.drain(..pos).collect();
                    stream.visible.push_str(&to_emit);
                    if let Some(cb) = on_chunk.as_deref_mut() {
                        cb(&to_emit, false);
                    }
                }
                stream.pending.drain(..("<think>".len()));
                stream.in_think = true;
                continue;
            }
            emit_safe(
                &mut stream.pending,
                false,
                &mut stream.visible,
                &mut on_chunk,
            );
            break;
        }
    }
}

/// Emit pending text up to the guard window.
fn emit_safe<F: FnMut(&str, bool)>(
    pending: &mut String,
    is_think: bool,
    visible: &mut String,
    on_chunk: &mut Option<&mut F>,
) {
    if pending.len() <= GUARD_WINDOW {
        return;
    }
    let split = floor_char_boundary(pending, pending.len() - GUARD_WINDOW);
    if split == 0 {
        return;
    }
    let to_emit: String = pending.drain(..split).collect();
    if !is_think {
        visible.push_str(&to_emit);
    }
    if let Some(cb) = on_chunk.as_deref_mut() {
        cb(&to_emit, is_think);
    }
}

fn floor_char_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}
