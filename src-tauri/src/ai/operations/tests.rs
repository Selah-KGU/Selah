use super::*;
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
use std::time::Duration;
use tauri::ipc::IpcResponse;

fn config(model: &str, language: &str) -> AiConfig {
    AiConfig {
        ai_enabled: true,
        provider: "openai".into(),
        model: model.into(),
        api_key: "placeholder".into(),
        reply_language: language.into(),
        ..Default::default()
    }
}

struct File(std::path::PathBuf);
impl File {
    fn new() -> Arc<Self> {
        let file = Arc::new(Self(
            std::env::temp_dir().join(format!("selah-ai-operations-{}.json", uuid::Uuid::new_v4())),
        ));
        file.write(&config("original", "ja")).unwrap();
        file
    }
    fn write(&self, cfg: &AiConfig) -> Result<(), String> {
        std::fs::write(&self.0, serde_json::to_vec(cfg).unwrap()).map_err(|e| e.to_string())
    }
    fn read(&self) -> AiConfig {
        serde_json::from_slice(&std::fs::read(&self.0).unwrap()).unwrap()
    }
}
impl Drop for File {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn reply(prepared: Prepared) -> serde_json::Value {
    let Prepared::Reply(response) = prepared else {
        panic!("expected immediate response")
    };
    let tauri::ipc::InvokeResponseBody::Json(json) = response.body().unwrap() else {
        panic!("expected JSON")
    };
    serde_json::from_str(&json).unwrap()
}

#[test]
fn ipc_decode_keeps_complete_settings_messages_images_and_original_test_prompt() {
    let cfg = config("  configured model  ", "ko");
    let value = serde_json::to_value(&cfg).unwrap();
    let request = decode(
        Kind::Save,
        &InvokeBody::Json(json!({"config": value, "extra": 3})),
    )
    .unwrap();
    let Request::Save(decoded) = request else {
        panic!("save")
    };
    assert_eq!(serde_json::to_value(decoded).unwrap(), value);
    let messages = vec![
        ChatMessage {
            role: "system".into(),
            content: "  指示\n中文 🌕 ".repeat(1000),
            images: Vec::new(),
        },
        ChatMessage {
            role: "user".into(),
            content: "quotes \"\\\n full text ".repeat(2000),
            images: vec![super::super::config::ImagePart {
                mime: "image/png".into(),
                data_base64: "full-image-buffer".repeat(120_000),
            }],
        },
    ];
    let expected = serde_json::to_value(&messages).unwrap();
    let request = decode(Kind::Chat, &InvokeBody::Json(json!({"messages": expected}))).unwrap();
    let Request::Infer(decoded) = request else {
        panic!("chat")
    };
    assert_eq!(serde_json::to_value(decoded).unwrap(), expected);
    let Request::Infer(messages) = decode(Kind::Test, &InvokeBody::Raw(Vec::new())).unwrap() else {
        panic!("test")
    };
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "Reply OK in one word.");
    assert_eq!(messages[0].role, "user");
    assert!(messages[0].images.is_empty());
    assert!(matches!(
        decode(Kind::Read, &InvokeBody::Raw(Vec::new())),
        Ok(Request::Read)
    ));
    for (kind, body) in [
        (Kind::Save, json!({})),
        (Kind::Save, json!({"config": null})),
        (Kind::Save, json!({"config": {"temperature": "bad"}})),
        (Kind::Chat, json!({})),
        (Kind::Chat, json!({"messages": null})),
        (
            Kind::Chat,
            json!({"messages": [{"role": "user", "content": 8}]}),
        ),
    ] {
        assert!(decode(kind, &InvokeBody::Json(body)).is_err());
    }
    assert!(decode(Kind::Save, &InvokeBody::Raw(Vec::new())).is_err());
    for name in [
        "agent_send",
        "live_get_session",
        "get_local_ai_support",
        "list_local_models",
    ] {
        assert!(Kind::from_command(name).is_none());
    }
    for name in [
        "get_ai_config",
        "save_ai_config",
        "ai_chat",
        "ai_test_connection",
    ] {
        assert!(Kind::from_command(name).is_some());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn save_read_order_is_admitted_before_poll_and_dropped_replies_still_commit_and_notify() {
    let queue = Arc::new(Queue::new(FAILURE));
    let file = File::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let storing = file.clone();
    let notifying = events.clone();
    let caller = std::thread::current().id();
    let first = submit(
        &queue,
        move || {
            assert_ne!(std::thread::current().id(), caller);
            entered.send(()).unwrap();
            released.recv_timeout(Duration::from_secs(10)).unwrap();
            Ok(Request::Save(config("  first 🌕 ", "zh")))
        },
        || panic!("save reloaded configuration"),
        move |cfg| {
            super::super::commands::persist_and_publish(
                &cfg,
                |cfg| storing.write(cfg),
                || notifying.lock().unwrap().push("first"),
            )
        },
    );
    let storing = file.clone();
    let notifying = events.clone();
    let second = submit(
        &queue,
        || Ok(Request::Save(config("second", "en"))),
        || panic!("save loaded"),
        move |cfg| {
            super::super::commands::persist_and_publish(
                &cfg,
                |cfg| storing.write(cfg),
                || notifying.lock().unwrap().push("second"),
            )
        },
    );
    let reading = file.clone();
    let last = submit(
        &queue,
        || Ok(Request::Read),
        move || reading.read(),
        |_| panic!("read saved"),
    );
    started.await.unwrap();
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    assert_eq!(file.read().model, "original");
    drop(first);
    release.send(()).unwrap();
    // Read completion is polled first but sees both earlier writes and events.
    let read = reply(last.await.unwrap());
    assert_eq!(read["model"], "second");
    assert_eq!(read["reply_language"], "en");
    assert_eq!(read["max_tokens"], 8192);
    assert_eq!(reply(second.await.unwrap()), serde_json::Value::Null);
    assert_eq!(*events.lock().unwrap(), ["first", "second"]);
    // The existing IPC serializer writes f32 values in their shortest decimal
    // form; compare JSON text round-trips rather than f32 promoted to f64 Value.
    let legacy: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&file.read()).unwrap()).unwrap();
    assert_eq!(read, legacy);
    queue.seal();
    queue.drained().await;
}

