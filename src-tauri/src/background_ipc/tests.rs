use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use tauri::ipc::IpcResponse;

struct Probe {
    caller: std::thread::ThreadId,
    value: serde_json::Value,
    serialized: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
    entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
}
impl Serialize for Probe {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        assert_ne!(std::thread::current().id(), self.caller);
        self.serialized.fetch_add(1, Ordering::SeqCst);
        if let Some(entered) = self.entered.lock().unwrap().take() {
            entered.send(()).unwrap();
        }
        if let Some(release) = self.release.lock().unwrap().take() {
            release.recv().unwrap();
        }
        self.value.serialize(serializer)
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        assert_ne!(
            std::thread::current().id(),
            self.caller,
            "large rows dropped on the caller thread"
        );
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn work_serialization_and_row_cleanup_run_off_the_executor_and_reply_is_raw_json() {
    let value = serde_json::json!({"text": "日本語\n\"quoted\" 👩🏽‍💻".repeat(4096), "rows": [1,2,3], "absent": null});
    let expected = value.clone();
    let caller = std::thread::current().id();
    let serialized = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicBool::new(false));
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let probe = Probe {
        caller,
        value,
        serialized: serialized.clone(),
        dropped: dropped.clone(),
        entered: Mutex::new(Some(entered)),
        release: Mutex::new(Some(released)),
    };
    let task = tokio::spawn(respond("work failed", "encoding failed", move || {
        assert_ne!(std::thread::current().id(), caller);
        Ok(probe)
    }));
    started.await.unwrap();
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    assert!(!task.is_finished());
    assert!(!dropped.load(Ordering::SeqCst));
    release.send(()).unwrap();
    let response = task.await.unwrap().unwrap();
    assert!(dropped.load(Ordering::SeqCst));
    let parsed = response
        .body()
        .unwrap()
        .deserialize::<serde_json::Value>()
        .unwrap();
    assert_eq!(parsed, expected);
    assert!(parsed.is_object(), "raw JSON was encoded as a JSON string");
    assert_eq!(serialized.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn event_and_reply_share_one_encoding_and_preserve_the_exact_object() {
    let value = serde_json::json!({"saved": true, "markdown": "# 全文\n\"引用\" 👩🏽‍💻".repeat(512), "snapshot": {"active": false, "transcript_lines": [{"text":"full line", "at":"10:00"}]}});
    let expected = value.clone();
    let serialized = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicBool::new(false));
    let caller = std::thread::current().id();
    let probe = Probe {
        caller,
        value,
        serialized: serialized.clone(),
        dropped: dropped.clone(),
        entered: Mutex::new(None),
        release: Mutex::new(None),
    };
    let published = Arc::new(Mutex::new(None));
    let output = published.clone();
    let response = respond_with_event(
        "work failed",
        "encoding failed",
        move || Ok(probe),
        move |json| {
            assert_ne!(std::thread::current().id(), caller);
            *output.lock().unwrap() = Some(json);
        },
    )
    .await
    .unwrap();
    let reply = response
        .body()
        .unwrap()
        .deserialize::<serde_json::Value>()
        .unwrap();
    let event =
        serde_json::from_str::<serde_json::Value>(published.lock().unwrap().as_ref().unwrap())
            .unwrap();
    assert_eq!(reply, expected);
    assert_eq!(event, reply);
    assert_eq!(serialized.load(Ordering::SeqCst), 1);
    assert!(dropped.load(Ordering::SeqCst));
}

struct CannotEncode;
impl Serialize for CannotEncode {
    fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom("original serialization error"))
    }
}
#[tokio::test(flavor = "current_thread")]
async fn domain_errors_worker_panics_and_encoding_failures_remain_distinct() {
    let error = respond_with_event(
        "work failed",
        "encoding failed",
        || Err::<(), _>("original projection error".into()),
        |_| panic!("failed projection was published"),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(error, "original projection error");
    let error = respond_with_event(
        "work failed",
        "encoding failed",
        || -> Result<(), String> { panic!("projection panic") },
        |_| panic!("panicked projection was published"),
    )
    .await
    .err()
    .unwrap();
    assert!(error.starts_with("work failed:"));
    let error = respond("work failed", "encoding failed", || {
        Err::<(), _>("original database error".into())
    })
    .await
    .err()
    .unwrap();
    assert_eq!(error, "original database error");
    let error = respond(
        "work failed",
        "encoding failed",
        || -> Result<(), String> { panic!("worker panic") },
    )
    .await
    .err()
    .unwrap();
    assert!(error.starts_with("work failed:"));
    let error = respond("work failed", "encoding failed", || Ok(CannotEncode))
        .await
        .err()
        .unwrap();
    assert_eq!(error, "encoding failed: original serialization error");
    let error = respond_with_event(
        "work failed",
        "encoding failed",
        || Ok(CannotEncode),
        |_| panic!("invalid JSON was published"),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(error, "encoding failed: original serialization error");
}
