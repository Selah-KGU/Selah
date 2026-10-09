use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

fn state() -> ShutdownState {
    ShutdownState(Mutex::new(Phase::Idle))
}

#[test]
fn repeated_quit_reserves_one_drain_and_retains_the_exit_code() {
    let state = state();
    assert!(state.begin(7));
    assert!(!state.begin(0));
    assert_eq!(state.ready(), ExitIntent::Quit(7));
    assert!(!state.begin(0));
}

#[test]
fn restart_intent_survives_preventable_exit_and_repeated_requests() {
    let state = state();
    state.request_restart();
    assert_eq!(state.phase(), Phase::Requested(ExitIntent::Restart));
    assert!(state.begin(0));
    assert!(!state.begin(0));
    state.request_restart();
    assert_eq!(state.ready(), ExitIntent::Restart);
    state.request_restart();
    assert_eq!(state.phase(), Phase::Ready(ExitIntent::Restart));
    let state = self::state();
    assert!(state.begin(0));
    state.request_restart();
    assert_eq!(state.ready(), ExitIntent::Restart);
}

#[test]
fn failed_save_cancels_the_requested_restart_and_allows_another_quit() {
    let state = state();
    state.request_restart();
    assert!(state.begin(0));
    state.reset();
    assert_eq!(state.phase(), Phase::Idle);
    assert!(state.begin(9));
    assert_eq!(state.ready(), ExitIntent::Quit(9));
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_drains_config_conversations_and_cache_before_live_flush_and_can_reopen() {
    use crate::background_queue::Queue;
    let conversations = Arc::new(Queue::new("会話の処理に失敗しました"));
    let settings = Arc::new(Queue::new("AI設定処理失敗"));
    let cache = Arc::new(Queue::new("キャッシュ処理失敗"));
    let path =
        std::env::temp_dir().join(format!("selah-config-quit-{}.json", uuid::Uuid::new_v4()));
    let writing = path.clone();
    let notifications = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let notified = notifications.clone();
    let (config_entered, config_started) = tokio::sync::oneshot::channel();
    let (config_release, config_released) = std::sync::mpsc::channel();
    drop(settings.submit(move || {
        std::fs::write(writing, r#"{"model":"final setting 🌕"}"#).map_err(|e| e.to_string())?;
        config_entered.send(()).unwrap();
        // Disk commit has finished, but its notification is still pending.
        config_released
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        notified.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }));
    let notified = notifications.clone();
    let (db_entered, db_started) = tokio::sync::oneshot::channel();
    let (db_release, db_released) = std::sync::mpsc::channel();
    drop(conversations.submit(move || {
        db_entered.send(()).unwrap();
        db_released.recv_timeout(Duration::from_secs(10)).unwrap();
        notified.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }));
    let notified = notifications.clone();
    let (cache_entered, cache_started) = tokio::sync::oneshot::channel();
    let (cache_release, cache_released) = std::sync::mpsc::channel();
    drop(cache.submit(move || {
        cache_entered.send(()).unwrap();
        cache_released
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        notified.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }));
    config_started.await.unwrap();
    db_started.await.unwrap();
    cache_started.await.unwrap();
    settings.seal();
    conversations.seal();
    cache.seal();
    assert!(settings
        .submit(|| -> Result<(), String> { panic!("new config during quit") })
        .await
        .is_err());
    let saves = Arc::new(crate::pending_persistence::PendingPersistence::default());
    saves.begin_shutdown();
    let waiting = saves.clone();
    let waiting_settings = settings.clone();
    let waiting_conversations = conversations.clone();
    let waiting_cache = cache.clone();
    let flushed = Arc::new(AtomicBool::new(false));
    let flushing = flushed.clone();
    let expected_notifications = notifications.clone();
    let task = tokio::spawn(async move {
        drain_with(
            &waiting,
            || std::future::ready(Ok(true)),
            || async {
                tokio::join!(
                    waiting_settings.drained(),
                    waiting_conversations.drained(),
                    waiting_cache.drained()
                );
            },
            || async move {
                assert_eq!(expected_notifications.load(Ordering::SeqCst), 3);
                flushing.store(true, Ordering::SeqCst);
                Ok(())
            },
        )
        .await
    });
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    assert!(!task.is_finished());
    assert!(!flushed.load(Ordering::SeqCst));
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("final setting 🌕"));
    config_release.send(()).unwrap();
    settings.drained().await;
    assert!(!task.is_finished(), "quit waited only for settings");
    assert!(!flushed.load(Ordering::SeqCst));
    db_release.send(()).unwrap();
    conversations.drained().await;
    assert!(!task.is_finished(), "quit did not wait for cache writes");
    assert!(!flushed.load(Ordering::SeqCst));
    cache_release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(flushed.load(Ordering::SeqCst));
    // A canceled quit reopens all three lanes; errors stay in their
    // command replies and must not become a recording-save error.
    settings.reopen();
    conversations.reopen();
    cache.reopen();
    assert!(settings
        .submit(|| Err::<(), _>("setting write failed".into()))
        .await
        .is_err());
    assert_eq!(
        conversations
            .submit(|| Ok("conversation resumed"))
            .await
            .unwrap(),
        "conversation resumed"
    );
    assert_eq!(
        settings.submit(|| Ok("settings resumed")).await.unwrap(),
        "settings resumed"
    );
    assert_eq!(
        cache.submit(|| Ok("cache resumed")).await.unwrap(),
        "cache resumed"
    );
    settings.seal();
    conversations.seal();
    cache.seal();
    tokio::join!(settings.drained(), conversations.drained(), cache.drained());
    std::fs::remove_file(path).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn stop_timeouts_keep_waiting_and_include_the_final_callback_save_before_live_flush() {
    let saves = Arc::new(crate::pending_persistence::PendingPersistence::default());
    saves.begin_shutdown();
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = attempts.clone();
    let stopping = saves.clone();
    let saved = Arc::new(AtomicBool::new(false));
    let storage_flag = saved.clone();
    let (release, blocked) = std::sync::mpsc::channel();
    let block = Arc::new(Mutex::new(Some(blocked)));
    let flushed = Arc::new(AtomicBool::new(false));
    let flushing = flushed.clone();
    let waiting = saves.clone();
    let task = tokio::spawn(async move {
        drain_with(
            &waiting,
            move || {
                let attempt = count.fetch_add(1, Ordering::SeqCst);
                if attempt == 2 {
                    let permit = stopping.reserve().unwrap();
                    let block = block.lock().unwrap().take().unwrap();
                    let flag = storage_flag.clone();
                    tokio::task::spawn_blocking(move || {
                        block.recv().unwrap();
                        flag.store(true, Ordering::SeqCst);
                        permit.complete(Ok(()));
                    });
                }
                std::future::ready(Ok(attempt >= 2))
            },
            || std::future::ready(()),
            || async move {
                assert!(
                    saved.load(Ordering::SeqCst),
                    "LIVE flushed before accepted input was saved"
                );
                flushing.store(true, Ordering::SeqCst);
                Ok(())
            },
        )
        .await
    });
    while attempts.load(Ordering::SeqCst) < 3 {
        tokio::task::yield_now().await;
    }
    assert!(!task.is_finished());
    assert!(!flushed.load(Ordering::SeqCst));
    assert_eq!(tokio::spawn(async { 3 }).await.unwrap(), 3);
    release.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert!(flushed.load(Ordering::SeqCst));
    assert!(saves.reserve().is_err());
}

#[tokio::test]
async fn stop_or_storage_errors_never_report_a_completed_shutdown() {
    let saves = Arc::new(crate::pending_persistence::PendingPersistence::default());
    saves.begin_shutdown();
    let result = drain_with(
        &saves,
        || std::future::ready(Err("stop failed".into())),
        || std::future::ready(()),
        || async {
            panic!("saved after stop failure");
        },
    )
    .await;
    assert_eq!(result, Err("stop failed".into()));
    let permit = saves.reserve().unwrap();
    permit.complete(Err("input storage failed"));
    let result = drain_with(
        &saves,
        || std::future::ready(Ok(true)),
        || std::future::ready(()),
        || async {
            panic!("shutdown continued after input storage failure");
        },
    )
    .await;
    assert_eq!(result, Err("input storage failed".into()));
    saves.reopen();
    let result = drain_with(
        &saves,
        || std::future::ready(Ok(true)),
        || std::future::ready(()),
        || async { Err("LIVE disk full".into()) },
    )
    .await;
    assert_eq!(result, Err("LIVE disk full".into()));
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_waits_for_sqlite_mutation_and_commit_notification_even_when_reply_was_dropped() {
    let path = std::env::temp_dir().join(format!("selah-quit-mutation-{}", uuid::Uuid::new_v4()));
    let db = Arc::new(crate::db::Database::open(&path).unwrap());
    db.agent_create_conversation("conversation", "original")
        .unwrap();
    let conn = rusqlite::Connection::open(path.join("courses.db")).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE;").unwrap();
    let queue = Arc::new(crate::background_queue::Queue::new(
        "会話の処理に失敗しました",
    ));
    let saves = Arc::new(crate::pending_persistence::PendingPersistence::default());
    let input = saves.reserve().unwrap();
    saves.begin_shutdown();
    let writing = db.clone();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (committed, commit_seen) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let notification = Arc::new(AtomicBool::new(false));
    let notifying = notification.clone();
    let caller = std::thread::current().id();
    drop(queue.submit(move || {
        assert_ne!(std::thread::current().id(), caller);
        entered.send(()).unwrap();
        writing.agent_rename_conversation("conversation", "最後のタイトル 🌕")?;
        committed.send(()).unwrap();
        // The database has committed, but the admitted job is still publishing.
        released.recv().unwrap();
        notifying.store(true, Ordering::SeqCst);
        Ok(())
    }));
    started.await.unwrap();
    queue.seal();
    assert!(queue
        .submit(|| -> Result<(), String> { panic!("new mutation during quit") })
        .await
        .is_err());
    let flushing = db.clone();
    let flushed = Arc::new(AtomicBool::new(false));
    let flag = flushed.clone();
    let waiting = saves.clone();
    let task = tokio::spawn(async move {
        drain_with(
            &waiting,
            || std::future::ready(Ok(true)),
            || async move {
                queue.drained().await;
            },
            || async move {
                assert!(
                    notification.load(Ordering::SeqCst),
                    "exit bypassed commit notification"
                );
                assert_eq!(
                    flushing.agent_list_conversations().unwrap()[0].title,
                    "最後のタイトル 🌕"
                );
                flag.store(true, Ordering::SeqCst);
                Ok(())
            },
        )
        .await
    });
    input.complete(Ok(()));
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    assert!(!task.is_finished());
    assert!(!flushed.load(Ordering::SeqCst));
    conn.execute_batch("COMMIT;").unwrap();
    commit_seen.await.unwrap();
    assert_eq!(
        db.agent_list_conversations().unwrap()[0].title,
        "最後のタイトル 🌕"
    );
    assert!(!task.is_finished());
    assert!(!flushed.load(Ordering::SeqCst));
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(flushed.load(Ordering::SeqCst));
    drop(conn);
    drop(db);
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn input_save_failure_keeps_exit_unready_and_mutation_gate_can_reopen_for_retry() {
    let queue = Arc::new(crate::background_queue::Queue::new(
        "会話の処理に失敗しました",
    ));
    let saves = Arc::new(crate::pending_persistence::PendingPersistence::default());
    let shutdown = state();
    shutdown.request_restart();
    assert!(shutdown.begin(0));
    queue.seal();
    saves.begin_shutdown();
    let permit = saves.reserve().unwrap();
    permit.complete(Err("input disk full"));
    let waiting = queue.clone();
    let result = drain_with(
        &saves,
        || std::future::ready(Ok(true)),
        || async move {
            waiting.drained().await;
        },
        || async { panic!("exit finalized after input save failure") },
    )
    .await;
    assert_eq!(result, Err("input disk full".into()));
    assert_eq!(shutdown.phase(), Phase::Draining(ExitIntent::Restart));
    assert!(queue.submit(|| Ok(())).await.is_err());
    saves.reopen();
    queue.reopen();
    shutdown.reset();
    assert_eq!(shutdown.phase(), Phase::Idle);
    assert_eq!(
        queue
            .submit(|| Ok("accepted after cancelled quit"))
            .await
            .unwrap(),
        "accepted after cancelled quit"
    );
    let permit = saves.reserve().unwrap();
    permit.complete(Ok(()));
    assert!(shutdown.begin(9));
    queue.seal();
    saves.begin_shutdown();
    let waiting = queue.clone();
    drain_with(
        &saves,
        || std::future::ready(Ok(true)),
        || async move { waiting.drained().await },
        || async { Ok(()) },
    )
    .await
    .unwrap();
    assert_eq!(shutdown.ready(), ExitIntent::Quit(9));
}
