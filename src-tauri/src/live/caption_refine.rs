//! Side-channel correction and translation for LIVE captions.
//!
//! Partials stay on the existing display path. This worker waits for whole
//! sentences, then sends three or four of them in one model call. That call
//! corrects the group with the previous sentences in view, then translates the
//! correction. A call already in flight blocks the next one. Stored transcript
//! lines are never rewritten. A model failure degrades to the session glossary
//! and must not fail session start.

mod segment;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, watch};

use segment::{CommittedSegment, Segmenter};

const EVENT: &str = "live-caption-refined";
const TICK: Duration = Duration::from_millis(200);
const MODEL_BUDGET: Duration = Duration::from_secs(8);
const MODEL_DROP: Duration = Duration::from_secs(12);
const GLOSSARY_LIMIT: usize = 24;
const RECENT_LIMIT: usize = 8;
const CONTEXT_CHARS: usize = 1_200;
const BATCH_MIN: usize = 3;
const BATCH_MAX: usize = 4;
const CAPTION_CHARS: usize = 3_200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CaptionPreference {
    enabled: bool,
    revision: u64,
}

static PREFERENCE: LazyLock<watch::Sender<CaptionPreference>> = LazyLock::new(|| {
    watch::channel(CaptionPreference {
        enabled: true,
        revision: 0,
    })
    .0
});
static GENERATION: AtomicU64 = AtomicU64::new(0);
static CONTROL: Mutex<Option<Control>> = Mutex::new(None);

struct Control {
    session_id: String,
    generation: u64,
    tx: mpsc::UnboundedSender<CaptionInput>,
    stop: watch::Sender<bool>,
    latest: AtomicU64,
    logged_unavailable: AtomicBool,
}

enum CaptionInput {
    Partial { text: String, seq: u64 },
    Final { text: String, seq: Option<u64> },
    FinalStarted { seq: u64 },
    FinalFinished { seq: u64 },
    AudioDrained,
    Learned { from: String, to: String },
    Remembered { segment_id: u64, text: String },
    Settled,
}

#[derive(Clone, Default)]
struct Glossary {
    entries: VecDeque<(String, String)>,
}

impl Glossary {
    fn learn(&mut self, from: String, to: String) {
        if !learnable(&from) || !learnable(&to) || from == to {
            return;
        }
        self.entries.retain(|(existing, _)| existing != &from);
        self.entries.push_back((from, to));
        while self.entries.len() > GLOSSARY_LIMIT {
            self.entries.pop_front();
        }
    }

    fn apply(&self, text: &str) -> String {
        let mut entries: Vec<_> = self.entries.iter().collect();
        entries.sort_by_key(|(from, _)| std::cmp::Reverse(from.chars().count()));
        let mut out = text.to_string();
        for (from, to) in entries {
            out = replace_term(&out, from, to);
        }
        out
    }

    fn prompt_line(&self) -> String {
        self.entries
            .iter()
            .map(|(from, to)| format!("{from}→{to}"))
            .collect::<Vec<_>>()
            .join("；")
    }
}

struct ParsedCaption {
    corrected: String,
    translation: Option<String>,
}

#[derive(Clone, Serialize)]
struct RefinedEvent {
    session_id: String,
    segment_id: u64,
    source: String,
    corrected: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    translation: Option<String>,
    ready: bool,
    show_overlay: bool,
    clear: bool,
}

struct CaptionRequest {
    config: crate::ai::AiConfig,
    course_name: String,
    glossary: Glossary,
    recent: Vec<String>,
    caption: String,
    source_language: String,
    target_language: &'static str,
    translate: bool,
}

pub(crate) fn caption_translation_enabled() -> bool {
    PREFERENCE.borrow().enabled
}

pub(crate) fn set_caption_translation_enabled(app: &AppHandle, enabled: bool) {
    PREFERENCE.send_if_modified(|preference| {
        if preference.enabled == enabled {
            return false;
        }
        preference.enabled = enabled;
        preference.revision = preference.revision.wrapping_add(1);
        true
    });
    if enabled {
        return;
    }
    let session_id = control_lock()
        .as_ref()
        .map(|control| control.session_id.clone());
    if let Some(session_id) = session_id {
        emit_clear(app, &session_id);
    }
}

pub(crate) fn start_caption_refine(app: AppHandle, session_id: String, course_name: String) {
    let (tx, rx) = mpsc::unbounded_channel();
    let feedback = tx.clone();
    let (stop_tx, stop_rx) = watch::channel(false);
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    {
        let mut guard = control_lock();
        if let Some(previous) = guard.take() {
            let _ = previous.stop.send(true);
        }
        *guard = Some(Control {
            session_id: session_id.clone(),
            generation,
            tx,
            stop: stop_tx,
            latest: AtomicU64::new(0),
            logged_unavailable: AtomicBool::new(false),
        });
    }
    tauri::async_runtime::spawn(run_worker(
        app,
        session_id,
        course_name.trim().to_string(),
        generation,
        rx,
        stop_rx,
        feedback,
    ));
}

pub(crate) fn stop_caption_refine(session_id: &str) {
    let guard = control_lock();
    if let Some(control) = guard
        .as_ref()
        .filter(|control| control.session_id == session_id)
    {
        let _ = control.stop.send(true);
    }
}

pub(crate) fn observe_live_caption_partial(session_id: &str, text: &str, seq: u64) {
    send_caption(
        session_id,
        CaptionInput::Partial {
            text: text.trim().to_string(),
            seq,
        },
    );
}

pub(crate) fn observe_live_caption_final(session_id: &str, text: &str, seq: Option<u64>) {
    send_caption(
        session_id,
        CaptionInput::Final {
            text: text.trim().to_string(),
            seq,
        },
    );
}

pub(crate) fn live_caption_final_started(session_id: &str, seq: u64) {
    send_caption(session_id, CaptionInput::FinalStarted { seq });
}

pub(crate) fn live_caption_final_finished(session_id: &str, seq: u64) {
    send_caption(session_id, CaptionInput::FinalFinished { seq });
}

