use super::*;

#[tokio::test(flavor = "current_thread")]
async fn admission_order_survives_reverse_reply_polling_dropped_replies_errors_and_panics() {
    let queue = Arc::new(Queue::new("会話の処理に失敗しました"));
    let ledger = Arc::new(Mutex::new(Vec::new()));
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let mut first = Some((entered, released));
    let mut replies = Vec::new();
    for n in 0..32 {
        let gate = if n == 0 { first.take() } else { None };
        let ledger = ledger.clone();
        replies.push(queue.submit(move || {
            if let Some((entered, released)) = gate {
                entered.send(()).unwrap();
                released.recv().unwrap();
            }
            ledger.lock().unwrap().push(n);
            if n == 10 {
                return Err("original failure".into());
            }
            if n == 20 {
                panic!("one failed job must not strand the queue");
            }
            Ok(n)
        }));
    }
    // Every job was admitted, but no completion future has been polled.
    started.await.unwrap();
    assert!(ledger.lock().unwrap().is_empty());
    drop(replies.remove(0));
    release.send(()).unwrap();
    for (index, reply) in replies.into_iter().enumerate().rev() {
        let n = index + 1;
        let result = tokio::time::timeout(std::time::Duration::from_secs(3), reply)
            .await
            .unwrap();
        match n {
            10 => assert_eq!(result.unwrap_err(), "original failure"),
            20 => assert!(result.unwrap_err().contains("worker panic")),
            _ => assert_eq!(result.unwrap(), n),
        }
    }
    assert_eq!(*ledger.lock().unwrap(), (0..32).collect::<Vec<_>>());
    // The queue can restart after its drain becomes idle, or accept work while
    // the last worker is exiting; neither path loses the new job.
    assert_eq!(queue.submit(|| Ok(32)).await.unwrap(), 32);
}

#[tokio::test(flavor = "current_thread")]
async fn a_running_job_can_admit_another_job_without_holding_the_queue_mutex() {
    let queue = Arc::new(Queue::new("会話の処理に失敗しました"));
    let nested = queue.clone();
    let reply = queue.submit(move || Ok(nested.submit(|| Ok("nested mutation"))));
    let nested_reply = tokio::time::timeout(std::time::Duration::from_secs(3), reply)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(nested_reply.await.unwrap(), "nested mutation");
}

#[tokio::test(flavor = "current_thread")]
async fn sealing_rejects_new_work_and_notifies_every_waiter_after_dropped_replies_and_failures() {
    let queue = Arc::new(Queue::new("会話の処理に失敗しました"));
    let ledger = Arc::new(Mutex::new(Vec::new()));
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let first_ledger = ledger.clone();
    drop(queue.submit(move || {
        entered.send(()).unwrap();
        released.recv().unwrap();
        first_ledger.lock().unwrap().push(0);
        Ok(())
    }));
    for index in 1..=3 {
        let ledger = ledger.clone();
        drop(queue.submit(move || -> Result<(), String> {
            ledger.lock().unwrap().push(index);
            match index {
                1 => Err("handled mutation failure".into()),
                2 => panic!("handled worker panic"),
                _ => Ok(()),
            }
        }));
    }
    started.await.unwrap();
    queue.seal();
    let rejected =
        queue.submit(|| -> Result<(), String> { panic!("sealed queue executed new work") });
    assert_eq!(rejected.await.unwrap_err(), "アプリケーションを終了中です");
    let mut waiters = Vec::new();
    for _ in 0..8 {
        let queue = queue.clone();
        let (entered, started) = tokio::sync::oneshot::channel();
        let waiter = tokio::spawn(async move {
            entered.send(()).unwrap();
            queue.drained().await;
        });
        started.await.unwrap();
        assert!(!waiter.is_finished());
        waiters.push(waiter);
    }
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    release.send(()).unwrap();
    for waiter in waiters {
        tokio::time::timeout(std::time::Duration::from_secs(3), waiter)
            .await
            .unwrap()
            .unwrap();
    }
    assert_eq!(*ledger.lock().unwrap(), [0, 1, 2, 3]);
    assert!(
        queue.submit(|| Ok(())).await.is_err(),
        "draining unsealed admission"
    );
    // An already idle sealed queue never waits for a future notification.
    tokio::time::timeout(std::time::Duration::from_secs(3), queue.drained())
        .await
        .unwrap();
    queue.reopen();
    assert_eq!(
        queue.submit(|| Ok("new mutation")).await.unwrap(),
        "new mutation"
    );
    queue.seal();
    queue.drained().await;
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_shutdown_reopens_admission_without_losing_or_reordering_pending_work() {
    let queue = Arc::new(Queue::new("会話の処理に失敗しました"));
    let ledger = Arc::new(Mutex::new(Vec::new()));
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let first_ledger = ledger.clone();
    drop(queue.submit(move || {
        entered.send(()).unwrap();
        released.recv().unwrap();
        first_ledger.lock().unwrap().push(0);
        Ok(())
    }));
    let second_ledger = ledger.clone();
    drop(queue.submit(move || {
        second_ledger.lock().unwrap().push(1);
        Ok(())
    }));
    started.await.unwrap();
    queue.seal();
    assert!(queue
        .submit(|| -> Result<(), String> { panic!("rejected work ran") })
        .await
        .is_err());
    queue.reopen();
    let third_ledger = ledger.clone();
    let third = queue.submit(move || {
        third_ledger.lock().unwrap().push(2);
        Ok(())
    });
    queue.seal();
    release.send(()).unwrap();
    third.await.unwrap();
    queue.drained().await;
    assert_eq!(*ledger.lock().unwrap(), [0, 1, 2]);
}

#[tokio::test(flavor = "current_thread")]
async fn concurrent_seal_and_admission_execute_exactly_the_accepted_jobs() {
    for _ in 0..24 {
        let queue = Arc::new(Queue::new("会話の処理に失敗しました"));
        let ledger = Arc::new(Mutex::new(Vec::new()));
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, released) = std::sync::mpsc::channel();
        drop(queue.submit(move || {
            entered.send(()).unwrap();
            released.recv().unwrap();
            Ok(0)
        }));
        started.await.unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(9));
        let workers: Vec<_> = (0..8)
            .map(|index| {
                let queue = queue.clone();
                let ledger = ledger.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    queue.submit(move || {
                        ledger.lock().unwrap().push(index);
                        Ok(index)
                    })
                })
            })
            .collect();
        barrier.wait();
        queue.seal();
        let replies: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        release.send(()).unwrap();
        let mut accepted = Vec::new();
        for reply in replies {
            match reply.await {
                Ok(index) => accepted.push(index),
                Err(error) => assert_eq!(error, "アプリケーションを終了中です"),
            }
        }
        tokio::time::timeout(std::time::Duration::from_secs(3), queue.drained())
            .await
            .unwrap();
        let mut executed = ledger.lock().unwrap().clone();
        accepted.sort_unstable();
        executed.sort_unstable();
        assert_eq!(executed, accepted);
        assert!(queue.submit(|| Ok(9)).await.is_err());
    }
}
