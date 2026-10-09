use super::*;
use tokio::sync::oneshot;

#[tokio::test(flavor = "current_thread")]
async fn pending_blocking_save_keeps_exit_waiting_without_occupying_the_executor() {
    let saves = Arc::new(PendingPersistence::default());
    let permit = saves.reserve().unwrap();
    saves.begin_shutdown();
    saves.seal();
    assert!(saves.reserve().is_err());
    let (started, entered) = oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let storage = tokio::task::spawn_blocking(move || {
        started.send(()).unwrap();
        blocked.recv().unwrap();
        permit.complete(Ok(()));
    });
    entered.await.unwrap();
    let waiting = saves.clone();
    let exit = tokio::spawn(async move { waiting.drained().await });
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    assert!(!exit.is_finished());
    release.send(()).unwrap();
    storage.await.unwrap();
    exit.await.unwrap().unwrap();
}

#[tokio::test]
async fn completion_before_or_during_notification_registration_wakes_every_waiter() {
    for _ in 0..100 {
        let saves = Arc::new(PendingPersistence::default());
        let permit = saves.reserve().unwrap();
        saves.begin_shutdown();
        saves.seal();
        let tasks: Vec<_> = (0..8)
            .map(|_| {
                let saves = saves.clone();
                tokio::spawn(async move { saves.drained().await })
            })
            .collect();
        permit.complete(Ok(()));
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            for task in tasks {
                task.await.unwrap().unwrap();
            }
        })
        .await
        .expect("lost drain notification");
    }
}

#[tokio::test]
async fn failed_or_abandoned_save_during_exit_reports_failure_and_reopen_allows_retry() {
    for abandon in [false, true] {
        let saves = Arc::new(PendingPersistence::default());
        let permit = saves.reserve().unwrap();
        saves.begin_shutdown();
        saves.seal();
        if abandon {
            drop(permit);
        } else {
            permit.complete(Err("disk full"));
        }
        assert!(saves.drained().await.is_err());
        assert!(saves.reserve().is_err());
        saves.reopen();
        let retry = saves.reserve().unwrap();
        saves.begin_shutdown();
        saves.seal();
        retry.complete(Ok(()));
        saves.drained().await.unwrap();
    }
}

#[tokio::test]
async fn storage_error_already_reported_before_quit_does_not_poison_a_later_exit() {
    let saves = Arc::new(PendingPersistence::default());
    saves
        .reserve()
        .unwrap()
        .complete(Err("previous write failure"));
    saves.begin_shutdown();
    saves.seal();
    saves.drained().await.unwrap();
}
