use super::super::{
    APPLE_CONTEXT_OVERHEAD_TOKENS, APPLE_CONTEXT_WINDOW_TOKENS, APPLE_PROMPT_TOKEN_BUDGET,
};
use super::json::matching_close;
use super::{
    apple_request_parts, compact_json_value, estimate_apple_tokens, fit_apple_request,
    recover_context_limit, trim_apple_text,
};
use crate::ai::ChatMessage;

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
    let (instructions, prompt) = apple_request_parts(&[message("user", "JSONで")], "{\"tools\":[");
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
    assert!(braces_balanced(&trimmed), "{trimmed}");
    assert!(trimmed.contains("current_week"));
    let start = trimmed.find('{').expect("schema");
    let end = matching_close(&trimmed, start).expect("close");
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
    assert!(braces_balanced(&trimmed), "{trimmed}");
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
    assert!(braces_balanced(&trimmed), "{trimmed}");
    assert!(!trimmed.contains("同じ形式"), "{trimmed}");
    assert!(trimmed.contains("末尾の質問") || trimmed.contains("先頭"));
}

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
