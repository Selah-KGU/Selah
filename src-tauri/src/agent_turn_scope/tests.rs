use super::*;
use crate::agent_error::AgentError;
use crate::agent_provider::AgentProvider;
use tokio::sync::oneshot;

fn conversation() -> String {
    format!("scope-test-{}", uuid::Uuid::new_v4())
}

#[test]
fn committed_input_boundary_is_set_once_and_derived_history_belongs_to_its_request() {
    let id = conversation();
    let old = RunningTurn::begin(&id, None);
    assert!(old.turn.history().is_err());
    for invalid in [0, -1] {
        assert!(old.turn.set_input_message(invalid).is_err());
    }
    old.turn.set_input_message(40).unwrap();
    assert!(old.turn.set_input_message(40).is_err());
    assert!(old.turn.set_input_message(41).is_err());
    old.turn.record_message(42);
    let snapshot = old.turn.history().unwrap();
    let latest = RunningTurn::begin(&id, None);
    assert!(latest.turn.history().is_err());
    latest.turn.set_input_message(43).unwrap();
    latest.turn.record_message(44);
    // A retained storage worker may finish after its request was superseded.
    // Its audit record stays owned by that old request, not the new planner.
    old.turn.record_message(45);
    assert_eq!(snapshot.input_message, 40);
    assert_eq!(snapshot.messages, [42]);
    assert_eq!(old.turn.history().unwrap().messages, [42, 45]);
    assert_eq!(latest.turn.history().unwrap().input_message, 43);
    assert_eq!(latest.turn.history().unwrap().messages, [44]);
}
fn registered(id: &str, generation: &str) -> (bool, bool) {
    let registry = REGISTRY.lock().unwrap();
    (
        registry.conversations.contains_key(id),
        registry.generations.contains_key(generation),
    )
}

#[test]
fn replacement_cancels_only_previous_generation_and_preserves_latest_registry_entry() {
    let id = conversation();
    let other_id = conversation();
    let old = RunningTurn::begin(&id, Some("old-request".into()));
    let old_gen = old.turn.generation().to_owned();
    let other = RunningTurn::begin(&other_id, None);
    let latest = RunningTurn::begin(&id, Some("new-request".into()));
    assert_ne!(old_gen, latest.turn.generation());
    assert!(old.turn.cancelled());
    assert!(!old.turn.accepts_event(false));
    assert!(!old.turn.accepts_event(true));
    assert!(!latest.turn.cancelled());
    assert!(!other.turn.cancelled());
    drop(old);
    assert_eq!(registered(&id, &old_gen), (true, false));
    assert!(!cancel(&id, Some("old-request")));
    assert!(!latest.turn.cancelled());
    assert!(cancel(&id, Some("new-request")));
    assert!(latest.turn.cancelled());
    assert!(latest.turn.accepts_event(true));
    assert!(!latest.turn.accepts_event(false));
    assert!(!other.turn.cancelled());
    let latest_gen = latest.turn.generation().to_owned();
    drop(latest);
    assert_eq!(registered(&id, &latest_gen), (false, false));
    assert!(!cancel(&id, None));
}

#[test]
fn backend_generations_remain_distinct_even_when_a_caller_reuses_its_request_id() {
    let id = conversation();
    let old = RunningTurn::begin(&id, Some("reused".into()));
    let new = RunningTurn::begin(&id, Some("reused".into()));
    assert_ne!(old.turn.generation(), new.turn.generation());
    assert!(generation_cancelled(old.turn.generation()));
    assert!(!generation_cancelled(new.turn.generation()));
    assert!(cancel(&id, None));
    assert!(new.turn.cancelled());
    let generated = RunningTurn::begin(&conversation(), Some(String::new()));
    assert_eq!(generated.turn.request(), generated.turn.generation());
}

