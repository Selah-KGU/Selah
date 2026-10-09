use super::*;
use crate::ai::{AiConfig, ChatMessage, ImagePart};
use requests::Mode;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

fn config() -> AiConfig {
    AiConfig {
        ai_enabled: true,
        provider: "openai".into(),
        api_key: "test-placeholder".into(),
        model: "model /? 日本語".into(),
        base_url: "http://127.0.0.1:8080///".into(),
        max_tokens: 0,
        temperature: 0.7,
        ..Default::default()
    }
}
fn messages(large: bool) -> Vec<ChatMessage> {
    let count = if large { 10_000 } else { 1 };
    vec![
        ChatMessage {
            role: "system".into(),
            content: "指示 <call:tool>(x) task_call: ‹　›\n".repeat(count),
            images: vec![],
        },
        ChatMessage {
            role: "user".into(),
            content: "発話 👩🏽‍💻\n\"quote\" \\ (x) <y>".repeat(count),
            images: vec![ImagePart {
                mime: "image/png".into(),
                data_base64: "QUJD".repeat(count * 40),
            }],
        },
        ChatMessage {
            role: "assistant".into(),
            content: "全文の回答 🌕".repeat(count),
            images: vec![],
        },
        ChatMessage {
            role: "system".into(),
            content: "追加指示 tool_call: function_call: call:".into(),
            images: vec![],
        },
        ChatMessage {
            role: "tool".into(),
            content: "資料の内容".into(),
            images: vec![],
        },
    ]
}
fn assert_request_eq(actual: reqwest::RequestBuilder, expected: reqwest::RequestBuilder) {
    let actual = actual.build().unwrap();
    let expected = expected.build().unwrap();
    assert_eq!(actual.method(), expected.method());
    assert_eq!(actual.url(), expected.url());
    assert_eq!(actual.headers(), expected.headers());
    assert_eq!(
        actual.body().unwrap().as_bytes(),
        expected.body().unwrap().as_bytes()
    );
    let retry = actual.try_clone().unwrap();
    assert_eq!(
        actual.body().unwrap().as_bytes().unwrap().as_ptr(),
        retry.body().unwrap().as_bytes().unwrap().as_ptr()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn worker_requests_match_original_bytes_headers_and_provider_options() {
    let fixtures = [
        vec![],
        messages(false)[..1].to_vec(),
        messages(false),
        messages(true),
    ];
    for input in fixtures {
        let input = requests::share_messages(input);
        for max in [0, 12000] {
            let mut cfg = config();
            cfg.max_tokens = max;
            for images in [true, false] {
                for mode in [
                    Mode::Stream,
                    Mode::Plan {
                        max_tokens: 0,
                        temperature: 0.0,
                        json_mode: false,
                    },
                    Mode::Plan {
                        max_tokens: 19,
                        temperature: 1.3,
                        json_mode: true,
                    },
                    Mode::Plan {
                        max_tokens: 8192,
                        temperature: f32::NAN,
                        json_mode: true,
                    },
                ] {
                    let (max, temp, json_mode) = match mode {
                        Mode::Plan {
                            max_tokens,
                            temperature,
                            json_mode,
                        } => (max_tokens, temperature, json_mode),
                        Mode::Stream => (cfg.max_tokens, cfg.temperature, false),
                    };
                    let old_openai = if matches!(mode, Mode::Stream) {
                        legacy_http::openai_stream(&cfg, &input, images, max, temp, json_mode)
                    } else {
                        legacy_http::openai_plan(&cfg, &input, images, max, temp, json_mode)
                    };
                    assert_request_eq(
                        requests::openai(&cfg, input.clone(), images, mode)
                            .await
                            .unwrap(),
                        old_openai,
                    );
                    let old_gemini = if matches!(mode, Mode::Stream) {
                        legacy_http::gemini_stream(&cfg, &input, images, max, temp, json_mode)
                    } else {
                        legacy_http::gemini_plan(&cfg, &input, images, max, temp, json_mode)
                    };
                    assert_request_eq(
                        requests::gemini(&cfg, input.clone(), images, mode)
                            .await
                            .unwrap(),
                        old_gemini,
                    );
                }
            }
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn worker_response_parsing_preserves_full_text_reasoning_tools_and_errors() {
    let large = "授業 👩🏽‍💻\n引用\"\\".repeat(30_000);
    let openai = [
        json!({"choices":[{"message":{"content":large}}]}),
        json!({"choices":[{"message":{"content":[{"text":"甲"},{"content":"乙"},{"text":null},{"text":"🌕"}]}}]}),
        json!({"choices":[{"message":{"content":null,"reasoning":large}}]}),
        json!({"choices":[{"message":{"content":"　\n","reasoning":"答案"}}]}),
        json!({"choices":[{"message":{"content":" \n"},"finish_reason":"length"}]}),
        json!({"choices":[{"message":{"content":123,"reasoning":false}}]}),
        json!({"choices":[]}),
        json!(null),
        json!({}),
    ];
    let gemini = [
        json!({"candidates":[{"content":{"parts":[{"text":large},{"text":"末尾"},{"text":null}]}}]}),
        json!({"candidates":[{"content":{"parts":[{"text":""},{"text":" "}]}}]}),
        json!({"candidates":[{"content":{"parts":[{"functionCall":{"name":"read_downloaded_file","args":{"path":"/tmp/a.md"}}}]}}]}),
        json!({"candidates":[{"content":{},"finishReason":"MALFORMED_FUNCTION_CALL","finishMessage":"Malformed function call: call:read_downloaded_file {\"path\":\"/tmp/a.md\"}"}]}),
        json!({"candidates":[{"content":{"parts":false}}]}),
        json!({"candidates":[]}),
        json!(null),
        json!({}),
    ];
    for payload in openai {
        let text = payload.to_string();
        assert_eq!(
            processing::openai(text.clone()).await,
            legacy_http::parse_openai(&text)
        );
    }
    for payload in gemini {
        let text = payload.to_string();
        assert_eq!(
            processing::gemini(text.clone()).await,
            legacy_http::parse_gemini(&text)
        );
    }
    for text in ["", "{", "[not JSON]", "\"unterminated"] {
        assert_eq!(
            processing::openai(text.into()).await,
            legacy_http::parse_openai(text)
        );
        assert_eq!(
            processing::gemini(text.into()).await,
            legacy_http::parse_gemini(text)
        );
    }
}

fn buffers(messages: &[ChatMessage]) -> (usize, usize, usize) {
    (
        messages.as_ptr() as usize,
        messages[1].content.as_ptr() as usize,
        messages[1].images[0].data_base64.as_ptr() as usize,
    )
}

#[tokio::test(flavor = "current_thread")]
async fn worker_shares_original_input_and_leaves_the_async_thread_available() {
    let input = messages(true);
    let pointers = buffers(&input);
    let shared = requests::share_messages(input);
    assert_eq!(buffers(&shared), pointers);
    let stream = shared.clone();
    assert!(Arc::ptr_eq(&shared, &stream));
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, held) = std::sync::mpsc::channel();
    let thread = std::thread::current().id();
    let task = tokio::spawn(requests::prepare(config(), stream, move |_, input| {
        assert_ne!(std::thread::current().id(), thread);
        started.send(buffers(&input)).unwrap();
        held.recv_timeout(Duration::from_secs(5)).unwrap();
        input
    }));
    assert_eq!(ready.await.unwrap(), pointers);
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    release.send(()).unwrap();
    let result = task.await.unwrap().unwrap();
    assert!(Arc::ptr_eq(&shared, &result));
    assert_eq!(buffers(&result), pointers);
    let error = requests::prepare(config(), result, |_, _| -> () { panic!("worker fixture") })
        .await
        .unwrap_err();
    assert!(error.starts_with("Agentリクエスト準備失敗:"));
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_during_preparation_never_resumes_http_and_preserves_replacement() {
    use crate::agent_error::AgentError;
    use crate::agent_turn_scope::{until_cancelled, RunningTurn};
    let id = format!("request-worker-{}", uuid::Uuid::new_v4());
    let old = RunningTurn::begin(&id, None);
    let owner = old.turn.clone();
    let sent = Arc::new(AtomicBool::new(false));
    let sent_in_task = sent.clone();
    let (started, ready) = tokio::sync::oneshot::channel();
    let (ended, done) = tokio::sync::oneshot::channel();
    let (release, held) = std::sync::mpsc::channel();
    let task = tokio::spawn(async move {
        until_cancelled(Some(&owner), async move {
            let result = requests::prepare(
                config(),
                requests::share_messages(messages(true)),
                move |_, input| {
                    started.send(()).unwrap();
                    held.recv_timeout(Duration::from_secs(5)).unwrap();
                    ended.send(()).unwrap();
                    input
                },
            )
            .await
            .map_err(AgentError::model)?;
            sent_in_task.store(true, Ordering::Release);
            Ok(result.len())
        })
        .await
    });
    ready.await.unwrap();
    let latest = RunningTurn::begin(&id, None);
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap(),
        Err(AgentError::Cancelled)
    ));
    release.send(()).unwrap();
    done.await.unwrap();
    assert!(!sent.load(Ordering::Acquire));
    assert!(!latest.turn.cancelled());
    assert!(latest.turn.accepts_event(false));
    assert!(old.turn.cancelled());
}

struct Captured {
    head: String,
    body: Vec<u8>,
}
struct Reply {
    status: &'static str,
    content_type: &'static str,
    body: String,
}
fn reply(status: &'static str, body: Value) -> Reply {
    Reply {
        status,
        content_type: "application/json",
        body: body.to_string(),
    }
}
fn server(replies: Vec<Reply>) -> (String, std::thread::JoinHandle<Vec<Captured>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let thread = std::thread::spawn(move || {
        let mut captured = Vec::new();
        for response in replies {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            let mut len = 0;
            loop {
                let mut line = String::new();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                head.push_str(&line);
                if line == "\r\n" {
                    break;
                }
                if let Some((name, value)) = line.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        len = value.trim().parse().unwrap();
                    }
                }
            }
            let mut body = vec![0; len];
            reader.read_exact(&mut body).unwrap();
            captured.push(Captured { head, body });
            write!(stream, "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response.status, response.content_type, response.body.len(), response.body).unwrap();
            stream.flush().unwrap();
        }
        captured
    });
    (format!("http://{address}"), thread)
}
fn assert_wire(request: &Captured, expected: reqwest::RequestBuilder) {
    let expected = expected.build().unwrap();
    assert!(request
        .head
        .starts_with("POST /chat/completions HTTP/1.1\r\n"));
    let headers = request.head.to_ascii_lowercase();
    for (name, value) in expected.headers() {
        assert!(headers
            .contains(&format!("{}: {}\r\n", name, value.to_str().unwrap()).to_ascii_lowercase()));
    }
    assert_eq!(&request.body, expected.body().unwrap().as_bytes().unwrap());
}

#[tokio::test(flavor = "current_thread")]
async fn actual_plan_retries_images_then_json_format_without_changing_full_input() {
    let answer = "計画の全文 🌕\n".repeat(10_000);
    let (url, server) = server(vec![
        reply("400 Bad Request", json!({"error":"unsupported image_url"})),
        reply(
            "400 Bad Request",
            json!({"error":"response_format not supported"}),
        ),
        reply(
            "200 OK",
            json!({"choices":[{"message":{"content":null,"reasoning":answer}}]}),
        ),
    ]);
    let mut cfg = config();
    cfg.base_url = url;
    let input = messages(true);
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        remote_chat_completion(&cfg, input.clone(), 0, 0.0, "", true),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result, answer);
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 3);
    for (request, (images, json_mode)) in
        requests
            .iter()
            .zip([(true, true), (false, true), (false, false)])
    {
        assert_wire(
            request,
            legacy_http::openai_plan(&cfg, &input, images, 0, 0.0, json_mode),
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn actual_empty_stream_fallback_preserves_images_retries_and_filtered_callbacks() {
    let answer = "回答の全文 👩🏽‍💻\n".repeat(10_000);
    let (url, server) = server(vec![
        reply("400 Bad Request", json!({"error":"unsupported image_url"})),
        Reply { status: "200 OK", content_type: "text/event-stream", body:
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"推論\"}}]}\n\ndata: [DONE]\n\n".into() },
        reply("400 Bad Request", json!({"error":"unsupported image_url"})),
        reply("200 OK", json!({"choices":[{"message":{"content":answer}}]})),
    ]);
    let mut cfg = config();
    cfg.base_url = url;
    let input = messages(true);
    let chunks = Arc::new(Mutex::new(Vec::new()));
    let output = chunks.clone();
    let generation = format!("stream-worker-{}", uuid::Uuid::new_v4());
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        remote_stream_answer(
            &cfg,
            input.clone(),
            &generation,
            move |text, think| output.lock().unwrap().push((text.to_owned(), think)),
            30,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result, answer);
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 4);
    assert_wire(
        &requests[0],
        legacy_http::openai_stream(&cfg, &input, true, 0, 0.7, false),
    );
    assert_wire(
        &requests[1],
        legacy_http::openai_stream(&cfg, &input, false, 0, 0.7, false),
    );
    // Fallback uses the original input, even after the stream dropped images.
    assert_wire(
        &requests[2],
        legacy_http::openai_plan(&cfg, &input, true, 32768, 0.7, false),
    );
    assert_wire(
        &requests[3],
        legacy_http::openai_plan(&cfg, &input, false, 32768, 0.7, false),
    );
    let chunks = chunks.lock().unwrap();
    assert_eq!(
        chunks
            .iter()
            .filter(|(_, think)| *think)
            .map(|(text, _)| text.as_str())
            .collect::<String>(),
        "推論"
    );
    assert_eq!(
        chunks
            .iter()
            .filter(|(_, think)| !*think)
            .map(|(text, _)| text.as_str())
            .collect::<String>(),
        answer
    );
}

#[tokio::test(flavor = "current_thread")]
async fn pseudo_tool_fallback_is_returned_without_displaying_tool_syntax() {
    let call = "call:read_downloaded_file {\"path\":\"/tmp/a.md\"}";
    let (url, server) = server(vec![
        Reply {
            status: "200 OK",
            content_type: "text/event-stream",
            body: "data: [DONE]\n\n".into(),
        },
        reply("200 OK", json!({"choices":[{"message":{"content":call}}]})),
    ]);
    let mut cfg = config();
    cfg.base_url = url;
    let emitted = Arc::new(AtomicBool::new(false));
    let in_callback = emitted.clone();
    let generation = uuid::Uuid::new_v4().to_string();
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        remote_stream_answer(
            &cfg,
            messages(false),
            &generation,
            move |_, _| {
                in_callback.store(true, Ordering::Release);
            },
            0,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result, call);
    assert!(!emitted.load(Ordering::Acquire));
    assert_eq!(server.join().unwrap().len(), 2);
}
