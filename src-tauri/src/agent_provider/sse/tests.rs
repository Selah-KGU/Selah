use super::*;
use crate::agent_provider::{
    clear_remote_cancel, gemini::receive_gemini, http_client, openai::receive_openai,
};
use serde_json::json;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// Frozen framing from the original provider loops. Keep the per-chunk decode
// and remainder copy so compatibility and the corruption reproducer are real.
fn legacy(chunks: &[&[u8]], mut on_data: impl FnMut(&str)) {
    let mut buffer = String::new();
    for bytes in chunks {
        buffer.push_str(&String::from_utf8_lossy(bytes));
        while let Some(line_end) = buffer.find('\n') {
            let line = buffer[..line_end].trim_end_matches('\r').to_string();
            buffer = buffer[line_end + 1..].to_string();
            if line == "data: [DONE]" {
                break;
            }
            if let Some(data) = line.strip_prefix("data: ") {
                on_data(data);
            }
        }
    }
}
fn decode(chunks: &[&[u8]]) -> Vec<String> {
    let mut decoder = Decoder::default();
    let mut events = Vec::new();
    for chunk in chunks {
        assert!(decoder.push(chunk, |data| {
            events.push(data.to_owned());
            true
        }));
    }
    events
}

#[test]
fn every_byte_boundary_preserves_unicode_bom_crlf_and_multiline_events() {
    let stream = concat!(
        "\u{feff}: keepalive\r\nretry: 2000\r\nid: 17\r\nevent: message\r\n",
        "data: 中文・日本語・한글 👩🏽‍💻 🌕\r\n\r\n",
        "data: {\rdata:\"answer\": \"資料 🌕\"\rdata: }\r\r",
        "data:  two leading spaces\n\n",
        "data\n\n",
        "Data: ignored\nunknown: ignored\n: comment\n\n",
        "data: last\r\n\r\n",
    )
    .as_bytes();
    let expected = [
        "中文・日本語・한글 👩🏽‍💻 🌕",
        "{\n\"answer\": \"資料 🌕\"\n}",
        " two leading spaces",
        "",
        "last",
    ];
    for offset in 0..=stream.len() {
        assert_eq!(
            decode(&[&stream[..offset], &stream[offset..]]),
            expected,
            "split {offset}"
        );
    }
    for size in 1..=31 {
        let chunks: Vec<_> = stream.chunks(size).collect();
        assert_eq!(decode(&chunks), expected, "chunk size {size}");
    }
    // Variable network-sized pieces also split fields and every UTF-8 sequence.
    let mut seed: usize = 19;
    for _ in 0..40 {
        let mut offset = 0;
        let mut chunks = Vec::new();
        while offset < stream.len() {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let end = (offset + 1 + seed % 29).min(stream.len());
            chunks.push(&stream[offset..end]);
            offset = end;
        }
        assert_eq!(decode(&chunks), expected);
    }
}

#[test]
fn original_decoder_corrupts_split_hanzi_and_emoji_while_new_one_preserves_json() {
    let payload = json!({"choices":[{"delta":{"content":"中文 日本語 🌕 👩🏽‍💻"}}]}).to_string();
    let wire = format!("data: {payload}\n\n");
    let chunks: Vec<_> = wire.as_bytes().chunks(1).collect();
    let mut old = Vec::new();
    legacy(&chunks, |data| old.push(data.to_owned()));
    assert_ne!(old, [payload.clone()]);
    assert!(old[0].contains('\u{fffd}'));
    assert_eq!(decode(&chunks), [payload]);
}

#[test]
fn invalid_utf8_has_chunk_independent_replacement_and_only_initial_bom_is_stripped() {
    let stream = b"\xef\xbb\xbfdata: A\xf0\x9f\x8c\x95\xff\xe3\x81Z\r\n\r\ndata: \xef\xbb\xbfB\n\n";
    let expected = vec![
        String::from_utf8_lossy(b"A\xf0\x9f\x8c\x95\xff\xe3\x81Z").into_owned(),
        "\u{feff}B".into(),
    ];
    for split in 0..=stream.len() {
        assert_eq!(decode(&[&stream[..split], &stream[split..]]), expected);
    }
    assert_eq!(decode(&stream.chunks(1).collect::<Vec<_>>()), expected);
}

#[test]
fn incomplete_events_wait_for_blank_line_and_are_not_dispatched_at_eof() {
    for wire in [
        "data: no newline",
        "data: one newline\n",
        "data: first\ndata: second\n",
        "data: after cr\r",
        "data: \n",
    ] {
        assert!(decode(&[wire.as_bytes()]).is_empty(), "{wire:?}");
    }
    let mut decoder = Decoder::default();
    let mut events = Vec::new();
    decoder.push(b"data: complete\r", |data| {
        events.push(data.to_owned());
        true
    });
    assert!(events.is_empty());
    decoder.push(b"\n", |data| {
        events.push(data.to_owned());
        true
    });
    assert!(events.is_empty());
    decoder.push(b"\r", |data| {
        events.push(data.to_owned());
        true
    });
    assert_eq!(events, ["complete"]);
    decoder.push(b"\ndata: unfinished\n", |data| {
        events.push(data.to_owned());
        true
    });
    assert_eq!(events, ["complete"]);
}

