//! One pending UI value; queued dispatchers carry only a lifetime-safe ticket.
use std::sync::Mutex;

pub(crate) trait CurrentUiValue {
    fn is_current(&self) -> bool;
}

pub(crate) struct LatestUiMailbox<T>(Mutex<MailboxState<T>>);
struct MailboxState<T> {
    pending: Option<T>,
    scheduled: Option<u64>,
    next_ticket: u64,
}

impl<T> Default for LatestUiMailbox<T> {
    fn default() -> Self {
        Self(Mutex::new(MailboxState {
            pending: None,
            scheduled: None,
            next_ticket: 0,
        }))
    }
}

impl<T: CurrentUiValue> LatestUiMailbox<T> {
    pub(crate) fn push(&self, value: T) -> Option<u64> {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if !value.is_current() {
            return None;
        }
        state.pending = Some(value);
        if state.scheduled.is_some() {
            return None;
        }
        state.next_ticket = state.next_ticket.wrapping_add(1);
        let ticket = state.next_ticket;
        state.scheduled = Some(ticket);
        Some(ticket)
    }

    #[cfg(any(target_os = "windows", test))]
    pub(crate) fn is_scheduled(&self, ticket: u64) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .scheduled
            == Some(ticket)
    }

    pub(crate) fn take(&self, ticket: u64) -> Option<T> {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if state.scheduled != Some(ticket) {
            return None;
        }
        state.scheduled = None;
        state.pending.take().filter(CurrentUiValue::is_current)
    }

    pub(crate) fn cancel(&self, ticket: u64) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if state.scheduled == Some(ticket) {
            state.scheduled = None;
            state.pending = None;
        }
    }

    pub(crate) fn clear(&self) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        state.pending = None;
        state.scheduled = None;
    }
}