pub(crate) fn live_caption_audio_drained(session_id: &str) {
    send_caption(session_id, CaptionInput::AudioDrained);
}

fn control_lock() -> std::sync::MutexGuard<'static, Option<Control>> {
    CONTROL.lock().unwrap_or_else(|error| error.into_inner())
}

fn send_caption(session_id: &str, input: CaptionInput) {
    match &input {
        CaptionInput::Partial { text, .. } | CaptionInput::Final { text, .. }
            if text.is_empty() =>
        {
            return
        }
        _ => {}
    }
    let guard = control_lock();
    let Some(control) = guard.as_ref() else {
        return;
    };
    if control.session_id != session_id {
        return;
    }
    let _ = control.tx.send(input);
}

fn still_current(generation: u64, session_id: &str) -> bool {
    control_lock()
        .as_ref()
        .is_some_and(|control| control.generation == generation && control.session_id == session_id)
}

fn latest_segment(generation: u64) -> u64 {
    control_lock()
        .as_ref()
        .filter(|control| control.generation == generation)
        .map(|control| control.latest.load(Ordering::Relaxed))
        .unwrap_or(0)
}

fn set_latest(generation: u64, segment_id: u64) {
    if let Some(control) = control_lock().as_ref() {
        if control.generation == generation {
            control.latest.store(segment_id, Ordering::Relaxed);
        }
    }
}

fn log_unavailable(generation: u64, err: &str) {
    let guard = control_lock();
    let Some(control) = guard.as_ref() else {
        return;
    };
    if control.generation != generation || control.logged_unavailable.swap(true, Ordering::Relaxed)
    {
        return;
    }
    log::warn!("[Live] caption translation skipped: {err}");
}

async fn run_worker(
    app: AppHandle,
    session_id: String,
    course_name: String,
    generation: u64,
    mut rx: mpsc::UnboundedReceiver<CaptionInput>,
    mut stop: watch::Receiver<bool>,
    feedback: mpsc::UnboundedSender<CaptionInput>,
) {
    let mut segmenter = Segmenter::default();
    let mut glossary = Glossary::default();
    let mut recent = VecDeque::new();
    let mut gate = CaptionGate::default();
    let mut stopping = false;
    let mut stop_flushed = false;
    let origin = Instant::now();
    let mut interval = tokio::time::interval(TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let now = origin.elapsed().as_millis() as u64;
        if stopping {
            while let Ok(input) = rx.try_recv() {
                hold_input(
                    &mut segmenter,
                    &mut glossary,
                    &mut recent,
                    &mut gate,
                    input,
                    now,
                );
            }
            gate.push_hold(segmenter.finish(now));
            if !gate.in_flight && !stop_flushed {
                stop_flushed = true;
                if let Some(batch) = gate.take_flush() {
                    let launch = BatchLaunch {
                        app: &app,
                        feedback: &feedback,
                        generation,
                        session_id: &session_id,
                        course_name: &course_name,
                        glossary: &glossary,
                        recent: &recent,
                    };
                    launch.dispatch(&mut gate, batch);
                }
            }
            if !gate.in_flight {
                break;
            }
            tokio::select! {
                incoming = rx.recv() => {
                    let Some(input) = incoming else { break };
                    hold_input(
                        &mut segmenter,
                        &mut glossary,
                        &mut recent,
                        &mut gate,
                        input,
                        origin.elapsed().as_millis() as u64,
                    );
                }
                _ = tokio::time::sleep(MODEL_BUDGET + MODEL_DROP) => {
                    gate.abandon();
                    break;
                }
            }
            continue;
        }
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    stopping = true;
                }
            }
            // Sentence ends and the post-final join are measured here.
            _ = interval.tick() => {
                let launch = BatchLaunch {
                    app: &app,
                    feedback: &feedback,
                    generation,
                    session_id: &session_id,
                    course_name: &course_name,
                    glossary: &glossary,
                    recent: &recent,
                };
                launch.offer(&mut gate, segmenter.on_tick(now));
            }
            incoming = rx.recv() => {
                let Some(input) = incoming else {
                    stopping = true;
                    continue;
                };
                let mut inputs = vec![input];
                while let Ok(next) = rx.try_recv() {
                    inputs.push(next);
                }
                for input in inputs {
                    apply_input(
                        &app,
                        &feedback,
                        generation,
                        &session_id,
                        &course_name,
                        &mut segmenter,
                        &mut glossary,
                        &mut recent,
                        &mut gate,
                        input,
                        now,
                    );
                }
            }
        }
    }
    let mut guard = control_lock();
    if guard
        .as_ref()
        .is_some_and(|control| control.generation == generation)
    {
        if let Some(control) = guard.take() {
            let _ = control.stop.send(true);
        }
    }
}

fn apply_input(
    app: &AppHandle,
    feedback: &mpsc::UnboundedSender<CaptionInput>,
    generation: u64,
    session_id: &str,
    course_name: &str,
    segmenter: &mut Segmenter,
    glossary: &mut Glossary,
    recent: &mut VecDeque<(u64, String)>,
    gate: &mut CaptionGate,
    input: CaptionInput,
    now: u64,
) {
    match input {
        CaptionInput::Settled => {
            let launch = BatchLaunch {
                app,
                feedback,
                generation,
                session_id,
                course_name,
                glossary,
                recent,
            };
            launch.settle(gate);
        }
        other => {
            let segments = accept(segmenter, glossary, recent, other, now);
            let launch = BatchLaunch {
                app,
                feedback,
                generation,
                session_id,
                course_name,
                glossary,
                recent,
            };
            launch.offer(gate, segments);
        }
    }
}

fn hold_input(
    segmenter: &mut Segmenter,
    glossary: &mut Glossary,
    recent: &mut VecDeque<(u64, String)>,
    gate: &mut CaptionGate,
    input: CaptionInput,
    now: u64,
) {
    match input {
        CaptionInput::Settled => gate.note_settled(),
        other => gate.push_hold(accept(segmenter, glossary, recent, other, now)),
    }
}

