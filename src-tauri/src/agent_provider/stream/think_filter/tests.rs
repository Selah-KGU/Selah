use super::*;
use std::sync::{Arc, Mutex};

#[path = "before.rs"]
mod before;

type Callbacks = Vec<(String, bool)>;

#[derive(Clone, Copy)]
enum Step<'a> {
    Feed(&'a str, bool),
    Flush,
}
fn run(steps: &[Step<'_>], previous: bool, panic_once: bool) -> (Callbacks, usize) {
    let output = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&output);
    let mut fail = panic_once;
    let callback = move |chunk: &str, think: bool| {
        captured.lock().unwrap().push((chunk.to_owned(), think));
        if fail {
            fail = false;
            panic!("injected thinking callback panic");
        }
    };
    let (mut feed, mut flush) = if previous {
        before::ThinkFilter::wrap_with_flush(callback)
    } else {
        ThinkFilter::wrap_with_flush(callback)
    };
    let mut panics = 0;
    for step in steps {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match step {
            Step::Feed(text, think) => feed(text, *think),
            Step::Flush => flush(),
        }));
        panics += usize::from(result.is_err());
    }
    drop(feed);
    drop(flush);
    let output = Arc::try_unwrap(output).unwrap().into_inner().unwrap();
    (output, panics)
}

#[test]
fn exact_callbacks_match_at_every_two_cut_partition_of_tags_utf8_and_malformed_prefixes() {
    let corpus = [
        "",
        "plain 完全な回答 中文 한국어 👩🏽‍💻\n\r\0\"引用\"",
        "<think>推論 🌕</think>visible",
        "before<thought>hidden</thought>after",
        "before<thinking>hidden</thinking>after",
        "<thoughtThe user wants tools</thought> answer",
        "<think unfinished",
        "before<thought>unclosed reasoning 中文",
        "literal <thinkx value</thinking> end",
        "<think>a</think><thought>b</thought><thinking>c</thinking>全文",
        "a<thinking><think>nested</think>text</thinking>z",
        "x</thought>y<th>partial<think>z</thi",
    ];
    for text in corpus {
        let mut boundaries: Vec<_> = text.char_indices().map(|(i, _)| i).collect();
        boundaries.push(text.len());
        for (position, &first) in boundaries.iter().enumerate() {
            for &second in &boundaries[position..] {
                let steps = [
                    Step::Feed(&text[..first], false),
                    Step::Feed(&text[first..second], false),
                    Step::Feed(&text[second..], false),
                    Step::Flush,
                    Step::Flush,
                ];
                assert_eq!(
                    run(&steps, false, false),
                    run(&steps, true, false),
                    "{text:?} {first}/{second}"
                );
            }
        }
        let mut steps = Vec::new();
        for pair in boundaries.windows(2) {
            steps.push(Step::Feed(&text[pair[0]..pair[1]], false));
        }
        steps.push(Step::Flush);
        assert_eq!(run(&steps, false, false), run(&steps, true, false));
    }
}

#[test]
fn upstream_thinking_flushes_and_feed_after_flush_keep_the_same_state_and_callback_order() {
    for prefix in ["", "a<thou", "<think>", "abcdefghi", "全文 👩🏽‍💻"] {
        let steps = [
            Step::Feed(prefix, false),
            Step::Feed("native <think>thought</think> 🌕", true),
            Step::Feed("", true), // Upstream thinking emits even an empty chunk.
            Step::Feed("ght>推論</thought>回答", false),
            Step::Flush,
            Step::Feed("<thinking>続き", false),
            Step::Flush,
            Step::Feed("</thinking>last", false),
            Step::Flush,
            Step::Flush,
        ];
        assert_eq!(run(&steps, false, false), run(&steps, true, false));
    }
}

#[test]
fn callback_panics_preserve_buffer_mutation_order_and_poisoned_lock_recovery() {
    for first in [
        "plain long prefix before more text",
        "before<thinking>reasoning</thinking>after",
        "<think>long reasoning prefix that is emitted",
        "<thought>hidden</thought>done",
        "short", // The first callback will be in flush's mem::take path.
    ] {
        let steps = [
            Step::Feed(first, false),
            Step::Flush,
            Step::Feed(" recovery</think></thinking>tail", false),
            Step::Feed("native thinking after poisoned lock", true),
            Step::Flush,
        ];
        let old = run(&steps, true, true);
        let new = run(&steps, false, true);
        assert_eq!(new, old, "{first}");
        assert_eq!(new.1, 1);
    }
}

#[test]
fn normal_drain_callbacks_borrow_the_filter_buffer_before_it_is_mutated() {
    for (text, thinking, expected_thinking) in [
        ("visible prefix long enough for holdback", false, false),
        ("hidden prefix long enough for holdback", true, true),
        ("visible<think>held", false, false),
        ("hidden</think>held", true, true),
    ] {
        let buffer = text.to_owned();
        let start = buffer.as_ptr() as usize;
        let end = start + buffer.len();
        let calls = Arc::new(Mutex::new(0));
        let captured = Arc::clone(&calls);
        let mut filter = ThinkFilter {
            inner: move |chunk: &str, think: bool| {
                let pointer = chunk.as_ptr() as usize;
                assert!(!chunk.is_empty());
                assert!(pointer >= start && pointer + chunk.len() <= end);
                assert_eq!(think, expected_thinking);
                *captured.lock().unwrap() += 1;
            },
            buf: buffer,
            in_think: thinking,
        };
        filter.drain(false);
        assert_eq!(*calls.lock().unwrap(), 1);
    }
}
