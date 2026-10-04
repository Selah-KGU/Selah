#[cfg(target_os = "macos")]
use super::budget::{apple_request_parts, fit_apple_request, recover_context_limit};
#[cfg(target_os = "macos")]
use super::types::{BridgeStatus, InferenceRequest};
#[cfg(target_os = "macos")]
use super::CANCELLED_MSG;
#[cfg(target_os = "macos")]
use crate::local_ai_support;
#[cfg(target_os = "macos")]
use serde::Deserialize;
#[cfg(target_os = "macos")]
use std::collections::HashSet;
#[cfg(target_os = "macos")]
use std::ffi::{CStr, CString};
#[cfg(target_os = "macos")]
use std::os::raw::{c_char, c_int, c_void};
#[cfg(target_os = "macos")]
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::sync::{LazyLock, Mutex, OnceLock};

#[cfg(target_os = "macos")]
const RTLD_NOW: i32 = 2;
#[cfg(target_os = "macos")]
const RTLD_LOCAL: i32 = 4;

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
