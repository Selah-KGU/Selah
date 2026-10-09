use super::*;
use crate::agent_turn_scope::{RunningTurn, CURRENT};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[tokio::test(flavor = "current_thread")]
async fn shared_tool_wait_keeps_full_results_and_existing_timeout_payload() {
    let id = uuid::Uuid::new_v4().to_string();
    let running = RunningTurn::begin(&id, None);
    CURRENT
        .scope(running.turn.clone(), async {
            let result =
                json!({ "text": "全文", "nested": [1, 2, 3], "image_data_base64": "AA==" });
            assert_eq!(
                await_tool_result(&id, std::time::Duration::from_secs(30), async {
                    result.clone()
                })
                .await
                .unwrap(),
                result
            );
            let result = await_tool_result(&id, std::time::Duration::ZERO, std::future::pending())
                .await
                .unwrap();
            assert_eq!(result, json!({ "error": "tool timed out after 0s" }));
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_tool_wait_drops_pending_dispatch_and_cannot_start_a_followup() {
    struct Dropped(Arc<AtomicBool>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let running = RunningTurn::begin(&id, Some("tool-request".into()));
    let owner = running.turn.clone();
    let dropped = Arc::new(AtomicBool::new(false));
    let release = Dropped(dropped.clone());
    let (entered, started) = tokio::sync::oneshot::channel();
    let task_id = id.clone();
    let task = tokio::spawn(CURRENT.scope(owner, async move {
        await_tool_result(&task_id, std::time::Duration::from_secs(300), async move {
            let _release = release;
            entered.send(()).unwrap();
            std::future::pending().await
        })
        .await
    }));
    started.await.unwrap();
    assert!(crate::agent_turn_scope::cancel(&id, Some("tool-request")));
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Err(AgentError::Cancelled)));
    assert!(dropped.load(Ordering::Acquire));
    CURRENT
        .scope(running.turn.clone(), async {
            let result = await_tool_result(&id, std::time::Duration::from_secs(300), async {
                panic!("cancelled request started an automatic tool");
            })
            .await;
            assert!(matches!(result, Err(AgentError::Cancelled)));
        })
        .await;
}
