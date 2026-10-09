use crate::agent_provider::AgentProvider;
use crate::ai::ChatMessage;
use crate::db::{epoch_secs, Database};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

mod artifacts;
mod commands;
mod context;
mod course_cycle;
mod items;
mod jobs;
mod organize;
mod organize_run;
mod persist;
mod plus_ai;
mod print;
mod scheduler;
mod source_events;
mod status_log;
mod summary;
mod todos;

pub(in crate::course_automation) use artifacts::*;
pub use commands::*;
pub(in crate::course_automation) use course_cycle::*;
use items::Item;
pub(in crate::course_automation) use jobs::{enqueue_job, process_job};
pub(in crate::course_automation) use organize_run::*;
pub(in crate::course_automation) use persist::*;
pub(in crate::course_automation) use plus_ai::*;
pub(in crate::course_automation) use print::*;
#[cfg(test)]
pub(in crate::course_automation) use scheduler::scheduled_cycle_is_due;
pub use scheduler::start_course_automation_loop;
pub(in crate::course_automation) use scheduler::{
    is_automatic_trigger, luna_is_authenticated, schedule_deferred_delta_followup,
    should_queue_deferred_delta_followup, should_skip_automatic_cycle,
};
pub(in crate::course_automation) use source_events::*;
pub(in crate::course_automation) use status_log::*;
#[cfg(test)]
pub(in crate::course_automation) use summary::{consolidate_archive, ARCHIVE_FALLBACK_LABEL};
pub(in crate::course_automation) use summary::{
    document_notification_key, has_observe_followup, material_source_fingerprint,
    normalize_category, normalize_course_analysis, normalize_short_list, prioritize_summary_items,
    proactive_notification_body, prune_item_states, should_refresh_summary,
};
pub(in crate::course_automation) use todos::*;

const CONFIG_PREFIX: &str = "course_automation:config:";
const STATUS_PREFIX: &str = "course_automation:status:";
const DEFAULT_INTERVAL_MINUTES: u32 = 30;
const CHECK_INTERVAL_SECS: u64 = 5 * 60;
const STARTUP_DELAY_SECS: u64 = 75;
const DEFERRED_DELTA_FOLLOWUP_SECS: u64 = 20;
const MAX_FILE_TEXT_CHARS: usize = 16_000;
const FULL_SUMMARY_NEW_ITEM_THRESHOLD: usize = 4;
const PRINT_CONFIDENCE_THRESHOLD: f32 = 0.8;
const PLUS_AI_ATTEMPTS: usize = 2;
const PLUS_AI_TIMEOUT_SECS: u64 = 300;
const LEGACY_ALL_DOCUMENTS_FAILED_ERROR: &str = "全資料の分析に失敗しました";
const LEGACY_AI_TIMEOUT_180_ERROR: &str = "AI リクエストが 180 秒でタイムアウトしました";
const LEGACY_AI_TIMEOUT_ERROR: &str = "AI リクエストがタイムアウトしました";
/// Whole-run watchdog. A hung download / print / render must never hold the
/// global run lock forever; incremental status saves let a timed-out run resume
/// on the next cycle, so this can be generous without losing work.
const RUN_TIMEOUT_SECS: u64 = 600;
const PLUS_DOCUMENT_MAX_TOKENS: u32 = 4096;
const ACTIVITY_DETAIL_REVALIDATE_AFTER_SECS: i64 = 30 * 60;
/// Internal sentinel placed on `AnalysisDocument.load_error` when a PDF yields
/// neither a text layer nor extractable images: it is skipped as a terminal,
/// non-retryable outcome rather than counted as a failure.
const DOC_SKIP_MARKER: &str = "__doc_skip__";

/// One unit of SenseA work. Every operation that issues SenseA AI requests —
/// the scheduled/manual full cycle, "re-analyze all", and single-document
/// re-analysis — is expressed as a Job and processed through the one queue, so
/// there is a single serialised mechanism rather than per-operation locks.
enum JobKind {
    Cycle { force_all: bool },
    ReanalyzeDoc { document_id: String },
    RebuildMemory,
    ConfirmPrint { category: String },
}

struct Job {
    account: crate::db::AccountContext,
    luna_id: String,
    course_name: String,
    trigger: String,
    kind: JobKind,
    /// Present for command-triggered jobs that await the result; absent for the
    /// scheduler's fire-and-forget cycles.
    respond: Option<tokio::sync::oneshot::Sender<Result<CourseAutomationView, String>>>,
}

/// How many queued jobs may run at once (queue + bounded parallelism).
const JOB_PARALLELISM: usize = 3;

