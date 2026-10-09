//! A reader owns one delivery. Reopening, consuming, acknowledging or closing
//! it invalidates both a slow file read and its later retry without copying text.
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(super) struct Registry(Mutex<RegistryState>);

#[derive(Default)]
struct RegistryState {
    revision: u64,
    readers: HashMap<String, Arc<Delivery>>,
}

pub(super) struct Delivery {
    pub revision: u64,
    state: Mutex<DeliveryState>,
}

struct DeliveryState {
    active: bool,
    payload: Option<Arc<Value>>,
}

impl Delivery {
    pub fn publish(&self, payload: Value) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.active {
            return false;
        }
        state.payload = Some(Arc::new(payload));
        true
    }

    pub fn snapshot(&self) -> Option<Arc<Value>> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .payload
            .clone()
    }

    fn cancel(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.active = false;
        state.payload = None;
    }
}

impl Registry {
    pub fn reserve(&self, label: &str) -> Arc<Delivery> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.revision = state
            .revision
            .checked_add(1)
            .expect("Markdown delivery revision overflow");
        let delivery = Arc::new(Delivery {
            revision: state.revision,
            state: Mutex::new(DeliveryState {
                active: true,
                payload: None,
            }),
        });
        if let Some(old) = state.readers.insert(label.into(), delivery.clone()) {
            old.cancel();
        }
        delivery
    }

    pub fn take(&self, label: &str) -> Option<Arc<Value>> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        // A startup poll before the read finishes must keep its reservation.
        let payload = state.readers.get(label)?.snapshot()?;
        state.readers.remove(label)?.cancel();
        Some(payload)
    }

    pub fn acknowledge(&self, label: &str, revision: u64) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state
            .readers
            .get(label)
            .is_some_and(|d| d.revision == revision)
        {
            state.readers.remove(label).unwrap().cancel();
        }
    }

    pub fn discard(&self, label: &str) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(delivery) = state.readers.remove(label) {
            delivery.cancel();
        }
    }
}

#[cfg(test)]
#[path = "markdown_delivery_tests.rs"]
mod tests;
