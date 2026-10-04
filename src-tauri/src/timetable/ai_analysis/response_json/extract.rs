pub(in crate::timetable::ai_analysis) fn extract_json_from_local_response(
    response: &str,
) -> Result<String, String> {
    let lower = response.to_ascii_lowercase();
    let mut segments: Vec<&str> = Vec::new();
    let mut pos: usize = 0;
    let mut had_think = false;

    loop {
        match lower[pos..].find("<think") {
            Some(start_rel) => {
                had_think = true;
                let start = pos + start_rel;
                if start > pos {
                    segments.push(&response[pos..start]);
                }
                let after_open = match lower[start..].find('>') {
                    Some(i) => start + i + 1,
                    None => break,
                };
                match lower[after_open..].find("</think>") {
                    Some(close_rel) => {
                        pos = after_open + close_rel + "</think>".len();
                    }
                    None => {
                        break;
                    }
                }
            }
            None => {
                segments.push(&response[pos..]);
                break;
            }
        }
    }

    let clean = segments.join("");
    let trimmed = clean.trim();

    if trimmed.is_empty() {
        if had_think {
            return Err(
                "AIモデルが推論のみで出力トークンを使い切り、JSONが生成されませんでした。\
                モデルのmax_tokensを増やすか、プロンプトを短くしてください。"
                    .into(),
            );
        } else {
            return Err("AIモデルから空の応答が返されました。".into());
        }
    }

    log::debug!(
        "extract_json_from_local_response: response {}→{} chars (think={})",
        response.len(),
        trimmed.len(),
        had_think
    );

    Ok(extract_json_from_response(trimmed).to_string())
}

pub(in crate::timetable::ai_analysis) fn extract_json_from_response(text: &str) -> &str {
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```") {
            return after[..end].trim();
        }
    }
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        if let Some(end) = after.find("```") {
            return after[..end].trim();
        }
    }
    if let Some(start) = text.find('{') {
        if let Some(end) = find_matching_brace(text, start) {
            return &text[start..=end];
        }
        if let Some(end) = text.rfind('}') {
            if end > start {
                return &text[start..=end];
            }
        }
        return &text[start..];
    }
    text.trim()
}

fn scan_json_structure(s: &str, offset: usize, mut f: impl FnMut(usize, char)) -> bool {
    let mut in_string = false;
    let mut escape_next = false;

    for (i, ch) in s[offset..].char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }
        if ch == '\\' && in_string {
            escape_next = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        f(offset + i, ch);
    }
    in_string
}

fn find_matching_brace(text: &str, start: usize) -> Option<usize> {
    let mut depth: i32 = 0;
    let mut result = None;
    scan_json_structure(text, start, |i, ch| {
        if result.is_some() {
            return;
        }
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    result = Some(i);
                }
            }
            _ => {}
        }
    });
    result
}

pub(in crate::timetable::ai_analysis) fn repair_truncated_json(input: &str) -> String {
    let s = input.trim_end();

    if serde_json::from_str::<serde_json::Value>(s).is_ok() {
        return s.to_string();
    }

    let mut cleaned = s.to_string();
    loop {
        let t = cleaned.trim_end();
        if t.is_empty() {
            break;
        }
        let last = t.as_bytes()[t.len() - 1];
        if last == b':' || last == b',' || last == b'\\' {
            cleaned = t[..t.len() - 1].to_string();
            continue;
        }
        break;
    }

    let attempt1 = close_json_brackets(&cleaned);
    if serde_json::from_str::<serde_json::Value>(&attempt1).is_ok() {
        return attempt1;
    }

    let commas = find_non_string_commas(&cleaned);
    for &pos in commas.iter().rev().take(20) {
        let candidate = close_json_brackets(&cleaned[..pos]);
        if serde_json::from_str::<serde_json::Value>(&candidate).is_ok() {
            return candidate;
        }
    }

    attempt1
}

fn close_json_brackets(s: &str) -> String {
    let mut stack: Vec<char> = Vec::new();
    let in_string = scan_json_structure(s, 0, |_, ch| match ch {
        '{' => stack.push('{'),
        '[' => stack.push('['),
        '}' => {
            if stack.last() == Some(&'{') {
                stack.pop();
            }
        }
        ']' => {
            if stack.last() == Some(&'[') {
                stack.pop();
            }
        }
        _ => {}
    });

    let mut result = s.to_string();
    if in_string {
        result.push('"');
    }
    for &bracket in stack.iter().rev() {
        match bracket {
            '{' => result.push('}'),
            '[' => result.push(']'),
            _ => {}
        }
    }
    result
}

fn find_non_string_commas(s: &str) -> Vec<usize> {
    let mut positions = Vec::new();
    scan_json_structure(s, 0, |i, ch| {
        if ch == ',' {
            positions.push(i);
        }
    });
    positions
}
