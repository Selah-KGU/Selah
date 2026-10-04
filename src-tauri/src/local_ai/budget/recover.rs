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
