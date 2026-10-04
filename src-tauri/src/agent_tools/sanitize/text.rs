use serde_json::Value;

pub(in crate::agent_tools) fn sanitize_text_arg(
    args: &Value,
    key: &str,
    max_len: usize,
) -> Option<String> {
    let value = args.get(key).and_then(|v| v.as_str())?.trim();
    if value.is_empty() {
        return None;
    }
    let mut out = value.chars().take(max_len).collect::<String>();
    out = out.replace(['\n', '\r'], " ");
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

pub(in crate::agent_tools) fn sanitize_copilot_page_args(args: &Value) -> Option<Value> {
    let page = sanitize_text_arg(args, "page", 40)?.to_ascii_lowercase();
    if !matches!(
        page.as_str(),
        "new_tab"
            | "files"
            | "luna"
            | "luna_course"
            | "luna_activity"
            | "kwic"
            | "kwic_notification"
            | "kwic_cabinet"
            | "kgc"
            | "kgc_notification"
    ) {
        return None;
    }
    let mut out = serde_json::Map::new();
    let context = sanitize_text_arg(args, "context", 120)
        .or_else(|| sanitize_text_arg(args, "course", 120))
        .or_else(|| sanitize_text_arg(args, "course_name", 120));
    let identifier = sanitize_text_arg(args, "identifier", 240)
        .or_else(|| sanitize_text_arg(args, "item_id", 240))
        .or_else(|| sanitize_text_arg(args, "id", 240));
    if matches!(page.as_str(), "kwic_notification" | "kgc_notification")
        && context.is_none()
        && identifier.is_none()
    {
        return None;
    }
    if matches!(page.as_str(), "luna_activity" | "luna_course") && context.is_none() {
        return None;
    }
    if matches!(page.as_str(), "luna_activity" | "luna_course") {
        if let Some(luna_id) = sanitize_text_arg(args, "luna_id", 80) {
            out.insert("luna_id".into(), Value::String(luna_id));
        }
    }
    out.insert("page".into(), Value::String(page));
    if let Some(context) = context {
        out.insert("context".into(), Value::String(context));
    }
    if let Some(identifier) = identifier {
        out.insert("identifier".into(), Value::String(identifier));
    }
    Some(Value::Object(out))
}

pub(in crate::agent_tools) fn sanitize_filename_arg(
    args: &Value,
    key: &str,
    max_len: usize,
) -> Option<String> {
    let value = args.get(key).and_then(|v| v.as_str())?.trim();
    if value.is_empty() || value.chars().count() > max_len {
        return None;
    }
    if value.contains('\0') || value.contains('/') || value.contains('\\') {
        return None;
    }
    let out = value.replace(['\n', '\r'], " ");
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

pub(in crate::agent_tools) fn sanitize_course_code(args: &Value, key: &str) -> Option<String> {
    let raw = sanitize_text_arg(args, key, 32)?;
    let code = raw.to_uppercase();
    if code
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        Some(code)
    } else {
        None
    }
}

pub(in crate::agent_tools) fn sanitize_file_path_arg(args: &Value, key: &str) -> Option<String> {
    let value = args.get(key).and_then(|v| v.as_str())?.trim();
    if value.is_empty() || value.len() > 600 {
        return None;
    }
    if value.contains('\0') {
        return None;
    }
    Some(value.to_string())
}

pub(in crate::agent_tools) fn sanitize_text_blob_arg(
    args: &Value,
    key: &str,
    max_len: usize,
) -> Option<String> {
    let value = args.get(key).and_then(|v| v.as_str())?;
    if value.is_empty() || value.len() > max_len {
        return None;
    }
    Some(value.replace('\0', ""))
}

pub(in crate::agent_tools) fn sanitize_url_arg(args: &Value, key: &str) -> Option<String> {
    let raw = args.get(key).and_then(|v| v.as_str())?.trim();
    if raw.is_empty() || raw.len() > 1000 {
        return None;
    }
    let parsed = url::Url::parse(raw).ok()?;
    match parsed.scheme() {
        "http" | "https" => Some(parsed.to_string()),
        _ => None,
    }
}
