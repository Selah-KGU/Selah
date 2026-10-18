//! Shared native subtitle admission and a latest-value UI mailbox.
//! Transcript storage remains append-only; only redundant display work is merged.

pub(crate) mod pages;

use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use tauri::{AppHandle, Emitter, Listener, Manager};

use crate::live::{LiveSessionStatus, LiveState};

const PARTIAL_INTERVAL: Duration = Duration::from_millis(120);
const OVERLAY_CLOSED: &str = "subtitle-overlay-closed";

pub(crate) fn overlay_closed(app: &AppHandle) {
    let _ = app.emit(OVERLAY_CLOSED, ());
}

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

    fn page(&self, text: String, last: bool) -> Self {
        Self {
            text,
            session_id: self.session_id.clone(),
            is_final: self.is_final && last,
            revision: self.revision,
            current_revision: self.current_revision.clone(),
        }
    }
}

#[derive(Default)]
struct CaptionGate {
    owner: Option<String>,
    live_revision: u64,
    last_seq: u64,
    last_committed_seq: u64,
    last_line_count: usize,
    last_partial: Option<Instant>,
    holding_translation: bool,
    last_refined_segment: u64,
    latest_original: Option<(String, bool)>,
    caption_revision: Arc<AtomicU64>,
}

#[derive(Deserialize)]
struct RefinedCaption<'a> {
    session_id: &'a str,
    #[serde(default)]
    segment_id: u64,
    source_seq: Option<u64>,
    #[serde(default)]
    source: String,
    #[serde(default)]
    translation: Option<String>,
    #[serde(default)]
    ready: bool,
    #[serde(default)]
    show_overlay: bool,
    #[serde(default)]
    clear: bool,
}

impl CaptionGate {
    fn surface_closed(&mut self) {
        self.release_hold();
        self.last_partial = None;
        // Cancel queued pages even if the overlay is reopened before the next
        // delivery timer wakes. Capture order still belongs to this recording.
        self.caption_revision.fetch_add(1, Ordering::Relaxed);
    }

    fn sync_owner(&mut self, owner: Option<&str>) {
        if self.owner.as_deref() != owner {
            self.owner = owner.map(str::to_owned);
            self.last_seq = 0;
            self.last_committed_seq = 0;
            self.last_line_count = 0;
            self.last_partial = None;
            self.holding_translation = false;
            self.last_refined_segment = 0;
            self.latest_original = None;
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
        self.latest_original = Some((payload.text.to_string(), false));
        if self.holding_translation && enabled {
            return None;
        }
        if !enabled
            || self
                .last_partial
                .is_some_and(|last| now.saturating_duration_since(last) < PARTIAL_INTERVAL)
        {
            return None;
        }
        self.last_partial = Some(now);
        Some(self.original_caption(payload.text, false))
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
        let resumed = if payload.seq.is_some_and(|seq| seq > self.last_committed_seq) {
            self.last_committed_seq = payload.seq.unwrap();
            self.resume_original(enabled, "")
        } else {
            None
        };
        match payload.seq {
            Some(seq) if seq > self.last_seq => self.last_seq = seq,
            // Manual lines have no capture order. Once STT has supplied order,
            // an unordered line must not cover its newer partial.
            None if self.last_seq == 0 && payload.line_count > self.last_line_count => {}
            _ => return resumed,
        }
        self.last_line_count = self.last_line_count.max(payload.line_count);
        self.latest_original = Some((payload.line.text.to_string(), true));
        if self.holding_translation && enabled {
            return None;
        }
        if !enabled {
            return None;
        }
        self.last_partial = None;
        Some(self.original_caption(payload.line.text, true))
    }

    fn refined(
        &mut self,
        owner: Option<&str>,
        payload: RefinedCaption<'_>,
        enabled: bool,
    ) -> Option<Caption> {
        self.sync_owner(owner);
        if owner != Some(payload.session_id) {
            return None;
        }
        if payload.clear {
            return self.resume_original(enabled, "");
        }
        if payload.segment_id < self.last_refined_segment {
            return None;
        }
        self.last_refined_segment = payload.segment_id;
        let Some(translation) = payload
            .translation
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            return self.resume_original(enabled, &payload.source);
        };
        if !payload.ready {
            return self.resume_original(enabled, &payload.source);
        }
        // Translation necessarily finishes after newer partials have arrived.
        // Only a newer confirmed capture makes its source obsolete; otherwise
        // normal continuous speech would suppress every translated result.
        let newer_original = payload
            .source_seq
            .is_some_and(|seq| seq < self.last_committed_seq);
        let same_original = self.latest_original.as_ref().is_some_and(|(text, _)| {
            text.chars()
                .filter(|ch| !ch.is_whitespace())
                .eq(payload.source.chars().filter(|ch| !ch.is_whitespace()))
        });
        if !enabled || !payload.show_overlay || newer_original && !same_original {
            return self.resume_original(enabled, "");
        }
        self.holding_translation = true;
        Some(self.caption(Cow::Borrowed(translation), false))
    }