fn accept(
    segmenter: &mut Segmenter,
    glossary: &mut Glossary,
    recent: &mut VecDeque<(u64, String)>,
    input: CaptionInput,
    now: u64,
) -> Vec<CommittedSegment> {
    match input {
        CaptionInput::Partial { text, seq } => segmenter.push_partial_ordered(&text, now, seq),
        CaptionInput::Final { text, seq } => segmenter.push_final_ordered(&text, now, seq),
        CaptionInput::FinalStarted { seq } => {
            segmenter.begin_final(seq);
            Vec::new()
        }
        CaptionInput::FinalFinished { seq } => segmenter.end_final(seq, now),
        CaptionInput::AudioDrained => segmenter.audio_drained(now),
        CaptionInput::Learned { from, to } => {
            glossary.learn(from, to);
            Vec::new()
        }
        CaptionInput::Remembered { segment_id, text } => {
            remember(recent, segment_id, text);
            Vec::new()
        }
        CaptionInput::Settled => Vec::new(),
    }
}

fn remember(recent: &mut VecDeque<(u64, String)>, segment_id: u64, text: String) {
    if let Some((_, existing)) = recent.iter_mut().find(|(id, _)| *id == segment_id) {
        *existing = text;
        return;
    }
    if recent.back().is_some_and(|(id, _)| segment_id < *id) {
        return;
    }
    recent.push_back((segment_id, text));
    while recent.len() > RECENT_LIMIT {
        recent.pop_front();
    }
}

struct CaptionGate {
    queued: VecDeque<CommittedSegment>,
    in_flight: bool,
    flush_once: bool,
    flushed: bool,
    preference_revision: Option<u64>,
}

impl Default for CaptionGate {
    fn default() -> Self {
        Self {
            queued: VecDeque::new(),
            in_flight: false,
            flush_once: false,
            flushed: false,
            preference_revision: None,
        }
    }
}

impl CaptionGate {
    fn sync_preference(&mut self, revision: u64) {
        if self
            .preference_revision
            .replace(revision)
            .is_some_and(|old| old != revision)
        {
            self.clear_pending();
        }
    }

    fn push(
        &mut self,
        segments: impl IntoIterator<Item = CommittedSegment>,
    ) -> Option<Vec<CommittedSegment>> {
        self.queued.extend(segments);
        self.poll()
    }

    fn push_hold(&mut self, segments: impl IntoIterator<Item = CommittedSegment>) {
        self.queued.extend(segments);
    }

    fn settled(&mut self, enabled: bool) -> Option<Vec<CommittedSegment>> {
        self.note_settled();
        if !enabled {
            self.clear_pending();
            return None;
        }
        self.poll()
    }

    fn note_settled(&mut self) {
        self.in_flight = false;
    }

    fn take_flush(&mut self) -> Option<Vec<CommittedSegment>> {
        if self.flushed {
            return None;
        }
        self.flush_once = true;
        self.poll()
    }

    fn clear_pending(&mut self) {
        self.queued.clear();
        self.flush_once = false;
    }

    fn abandon(&mut self) {
        self.flushed = true;
        self.cancel_launch();
    }

    fn cancel_launch(&mut self) {
        self.in_flight = false;
        self.clear_pending();
    }

    fn poll(&mut self) -> Option<Vec<CommittedSegment>> {
        if self.in_flight || self.queued.is_empty() || self.flushed {
            return None;
        }
        let Some(count) = batch_len(&self.queued, self.flush_once) else {
            return None;
        };
        let batch: Vec<_> = self.queued.drain(..count).collect();
        self.in_flight = true;
        if self.flush_once {
            self.flush_once = false;
            self.flushed = true;
        }
        Some(batch)
    }
}

fn batch_len(queued: &VecDeque<CommittedSegment>, flush: bool) -> Option<usize> {
    if flush {
        return flush_count(queued);
    }
    let mut sentences = 0usize;
    let mut chars = 0usize;
    let mut end = 0usize;
    for (index, segment) in queued.iter().enumerate() {
        let next_chars = chars + usize::from(index > 0) + segment.text.trim().chars().count();
        if index > 0 && next_chars > CAPTION_CHARS {
            break;
        }
        let next = sentences + segment::sentence_units(&segment.text).max(1);
        if index > 0 && sentences >= BATCH_MIN && next > BATCH_MAX {
            break;
        }
        sentences = next;
        chars = next_chars;
        end = index + 1;
        if sentences >= BATCH_MAX {
            break;
        }
    }
    (sentences >= BATCH_MIN).then_some(end)
}

fn flush_count(queued: &VecDeque<CommittedSegment>) -> Option<usize> {
    let mut chars = 0usize;
    let mut count = 0usize;
    for segment in queued {
        let len = segment.text.trim().chars().count() + usize::from(count > 0);
        if count > 0 && chars + len > CAPTION_CHARS {
            break;
        }
        chars += len;
        count += 1;
    }
    (count > 0).then_some(count)
}

struct BatchLaunch<'a> {
    app: &'a AppHandle,
    feedback: &'a mpsc::UnboundedSender<CaptionInput>,
    generation: u64,
    session_id: &'a str,
    course_name: &'a str,
    glossary: &'a Glossary,
    recent: &'a VecDeque<(u64, String)>,
}