#[tokio::test(flavor = "current_thread")]
async fn model_preparation_reads_once_off_executor_and_model_wait_does_not_block_new_settings() {
    let queue = Arc::new(Queue::new(FAILURE));
    let file = File::new();
    let loads = Arc::new(AtomicUsize::new(0));
    let counted = loads.clone();
    let reading = file.clone();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let caller = std::thread::current().id();
    let preparation = submit(
        &queue,
        || decode(Kind::Test, &InvokeBody::Json(json!({}))),
        move || {
            assert_ne!(std::thread::current().id(), caller);
            counted.fetch_add(1, Ordering::SeqCst);
            entered.send(()).unwrap();
            released.recv_timeout(Duration::from_secs(10)).unwrap();
            reading.read()
        },
        |_| panic!("inference saved"),
    );
    started.await.unwrap();
    assert_eq!(tokio::spawn(async { 7 }).await.unwrap(), 7);
    release.send(()).unwrap();
    let prepared = preparation.await.unwrap();
    let (infer_entered, infer_started) = tokio::sync::oneshot::channel();
    let (infer_release, infer_released) = tokio::sync::oneshot::channel();
    let text = "完全な応答 🌕\n\"quote\" \\ slash\n".repeat(10_000);
    let output = text.clone();
    let model = tokio::spawn(finish(prepared, move |cfg, messages| async move {
        assert_eq!(cfg.model, "original");
        assert_eq!(cfg.reply_language, "ja");
        assert_eq!(messages[0].content, "Reply OK in one word.");
        infer_entered.send(()).unwrap();
        infer_released.await.unwrap();
        Ok(output)
    }));
    infer_started.await.unwrap();
    let storing = file.clone();
    let saved = submit(
        &queue,
        || Ok(Request::Save(config("new model", "ko"))),
        || panic!("save loaded"),
        move |cfg| storing.write(&cfg),
    );
    assert_eq!(reply(saved.await.unwrap()), serde_json::Value::Null);
    let reading = file.clone();
    let read = submit(
        &queue,
        || Ok(Request::Read),
        move || reading.read(),
        |_| panic!("read saved"),
    );
    assert_eq!(reply(read.await.unwrap())["model"], "new model");
    assert!(!model.is_finished());
    assert_eq!(loads.load(Ordering::SeqCst), 1);
    queue.seal();
    queue.drained().await;
    assert!(
        !model.is_finished(),
        "config drain waited for model inference"
    );
    infer_release.send(()).unwrap();
    assert_eq!(
        reply(Prepared::Reply(model.await.unwrap().unwrap())),
        json!(text)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn invalid_settings_failed_saves_and_panics_do_not_notify_or_strand_later_jobs() {
    let queue = Arc::new(Queue::new(FAILURE));
    let notifications = Arc::new(AtomicUsize::new(0));
    for (cfg, expected) in [
        (config("  ", "en"), "モデル名を入力してください"),
        (
            AiConfig {
                provider: "unknown".into(),
                ..config("m", "en")
            },
            "不明なプロバイダーです",
        ),
        (
            AiConfig {
                base_url: "http://example.org".into(),
                ..config("m", "en")
            },
            "Base URLは https:// で始まる必要があります",
        ),
    ] {
        let result = submit(
            &queue,
            || Ok(Request::Save(cfg)),
            || panic!("invalid save loaded"),
            |_| panic!("invalid save wrote"),
        );
        assert!(matches!(result.await, Err(error) if error == expected));
    }
    let notifying = notifications.clone();
    let failed = submit(
        &queue,
        || Ok(Request::Save(config("valid", "en"))),
        || panic!("save loaded"),
        move |cfg| {
            super::super::commands::persist_and_publish(
                &cfg,
                |_| Err("original write failure".into()),
                || {
                    notifying.fetch_add(1, Ordering::SeqCst);
                },
            )
        },
    );
    assert!(matches!(failed.await, Err(error) if error == "original write failure"));
    let panicked = submit(
        &queue,
        || -> Result<Request, String> { panic!("decoder panic") },
        || panic!("loaded"),
        |_| panic!("saved"),
    );
    assert!(matches!(panicked.await, Err(error) if error == "AI設定処理失敗: worker panic"));
    assert_eq!(notifications.load(Ordering::SeqCst), 0);
    let notifying = notifications.clone();
    let next = submit(
        &queue,
        || Ok(Request::Save(config("  next  ", "zh"))),
        || panic!("save loaded"),
        move |cfg| {
            assert_eq!(cfg.model, "next");
            super::super::commands::persist_and_publish(
                &cfg,
                |_| Ok(()),
                || {
                    notifying.fetch_add(1, Ordering::SeqCst);
                },
            )
        },
    );
    assert!(next.await.is_ok());
    assert_eq!(notifications.load(Ordering::SeqCst), 1);
    let domain = finish(
        Prepared::Infer(config("m", "en"), Vec::new()),
        |_, _| async { Err("original model failure".into()) },
    )
    .await;
    assert!(matches!(domain, Err(error) if error == "original model failure"));
}

#[test]
fn settings_validation_preserves_provider_rules_and_normalization() {
    for provider in ["openai", "openrouter", "deepseek", "gemini"] {
        for url in [
            "",
            " https://example.org ",
            "http://localhost:8000",
            "http://127.0.0.1:8000",
        ] {
            let cfg = super::super::commands::validate_config(AiConfig {
                provider: provider.into(),
                model: "  custom model  ".into(),
                base_url: url.into(),
                api_key: "  placeholder  ".into(),
                max_tokens: u32::MAX,
                temperature: 9.0,
                live_summary_interval_minutes: 1,
                ..config("m", "zh")
            })
            .unwrap();
            assert_eq!(cfg.provider, provider);
            assert_eq!(cfg.model, "custom model");
            assert_eq!(cfg.base_url, url.trim());
            assert_eq!(cfg.api_key, "placeholder");
            assert_eq!(cfg.max_tokens, 32768);
            assert_eq!(cfg.temperature, 2.0);
            assert_eq!(cfg.live_summary_interval_minutes, 5);
            assert_eq!(cfg.reply_language, "zh");
            assert_eq!(
                cfg.local_model,
                crate::local_ai_support::APPLE_INTELLIGENCE_MODEL_ID
            );
        }
    }
}