pub struct CourseAutomationState {
    /// Sender into the unified job queue. All SenseA AI work goes through here.
    job_tx: tokio::sync::mpsc::UnboundedSender<Job>,
    /// Receiver, taken once by the dispatcher in `start_course_automation_loop`.
    job_rx: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<Job>>>,
    /// Coordinates the parallel jobs safely: a full cycle takes the write side
    /// (exclusive — it rewrites the whole status), single-document jobs take the
    /// read side (they run their AI in parallel with each other).
    cycle_lock: tokio::sync::RwLock<()>,
    /// Serialises the brief status read-modify-write of concurrent document
    /// jobs so their upserts merge instead of clobbering.
    status_write: tokio::sync::Mutex<()>,
    /// Hash of the last status we actually persisted+emitted, per luna_id. Lets
    /// `save_status_and_emit` skip the SQLite write and the UI event when the
    /// status is byte-identical to what the frontend already has — the common
    /// steady-state case where every reused artifact/document still triggers a
    /// save.
    last_emitted: Mutex<HashMap<String, u64>>,
}

impl CourseAutomationState {
    pub fn new() -> Self {
        let (job_tx, job_rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            job_tx,
            job_rx: Mutex::new(Some(job_rx)),
            cycle_lock: tokio::sync::RwLock::new(()),
            status_write: tokio::sync::Mutex::new(()),
            last_emitted: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for CourseAutomationState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseAutomationConfig {
    pub luna_id: String,
    pub course_name: String,
    pub enabled: bool,
    pub interval_minutes: u32,
    pub monitor_materials: bool,
    pub monitor_announcements: bool,
    pub monitor_assignments: bool,
    pub analyze_all: bool,
    /// Master switch for printing. When on, printable candidates are gated by
    /// per-category approval: the first file of a type waits for the user to
    /// confirm, after which that category prints automatically.
    pub auto_print: bool,
    /// Print categories the user has approved. A candidate whose category is
    /// listed prints without asking; others wait as `needs_confirmation`.
    #[serde(default)]
    pub approved_print_categories: Vec<String>,
    pub notify_seat_changes: bool,
}

impl CourseAutomationConfig {
    fn new(luna_id: String, course_name: String) -> Self {
        Self {
            luna_id,
            course_name,
            enabled: false,
            interval_minutes: DEFAULT_INTERVAL_MINUTES,
            monitor_materials: true,
            monitor_announcements: true,
            monitor_assignments: true,
            analyze_all: false,
            auto_print: true,
            approved_print_categories: Vec::new(),
            notify_seat_changes: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SeatConclusion {
    #[serde(default)]
    pub assignment: String,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PrintCandidate {
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub confidence: f32,
    /// A short type name grouping similar printables (例: ワークシート / 小テスト /
    /// 出席カード). Per-category user approval decides whether this prints
    /// automatically.
    #[serde(default)]
    pub category: String,
}

/// A consolidated cluster of past (expired) memories under one short heading,
/// e.g. label「完了した課題」with items for each finished assignment.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedGroup {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub items: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentCourseAnalysis {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<Item>,
    #[serde(default)]
    pub standing_context: Vec<Item>,
    /// Past memory, consolidated. When a standing memory expires it is not kept
    /// verbatim — it is folded into a short labeled group here (e.g. a finished
    /// assignment becomes one sub-item under「完了した課題」), so old context stays
    /// available to relate to future material without the list growing forever.
    /// The model maintains and condenses these groups each round.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archived_context: Vec<ArchivedGroup>,
    #[serde(default)]
    pub seat: SeatConclusion,
    #[serde(default)]
    pub print_candidates: Vec<PrintCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DocumentAnalysis {
    pub id: String,
    pub fingerprint: String,
    #[serde(default)]
    pub source_fingerprint: String,
    pub kind: String,
    pub title: String,
    pub filename: String,
    pub path: String,
    pub status: String,
    /// The text that was analysed, kept only for fileless documents
    /// (announcements have no downloaded file to re-read), so a later
    /// single-document re-analysis can run without re-fetching.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<String>,
    #[serde(default)]
    pub seat_evidence: Vec<String>,
    #[serde(default)]
    pub print_instruction: String,
    #[serde(default)]
    pub trigger_decision: String,
    #[serde(default)]
    pub observation_context: String,
    #[serde(default)]
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceEvent {
    pub id: String,
    pub event: String,
    pub document_id: String,
    pub kind: String,
    pub title: String,
    pub filename: String,
    #[serde(default)]
    pub previous_summary: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub attention: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CourseArtifactRecord {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub filename: String,
    pub path: String,
    pub source_fingerprint: String,
    pub status: String,
    #[serde(default)]
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ActivityDetailCacheRecord {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub detail_path: String,
    pub list_fingerprint: String,
    pub source_fingerprint: String,
    pub checked_at: i64,
}

/// One entry in the run log shown on the control capsule's detail page. Each
/// entry describes a single operation (file + what happened). `level` is one of
/// "ok" / "warn" / "error" and drives the dot colour; `message` is ready-to-show
/// text such as「『資料X』を分析」.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RunLogEntry {
    #[serde(default)]
    pub at: i64,
    #[serde(default)]
    pub level: String,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AiUsageEstimate {
    #[serde(default)]
    pub at: i64,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub prompt_tokens: usize,
    #[serde(default)]
    pub response_tokens: usize,
    #[serde(default)]
    pub max_tokens: u32,
    #[serde(default)]
    pub attempts: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PrintResult {
    /// Stable identity for one print action. New records include the source
    /// fingerprint when available, so a revised file with the same filename is not
    /// mistaken for an already-printed old version.
    #[serde(default)]
    pub action_key: String,
    pub filename: String,
    pub path: String,
    pub status: String,
    #[serde(default)]
    pub detail: String,
    /// The candidate's print category, so the confirm UI can approve the whole
    /// type at once. Empty for legacy records.
    #[serde(default)]
    pub category: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CourseAutomationStatus {
    pub luna_id: String,
    pub course_name: String,
    pub running: bool,
    #[serde(default)]
    pub stage: String,
    pub last_run: Option<i64>,
    pub last_ok: Option<bool>,
    #[serde(default)]
    pub last_error: String,
    #[serde(default)]
    pub trigger: String,
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default)]
    pub downloaded_files: Vec<String>,
    #[serde(default)]
    pub external_links: Vec<String>,
    #[serde(default)]
    pub total_documents: usize,
    #[serde(default)]
    pub processed_documents: usize,
    #[serde(default)]
    pub current_document: String,
    #[serde(default)]
    pub document_analyses: Vec<DocumentAnalysis>,
    #[serde(default)]
    pub artifacts: Vec<CourseArtifactRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activity_detail_cache: Vec<ActivityDetailCacheRecord>,
    #[serde(default)]
    pub pending_summary_ids: Vec<String>,
    #[serde(default)]
    pub source_events: Vec<SourceEvent>,
    #[serde(default)]
    pub pending_source_event_ids: Vec<String>,
    #[serde(default)]
    pub last_summary_document_ids: Vec<String>,
    #[serde(default)]
    pub pending_notification_ids: Vec<String>,
    #[serde(default)]
    pub notified_document_ids: Vec<String>,
    #[serde(default)]
    pub pending_seat_notification: bool,
    #[serde(default)]
    pub last_notified_seat_assignment: String,
    #[serde(default)]
    pub analysis: AgentCourseAnalysis,
    #[serde(default)]
    pub print_results: Vec<PrintResult>,
    /// Theme groups the agent auto-filed the downloaded documents into. Display
    /// only — the disk is the source of truth via the per-document paths.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub organize_groups: Vec<organize::OrganizeGroup>,
    /// The last organize batch's moves, kept so it can be reverted in one step.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub organize_undo: Vec<organize::OrganizeMove>,
    /// Whether an organize batch is available to undo (mirrors `organize_undo`
    /// for the dock, which doesn't receive the raw move log).
    #[serde(default)]
    pub organize_can_undo: bool,
    /// When the documents were last auto-filed (epoch seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_organized: Option<i64>,
    /// Fingerprint of the organize candidate set the last time we ran the AI
    /// planner. While it is unchanged, every file is already in its theme folder
    /// (filing is idempotent), so the planner is skipped — no wasted request on a
    /// set we've already planned. Internal bookkeeping; not shown in the UI.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub organize_signature: String,
    /// Per-course user-state overlay keyed by item id (e.g. "done" / "known").
    /// Unified across facets; survives the model regenerating items because the
    /// id is a stable content fingerprint. Pruned to currently-present ids.
    #[serde(default)]
    pub item_states: HashMap<String, String>,
    /// Recent run history shown on the control capsule's detail page (bounded).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub run_log: Vec<RunLogEntry>,
    /// Bounded estimated AI token telemetry for SenseA requests. Providers do not
    /// consistently return usage, so this stores local estimates for debugging
    /// timeout/limit behaviour without cluttering the dock UI.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ai_usage: Vec<AiUsageEstimate>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseAutomationView {
    pub config: CourseAutomationConfig,
    pub status: CourseAutomationStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnalysisDocument {
    kind: String,
    title: String,
    filename: String,
    path: String,
    content: String,
    source_fingerprint: String,
    load_error: String,
    // Page images for a scanned PDF with no text layer; sent to the vision
    // model instead of text. Never serialized into the prompt JSON.
    #[serde(skip)]
    images: Vec<crate::ai::ImagePart>,
}

#[derive(Debug, Clone, Default)]
struct SourceDocumentInfo {
    kind: String,
    title: String,
    filename: String,
    source_fingerprint: String,
}

/// The shared data-cache key the main TODO page reads detail-generated todos from
/// (must match the frontend's `DETAIL_GENERATED_TODO_KEY`).
const DETAIL_TODO_CACHE_KEY: &str = "detail_generated_todo";
const LIVE_TODO_CACHE_KEY: &str = "live_generated_todo";
const LUNA_TODO_CACHE_KEY: &str = "luna_todo";

/// One detail-generated todo, mirroring the frontend `DetailGeneratedTodo` shape
/// (snake_case JSON). `extra` preserves any fields the frontend owns so other
/// todos round-trip untouched.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
struct DetailTodo {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    course_name: String,
    #[serde(default)]
    content_type: String,
    #[serde(default)]
    deadline: String,
    #[serde(default)]
    source_url: String,
    #[serde(default)]
    source_excerpt: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    archived_at: Option<String>,
    #[serde(flatten)]
    extra: serde_json::Map<String, Value>,
}

#[cfg(test)]
mod tests;
