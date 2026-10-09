//! Borrow tool results while serializing only the visible JSON prefix.
use super::*;
use serde::ser::{SerializeMap, SerializeSeq};
use std::borrow::Cow;
use std::io::{self, Write};

pub(super) fn hidden_field(key: &str) -> bool {
    matches!(
        key,
        "download_action"
            | "download_params"
            | "object_name"
            | "action"
            | "_cid"
            | "form_params"
            | "data_base64"
    )
}

struct Sanitized<'a>(&'a Value, usize);
impl Serialize for Sanitized<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Value::Object(fields) => {
                let mut map = serializer.serialize_map(None)?;
                for (key, value) in fields {
                    if !hidden_field(key) {
                        map.serialize_entry(key, &Sanitized(value, self.1))?;
                    }
                }
                map.end()
            }
            Value::Array(values) => {
                let mut seq = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    seq.serialize_element(&Sanitized(value, self.1))?;
                }
                seq.end()
            }
            Value::String(text) => {
                let text = crate::agent_text::neutralized_tool_prefix(text, self.1);
                // serde_json scans the entire string for escaping before its
                // first write. Give it only enough source bytes for this output
                // prefix. Escaping never shrinks UTF-8; three extra bytes allow
                // a complete codepoint, so an artificial closing quote cannot
                // enter the requested prefix. Neutralize before cutting so a
                // pseudo-call marker crossing the boundary is still handled.
                let mut end = self.1.saturating_add(3).min(text.len());
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                serializer.serialize_str(&text[..end])
            }
            other => other.serialize(serializer),
        }
    }
}

struct PrefixWriter {
    bytes: Vec<u8>,
    limit: usize,
    truncated: bool,
}
impl Write for PrefixWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let remaining = self.limit.saturating_sub(self.bytes.len());
        self.bytes
            .extend_from_slice(&bytes[..bytes.len().min(remaining)]);
        if bytes.len() > remaining {
            self.truncated = true;
            return Err(io::Error::other("JSON prefix complete"));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn json_bytes(value: &Value, limit: usize) -> (String, bool) {
    let mut writer = PrefixWriter {
        bytes: Vec::new(),
        limit,
        truncated: false,
    };
    let result = serde_json::to_writer(&mut writer, &Sanitized(value, limit));
    if result.is_err() && !writer.truncated {
        return (String::new(), false);
    }
    // A byte limit may split a multibyte character. Keep only the valid prefix,
    // matching the previous preview's UTF-8 boundary adjustment.
    let valid = match std::str::from_utf8(&writer.bytes) {
        Ok(_) => writer.bytes.len(),
        Err(error) => error.valid_up_to(),
    };
    writer.bytes.truncate(valid);
    (
        String::from_utf8(writer.bytes).expect("validated JSON prefix"),
        writer.truncated,
    )
}

pub(super) fn preview(value: &Value, bytes: usize) -> String {
    let (mut prefix, truncated) = json_bytes(value, bytes);
    if truncated {
        prefix.push('…');
    }
    prefix
}

pub(super) fn json_prefix(value: &Value, chars: usize) -> String {
    // Four UTF-8 bytes per character ensure this byte prefix contains all the
    // requested characters, without encoding the remainder of a large result.
    let (mut prefix, truncated) = json_bytes(value, chars.saturating_mul(4));
    if let Some((end, _)) = prefix.char_indices().nth(chars) {
        prefix.truncate(end);
        prefix.push('…');
    } else if truncated {
        prefix.push('…');
    }
    prefix
}

#[derive(Deserialize)]
struct ScreenshotMetadata {
    #[serde(default)]
    screen_rect: Value,
    #[serde(default)]
    target: Value,
}

pub(super) fn screenshot_metadata(json: &str) -> Result<Value, serde_json::Error> {
    match serde_json::from_str::<ScreenshotMetadata>(json) {
        Ok(metadata) => Ok(json!({"screen_rect": metadata.screen_rect, "target": metadata.target})),
        // Preserve the old behavior for non-object JSON or duplicate fields.
        Err(_) => serde_json::from_str(json),
    }
}

#[derive(Deserialize)]
struct Screenshot<'a> {
    #[serde(borrow)]
    image: Option<ScreenshotImage<'a>>,
}
#[derive(Deserialize)]
struct ScreenshotImage<'a> {
    #[serde(borrow)]
    mime: Cow<'a, str>,
    #[serde(borrow)]
    data_base64: Cow<'a, str>,
}

// An unescaped top-level image key necessarily contains this literal. A
// backslash may encode a key (e.g. "\u0069mage"), so uncertain JSON always
// goes through the existing decoder. This only rejects definite non-images;
// it never accepts JSON or bypasses validation of a candidate screenshot.
fn may_have_screenshot_image(json: &str) -> bool {
    json.contains("\"image\"") || json.contains('\\')
}

pub(super) fn screenshot_image(json: &str) -> Option<ImagePart> {
    if !may_have_screenshot_image(json) {
        return None;
    }
    match serde_json::from_str::<Screenshot<'_>>(json) {
        Ok(screenshot) => screenshot.image.map(|image| ImagePart {
            mime: image.mime.into_owned(),
            data_base64: image.data_base64.into_owned(),
        }),
        Err(_) => {
            let value: Value = serde_json::from_str(json).ok()?;
            let image = value.get("image")?;
            Some(ImagePart {
                mime: image.get("mime")?.as_str()?.to_owned(),
                data_base64: image.get("data_base64")?.as_str()?.to_owned(),
            })
        }
    }
}

pub(super) fn has_screenshot_image(json: &str) -> bool {
    if !may_have_screenshot_image(json) {
        return false;
    }
    match serde_json::from_str::<Screenshot<'_>>(json) {
        Ok(screenshot) => screenshot.image.is_some(),
        Err(_) => serde_json::from_str::<Value>(json)
            .ok()
            .is_some_and(|value| {
                value
                    .pointer("/image/mime")
                    .and_then(Value::as_str)
                    .is_some()
                    && value
                        .pointer("/image/data_base64")
                        .and_then(Value::as_str)
                        .is_some()
            }),
    }
}

#[cfg(test)]
#[path = "tool_result/tests.rs"]
mod tests;