#[test]
fn stop_prevents_coalesced_and_later_events_without_retaining_a_remainder() {
    let mut decoder = Decoder::default();
    let mut events = Vec::new();
    assert!(!decoder.push(
        b"data: first\n\ndata: [DONE]\n\ndata: forbidden\n\n",
        |data| {
            events.push(data.to_owned());
            data != "[DONE]"
        }
    ));
    assert_eq!(events, ["first", "[DONE]"]);
    assert!(!decoder.push(b"data: later\n\n", |_| panic!("stopped decoder emitted")));
    assert!(decoder.partial_line.is_empty());
    assert!(decoder.event.data.is_empty());
}

#[test]
fn normal_provider_payloads_match_original_and_reuse_buffers_in_a_large_batch() {
    let mut wire = String::new();
    let mut expected = Vec::new();
    for i in 0..4000 {
        let payload = json!({"choices":[{"delta":{"content":format!("全文 🌕 {i}")}}]}).to_string();
        expected.push(payload.clone());
        wire.push_str(&format!("data: {payload}\r\n\r\n"));
    }
    let mut old = Vec::new();
    legacy(&[wire.as_bytes()], |data| old.push(data.to_owned()));
    assert_eq!(old, expected);
    assert_eq!(decode(&[wire.as_bytes()]), expected);
    let mut decoder = Decoder::default();
    let mut pointer = None;
    let mut count = 0;
    decoder.push(wire.as_bytes(), |data| {
        if let Some(pointer) = pointer {
            assert_eq!(data.as_ptr() as usize, pointer);
        } else {
            pointer = Some(data.as_ptr() as usize);
        }
        count += 1;
        true
    });
    assert_eq!(count, 4000);
    // All complete lines borrow the incoming batch, not a copied full batch.
    assert_eq!(decoder.partial_line.capacity(), 0);
    assert!(decoder.event.data.capacity() < 256);
    assert!(decoder.event.data.is_empty());
    decoder.push(b"data: part", |_| true);
    decoder.push(b"ial\n\n", |_| true);
    let partial_pointer = decoder.partial_line.as_ptr();
    decoder.push(b"data: part", |_| true);
    assert_eq!(decoder.partial_line.as_ptr(), partial_pointer);
}

enum End {
    Complete,
    Hold,
    Truncate,
}
async fn http_response(
    chunks: Vec<Vec<u8>>,
    end: End,
) -> (reqwest::Response, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut request = Vec::new();
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let mut bytes = [0; 4096];
            let count = tokio::time::timeout(Duration::from_secs(3), socket.read(&mut bytes))
                .await
                .unwrap()
                .unwrap();
            assert_ne!(count, 0);
            request.extend_from_slice(&bytes[..count]);
        }
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await.unwrap();
        for chunk in chunks {
            socket
                .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                .await
                .unwrap();
            socket.write_all(&chunk).await.unwrap();
            socket.write_all(b"\r\n").await.unwrap();
            socket.flush().await.unwrap();
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        match end {
            End::Complete => socket.write_all(b"0\r\n\r\n").await.unwrap(),
            End::Truncate => {}
            End::Hold => {
                // No HTTP end marker: terminal SSE must release the connection.
                let mut byte = [0];
                let closed = tokio::time::timeout(Duration::from_secs(3), socket.read(&mut byte))
                    .await
                    .unwrap();
                assert!(
                    matches!(closed, Ok(0) | Err(_)),
                    "SSE terminal left connection open"
                );
            }
        }
    });
    let response = tokio::time::timeout(Duration::from_secs(3), http_client().get(url).send())
        .await
        .unwrap()
        .unwrap();
    (response, server)
}
fn fragmented(wire: &str) -> Vec<Vec<u8>> {
    // Cut inside every multibyte sequence, and inside CRLF/field delimiters.
    wire.as_bytes()
        .chunks(7)
        .map(|chunk| chunk.to_vec())
        .collect()
}
fn generation() -> String {
    format!("sse-{}", uuid::Uuid::new_v4())
}

