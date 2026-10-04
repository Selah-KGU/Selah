pub(in crate::timetable::ai_analysis) fn sanitize_ai_response_text(text: &str) -> String {
    let mut s = strip_tag_block_case_insensitive(text, "think");
    s = strip_token_case_insensitive(&s, "<think>");
    s = strip_token_case_insensitive(&s, "</think>");
    s.trim().to_string()
}

fn strip_tag_block_case_insensitive(text: &str, tag: &str) -> String {
    let mut out = text.to_string();
    let open_prefix = format!("<{}", tag.to_ascii_lowercase());
    let close_tag = format!("</{}>", tag.to_ascii_lowercase());

    loop {
        let lower = out.to_ascii_lowercase();
        let Some(start) = lower.find(&open_prefix) else {
            break;
        };

        let Some(open_end_rel) = lower[start..].find('>') else {
            out.truncate(start);
            break;
        };
        let content_start = start + open_end_rel + 1;

        if let Some(close_rel) = lower[content_start..].find(&close_tag) {
            let end = content_start + close_rel + close_tag.len();
            out.replace_range(start..end, "");
        } else {
            out.replace_range(start..out.len(), "");
            break;
        }
    }

    out
}

fn strip_token_case_insensitive(text: &str, token: &str) -> String {
    let mut out = text.to_string();
    let token_lower = token.to_ascii_lowercase();

    loop {
        let lower = out.to_ascii_lowercase();
        let Some(start) = lower.find(&token_lower) else {
            break;
        };
        let end = start + token.len();
        if end <= out.len() {
            out.replace_range(start..end, "");
        } else {
            break;
        }
    }

    out
}