#[tokio::test(flavor = "current_thread")]
async fn old_scope_and_plan_cancellation_cannot_be_revived_by_provider_flag_resets() {
    let id = conversation();
    let old = RunningTurn::begin(&id, None);
    let owner = old.turn.clone();
    CURRENT
        .scope(owner.clone(), async {
            assert!(Arc::ptr_eq(&current(&id).unwrap(), &owner));
            assert!(current("unrelated-conversation").is_none());
            let latest = RunningTurn::begin(&id, None);
            AgentProvider::clear_cancel(owner.generation());
            AgentProvider::clear_cancel(latest.turn.generation());
            assert!(AgentProvider::is_cancelled(&id));
            assert!(AgentProvider::is_cancelled(owner.generation()));
            assert!(AgentProvider::is_cancelled(&format!(
                "plan:{}",
                owner.generation()
            )));
            assert!(!AgentProvider::is_cancelled(latest.turn.generation()));
            assert!(tokio::spawn(async { current("anything").is_none() })
                .await
                .unwrap());
            assert!(tokio::task::spawn_blocking(move || current(&id).is_none())
                .await
                .unwrap());
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn abort_cancels_a_retained_blocking_worker_and_cleanup_waits_for_its_exit() {
    let id = conversation();
    let task_id = id.clone();
    let (started, ready) = oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let (finished, ended) = oneshot::channel();
    let task = tokio::spawn(async move {
        let running = RunningTurn::begin(&task_id, None);
        let worker = running.turn.clone();
        let generation = worker.generation().to_owned();
        tokio::task::spawn_blocking(move || {
            started.send(generation).unwrap();
            released.recv().unwrap();
            assert!(worker.cancelled());
            assert!(AgentProvider::is_cancelled(worker.generation()));
            assert!(!worker.accepts_event(false));
            drop(worker);
            finished.send(()).unwrap();
        });
        // Aborting this root must cancel even though spawn_blocking continues.
        let _running = running;
        std::future::pending::<()>().await;
    });
    let generation = ready.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(generation_cancelled(&generation));
    assert_eq!(registered(&id, &generation), (true, true));
    let latest = RunningTurn::begin(&id, Some("replacement".into()));
    release.send(()).unwrap();
    ended.await.unwrap();
    assert_eq!(registered(&id, &generation), (true, false));
    assert!(!AgentProvider::is_cancelled(&generation));
    assert!(!AgentProvider::is_cancelled(&format!("plan:{generation}")));
    assert!(!latest.turn.cancelled());
}

#[tokio::test(flavor = "current_thread")]
async fn completion_and_timeout_keep_orphan_worker_cancellation_latched_until_final_arc() {
    let id = conversation();
    let mut running = RunningTurn::begin(&id, None);
    let worker = running.turn.clone();
    let generation = worker.generation().to_owned();
    running.finish();
    drop(running);
    AgentProvider::clear_cancel(&generation);
    assert!(worker.cancelled());
    assert!(!worker.accepts_event(false));
    assert_eq!(registered(&id, &generation), (true, true));
    let result = until_cancelled(Some(&worker), async { Ok(123) }).await;
    assert!(matches!(result, Err(AgentError::Cancelled)));
    drop(worker);
    assert_eq!(registered(&id, &generation), (false, false));
    assert!(!AgentProvider::is_cancelled(&generation));
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_wakes_every_waiter_and_drops_pending_work_without_waiting_for_data() {
    let id = conversation();
    let running = RunningTurn::begin(&id, None);
    let mut waiters = Vec::new();
    for _ in 0..8 {
        let owner = running.turn.clone();
        waiters.push(tokio::spawn(async move {
            until_cancelled(
                Some(&owner),
                std::future::pending::<Result<(), AgentError>>(),
            )
            .await
        }));
    }
    tokio::task::yield_now().await;
    assert!(cancel(&id, None));
    for waiter in waiters {
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), waiter)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(result, Err(AgentError::Cancelled)));
    }
    // A cancellation before waiter registration must also be observed.
    assert!(matches!(
        until_cancelled(Some(&running.turn), async { Ok(()) }).await,
        Err(AgentError::Cancelled)
    ));
    assert_eq!(until_cancelled(None, async { Ok(42) }).await.unwrap(), 42);
}

#[test]
fn concurrent_replacements_leave_only_the_latest_owner_and_cleanup_all_generations() {
    let id = conversation();
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let id = id.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                RunningTurn::begin(&id, None)
            })
        })
        .collect();
    let owners: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(
        owners
            .iter()
            .filter(|owner| !owner.turn.cancelled())
            .count(),
        1
    );
    assert_eq!(
        owners
            .iter()
            .filter(|owner| owner.turn.accepts_event(true))
            .count(),
        1
    );
    let generations: Vec<_> = owners
        .iter()
        .map(|owner| owner.turn.generation().to_owned())
        .collect();
    assert!(cancel(&id, None));
    assert!(owners.iter().all(|owner| owner.turn.cancelled()));
    drop(owners);
    for generation in generations {
        assert_eq!(registered(&id, &generation), (false, false));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn remote_provider_cancel_exits_while_waiting_for_http_headers_or_more_sse_data() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for streaming in [false, true] {
        let id = conversation();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let running = RunningTurn::begin(&id, Some("http-request".into()));
        let owner = running.turn.clone();
        let request_id = id.clone();
        let (token_received, first_token) = oneshot::channel();
        let provider_task = tokio::spawn(CURRENT.scope(owner, async move {
            let provider = AgentProvider::Remote {
                config: crate::ai::AiConfig {
                    provider: "openai".into(),
                    base_url: url,
                    api_key: "test-placeholder".into(),
                    model: "local-http-fixture".into(),
                    ..Default::default()
                },
            };
            if streaming {
                let mut token_received = Some(token_received);
                provider
                    .answer(Vec::new(), &request_id, 0, move |chunk, _| {
                        if !chunk.is_empty() {
                            if let Some(sender) = token_received.take() {
                                let _ = sender.send(());
                            }
                        }
                    })
                    .await
            } else {
                provider.plan(Vec::new(), 32, 0.0, "", 0, &request_id).await
            }
        }));
        let (mut socket, _) =
            tokio::time::timeout(std::time::Duration::from_secs(3), listener.accept())
                .await
                .unwrap()
                .unwrap();
        let mut request = Vec::new();
        // Wait for a complete HTTP request to prove this is an active provider
        // request, not an arbitrary pending future or an unopened connection.
        loop {
            let mut bytes = [0; 4096];
            let count =
                tokio::time::timeout(std::time::Duration::from_secs(3), socket.read(&mut bytes))
                    .await
                    .unwrap()
                    .unwrap();
            assert_ne!(count, 0);
            request.extend_from_slice(&bytes[..count]);
            if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&request[..end]).to_lowercase();
                let body_len: usize = header
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                if request.len() >= end + 4 + body_len {
                    break;
                }
            }
        }
        assert!(String::from_utf8_lossy(&request).contains("/chat/completions"));
        if streaming {
            let data = format!(
                "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}}}}]}}\n\n",
                "live token ".repeat(20)
            );
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n", data.len(), data);
            socket.write_all(response.as_bytes()).await.unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(3), first_token)
                .await
                .unwrap()
                .unwrap();
        }
        // The server sends neither headers (plan) nor the next SSE event
        // (answer). Cancellation must not wait for another response byte.
        assert!(cancel(&id, Some("http-request")));
        let result = tokio::time::timeout(std::time::Duration::from_secs(3), provider_task)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(result, Err(AgentError::Cancelled)));
        let mut byte = [0; 1];
        let closed =
            tokio::time::timeout(std::time::Duration::from_secs(3), socket.read(&mut byte))
                .await
                .unwrap();
        assert!(
            matches!(closed, Ok(0) | Err(_)),
            "cancel left the HTTP connection waiting"
        );
        let generation = running.turn.generation().to_owned();
        drop(running);
        assert_eq!(registered(&id, &generation), (false, false));
    }
}

