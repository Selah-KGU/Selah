//! Shared native subtitle admission and a latest-value UI mailbox.
//! Transcript storage remains append-only; only redundant display work is merged.

use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use tauri::{AppHandle, Listener, Manager};

use crate::live::{LiveSessionStatus, LiveState};

const PARTIAL_INTERVAL: Duration = Duration::from_millis(120);

#[derive(Deserialize)]
struct PartialCaption<'a> {
    #[serde(borrow)]
    text: Cow<'a, str>,
    caller: &'a str,
    live_session_id: Option<&'a str>,
    seq: u64,
}

#[derive(Deserialize)]
struct CaptionLine<'a> {
    #[serde(borrow)]
    text: Cow<'a, str>,
}

#[derive(Deserialize)]
struct CommittedCaption<'a> {
    session_id: &'a str,
    line_count: usize,
    #[serde(borrow)]
    line: CaptionLine<'a>,
    seq: Option<u64>,
}

pub(crate) struct Caption {
    pub(crate) text: String,
    pub(crate) session_id: String,
    pub(crate) is_final: bool,
    revision: u64,
    current_revision: Arc<AtomicU64>,
}

impl Caption {
    pub(crate) fn is_current(&self) -> bool {
        self.current_revision.load(Ordering::Relaxed) == self.revision
    }
}

#[derive(Default)]
struct CaptionGate {
    owner: Option<String>,
    live_revision: u64,
    last_seq: u64,
    last_line_count: usize,
    last_partial: Option<Instant>,
    caption_revision: Arc<AtomicU64>,
}

impl CaptionGate {
    fn sync_owner(&mut self, owner: Option<&str>) {
        if self.owner.as_deref() != owner {
            self.owner = owner.map(str::to_owned);
            self.last_seq = 0;
            self.last_line_count = 0;
            self.last_partial = None;
            self.caption_revision.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn status(&mut self, owner: Option<&str>, status: &LiveSessionStatus) -> bool {
        self.sync_owner(owner);
        if status.update_revision <= self.live_revision
            || status.active != owner.is_some()
            || status.session_id.as_deref() != owner
        {
            return false;
        }
        self.live_revision = status.update_revision;
        true
    }

    fn partial(
        &mut self,
        owner: Option<&str>,
        payload: PartialCaption<'_>,
        enabled: bool,
        now: Instant,
    ) -> Option<Caption> {
        self.sync_owner(owner);
        if owner.is_none()
            || payload.caller != "live"
            || payload.live_session_id != owner
            || payload.text.trim().is_empty()
            || payload.seq <= self.last_seq
        {
            return None;
        }
        // Even a throttled or hidden newer partial must outrank older finals.
        self.last_seq = payload.seq;
        if !enabled
            || self
                .last_partial
                .is_some_and(|last| now.saturating_duration_since(last) < PARTIAL_INTERVAL)
        {
            return None;
        }
        self.last_partial = Some(now);
        Some(self.caption(payload.text, false))
    }

    fn committed(
        &mut self,
        owner: Option<&str>,
        payload: CommittedCaption<'_>,
        enabled: bool,
    ) -> Option<Caption> {
        self.sync_owner(owner);
        if owner != Some(payload.session_id)
            || payload.text().trim().is_empty()
            || payload.line_count == 0
        {
            return None;
        }
        match payload.seq {
            Some(seq) if seq > self.last_seq => self.last_seq = seq,
            // Manual lines have no capture order. Once STT has supplied order,
            // an unordered line must not cover its newer partial.
            None if self.last_seq == 0 && payload.line_count > self.last_line_count => {}
            _ => return None,
        }
        self.last_line_count = self.last_line_count.max(payload.line_count);
        if !enabled {
            return None;
        }
        self.last_partial = None;
        Some(self.caption(payload.line.text, true))
    }

    fn caption(&self, text: Cow<'_, str>, is_final: bool) -> Caption {
        let revision = self.caption_revision.fetch_add(1, Ordering::Relaxed) + 1;
        Caption {
            text: text.into_owned(),
            session_id: self.owner.clone().expect("validated owner"),
            is_final,
            revision,
            current_revision: self.caption_revision.clone(),
        }
    }
}

impl CommittedCaption<'_> {
    fn text(&self) -> &str {
        &self.line.text
    }
}

pub(crate) type CaptionMailbox = crate::latest_ui_mailbox::LatestUiMailbox<Caption>;

impl crate::latest_ui_mailbox::CurrentUiValue for Caption {
    fn is_current(&self) -> bool {
        Caption::is_current(self)
    }
}

pub(crate) fn subscribe(
    app: &AppHandle,
    enabled: fn() -> bool,
    show: fn(&AppHandle, Caption),
    status_changed: fn(&AppHandle, bool),
) -> Vec<tauri::EventId> {
    let gate = Arc::new(Mutex::new(CaptionGate::default()));
    let app_status = app.clone();
    let status_gate = gate.clone();
    let status = app.listen("live-session-updated", move |event| {
        let Ok(status) = serde_json::from_str::<LiveSessionStatus>(event.payload()) else {
            return;
        };
        let accepted = app_status
            .state::<LiveState>()
            .with_active_session_id(|owner| {
                status_gate
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .status(owner, &status)
            });
        if accepted == Some(true) && enabled() {
            status_changed(&app_status, status.active);
        }
    });
    let app_final = app.clone();
    let final_gate = gate.clone();
    let committed = app.listen("live-transcript-appended", move |event| {
        let Ok(payload) = serde_json::from_str::<CommittedCaption<'_>>(event.payload()) else {
            return;
        };
        let caption = app_final
            .state::<LiveState>()
            .with_active_session_id(|owner| {
                final_gate
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .committed(owner, payload, enabled())
            })
            .flatten();
        if let Some(caption) = caption {
            show(&app_final, caption);
        }
    });
    let app_partial = app.clone();
    let partial = app.listen("stt-partial", move |event| {
        let Ok(payload) = serde_json::from_str::<PartialCaption<'_>>(event.payload()) else {
            return;
        };
        let caption = app_partial
            .state::<LiveState>()
            .with_active_session_id(|owner| {
                gate.lock().unwrap_or_else(|e| e.into_inner()).partial(
                    owner,
                    payload,
                    enabled(),
                    Instant::now(),
                )
            })
            .flatten();
        if let Some(caption) = caption {
            show(&app_partial, caption);
        }
    });
    vec![status, committed, partial]
}

#[cfg(test)]
mod tests;