impl BatchLaunch<'_> {
    fn offer(&self, gate: &mut CaptionGate, segments: Vec<CommittedSegment>) {
        let preference = *PREFERENCE.borrow();
        gate.sync_preference(preference.revision);
        if !preference.enabled {
            gate.clear_pending();
            return;
        }
        if let Some(batch) = gate.push(segments) {
            self.dispatch(gate, batch);
        }
    }

    fn settle(&self, gate: &mut CaptionGate) {
        let preference = *PREFERENCE.borrow();
        gate.sync_preference(preference.revision);
        if let Some(batch) = gate.settled(preference.enabled) {
            self.dispatch(gate, batch);
        }
    }

    fn dispatch(&self, gate: &mut CaptionGate, batch: Vec<CommittedSegment>) {
        let preference = *PREFERENCE.borrow();
        if gate
            .preference_revision
            .is_some_and(|revision| revision != preference.revision)
        {
            gate.sync_preference(preference.revision);
            gate.cancel_launch();
            return;
        }
        if self.spawn(batch, preference) {
            return;
        }
        // A toggle can race admission. Skipping this launch must not close the
        // gate for the rest of the recording; only stopping is terminal.
        gate.cancel_launch();
    }

    fn spawn(&self, batch: Vec<CommittedSegment>, preference: CaptionPreference) -> bool {
        if !still_current(self.generation, self.session_id)
            || !preference.enabled
            || *PREFERENCE.borrow() != preference
        {
            return false;
        }
        let caption = batch_caption(&batch);
        let Some(segment_id) = batch.last().map(|segment| segment.id) else {
            return false;
        };
        if caption.is_empty() {
            return false;
        }
        set_latest(self.generation, segment_id);
        let seed = RefineSeed {
            app: self.app.clone(),
            feedback: self.feedback.clone(),
            generation: self.generation,
            preference,
            session_id: self.session_id.to_string(),
            course_name: self.course_name.to_string(),
            glossary: self.glossary.clone(),
            recent: self.recent.iter().map(|(_, text)| text.clone()).collect(),
            segment: CommittedSegment {
                id: segment_id,
                text: caption,
            },
        };
        let feedback = self.feedback.clone();
        tauri::async_runtime::spawn(async move {
            let _guard = SettledGuard(feedback);
            refine_segment(seed).await;
        });
        true
    }
}

struct SettledGuard(mpsc::UnboundedSender<CaptionInput>);

impl Drop for SettledGuard {
    fn drop(&mut self) {
        let _ = self.0.send(CaptionInput::Settled);
    }
}