#[test]
fn admission_is_registered_and_runs_preparation_before_any_completion_future_is_polled() {
    let id = conversation();
    let (entered, started) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    // This test has no entered Tokio runtime, just like the IPC receiver.
    let mut first = Admission::start(&id, Some("first-admission".into()), move |owner| {
        entered.send(()).unwrap();
        released.recv().unwrap();
        assert!(owner.cancelled());
        Err::<(), _>(AgentError::Cancelled)
    });
    let old_generation = first.running.turn.generation().to_owned();
    started
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap();
    let completion = async move { first.prepared().await };
    assert!(cancel(&id, Some("first-admission")));
    let mut latest = Admission::start(&id, Some("latest-admission".into()), |owner| {
        assert!(!owner.cancelled());
        Ok("latest prepared")
    });
    // A late old cancel cannot reach the newer admitted request, even while its
    // own completion future has never been polled either.
    assert!(!cancel(&id, Some("first-admission")));
    assert!(!latest.running.turn.cancelled());
    let latest_value = tauri::async_runtime::block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(3), latest.prepared())
            .await
            .unwrap()
            .unwrap()
    });
    assert_eq!(latest_value, "latest prepared");
    release.send(()).unwrap();
    let old_result = tauri::async_runtime::block_on(completion);
    assert!(matches!(old_result, Err(AgentError::Cancelled)));
    assert_eq!(registered(&id, &old_generation), (true, false));
    assert!(!latest.running.turn.cancelled());
    assert!(latest.running.turn.accepts_event(false));
    latest.running.finish();
}
