use super::*;
use std::borrow::Cow;

#[path = "before.rs"]
mod before;

fn compare(text: &str) {
    assert_eq!(
        find_start(text),
        before::find_start(text),
        "start: {text:?}"
    );
    assert_eq!(has_any(text), before::has_any(text), "visible: {text:?}");
    assert_eq!(
        agent_text::contains_leading_pseudo_tool_call(text),
        before::contains_leading(text),
        "leading: {text:?}"
    );
    assert_eq!(
        agent_text::visible_without_thinking(text).as_ref(),
        before::visible(text),
        "thinking: {text:?}"
    );
    assert_eq!(agent_text::strip_think(text), before::visible(text));
}

#[test]
fn all_markers_preserve_case_wrappers_and_earliest_byte_boundaries() {
    let prefixes = [
        "",
        " ",
        "  ",
        "\r\n",
        "\u{a0}\u{2003}",
        "`",
        "```",
        "<",
        "‹",
        "〈",
        " ‹ `\u{3000}〈 < ",
        "(",
        "[",
        "{",
        "\"",
        "'",
        "「",
        "『",
        ")",
        "]",
        "}",
        ">",
        "›",
        "〉",
        "正文",
        "正文 ",
        "正文  ",
        "正文<",
        "正文‹ ",
        "正文( ",
        "正文) ",
        "🙂\t\u{2003}",
        "callback: ",
        "xcall:",
        "\0",
        "\u{200b}",
    ];
    for marker in agent_text::PSEUDO_TOOL_MARKERS {
        let mixed: String = marker
            .chars()
            .enumerate()
            .map(|(i, ch)| {
                if i % 2 == 0 {
                    ch.to_ascii_uppercase()
                } else {
                    ch
                }
            })
            .collect();
        for variant in [marker.to_string(), marker.to_ascii_uppercase(), mixed] {
            for prefix in prefixes {
                for suffix in [
                    "",
                    "read_file(path=\"文🙂\")",
                    " ordinary text",
                    "\n<call:next>",
                ] {
                    compare(&format!("{prefix}{variant}{suffix}"));
                }
            }
        }
    }
    for (text, expected) in [
        ("正文 call:x", Some("正文 ".len())),
        ("正文  call:x", Some("正文 ".len())),
        ("正文<call:x", Some("正文<".len())),
        ("正文 ‹call:x", Some("正文 ".len())),
        (" ‹ `〈 call:x", Some(0)),
        ("( call:x", Some(1)),
        ("xcall:x", None),
        ("callback:x", None),
        ("🙂call:x", None),
        ("calligraphy", None),
    ] {
        assert_eq!(find_start(text), expected, "{text:?}");
    }
}

#[test]
fn fragmented_utf8_and_thinking_inputs_match_frozen_detectors() {
    let corpus = [
        "正文\u{a0} ‹TaSk_CaLl：read_file(path=\"🙂\")› 末尾",
        "before<thinking>call:hidden</thinking>after call:visible",
        "<think>hidden</thought>‹call:x",
        "<thoughtmalformed call:hidden",
        "<think><think>nested</think>call:visible</think>",
        "<thinking>hidden</thinking><thought>hidden</thought>normal",
        "</thinking>visible function_call:x\r\n",
        "<THINK>case-sensitive</THINK>CALL:x",
        "<think>hidden</think>call:visible<thoughtunfinished",
        "\0\r\n漢字e\u{301}🙂〈`\u{3000}TOOL_CALL：x",
    ];
    for text in corpus {
        compare(text);
        for cut in text.char_indices().map(|(i, _)| i).chain([text.len()]) {
            compare(&text[..cut]);
            compare(&text[cut..]);
        }
    }
}

#[test]
fn deterministic_mixed_text_matches_frozen_detectors() {
    let pieces = [
        "word",
        "call:",
        "CALL：",
        "call ",
        "tool_call:",
        "TaSk_CaLl：",
        "function_call:",
        "callback:",
        "task_",
        "call",
        "c",
        " ",
        "\r",
        "\n",
        "\t",
        "\u{a0}",
        "\u{2003}",
        "\u{200b}",
        "\u{3000}",
        "`",
        "<",
        "‹",
        "〈",
        "(",
        "[",
        "{",
        "\"",
        "'",
        "「",
        "『",
        ")",
        "]",
        "}",
        ">",
        "›",
        "〉",
        "🙂",
        "漢字",
        "e\u{301}",
        "\0",
        ":",
        "：",
        "<think>",
        "<thinking",
        "<thought>",
        "</think>",
        "</thinking>",
        "</thought>",
    ];
    let mut seed = 0x8b72_a941_035f_cde6u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed as usize
    };
    for _ in 0..4_000 {
        let count = next() % 50;
        let mut text = String::new();
        for _ in 0..count {
            text.push_str(pieces[next() % pieces.len()]);
        }
        compare(&text);
    }
}

#[test]
fn untagged_text_is_borrowed_and_long_wrapper_runs_are_scanned_once() {
    let ordinary = "本文🙂 no tags\r\n".repeat(10_000);
    match agent_text::visible_without_thinking(&ordinary) {
        Cow::Borrowed(text) => {
            assert_eq!(text.as_ptr(), ordinary.as_ptr());
            assert_eq!(text.len(), ordinary.len());
        }
        Cow::Owned(_) => panic!("untagged input was copied"),
    }
    assert_eq!(find_start(&ordinary), None);
    let wrappers = " ‹`〈<\u{3000}".repeat(20_000);
    assert_eq!(find_start(&wrappers), None);
    assert_eq!(find_start(&format!("{wrappers}CaLl：x")), Some(0));
    let prefixed = format!("正文{wrappers}CALL:x");
    assert_eq!(find_start(&prefixed), Some("正文 ".len()));
    for length in [0, 1, 2, 7, 32, 128] {
        compare(&format!("正文{}call:x", " ‹`〈<\u{3000}".repeat(length)));
    }
    assert!(matches!(
        agent_text::visible_without_thinking("before<think>hidden</think>after"),
        Cow::Owned(_)
    ));
}