fn batch_caption(segments: &[CommittedSegment]) -> String {
    segments
        .iter()
        .map(|segment| segment.text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

struct RefineSeed {
    app: AppHandle,
    feedback: mpsc::UnboundedSender<CaptionInput>,
    generation: u64,
    preference: CaptionPreference,
    session_id: String,
    course_name: String,
    glossary: Glossary,
    recent: Vec<String>,
    segment: CommittedSegment,
}

async fn refine_segment(seed: RefineSeed) {
    let preference = seed.preference;
    let _ = until_preference_changes(
        PREFERENCE.subscribe(),
        preference,
        refine_current_segment(seed),
    )
    .await;
}

async fn until_preference_changes<T>(
    mut preference: watch::Receiver<CaptionPreference>,
    expected: CaptionPreference,
    work: impl std::future::Future<Output = T>,
) -> Option<T> {
    if !expected.enabled || *preference.borrow() != expected {
        return None;
    }
    tokio::select! {
        biased;
        _ = preference.changed() => None,
        result = work => Some(result),
    }
}

fn seed_current(seed: &RefineSeed) -> bool {
    *PREFERENCE.borrow() == seed.preference
        && seed.preference.enabled
        && still_current(seed.generation, &seed.session_id)
}

async fn refine_current_segment(seed: RefineSeed) {
    if !seed_current(&seed) {
        return;
    }
    let glossary_text = seed.glossary.apply(&seed.segment.text);
    let mut config = match super::live_ai_config() {
        Ok(config) => config,
        Err(err) => {
            log_unavailable(seed.generation, &err);
            fallback_caption(&seed, &glossary_text);
            return;
        }
    };
    config.temperature = 0.0;
    config.max_tokens = 3072;
    let source_language = crate::stt::stt_selected_language();
    let target_language = target_code(&config.reply_language);
    let translate = !same_caption_language(&source_language, target_language);
    let request = CaptionRequest {
        config,
        course_name: seed.course_name.clone(),
        glossary: seed.glossary.clone(),
        recent: seed.recent.clone(),
        caption: glossary_text.clone(),
        source_language,
        target_language,
        translate,
    };
    let request = request_caption(request);
    tokio::pin!(request);
    match tokio::time::timeout(MODEL_BUDGET, &mut request).await {
        Ok(Ok(parsed)) => finish_parsed(&seed, &glossary_text, parsed),
        Ok(Err(err)) => {
            log::warn!("[Live] caption model failed: {err}");
            fallback_caption(&seed, &glossary_text);
        }
        Err(_) => {
            fallback_caption(&seed, &glossary_text);
            if !seed_current(&seed) {
                return;
            }
            if let Ok(Ok(parsed)) = tokio::time::timeout(MODEL_DROP, &mut request).await {
                finish_parsed(&seed, &glossary_text, parsed);
            }
        }
    }
}

fn fallback_caption(seed: &RefineSeed, glossary_text: &str) {
    if !seed_current(seed) {
        return;
    }
    let _ = seed.feedback.send(CaptionInput::Remembered {
        segment_id: seed.segment.id,
        text: glossary_text.to_string(),
    });
    publish_caption(seed, glossary_text, None, false);
}

fn finish_parsed(seed: &RefineSeed, glossary_text: &str, parsed: ParsedCaption) {
    if !seed_current(seed) {
        return;
    }
    if let Some((from, to)) = contiguous_replacement(glossary_text, &parsed.corrected) {
        let _ = seed.feedback.send(CaptionInput::Learned { from, to });
    }
    let _ = seed.feedback.send(CaptionInput::Remembered {
        segment_id: seed.segment.id,
        text: parsed.corrected.clone(),
    });
    publish_caption(&seed, &parsed.corrected, parsed.translation, true);
}

async fn request_caption(job: CaptionRequest) -> Result<ParsedCaption, String> {
    let messages = vec![
        crate::ai::ChatMessage {
            role: "system".into(),
            content: system_prompt(job.translate, language_label(job.target_language)),
            images: Vec::new(),
        },
        crate::ai::ChatMessage {
            role: "user".into(),
            content: user_prompt(
                &job.course_name,
                &job.recent,
                &job.glossary,
                &job.caption,
                language_label(&job.source_language),
                language_label(job.target_language),
                job.translate,
            ),
            images: Vec::new(),
        },
    ];
    let raw = crate::ai::chat_completion_public(&job.config, messages).await?;
    parse_model_caption(&raw, &job.caption, job.translate)
        .ok_or_else(|| "字幕の訂正結果を解釈できません".to_string())
}

fn publish_caption(seed: &RefineSeed, corrected: &str, translation: Option<String>, ready: bool) {
    if !seed_current(seed) {
        return;
    }
    let translation = translation.filter(|text| !text.trim().is_empty());
    let show_overlay = should_show_overlay(
        true,
        ready,
        translation.as_deref(),
        seed.segment.id,
        latest_segment(seed.generation),
    );
    let _ = seed.app.emit(
        EVENT,
        RefinedEvent {
            session_id: seed.session_id.clone(),
            segment_id: seed.segment.id,
            source: seed.segment.text.clone(),
            corrected: corrected.to_string(),
            translation,
            ready,
            show_overlay,
            clear: false,
        },
    );
}

fn emit_clear(app: &AppHandle, session_id: &str) {
    let _ = app.emit(
        EVENT,
        RefinedEvent {
            session_id: session_id.to_string(),
            segment_id: 0,
            source: String::new(),
            corrected: String::new(),
            translation: None,
            ready: false,
            show_overlay: false,
            clear: true,
        },
    );
}

fn should_show_overlay(
    enabled: bool,
    ready: bool,
    translation: Option<&str>,
    segment_id: u64,
    latest_id: u64,
) -> bool {
    enabled
        && ready
        && latest_id != 0
        && segment_id == latest_id
        && translation.is_some_and(|text| !text.trim().is_empty())
}

fn system_prompt(translate: bool, target_label: &str) -> String {
    let translation = if translate {
        format!(
            "3. translation は corrected だけを {target_label} に訳す。複数文でも全文を訳し、一文だけ残さない。上文は訳さない。字幕原文の誤認識には戻らない。訳せない固有名詞はそのまま残す。\n形式: {{\"corrected\":\"...\",\"translation\":\"...\"}}"
        )
    } else {
        "translation は書かない。形式: {\"corrected\":\"...\"}".to_string()
    };
    format!(
        "あなたは講義字幕の係です。出力はJSONオブジェクトだけです。説明、Markdown、思考過程は禁止です。\n手順:\n1. 上文があるときは、主語、指示語、専門用語、同音の誤認識を上文で判断する。上文の文は corrected にも translation にも繰り返さない。\n2. corrected を先に書く。今回の字幕だけを直す。字幕が複数文のときは全部を順に直し、一文にまとめない。要約、補足、言い換え、敬体の変更はしない。確信がなければ字幕のままにする。\n{translation}"
    )
}

fn context_block(recent: &[String]) -> String {
    let mut kept = Vec::new();
    let mut count = 0usize;
    for text in recent.iter().rev() {
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let len = text.chars().count();
        if !kept.is_empty() && count + len > CONTEXT_CHARS {
            break;
        }
        kept.push(text);
        count += len;
        if kept.len() == RECENT_LIMIT {
            break;
        }
    }
    kept.reverse();
    kept.join("\n")
}

fn user_prompt(
    course: &str,
    recent: &[String],
    glossary: &Glossary,
    caption: &str,
    source_label: &str,
    target_label: &str,
    translate: bool,
) -> String {
    let mut lines = Vec::new();
    if !course.trim().is_empty() {
        lines.push(format!("科目: {}", clip(course, 80)));
    }
    let context = context_block(recent);
    if !context.is_empty() {
        lines.push(format!("上文:\n{context}"));
    }
    let terms = glossary.prompt_line();
    if !terms.is_empty() {
        lines.push(format!("用語: {terms}"));
    }
    lines.push(format!("音声: {source_label}"));
    if translate {
        lines.push(format!("訳語: {target_label}"));
    }
    lines.push(format!("字幕: {}", clip(caption, CAPTION_CHARS)));
    lines.join("\n")
}

fn parse_model_caption(raw: &str, source: &str, translate: bool) -> Option<ParsedCaption> {
    let cleaned = super::sanitize_model_output(raw);
    let json = super::extract_json_object(&cleaned)?;
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let corrected = one_line(
        value
            .get("corrected")
            .and_then(|item| item.as_str())
            .unwrap_or(""),
    );
    if !acceptable_correction(source, &corrected) {
        return None;
    }
    let translation = if translate {
        value
            .get("translation")
            .and_then(|item| item.as_str())
            .map(one_line)
            .filter(|text| acceptable_translation(&corrected, text))
    } else {
        None
    };
    Some(ParsedCaption {
        corrected,
        translation,
    })
}

fn acceptable_correction(source: &str, corrected: &str) -> bool {
    let corrected_len = corrected.chars().count();
    if corrected_len == 0 || corrected_len > 4_000 {
        return false;
    }
    let source_len = source.chars().count().max(1);
    corrected_len <= source_len * 2 + 24 && corrected_len + 16 >= source_len / 2
}

fn acceptable_translation(corrected: &str, translation: &str) -> bool {
    let len = translation.chars().count();
    if len == 0 || len > 4_800 {
        return false;
    }
    len <= corrected.chars().count().max(1) * 4 + 24
}

fn contiguous_replacement(before: &str, after: &str) -> Option<(String, String)> {
    if before == after {
        return None;
    }
    let before: Vec<char> = before.chars().collect();
    let after: Vec<char> = after.chars().collect();
    let mut prefix = 0;
    while prefix < before.len() && prefix < after.len() && before[prefix] == after[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < before.len() - prefix
        && suffix < after.len() - prefix
        && before[before.len() - 1 - suffix] == after[after.len() - 1 - suffix]
    {
        suffix += 1;
    }
    if prefix + suffix < 2 {
        return None;
    }
    let mut from_start = prefix;
    let mut to_start = prefix;
    let from_end = before.len() - suffix;
    let to_end = after.len() - suffix;
    if from_start < from_end && to_start < to_end {
        while (from_end - from_start < 2 || to_end - to_start < 2) && from_start > 0 && to_start > 0
        {
            from_start -= 1;
            to_start -= 1;
        }
    }
    let from_chars = &before[from_start..from_end];
    let to_chars = &after[to_start..to_end];
    if shares_run(from_chars, to_chars, 2) {
        return None;
    }
    let from: String = from_chars.iter().collect();
    let to: String = to_chars.iter().collect();
    if from.is_empty() || to.is_empty() || from == to {
        return None;
    }
    Some((from, to))
}

fn shares_run(left: &[char], right: &[char], len: usize) -> bool {
    if left.len() < len || right.len() < len {
        return false;
    }
    for start in 0..=left.len() - len {
        let needle = &left[start..start + len];
        if right.windows(len).any(|window| window == needle) {
            return true;
        }
    }
    false
}

fn learnable(text: &str) -> bool {
    let len = text.chars().count();
    (2..=16).contains(&len)
        && text.chars().any(|ch| {
            ('\u{4e00}'..='\u{9fff}').contains(&ch)
                || ('\u{30a0}'..='\u{30ff}').contains(&ch)
                || ch.is_ascii_alphanumeric()
        })
}

fn replace_term(text: &str, from: &str, to: &str) -> String {
    if !from.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return text.replace(from, to);
    }
    let chars: Vec<char> = text.chars().collect();
    let from_chars: Vec<char> = from.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        let end = index + from_chars.len();
        let matched = end <= chars.len() && chars[index..end] == from_chars[..];
        let left_ok = index == 0 || !chars[index - 1].is_ascii_alphanumeric();
        let right_ok = end >= chars.len() || !chars[end].is_ascii_alphanumeric();
        if matched && left_ok && right_ok {
            out.push_str(to);
            index = end;
        } else {
            out.push(chars[index]);
            index += 1;
        }
    }
    out
}

fn target_code(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().as_str() {
        "en" => "en",
        "zh" => "zh",
        "ko" => "ko",
        _ => "ja",
    }
}

fn same_caption_language(source: &str, target: &str) -> bool {
    let source = source.trim().to_ascii_lowercase();
    source == target && matches!(source.as_str(), "ja" | "en" | "zh" | "ko")
}

fn language_label(code: &str) -> &'static str {
    match code.trim().to_ascii_lowercase().as_str() {
        "ja" => "日本語",
        "en" => "English",
        "zh" => "中文",
        "ko" => "한국어",
        "yue" => "広東語",
        "auto" => "自動検出",
        _ => "音声の言語",
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clip(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        trimmed.to_string()
    } else {
        trimmed.chars().take(max_chars).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stuck_call_cannot_hold_the_gate_open() {
        assert_eq!(MODEL_BUDGET, Duration::from_secs(8));
        assert_eq!(MODEL_DROP, Duration::from_secs(12));
        assert_eq!(BATCH_MIN, 3);
        assert_eq!(BATCH_MAX, 4);
    }

    #[tokio::test]
    async fn a_disabled_and_reenabled_request_cannot_start_from_an_old_preference() {
        let expected = CaptionPreference {
            enabled: true,
            revision: 0,
        };
        let (tx, rx) = watch::channel(expected);
        tx.send_replace(CaptionPreference {
            enabled: false,
            revision: 1,
        });
        tx.send_replace(CaptionPreference {
            enabled: true,
            revision: 2,
        });
        let started = AtomicBool::new(false);
        let result = until_preference_changes(rx, expected, async {
            started.store(true, Ordering::Relaxed);
        })
        .await;
        assert!(result.is_none());
        assert!(!started.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn changing_preferences_drops_the_running_request_future() {
        struct Dropped(Option<tokio::sync::oneshot::Sender<()>>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                let _ = self.0.take().unwrap().send(());
            }
        }
        let expected = CaptionPreference {
            enabled: true,
            revision: 0,
        };
        let (tx, rx) = watch::channel(expected);
        let (started, ready) = tokio::sync::oneshot::channel();
        let (dropped, released) = tokio::sync::oneshot::channel();
        let request = tokio::spawn(until_preference_changes(rx, expected, async move {
            let _guard = Dropped(Some(dropped));
            let _ = started.send(());
            std::future::pending::<()>().await;
        }));
        ready.await.unwrap();
        tx.send_replace(CaptionPreference {
            enabled: false,
            revision: 1,
        });
        assert!(request.await.unwrap().is_none());
        released.await.unwrap();
    }

    #[tokio::test]
    async fn unchanged_preferences_preserve_a_completed_request() {
        let expected = CaptionPreference {
            enabled: true,
            revision: 0,
        };
        let (_tx, rx) = watch::channel(expected);
        assert_eq!(
            until_preference_changes(rx, expected, async { 42 }).await,
            Some(42)
        );
    }

    fn seg(id: u64, text: &str) -> CommittedSegment {
        CommittedSegment {
            id,
            text: text.to_string(),
        }
    }

    #[test]
    fn decoder_completion_order_does_not_reverse_the_model_batch() {
        let mut segmenter = Segmenter::default();
        let mut glossary = Glossary::default();
        let mut recent = VecDeque::new();
        let mut gate = CaptionGate::default();
        let mut batches = Vec::new();
        let first = "まず期待値を定義します。";
        let second = "次に分散を定義します。";
        let third = "最後に正規分布です。";
        for (now, input) in [
            (0, CaptionInput::FinalStarted { seq: 10 }),
            (
                0,
                CaptionInput::Partial {
                    text: second.into(),
                    seq: 20,
                },
            ),
            (400, CaptionInput::FinalStarted { seq: 30 }),
            (
                500,
                CaptionInput::Final {
                    text: first.into(),
                    seq: Some(10),
                },
            ),
            (500, CaptionInput::FinalFinished { seq: 10 }),
            (
                600,
                CaptionInput::Final {
                    text: second.into(),
                    seq: Some(30),
                },
            ),
            (600, CaptionInput::FinalFinished { seq: 30 }),
            (
                700,
                CaptionInput::Final {
                    text: third.into(),
                    seq: Some(40),
                },
            ),
        ] {
            let segments = accept(&mut segmenter, &mut glossary, &mut recent, input, now);
            if let Some(batch) = gate.push(segments) {
                batches.push(batch_caption(&batch));
            }
        }
        assert_eq!(batches, [format!("{first}\n{second}\n{third}")]);
        assert!(gate.queued.is_empty());
    }

    #[test]
    fn a_later_three_sentence_partial_cannot_launch_a_model_call_ahead_of_a_final() {
        let mut segmenter = Segmenter::default();
        let mut gate = CaptionGate::default();
        let first = "まず期待値です。";
        let later = "次に分散です。標準偏差です。最後に正規分布です。";
        segmenter.begin_final(10);
        assert!(gate
            .push(segmenter.push_partial_ordered(later, 0, 20))
            .is_none());
        assert!(gate.push(segmenter.on_tick(5_000)).is_none());
        assert!(gate
            .push(segmenter.push_final_ordered(first, 5_100, Some(10)))
            .is_none());
        let batch = gate.push(segmenter.end_final(10, 5_100)).unwrap();
        assert_eq!(batch_caption(&batch), format!("{first}\n{later}"));
    }

    #[test]
    fn three_sentences_share_one_call_and_two_do_not() {
        let mut gate = CaptionGate::default();
        assert!(gate.push([seg(1, "一です。")]).is_none());
        assert!(gate.push([seg(2, "二です。")]).is_none());
        let batch = gate.push([seg(3, "三です。")]).unwrap();
        assert_eq!(
            batch.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(gate.push([seg(4, "四です。")]).is_none());
    }

    #[test]
    fn a_burst_sends_four_sentences_and_holds_the_fifth() {
        let mut gate = CaptionGate::default();
        let first = gate.push((1..=5).map(|id| seg(id, "文です。"))).unwrap();
        assert_eq!(
            first.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        assert!(gate.settled(true).is_none());
        assert!(gate.push([seg(6, "文です。")]).is_none());
        let second = gate.push([seg(7, "文です。")]).unwrap();
        assert_eq!(
            second.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![5, 6, 7]
        );
    }

    #[test]
    fn model_input_budget_includes_separators_without_truncating_a_segment() {
        let text = "あ".repeat(799) + "。";
        let mut gate = CaptionGate::default();
        let first = gate.push((1..=4).map(|id| seg(id, &text))).unwrap();
        assert_eq!(first.len(), 3);
        assert!(batch_caption(&first).chars().count() <= CAPTION_CHARS);
        assert!(gate.settled(true).is_none());
        let tail = gate.take_flush().unwrap();
        assert_eq!(tail[0].id, 4);
        assert_eq!(batch_caption(&tail), text);
        let mut flush = CaptionGate::default();
        flush.push_hold((1..=4).map(|id| seg(id, &text)));
        assert!(batch_caption(&flush.take_flush().unwrap()).chars().count() <= CAPTION_CHARS);
    }

    #[test]
    fn an_in_flight_call_does_not_start_another() {
        let mut gate = CaptionGate::default();
        assert!(gate
            .push([seg(1, "一です。"), seg(2, "二です。"), seg(3, "三です。")])
            .is_some());
        assert!(gate
            .push([
                seg(4, "四です。"),
                seg(5, "五です。"),
                seg(6, "六です。"),
                seg(7, "七です。"),
            ])
            .is_none());
        let next = gate.settled(true).unwrap();
        assert_eq!(next.len(), 4);
        assert!(gate.push([seg(8, "八です。")]).is_none());
        assert!(gate.settled(true).is_none());
    }

    #[test]
    fn disabling_during_a_call_discards_waiting_batches_but_allows_reenable() {
        let mut gate = CaptionGate::default();
        let group = |start| (start..start + 3).map(|id| seg(id, "文です。"));
        assert!(gate.push(group(1)).is_some());
        assert!(gate.push(group(4)).is_none());
        assert!(gate.settled(false).is_none());
        assert!(!gate.in_flight);
        assert!(gate.queued.is_empty());
        let resumed = gate.push(group(7)).unwrap();
        assert_eq!(resumed[0].id, 7);
    }

    #[test]
    fn a_fast_off_on_discards_old_batches_without_overlapping_the_running_request() {
        let mut gate = CaptionGate::default();
        gate.sync_preference(0);
        assert!(gate
            .push((1..=3).map(|id| seg(id, "古い文です。")))
            .is_some());
        assert!(gate
            .push((4..=6).map(|id| seg(id, "待機中です。")))
            .is_none());
        gate.sync_preference(2);
        assert!(gate.in_flight);
        assert!(gate.queued.is_empty());
        assert!(gate.settled(true).is_none());
        let resumed = gate
            .push((7..=9).map(|id| seg(id, "新しい文です。")))
            .unwrap();
        assert_eq!(resumed[0].id, 7);
    }

    #[test]
    fn a_toggle_racing_an_admitted_launch_does_not_end_the_recording_gate() {
        let mut gate = CaptionGate::default();
        assert!(gate.push((1..=3).map(|id| seg(id, "文です。"))).is_some());
        gate.cancel_launch();
        assert!(gate
            .push((4..=6).map(|id| seg(id, "次の文です。")))
            .is_some());
    }

    #[test]
    fn one_segment_of_three_sentences_is_still_one_call() {
        let mut gate = CaptionGate::default();
        let batch = gate.push([seg(1, "一です。二です。三です。")]).unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(segment::sentence_units(&batch[0].text), 3);
        assert!(gate.push([seg(2, "四です。")]).is_none());
    }

    #[test]
    fn stop_sends_a_short_tail_once_and_a_ready_group_together() {
        let mut gate = CaptionGate::default();
        assert!(gate
            .push([seg(1, "一です。"), seg(2, "二です。")])
            .is_none());
        let tail = gate.take_flush().unwrap();
        assert_eq!(tail.len(), 2);
        assert!(gate.take_flush().is_none());

        let mut held = CaptionGate::default();
        held.push_hold((1..=5).map(|id| seg(id, "文です。")));
        let batch = held.take_flush().unwrap();
        assert_eq!(batch.len(), 5);
        assert!(held.take_flush().is_none());
    }

    #[test]
    fn a_batch_prompt_keeps_every_sentence_and_the_previous_context() {
        let caption = batch_caption(&[
            seg(1, "期待値を定義します。"),
            seg(2, "次に分散です。"),
            seg(3, "最後に正規分布です。"),
        ]);
        assert_eq!(
            caption,
            "期待値を定義します。
次に分散です。
最後に正規分布です。"
        );
        let prompt = system_prompt(true, "中文");
        assert!(prompt.contains("複数文"));
        assert!(prompt.contains("一文にまとめない"));
        assert!(prompt.contains("一文だけ残さない"));
        let user = user_prompt(
            "確率論",
            &["前文です。".into()],
            &Glossary::default(),
            &caption,
            "日本語",
            "中文",
            true,
        );
        assert!(user.contains("上文:"));
        assert!(user.contains("前文です。"));
        assert!(user.contains("期待値を定義します。"));
        assert!(user.contains("次に分散です。"));
        assert!(user.contains("最後に正規分布です。"));
        let source = "あ".repeat(1_500);
        assert!(acceptable_correction(&source, &source));
        assert!(!acceptable_correction(&source, &"あ".repeat(4_001)));
    }

    #[test]
    fn glossary_learns_one_contiguous_term_and_only_rewrites_source_text() {
        let mut glossary = Glossary::default();
        let pair = contiguous_replacement("今日は確立の定義です", "今日は確率の定義です").unwrap();
        glossary.learn(pair.0, pair.1);
        assert_eq!(glossary.apply("次は確立を使う"), "次は確率を使う");
        assert!(
            contiguous_replacement("今日は確立の話と分散です", "今日は確率の話と方差です")
                .is_none()
        );
        assert!(contiguous_replacement("短い文です", "まったく別の文").is_none());
        let particle = contiguous_replacement("これはです", "これはます").unwrap();
        let mut particles = Glossary::default();
        particles.learn(particle.0, particle.1);
        assert!(particles.entries.is_empty());
    }

    #[test]
    fn glossary_keeps_the_newest_twenty_four_terms() {
        let mut glossary = Glossary::default();
        for index in 0..30 {
            glossary.learn(format!("用語{index:02}"), format!("正語{index:02}"));
        }
        assert_eq!(glossary.entries.len(), 24);
        assert_eq!(glossary.entries.front().unwrap().0, "用語06");
        glossary.learn("用語10".into(), "更新10".into());
        assert_eq!(glossary.entries.back().unwrap().1, "更新10");
        assert_eq!(
            glossary
                .entries
                .iter()
                .filter(|(from, _)| from == "用語10")
                .count(),
            1
        );
    }

    #[test]
    fn ascii_terms_do_not_rewrite_longer_tokens() {
        let mut glossary = Glossary::default();
        glossary.learn("HTTP".into(), "HTTPS".into());
        assert_eq!(
            glossary.apply("use HTTP, not HTTPSURL"),
            "use HTTPS, not HTTPSURL"
        );
    }

    #[test]
    fn translation_follows_the_corrected_json_and_same_language_drops_it() {
        let parsed = parse_model_caption(
            "note {\"corrected\":\"確率の定義\",\"translation\":\"definition of probability\"}",
            "確立の定義",
            true,
        )
        .unwrap();
        assert_eq!(parsed.corrected, "確率の定義");
        assert_eq!(
            parsed.translation.as_deref(),
            Some("definition of probability")
        );
        let same = parse_model_caption(
            "{\"corrected\":\"確率の定義\",\"translation\":\"should be ignored\"}",
            "確立の定義",
            false,
        )
        .unwrap();
        assert!(same.translation.is_none());
        assert!(parse_model_caption("{\"translation\":\"only\"}", "原文", true).is_none());
    }

    #[test]
    fn prompt_corrects_before_translating_the_correction() {
        let prompt = system_prompt(true, "中文");
        let corrected = prompt.find("corrected").unwrap();
        let translation = prompt.find("translation").unwrap();
        assert!(corrected < translation);
        assert!(prompt.contains("上文"));
        assert!(prompt.contains("corrected だけを 中文 に訳す"));
        assert!(!system_prompt(false, "日本語").contains("に訳す"));
        let mut glossary = Glossary::default();
        glossary.learn("確立".into(), "確率".into());
        let user = user_prompt(
            "確率論",
            &["期待値です".into()],
            &glossary,
            "確率の話",
            "日本語",
            "中文",
            true,
        );
        assert!(user.contains("科目: 確率論"));
        assert!(user.contains("上文:"));
        assert!(user.contains("期待値です"));
        assert!(user.contains("用語: 確立→確率"));
        assert!(user.contains("字幕: 確率の話"));
        assert!(!user.contains("字幕: 確立"));
        let long = "あ".repeat(80);
        let recent = (0..4)
            .map(|index| format!("{long}{index}。"))
            .collect::<Vec<_>>();
        let wide = user_prompt(
            "確率論",
            &recent,
            &glossary,
            "今回です",
            "日本語",
            "中文",
            true,
        );
        assert!(wide.contains(&recent[0]));
        assert!(wide.contains(&recent[3]));
    }

    #[test]
    fn overlay_belongs_only_to_the_latest_ready_translation() {
        assert!(should_show_overlay(true, true, Some("訳"), 4, 4));
        assert!(!should_show_overlay(true, true, Some("訳"), 3, 4));
        assert!(!should_show_overlay(true, false, Some("訳"), 4, 4));
        assert!(!should_show_overlay(true, true, None, 4, 4));
        assert!(!should_show_overlay(false, true, Some("訳"), 4, 4));
        assert!(!same_caption_language("auto", "ja"));
        assert!(!same_caption_language("yue", "zh"));
        assert!(same_caption_language("ja", "ja"));
        assert_eq!(target_code("zh"), "zh");
        assert_eq!(target_code("fr"), "ja");
    }
}
