use image::GenericImageView;
use std::sync::LazyLock;

use crate::config;

use super::super::types::{KwicAttachment, KwicNotificationDetail};
use super::{
    SEL_BLOCK_TITLE, SEL_CONTENTS_DETAIL, SEL_CONTENTS_HTML, SEL_CSRF, SEL_FILE_NAME,
    SEL_FILE_OBJECT, SEL_HEADER_BOLD, SEL_INPUT_AREA, SEL_OBJECT_NAME, SEL_OUTGOING_DIV,
};

/// Extract CSRF token from KWIC Portal HTML
pub(in crate::kwic_commands) fn extract_csrf_token(html: &str) -> Option<String> {
    use scraper::Html;
    let doc = Html::parse_document(html);
    if let Some(el) = doc.select(&SEL_CSRF).next() {
        return el.value().attr("value").map(|v| v.to_string());
    }
    None
}

/// Parse the detail HTML fragment returned by /lms/course/information/listdetail.
/// This is typically a dialog fragment containing info_preview with title, body, sender, date, attachments.
pub(in crate::kwic_commands) fn parse_detail_html(html: &str) -> KwicNotificationDetail {
    use scraper::Html;
    let doc = Html::parse_document(html);

    let text_of = |sel: &scraper::Selector| -> String {
        doc.select(sel)
            .next()
            .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
            .unwrap_or_default()
    };

    let html_of = |sel: &scraper::Selector| -> String {
        doc.select(sel)
            .next()
            .map(|el| el.inner_html().trim().to_string())
            .unwrap_or_default()
    };

    // Real KWIC detail structure:
    // Title: .block-title-txt
    // Body:  #contentsHtml (quill editor content)
    // Sender: .portal-information-outgoing-division (contains "配信部署:" + dept name)
    // Date:  掲載期間 section — we extract from the first .contents-input-area with date-like text
    let title = text_of(&SEL_BLOCK_TITLE);
    let body_html = html_of(&SEL_CONTENTS_HTML);

    // Sender: extract department from .portal-information-outgoing-division
    let sender = {
        let raw = text_of(&SEL_OUTGOING_DIV);
        raw.replace("配信部署:", "").trim().to_string()
    };

    // Date: look for 掲載期間 section, then get the spans inside its .contents-input-area
    let date = {
        let mut found = String::new();
        for detail in doc.select(&SEL_CONTENTS_DETAIL) {
            if let Some(header) = detail.select(&SEL_HEADER_BOLD).next() {
                let header_text = header.text().collect::<Vec<_>>().join("");
                if header_text.contains("掲載期間") {
                    if let Some(input) = detail.select(&SEL_INPUT_AREA).next() {
                        found = input.text().collect::<Vec<_>>().join("").trim().to_string();
                    }
                    break;
                }
            }
        }
        found
    };

    // Attachments: .file-object elements → .downloadFile (name), .objectName (object path)
    let mut attachments = Vec::new();
    for fo in doc.select(&SEL_FILE_OBJECT) {
        let name = fo
            .select(&SEL_FILE_NAME)
            .next()
            .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
            .unwrap_or_default();
        let object_name = fo
            .select(&SEL_OBJECT_NAME)
            .next()
            .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let url = format!(
            "{}/portal/home/information/detail/download?downloadFileName={}&objectName={}&downloadMode=1",
            config::KWIC_BASE,
            urlencoding::encode(&name),
            urlencoding::encode(&object_name),
        );
        attachments.push(KwicAttachment { name, url });
    }

    // Strip <script> tags from body for safety
    let body_clean = {
        static RE_SCRIPT: LazyLock<regex::Regex> = LazyLock::new(|| {
            regex::Regex::new(r"(?is)<script[^>]*>.*?</script>").expect("valid regex")
        });
        compact_inline_images(&RE_SCRIPT.replace_all(&body_html, "")).into_owned()
    };

    KwicNotificationDetail {
        title,
        date,
        sender,
        body_html: body_clean,
        attachments,
    }
}

const INLINE_IMAGE_B64_LIMIT: usize = 48 * 1024;
const INLINE_IMAGE_MAX_EDGE: u32 = 1280;
const INLINE_IMAGE_PLACEHOLDER: &str =
    "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

pub(in crate::kwic_commands) fn compact_inline_images(html: &str) -> std::borrow::Cow<'_, str> {
    if !html.contains("data:image/") {
        return std::borrow::Cow::Borrowed(html);
    }
    let mut out = String::new();
    let mut cursor = 0;
    let mut changed = false;
    while let Some(rel) = html[cursor..].find("data:image/") {
        let start = cursor + rel;
        out.push_str(&html[cursor..start]);
        let after_scheme = start + "data:image/".len();
        let Some(semi_rel) = html[after_scheme..].find(";base64,") else {
            out.push_str("data:image/");
            cursor = after_scheme;
            changed = true;
            continue;
        };
        let b64_start = after_scheme + semi_rel + ";base64,".len();
        let b64_end = html[b64_start..]
            .find(|ch: char| {
                !(ch.is_ascii_alphanumeric()
                    || matches!(ch, '+' | '/' | '=')
                    || ch.is_ascii_whitespace())
            })
            .map(|len| b64_start + len)
            .unwrap_or(html.len());
        if b64_end - b64_start <= INLINE_IMAGE_B64_LIMIT {
            out.push_str(&html[start..b64_end]);
        } else {
            changed = true;
            let compact = recompress_inline_image(&html[b64_start..b64_end])
                .filter(|uri| uri.len() < b64_end - start)
                .unwrap_or_else(|| INLINE_IMAGE_PLACEHOLDER.to_string());
            out.push_str(&compact);
        }
        cursor = b64_end;
    }
    if !changed {
        return std::borrow::Cow::Borrowed(html);
    }
    out.push_str(&html[cursor..]);
    std::borrow::Cow::Owned(out)
}

fn recompress_inline_image(b64: &str) -> Option<String> {
    let cleaned: String = b64.chars().filter(|ch| !ch.is_ascii_whitespace()).collect();
    let bytes =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &cleaned).ok()?;
    let image = image::load_from_memory(&bytes).ok()?;
    let (width, height) = image.dimensions();
    let longest = width.max(height);
    let image = if longest > INLINE_IMAGE_MAX_EDGE {
        let scale = INLINE_IMAGE_MAX_EDGE as f32 / longest as f32;
        image.resize(
            ((width as f32) * scale).round().max(1.0) as u32,
            ((height as f32) * scale).round().max(1.0) as u32,
            image::imageops::FilterType::Triangle,
        )
    } else {
        image
    };
    let mut encoded = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 72);
    encoder.encode_image(&image).ok()?;
    Some(format!(
        "data:image/jpeg;base64,{}",
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, encoded)
    ))
}
