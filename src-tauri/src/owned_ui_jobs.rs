//! Ticketed UI closures. Window teardown drops only that window's pending work.
use crate::main_thread_animation::MainThreadJob;
use std::collections::HashMap;

pub(crate) struct OwnedUiJobs<Owner> {
    next_ticket: u64,
    jobs: HashMap<u64, (Owner, MainThreadJob)>,
}
impl<Owner: Copy + Eq> Default for OwnedUiJobs<Owner> {
    fn default() -> Self {
        Self {
            next_ticket: 0,
            jobs: HashMap::new(),
        }
    }
}
impl<Owner: Copy + Eq> OwnedUiJobs<Owner> {
    pub(crate) fn insert(&mut self, owner: Owner, job: MainThreadJob) -> u64 {
        loop {
            self.next_ticket = self.next_ticket.wrapping_add(1);
            if !self.jobs.contains_key(&self.next_ticket) {
                break;
            }
        }
        self.jobs.insert(self.next_ticket, (owner, job));
        self.next_ticket
    }
    pub(crate) fn take(&mut self, owner: Owner, ticket: u64) -> Option<MainThreadJob> {
        if self
            .jobs
            .get(&ticket)
            .is_some_and(|(current, _)| *current == owner)
        {
            self.jobs.remove(&ticket).map(|(_, job)| job)
        } else {
            None
        }
    }
    /// Return closures so their acknowledgement senders are dropped outside locks.
    pub(crate) fn drain_owner(&mut self, owner: Owner) -> Vec<MainThreadJob> {
        let tickets: Vec<_> = self
            .jobs
            .iter()
            .filter_map(|(&id, (current, _))| (*current == owner).then_some(id))
            .collect();
        tickets
            .into_iter()
            .filter_map(|id| self.take(owner, id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::main_thread_animation::MainThreadAnimation;
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    };

    #[test]
    fn wrong_window_and_duplicate_message_cannot_consume_pending_work() {
        let mut jobs = OwnedUiJobs::default();
        let count = Arc::new(AtomicU64::new(0));
        let output = count.clone();
        let a = jobs.insert(
            10,
            Box::new(move || {
                output.fetch_add(1, Ordering::Relaxed);
            }),
        );
        assert!(jobs.take(20, a).is_none());
        jobs.take(10, a).unwrap()();
        assert!(jobs.take(10, a).is_none());
        assert_eq!(count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn destroying_one_window_wakes_its_waiter_and_preserves_another_windows_job() {
        let jobs = Arc::new(Mutex::new(OwnedUiJobs::default()));
        let generation = Box::leak(Box::new(AtomicU64::new(0)));
        let animation = MainThreadAnimation::start(generation);
        let queue = jobs.clone();
        let task = tokio::spawn(async move {
            animation
                .read_with(
                    |job| {
                        queue.lock().unwrap().insert(10, job);
                        Ok(())
                    },
                    || panic!("destroyed work executed"),
                )
                .await
        });
        tokio::task::yield_now().await;
        assert!(!task.is_finished());
        let other = jobs.lock().unwrap().insert(20, Box::new(|| {}));
        let removed = jobs.lock().unwrap().drain_owner(10);
        assert_eq!(removed.len(), 1);
        drop(removed);
        assert!(task.await.unwrap().is_none());
        jobs.lock().unwrap().take(20, other).unwrap()();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn slow_owned_ui_allows_one_frame_and_cancellation_skips_its_side_effect() {
        let jobs = Arc::new(Mutex::new(OwnedUiJobs::default()));
        let generation = Box::leak(Box::new(AtomicU64::new(0)));
        let animation = MainThreadAnimation::start(generation);
        let queue = jobs.clone();
        let count = Arc::new(AtomicU64::new(0));
        let output = count.clone();
        let task = tokio::spawn(async move {
            for _ in 0..100 {
                let output = output.clone();
                if animation
                    .read_with(
                        |job| {
                            queue.lock().unwrap().insert(10, job);
                            Ok(())
                        },
                        move || {
                            output.fetch_add(1, Ordering::Relaxed);
                        },
                    )
                    .await
                    .is_none()
                {
                    break;
                }
            }
        });
        for _ in 0..100 {
            tokio::task::yield_now().await;
        }
        assert_eq!(jobs.lock().unwrap().jobs.len(), 1);
        generation.fetch_add(1, Ordering::Relaxed);
        let pending = jobs.lock().unwrap().drain_owner(10);
        for job in pending {
            job();
        }
        task.await.unwrap();
        assert_eq!(count.load(Ordering::Relaxed), 0);
        assert!(jobs.lock().unwrap().jobs.is_empty());
    }
}
