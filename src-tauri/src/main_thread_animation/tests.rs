use super::*;
use std::sync::Arc;
use tokio::sync::mpsc;

fn generation() -> &'static AtomicU64 {
    Box::leak(Box::new(AtomicU64::new(0)))
}

#[tokio::test(flavor = "current_thread")]
async fn delayed_main_thread_does_not_block_other_async_work() {
    let (queue, mut jobs) = mpsc::unbounded_channel::<MainThreadJob>();
    let animation = MainThreadAnimation::start(generation());
    let task = tokio::spawn(async move {
        animation
            .read_with(
                |job| queue.send(job).map_err(|err| err.to_string()),
                || (540.0, 76.0, 720.0, 884.0),
            )
            .await
    });
    let job = jobs.recv().await.unwrap();
    assert!(!task.is_finished());
    // This runs on the same single executor thread while the UI read awaits.
    assert_eq!(tokio::spawn(async { 42 }).await.unwrap(), 42);
    job();
    assert_eq!(task.await.unwrap(), Some((540.0, 76.0, 720.0, 884.0)));
}

#[tokio::test(flavor = "current_thread")]
async fn slow_ui_allows_only_one_unacknowledged_frame_per_animation() {
    let (queue, mut jobs) = mpsc::unbounded_channel::<MainThreadJob>();
    let frames = Arc::new(AtomicU64::new(0));
    let rendered = frames.clone();
    let animation = MainThreadAnimation::start(generation());
    let task = tokio::spawn(async move {
        for _ in 0..60 {
            let rendered = rendered.clone();
            if animation
                .read_with(
                    |job| queue.send(job).map_err(|err| err.to_string()),
                    move || rendered.fetch_add(1, Ordering::Relaxed),
                )
                .await
                .is_none()
            {
                break;
            }
        }
    });
    let first = jobs.recv().await.unwrap();
    for _ in 0..60 {
        tokio::task::yield_now().await;
        assert!(jobs.try_recv().is_err());
    }
    assert_eq!(frames.load(Ordering::Relaxed), 0);
    first();
    for _ in 1..60 {
        jobs.recv().await.unwrap()();
    }
    task.await.unwrap();
    assert_eq!(frames.load(Ordering::Relaxed), 60);
}

#[tokio::test(flavor = "current_thread")]
async fn superseded_queued_frame_cannot_write_over_new_animation() {
    let (queue, mut jobs) = mpsc::unbounded_channel::<MainThreadJob>();
    let generation = generation();
    let animation = MainThreadAnimation::start(generation);
    let value = Arc::new(AtomicU64::new(0));
    let old_value = value.clone();
    let old_task = tokio::spawn(async move {
        animation
            .read_with(
                |job| queue.send(job).map_err(|err| err.to_string()),
                move || old_value.store(1, Ordering::Relaxed),
            )
            .await
    });
    let old_job = jobs.recv().await.unwrap();
    let current = MainThreadAnimation::start(generation);
    let current_value = value.clone();
    current
        .read_with(
            |job| {
                job();
                Ok(())
            },
            move || current_value.store(2, Ordering::Relaxed),
        )
        .await;
    old_job();
    assert_eq!(old_task.await.unwrap(), None);
    assert_eq!(value.load(Ordering::Relaxed), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn already_cancelled_animation_does_not_enqueue_ui_work() {
    let generation = generation();
    let old = MainThreadAnimation::start(generation);
    MainThreadAnimation::start(generation);
    assert_eq!(
        old.read_with(|_| panic!("obsolete work was queued"), || 42)
            .await,
        None
    );
}

#[tokio::test(flavor = "current_thread")]
async fn obsolete_hide_cannot_cancel_new_texts_resize_and_fade() {
    let (queue, mut jobs) = mpsc::unbounded_channel::<MainThreadJob>();
    let hide_generation = generation();
    let resize_generation = generation();
    let fade_generation = generation();
    let hide = MainThreadAnimation::start(hide_generation);
    let task = tokio::spawn(async move {
        hide.read_with(
            |job| queue.send(job).map_err(|err| err.to_string()),
            move || {
                MainThreadAnimation::start(resize_generation);
                MainThreadAnimation::start(fade_generation);
            },
        )
        .await
    });
    let obsolete_hide = jobs.recv().await.unwrap();
    // New caption executes before a delayed hide already in the UI queue.
    MainThreadAnimation::start(hide_generation);
    let resize = MainThreadAnimation::start(resize_generation);
    let fade = MainThreadAnimation::start(fade_generation);
    obsolete_hide();
    assert_eq!(task.await.unwrap(), None);
    assert!(resize.is_current());
    assert!(fade.is_current());
}

#[tokio::test(flavor = "current_thread")]
async fn dispatch_failure_and_dropped_job_complete_without_fallback_geometry() {
    let result = dispatch(|_| Err("event loop closed".to_owned()), || true, || 42).await;
    assert_eq!(result, Err("event loop closed".to_owned()));
    let result = dispatch(|_| Ok(()), || true, || 42).await;
    assert!(result
        .unwrap_err()
        .contains("dropped before acknowledgement"));
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_waiter_leaves_no_queued_ui_side_effect() {
    let (queue, mut jobs) = mpsc::unbounded_channel::<MainThreadJob>();
    let value = Arc::new(AtomicU64::new(0));
    let queued_value = value.clone();
    let task = tokio::spawn(async move {
        dispatch(
            |job| queue.send(job).map_err(|err| err.to_string()),
            || true,
            move || queued_value.store(1, Ordering::Relaxed),
        )
        .await
    });
    let job = jobs.recv().await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    job();
    assert_eq!(value.load(Ordering::Relaxed), 0);
}
