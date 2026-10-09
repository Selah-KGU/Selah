use super::*;
use crate::ai::ImagePart;
use serde_json::json;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[path = "legacy.rs"]
mod legacy;

fn config() -> AiConfig {
    AiConfig {
        ai_enabled: true,
        provider: "openai".into(),
        model: "model /? 日本語".into(),
        api_key: "placeholder".into(),
        base_url: "http://127.0.0.1:8080///".into(),
        max_tokens: 8192,
        temperature: 0.7,
        ..Default::default()
    }
}
fn messages(large: bool) -> Vec<ChatMessage> {
    let count = if large { 20_000 } else { 1 };
    vec![
        ChatMessage {
            role: "system".into(),
            content: "指示 <call:tool>(x) task_call: ‹　›\n".repeat(count),
            images: Vec::new(),
        },
        ChatMessage {
            role: "user".into(),
            content: "発話 👩🏽‍💻\n\"quote\" \\ (x) <y>".repeat(count),
            images: vec![ImagePart {
                mime: "image/png".into(),
                data_base64: "QUJD".repeat(count * 20),
            }],
        },
        ChatMessage {
            role: "assistant".into(),
            content: "全文の回答 🌕".repeat(count),
            images: Vec::new(),
        },
        ChatMessage {
            role: "system".into(),
            content: "追加指示 tool_call: function_call: call:".into(),
            images: Vec::new(),
        },
        ChatMessage {
            role: "tool".into(),
            content: "資料の内容".into(),
            images: Vec::new(),
        },
    ]
}
fn assert_request_eq(actual: reqwest::Request, expected: reqwest::Request) {
    assert_eq!(actual.method(), expected.method());
    assert_eq!(actual.url(), expected.url());
    assert_eq!(actual.headers(), expected.headers());
    assert_eq!(
        actual.body().unwrap().as_bytes(),
        expected.body().unwrap().as_bytes()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn request_workers_keep_complete_original_bytes_headers_urls_and_retry_body_ownership() {
    for large in [false, true] {
        for (tokens, temperature) in [(0, 0.0), (32768, 0.7), (8192, f32::NAN)] {
            let mut cfg = config();
            cfg.max_tokens = tokens;
            cfg.temperature = temperature;
            let input = messages(large);
            let openai = requests::openai(&cfg, input.clone()).await.unwrap();
            let original = openai.try_clone().unwrap().build().unwrap();
            let retry = openai.try_clone().unwrap().build().unwrap();
            assert_eq!(
                original.body().unwrap().as_bytes().unwrap().as_ptr(),
                retry.body().unwrap().as_bytes().unwrap().as_ptr()
            );
            assert_request_eq(
                original,
                legacy::build_openai(cfg.clone(), input.clone())
                    .build()
                    .unwrap(),
            );
            assert_request_eq(
                requests::gemini(&cfg, input.clone())
                    .await
                    .unwrap()
                    .build()
                    .unwrap(),
                legacy::build_gemini(cfg, input).build().unwrap(),
            );
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn response_workers_preserve_full_text_parts_reasoning_first_gemini_part_and_errors() {
    let full = "応答 👩🏽‍💻\n\"quote\" \\".repeat(20_000);
    let openai = [
        json!({"choices":[{"message":{"content":full},"finish_reason":"stop"}]}).to_string(),
        json!({"choices":[{"message":{"content":[{"text":"first"},{"content":full},{"text":3,"content":"last"},{}]}}]}).to_string(),
        json!({"choices":[{"message":{"content":null,"reasoning":full}}]}).to_string(),
        json!({"choices":[{"message":{"content":"　\t","reasoning":full}}]}).to_string(),
        json!({"choices":[{"message":{},"finish_reason":"length"}]}).to_string(),
        json!({"choices":[]}).to_string(), "{}".into(), "broken JSON".into(),
        json!({"choices":[{"message":{"reasoning":4}}]}).to_string(),
    ];
    for raw in openai {
        assert_eq!(
            processing::openai(reqwest::StatusCode::OK, raw.clone()).await,
            legacy::parse_openai(reqwest::StatusCode::OK, &raw)
        );
    }
    let gemini = [
        json!({"candidates":[{"content":{"parts":[{"text":full},{"text":"second part is ignored by existing non-streaming API"}]}},{"content":{"parts":[{"text":"second candidate"}]}}]}).to_string(),
        json!({"candidates":[{"content":{"parts":[{"text":""}]}}]}).to_string(),
        json!({"candidates":[{"content":{"parts":[]}}]}).to_string(),
        json!({"candidates":[{}]}).to_string(), "{}".into(), "broken JSON".into(),
        json!({"candidates":[{"content":{"parts":[{"text":null}]}}]}).to_string(),
    ];
    for raw in gemini {
        assert_eq!(
            processing::gemini(reqwest::StatusCode::OK, raw.clone()).await,
            legacy::parse_gemini(reqwest::StatusCode::OK, &raw)
        );
    }
    for status in [
        reqwest::StatusCode::BAD_REQUEST,
        reqwest::StatusCode::TOO_MANY_REQUESTS,
        reqwest::StatusCode::INTERNAL_SERVER_ERROR,
    ] {
        for raw in [
            "gateway failed".to_string(),
            json!({"error":{"message":"  original error  "}}).to_string(),
        ] {
            assert_eq!(
                processing::openai(status, raw.clone()).await,
                legacy::parse_openai(status, &raw)
            );
            assert_eq!(
                processing::gemini(status, raw.clone()).await,
                legacy::parse_gemini(status, &raw)
            );
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn delayed_preparation_moves_owned_inputs_off_executor_and_dropped_wait_does_not_continue_to_http(
) {
    let input = messages(true);
    let text_pointer = input[1].content.as_ptr() as usize;
    let image_pointer = input[1].images[0].data_base64.as_ptr() as usize;
    let caller = std::thread::current().id();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let prepared = tokio::spawn(requests::prepare(config(), input, move |cfg, input| {
        assert_ne!(std::thread::current().id(), caller);
        assert_eq!(input[1].content.as_ptr() as usize, text_pointer);
        assert_eq!(
            input[1].images[0].data_base64.as_ptr() as usize,
            image_pointer
        );
        entered.send(()).unwrap();
        released.recv_timeout(Duration::from_secs(10)).unwrap();
        requests::build_openai(cfg, input)
    }));
    started.await.unwrap();
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    assert!(!prepared.is_finished());
    release.send(()).unwrap();
    let actual = prepared.await.unwrap().unwrap().build().unwrap();
    assert_request_eq(
        actual,
        legacy::build_openai(config(), messages(true))
            .build()
            .unwrap(),
    );
    let panic = requests::prepare(config(), Vec::new(), |_, _| -> reqwest::RequestBuilder {
        panic!("encoding worker interrupted")
    })
    .await;
    assert!(matches!(panic, Err(error) if error.starts_with("AIリクエスト準備失敗:")));

    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let (finished, completed) = tokio::sync::oneshot::channel();
    let continued = Arc::new(AtomicBool::new(false));
    let continuing = continued.clone();
    let task = tokio::spawn(async move {
        let request = requests::prepare(config(), messages(true), move |cfg, input| {
            entered.send(()).unwrap();
            released.recv_timeout(Duration::from_secs(10)).unwrap();
            let request = requests::build_gemini(cfg, input);
            finished.send(()).unwrap();
            request
        })
        .await?;
        continuing.store(true, Ordering::SeqCst);
        Ok::<_, String>(request)
    });
    started.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), completed)
        .await
        .unwrap()
        .unwrap();
    assert!(!continued.load(Ordering::SeqCst));
}

#[test]
fn gemini_status_error_truncation_keeps_unicode_boundaries() {
    // Byte 150 falls inside a three-byte character. The preceding branch
    // skips this all-whitespace message, so this reaches Gemini's status case.
    let message = format!("{}\u{85}{}", "　".repeat(49), "\u{2003}".repeat(200));
    let raw = json!({"error":{"status":"RESOURCE_EXHAUSTED","message":message}}).to_string();
    assert_eq!(
        truncate_error(&raw),
        format!(
            "RESOURCE_EXHAUSTED: {}",
            message.chars().take(150).collect::<String>()
        )
    );
}

#[tokio::test(flavor = "current_thread")]
async fn real_non_streaming_http_retries_keep_full_encoded_input_headers_and_decoded_output() {
    use std::io::{BufRead, Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut cfg = config();
    cfg.base_url = format!("http://{}///", listener.local_addr().unwrap());
    let expected_request = legacy::build_openai(cfg.clone(), messages(true))
        .build()
        .unwrap();
    let expected_body = expected_request
        .body()
        .unwrap()
        .as_bytes()
        .unwrap()
        .to_vec();
    let expected_answer = "ローカル検証の完全な回答 🌕\n".repeat(5000);
    let response = json!({"choices":[{"message":{"content":[{"text":expected_answer}]},"finish_reason":"stop"}]}).to_string();
    let server = std::thread::spawn(move || {
        let mut captured = Vec::new();
        for status in ["429 Too Many Requests", "503 Service Unavailable", "200 OK"] {
            let until = std::time::Instant::now() + Duration::from_secs(10);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(stream) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < until,
                            "no request reached local test server"
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            };
            // macOS accepted sockets inherit the listener's nonblocking flag.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = std::io::BufReader::new(&mut stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert_eq!(line, "POST /chat/completions HTTP/1.1\r\n");
            let mut headers = std::collections::HashMap::new();
            loop {
                line.clear();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                let (key, value) = line.split_once(':').unwrap();
                headers.insert(key.to_ascii_lowercase(), value.trim().to_string());
            }
            assert_eq!(headers["authorization"], "Bearer placeholder");
            assert_eq!(headers["content-type"], "application/json");
            assert_eq!(headers["accept-encoding"], "identity");
            let mut body = vec![0; headers["content-length"].parse().unwrap()];
            reader.read_exact(&mut body).unwrap();
            drop(reader);
            captured.push(body);
            let body = if status == "200 OK" {
                response.as_str()
            } else {
                "temporary error"
            };
            write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nRetry-After: 0\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            stream.flush().unwrap();
        }
        captured
    });
    let answer = tokio::time::timeout(
        Duration::from_secs(20),
        chat_completion_public(&cfg, messages(true)),
    )
    .await;
    let captured = server.join().unwrap();
    assert_eq!(answer.unwrap().unwrap(), expected_answer);
    assert_eq!(captured.len(), 3);
    assert!(captured.iter().all(|body| body == &expected_body));
}