#[tokio::test(flavor = "current_thread")]
async fn actual_openai_receiver_preserves_unicode_reasoning_multiline_and_closes_at_done() {
    let text = "中文・日本語 👩🏽‍💻 🌕";
    let payload = json!({"choices":[{"delta":{"content":text,"reasoning_content":"推論 🌕"}}]});
    let multi = serde_json::to_string_pretty(&payload)
        .unwrap()
        .lines()
        .map(|line| format!("data:{line}\r\n"))
        .collect::<String>();
    let prefix = format!("\u{feff}: keepalive\r\nevent: message\r\n{multi}\r\n");
    let mut chunks = fragmented(&prefix);
    // Same HTTP chunk includes content after DONE; none may be emitted.
    chunks.push(
        format!(
            "data: [DONE]\n\ndata: {}\n\n",
            json!({"choices":[{"delta":{"content":"forbidden"}}]})
        )
        .into_bytes(),
    );
    let (response, server) = http_response(chunks, End::Hold).await;
    let mut emitted = Vec::new();
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        receive_openai(response, &generation(), |text, think| {
            emitted.push((text.to_owned(), think))
        }),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result, text);
    assert_eq!(
        emitted,
        [(text.to_owned(), false), ("推論 🌕".into(), true)]
    );
    server.await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn actual_gemini_receiver_preserves_all_parts_and_stops_on_tools() {
    for call in [
        json!({"candidates":[{"content":{"parts":[{"functionCall":{"name":"read_downloaded_file","args":{"path":"/tmp/資料🌕.md"}}}]}}]}),
        json!({"candidates":[{"content":{},"finishReason":"MALFORMED_FUNCTION_CALL","finishMessage":"Malformed function call: call:read_downloaded_file {\"path\":\"/tmp/資料🌕.md\"}"}]}),
    ] {
        let first = json!({"candidates":[{"content":{"parts":[{"text":"中文 👩🏽‍💻"},{"text":"日本語 🌕"},{"text":null}]}}]});
        let mut chunks = fragmented(&format!("data:{first}\r\n\r\n"));
        chunks.push(
            format!(
                "data: {call}\n\ndata: {}\n\n",
                json!({"candidates":[{"content":{"parts":[{"text":"forbidden"}]}}]})
            )
            .into_bytes(),
        );
        let (response, server) = http_response(chunks, End::Hold).await;
        let mut emitted = Vec::new();
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            receive_gemini(response, &generation(), |text, think| {
                emitted.push((text.to_owned(), think))
            }),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(result.starts_with("call:read_downloaded_file"));
        assert!(result.contains("/tmp/資料🌕.md"));
        assert_eq!(emitted, [("中文 👩🏽‍💻日本語 🌕".into(), false)]);
        server.await.unwrap();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn eof_discards_incomplete_event_and_transport_failure_keeps_its_error() {
    let first = json!({"choices":[{"delta":{"content":"完整 🌕"}}]});
    let incomplete = json!({"choices":[{"delta":{"content":"incomplete"}}]});
    let (response, server) = http_response(
        vec![format!("data: {first}\n\ndata: {incomplete}\n").into_bytes()],
        End::Complete,
    )
    .await;
    let mut emitted = Vec::new();
    assert_eq!(
        receive_openai(response, &generation(), |text, _| emitted
            .push(text.to_owned()))
        .await
        .unwrap(),
        "完整 🌕"
    );
    assert_eq!(emitted, ["完整 🌕"]);
    server.await.unwrap();
    let (response, server) = http_response(
        vec![format!("data: {first}\n\n").into_bytes()],
        End::Truncate,
    )
    .await;
    let error = receive_openai(response, &generation(), |_, _| {})
        .await
        .unwrap_err();
    assert!(error.starts_with("ストリーム読み取り失敗:"));
    server.await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_in_first_callback_suppresses_the_rest_of_a_coalesced_chunk() {
    let first = json!({"choices":[{"delta":{"content":"first"}}]});
    let second = json!({"choices":[{"delta":{"content":"forbidden"}}]});
    let (response, server) = http_response(
        vec![format!("data: {first}\n\ndata: {second}\n\n").into_bytes()],
        End::Hold,
    )
    .await;
    let id = generation();
    let mut emitted = Vec::new();
    let result = receive_openai(response, &id, |text, _| {
        emitted.push(text.to_owned());
        crate::agent_provider::cancel_remote(&id);
    })
    .await
    .unwrap();
    clear_remote_cancel(&id);
    assert_eq!(result, "first");
    assert_eq!(emitted, ["first"]);
    server.await.unwrap();
}

#[test]
#[ignore = "manual framing-only benchmark; excludes JSON, HTTP, model, UI and GPU"]
fn benchmark_sse_coalesced_batch() {
    for events in [2000, 8000] {
        let wire = "data: {\"choices\":[{\"delta\":{\"content\":\"全文 🌕\"}}]}\n\n".repeat(events);
        let mut old_times = Vec::new();
        let mut new_times = Vec::new();
        for i in 0..10 {
            let old = || {
                let begin = Instant::now();
                let mut count = 0;
                legacy(&[wire.as_bytes()], |data| {
                    std::hint::black_box(data);
                    count += 1;
                });
                assert_eq!(count, events);
                begin.elapsed().as_secs_f64() * 1000.0
            };
            let new = || {
                let begin = Instant::now();
                let mut count = 0;
                let mut decoder = Decoder::default();
                decoder.push(wire.as_bytes(), |data| {
                    std::hint::black_box(data);
                    count += 1;
                    true
                });
                assert_eq!(count, events);
                begin.elapsed().as_secs_f64() * 1000.0
            };
            let (before, after) = if i % 2 == 0 {
                (old(), new())
            } else {
                let after = new();
                (old(), after)
            };
            if i > 0 {
                old_times.push(before);
                new_times.push(after);
            }
        }
        old_times.sort_by(f64::total_cmp);
        new_times.sort_by(f64::total_cmp);
        println!("{events} events / {} bytes: old {:.3} ms -> new {:.3} ms (nine-run median, unoptimized test profile, framing only)", wire.len(), old_times[4], new_times[4]);
    }
}
