//! One policy for every caller: windows, startup, data jobs, and keepalive.
//! Times are supplied by the caller so cooldowns are deterministic in tests.
use super::{RecoveryOutcome, Service};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecoveryTrigger {
    Manual,
    RequestFailure,
    AutomaticRequest,
    Startup,
    Foreground,
    Background,
    Keepalive,
}
impl RecoveryTrigger {
    pub fn probes(self) -> bool {
        !matches!(self, Self::Manual | Self::Keepalive)
    }
    pub fn may_recover_kgc(self) -> bool {
        matches!(
            self,
            Self::Manual | Self::RequestFailure | Self::AutomaticRequest
        )
    }
}
#[derive(Clone, Copy, Default)]
struct Attempts {
    last: Option<i64>,
    failures: u32,
}
pub(super) struct RecoveryPolicy {
    entries: [Attempts; 3],
    started: i64,
    probes: [Option<i64>; 3],
}
impl RecoveryPolicy {
    pub fn new(now: i64) -> Self {
        Self {
            entries: [Attempts::default(); 3],
            started: now,
            probes: [None; 3],
        }
    }
    pub fn retry_at(&self, service: Service, trigger: RecoveryTrigger, now: i64) -> Option<i64> {
        if service == Service::Kgc && !trigger.may_recover_kgc() {
            return Some(i64::MAX);
        }
        let entry = self.entries[service.index()];
        let (last, delay) = if trigger == RecoveryTrigger::Keepalive {
            (entry.last.unwrap_or(self.started), 6 * 60 * 60)
        } else if let Some(last) = entry.last {
            let delay = match trigger {
                RecoveryTrigger::Manual | RecoveryTrigger::RequestFailure => {
                    if entry.failures == 0 {
                        2
                    } else {
                        30
                    }
                }
                _ if service == Service::Kgc => 30 * 60,
                _ if entry.failures == 0 => 30 * 60,
                _ => {
                    (10 * 60 * (1_i64 << entry.failures.saturating_sub(1).min(4))).min(2 * 60 * 60)
                }
            };
            (last, delay)
        } else {
            return None;
        };
        let due = last.saturating_add(delay);
        (now < due).then_some(due)
    }
    pub fn completed(&mut self, service: Service, now: i64, outcome: RecoveryOutcome) {
        let entry = &mut self.entries[service.index()];
        entry.last = Some(now);
        entry.failures = if outcome == RecoveryOutcome::Verified {
            0
        } else {
            entry.failures.saturating_add(1)
        };
    }
}

impl RecoveryPolicy {
    pub fn probed(&mut self, service: Service, now: i64) {
        self.probes[service.index()] = Some(now);
    }
    pub fn probe_due(
        &self,
        service: Service,
        health: super::Health,
        trigger: RecoveryTrigger,
        now: i64,
    ) -> bool {
        let Some(checked) = self.probes[service.index()] else {
            return true;
        };
        let reuse = match health {
            super::Health::Valid if trigger != RecoveryTrigger::RequestFailure => 60,
            super::Health::Unavailable => 30,
            _ => return true,
        };
        now.saturating_sub(checked) >= reuse
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_failures_probe_valid_sessions_but_share_temporary_unavailability() {
        let mut policy = RecoveryPolicy::new(0);
        policy.probed(Service::Luna, 100);
        assert!(!policy.probe_due(
            Service::Luna,
            super::super::Health::Valid,
            RecoveryTrigger::Foreground,
            110
        ));
        assert!(policy.probe_due(
            Service::Luna,
            super::super::Health::Valid,
            RecoveryTrigger::RequestFailure,
            110
        ));
        assert!(!policy.probe_due(
            Service::Luna,
            super::super::Health::Unavailable,
            RecoveryTrigger::RequestFailure,
            110
        ));
        assert!(policy.probe_due(
            Service::Luna,
            super::super::Health::Unavailable,
            RecoveryTrigger::RequestFailure,
            130
        ));
    }

    #[test]
    fn callers_share_failures_but_manual_retry_bypasses_long_background_backoff() {
        let mut policy = RecoveryPolicy::new(100);
        policy.completed(Service::Luna, 100, RecoveryOutcome::Unavailable);
        assert_eq!(
            policy.retry_at(Service::Luna, RecoveryTrigger::Background, 101),
            Some(700)
        );
        assert_eq!(
            policy.retry_at(Service::Luna, RecoveryTrigger::Manual, 101),
            Some(130)
        );
        assert_eq!(
            policy.retry_at(Service::Luna, RecoveryTrigger::Manual, 131),
            None
        );
        assert_eq!(
            policy.retry_at(Service::Kwic, RecoveryTrigger::Background, 101),
            None
        );
        // A skipped attempt does not slide the deadline.
        assert_eq!(
            policy.retry_at(Service::Luna, RecoveryTrigger::Background, 699),
            Some(700)
        );
        assert_eq!(
            policy.retry_at(Service::Luna, RecoveryTrigger::Background, 700),
            None
        );
    }
    #[test]
    fn kgc_is_request_driven_and_keepalive_uses_attempts_not_cookie_expiry() {
        let mut policy = RecoveryPolicy::new(100);
        assert_eq!(
            policy.retry_at(Service::Kgc, RecoveryTrigger::Foreground, 50000),
            Some(i64::MAX)
        );
        assert_eq!(
            policy.retry_at(Service::Kgc, RecoveryTrigger::AutomaticRequest, 100),
            None
        );
        assert_eq!(
            policy.retry_at(Service::Luna, RecoveryTrigger::Keepalive, 101),
            Some(21700)
        );
        policy.completed(Service::Luna, 1000, RecoveryOutcome::Verified);
        assert_eq!(
            policy.retry_at(Service::Luna, RecoveryTrigger::Keepalive, 21700),
            Some(22600)
        );
    }
}