    fn resume_original(&mut self, enabled: bool, fallback: &str) -> Option<Caption> {
        let was_holding = self.holding_translation;
        let original = self.latest_original.clone();
        self.release_hold();
        if !was_holding || !enabled {
            return None;
        }
        self.last_partial = None;
        let (text, is_final) = original.unwrap_or_else(|| (fallback.to_string(), true));
        Some(self.original_caption(Cow::Owned(text), is_final))
    }

    fn translation_finished(
        &mut self,
        owner: Option<&str>,
        revision: u64,
        enabled: bool,
    ) -> Option<Caption> {
        self.sync_owner(owner);
        if self.caption_revision.load(Ordering::Relaxed) != revision {
            return None;
        }
        self.resume_original(enabled, "")
    }

    fn release_hold(&mut self) {
        if !self.holding_translation {
            return;
        }
        self.holding_translation = false;
        self.caption_revision.fetch_add(1, Ordering::Relaxed);
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

    fn original_caption(&self, text: Cow<'_, str>, is_final: bool) -> Caption {
        self.caption(Cow::Owned(pages::live_tail(&text)), is_final)
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
    let closed_gate = gate.clone();
    let closed = app.listen(OVERLAY_CLOSED, move |_| {
        closed_gate
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .surface_closed();
    });
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
        let translation_enabled = crate::live::caption_translation_enabled();
        let caption = app_final
            .state::<LiveState>()
            .with_active_session_id(|owner| {
                let mut gate = final_gate.lock().unwrap_or_else(|e| e.into_inner());
                if !translation_enabled {
                    gate.release_hold();
                }
                gate.committed(owner, payload, enabled())
            })
            .flatten();
        if let Some(caption) = caption {
            show(&app_final, caption);
        }
    });
    let app_partial = app.clone();
    let partial_gate = gate.clone();
    let partial = app.listen("stt-partial", move |event| {
        let Ok(payload) = serde_json::from_str::<PartialCaption<'_>>(event.payload()) else {
            return;
        };
        let translation_enabled = crate::live::caption_translation_enabled();
        let caption = app_partial
            .state::<LiveState>()
            .with_active_session_id(|owner| {
                let mut gate = partial_gate.lock().unwrap_or_else(|e| e.into_inner());
                if !translation_enabled {
                    gate.release_hold();
                }
                gate.partial(owner, payload, enabled(), Instant::now())
            })
            .flatten();
        if let Some(caption) = caption {
            show(&app_partial, caption);
        }
    });
    let app_refined = app.clone();
    let refined = app.listen("live-caption-refined", move |event| {
        let Ok(payload) = serde_json::from_str::<RefinedCaption<'_>>(event.payload()) else {
            return;
        };
        let translation_enabled = crate::live::caption_translation_enabled();
        let caption = app_refined
            .state::<LiveState>()
            .with_active_session_id(|owner| {
                let mut gate = gate.lock().unwrap_or_else(|e| e.into_inner());
                if !translation_enabled && !payload.clear {
                    gate.release_hold();
                    return None;
                }
                gate.refined(owner, payload, enabled())
            })
            .flatten();
        if let Some(caption) = caption {
            let revision = caption.revision;
            let owner = caption.session_id.clone();
            let app_current = app_refined.clone();
            let app_show = app_refined.clone();
            let release_gate = gate.clone();
            let app_release = app_refined.clone();
            tauri::async_runtime::spawn(async move {
                pages::show_live_pages(
                    caption,
                    move || {
                        enabled() && app_current.state::<LiveState>().is_session_current(&owner)
                    },
                    move |page| show(&app_show, page),
                    pages::page_delay,
                )
                .await;
                let original = app_release
                    .state::<LiveState>()
                    .with_active_session_id(|owner| {
                        release_gate
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .translation_finished(owner, revision, enabled())
                    })
                    .flatten();
                if let Some(original) = original {
                    show(&app_release, original);
                }
            });
        }
    });
    vec![status, committed, partial, refined, closed]
}

#[cfg(test)]
mod tests;
