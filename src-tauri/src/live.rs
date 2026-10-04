use chrono::{DateTime, Datelike, Duration as ChronoDuration, Local};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};
use tokio::sync::Notify;

mod ai_output;
mod cache;
mod markdown;
mod types;

use self::cache::{
    auto_save_day_cache, formal_markdown_filename, live_storage_dir, load_day_cache,
    remove_day_cache, save_day_cache_full, write_formal_markdown_file, write_partial_markdown_file,
};
use self::markdown::build_markdown;
use ai_output::{
    clamp_chars, extract_json_object, format_latest_whiteboard_context, latest_whiteboard,
    parse_chunk_ai_result, reconcile_whiteboard, value_to_trimmed_string,
};
#[cfg(test)]
use cache::{
    replay_deltas_into, LiveDayCache, LiveDayCacheRef, LiveLineDeltaOwned, LiveLineDeltaRef,
};
use types::LiveChunkAiResult;
pub use types::{
    LiveCourseInfo, LiveSaveResult, LiveSessionSnapshot, LiveSummaryChunk, LiveTermExplanation,
    LiveTodoSuggestion, LiveTodoSuggestionsEvent, LiveTranscriptLine, LiveWhiteboard,
    LiveWhiteboardEdge, LiveWhiteboardNode,
};

const MIN_AI_SUMMARIZATION_DURATION_SECS: i64 = 120;
const MAX_LIVE_TERM_EXPLANATION_CHARS: usize = 220;
const LIVE_FLUSH_FORCE_WAIT_ATTEMPTS: usize = 1200;
const LIVE_FLUSH_FORCE_WAIT_MS: u64 = 250;
// Backend driver wake-up cadence only. The actual generation interval is the
// user setting measured from `batch_started_at`, not these polling caps.
const LIVE_FLUSH_DRIVER_MAX_SLEEP_SECS: u64 = 30;
const LIVE_FLUSH_DRIVER_IDLE_SLEEP_SECS: u64 = 30;
const LIVE_FLUSH_DRIVER_MIN_SLEEP_SECS: u64 = 1;
// Whiteboard nodes/edges are intentionally uncapped: the board must accumulate
// the full course/recording as it grows, so a hard ceiling silently forces the
// model to compress earlier branches. Per-field length and the relationship
// guards in `parse_live_whiteboard` are the remaining safety nets.
const FREE_NOTE_FOLDER_NAME: &str = "自由ノート";
const PRE_AI_OVERALL_SUMMARY: &str =
    "### 全体要約\n_(AI生成前の一時保存です。生成完了後に更新します)_";

pub struct LiveState(Mutex<Option<LiveSession>>, Arc<Notify>);

#[derive(Debug, Clone)]
struct LiveSession {
    session_id: String,
    course: LiveCourseInfo,
    started_at: DateTime<Local>,
    transcript_lines: Arc<Vec<LiveTranscriptLine>>,
    pending_lines: Arc<Vec<LiveTranscriptLine>>,
    summaries: Arc<Vec<LiveSummaryChunk>>,
    /// Timestamp of the last finalized subtitle line covered by a successful
    /// summary chunk. Initialized to session start for the first chunk.
    batch_started_at: DateTime<Local>,
    flush_in_flight: bool,
    /// True when this session began with no prior cache for today —
    /// i.e. it owns the on-disk .md/day_cache and cancel may scrub them.
    /// False when resumed from an earlier session today; cancel must leave
    /// the prior content intact.
    is_fresh_start: bool,
    /// How many entries of `transcript_lines` have already been persisted
    /// (either in the main cache snapshot or appended to the deltas log).
    /// Drives the incremental day-cache write.
    persisted_line_count: usize,
}

impl LiveSession {
    fn snapshot(&self) -> LiveSessionSnapshot {
        // Next scheduled periodic summary: the interval measured from the
        // effective batch start (last covered line / batch start). It's an
        // estimate — the actual flush also waits for ≥3 pending lines — but
        // good enough to drive a countdown.
        let next_summary_at_ms = Some(
            (effective_batch_started_at(self)
                + ChronoDuration::minutes(live_summary_interval_minutes()))
            .timestamp_millis(),
        );
        // All three Vec<...> are Arc-wrapped, so cloning is a refcount bump.
        LiveSessionSnapshot {
            active: true,
            course: Some(self.course.clone()),
            started_at: Some(format_datetime(self.started_at)),
            transcript_lines: Arc::clone(&self.transcript_lines),
            pending_lines: Arc::clone(&self.pending_lines),
            summaries: Arc::clone(&self.summaries),
            next_summary_at_ms,
            summarizing: self.flush_in_flight,
        }
    }
}

impl LiveState {
    pub fn new() -> Self {
        Self(Mutex::new(None), Arc::new(Notify::new()))
    }

    pub fn notify_flush_driver(&self) {
        self.1.notify_waiters();
    }

    fn flush_notify(&self) -> Arc<Notify> {
        Arc::clone(&self.1)
    }
}

fn empty_snapshot() -> LiveSessionSnapshot {
    LiveSessionSnapshot {
        active: false,
        course: None,
        started_at: None,
        transcript_lines: Arc::new(Vec::new()),
        pending_lines: Arc::new(Vec::new()),
        summaries: Arc::new(Vec::new()),
        next_summary_at_ms: None,
        summarizing: false,
    }
}

fn format_datetime(dt: DateTime<Local>) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn format_time(dt: DateTime<Local>) -> String {
    dt.format("%H:%M").to_string()
}

fn emit_live_finish_progress(app: &tauri::AppHandle, step: &str) {
    let _ = app.emit("live-finish-progress", serde_json::json!({ "step": step }));
}

fn clock_time_on_session_date(
    session_started_at: DateTime<Local>,
    value: &str,
) -> Option<DateTime<Local>> {
    let time = chrono::NaiveTime::parse_from_str(value.trim(), "%H:%M:%S")
        .or_else(|_| chrono::NaiveTime::parse_from_str(value.trim(), "%H:%M"))
        .ok()?;
    let mut candidate = session_started_at
        .date_naive()
        .and_time(time)
        .and_local_timezone(Local)
        .earliest()?;
    if candidate + ChronoDuration::hours(12) < session_started_at {
        candidate += ChronoDuration::days(1);
    }
    Some(candidate)
}

fn transcript_line_datetime(
    session_started_at: DateTime<Local>,
    line: &LiveTranscriptLine,
) -> Option<DateTime<Local>> {
    clock_time_on_session_date(session_started_at, &line.at)
}

fn summary_range_end_datetime(
    session_started_at: DateTime<Local>,
    summary: &LiveSummaryChunk,
) -> Option<DateTime<Local>> {
    let (_, end) = summary
        .range_label
        .rsplit_once('-')
        .or_else(|| summary.range_label.rsplit_once('–'))?;
    clock_time_on_session_date(session_started_at, end)
}

fn latest_summary_end_datetime(
    session_started_at: DateTime<Local>,
    summaries: &[LiveSummaryChunk],
) -> Option<DateTime<Local>> {
    summaries
        .last()
        .and_then(|summary| summary_range_end_datetime(session_started_at, summary))
}

fn last_transcript_line_datetime(
    session_started_at: DateTime<Local>,
    lines: &[LiveTranscriptLine],
    fallback: DateTime<Local>,
) -> DateTime<Local> {
    lines
        .last()
        .and_then(|line| transcript_line_datetime(session_started_at, line))
        .unwrap_or(fallback)
}

fn first_transcript_line_datetime(
    session_started_at: DateTime<Local>,
    lines: &[LiveTranscriptLine],
    fallback: DateTime<Local>,
) -> DateTime<Local> {
    lines
        .first()
        .and_then(|line| transcript_line_datetime(session_started_at, line))
        .unwrap_or(fallback)
}

fn effective_batch_started_at(session: &LiveSession) -> DateTime<Local> {
    if session.summaries.is_empty() {
        return first_transcript_line_datetime(
            session.started_at,
            session.pending_lines.as_ref(),
            session.batch_started_at,
        );
    }
    session.batch_started_at
}

fn sanitize_model_output(text: &str) -> String {
    let mut s = text.replace("<think>", "").replace("</think>", "");
    while let Some(start) = s.find("<think") {
        if let Some(end) = s[start..].find("</think>") {
            let end_idx = start + end + "</think>".len();
            s.replace_range(start..end_idx, "");
        } else {
            s.truncate(start);
            break;
        }
    }
    s.trim().to_string()
}

fn sanitize_filename_component(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            _ => c,
        })
        .collect();
    let trimmed = s.trim().trim_matches('.');
    if trimmed.is_empty() {
        "live".into()
    } else {
        trimmed.to_string()
    }
}

fn current_snapshot(state: &LiveState) -> LiveSessionSnapshot {
    state
        .0
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|session| session.snapshot()))
        .unwrap_or_else(empty_snapshot)
}

fn emit_live_update(app: &tauri::AppHandle, state: &LiveState) {
    let _ = app.emit("live-session-updated", current_snapshot(state));
}

fn live_ai_config() -> Result<crate::ai::AiConfig, String> {
    let cfg = crate::ai::load_ai_config();
    if !cfg.ai_enabled {
        return Err("Live要約にはAIを有効にしてください".into());
    }
    if cfg.provider == "local" {
        crate::local_ai_support::ensure_supported()?;
    } else if cfg.api_key.is_empty() {
        // Surfaces the real cause instead of letting the request 401 silently.
        // Most common in packaged builds when the keychain item is unreadable.
        log::warn!(
            "[Live] AI summarization aborted: api_key empty for provider '{}' (secret store unreadable?)",
            cfg.provider
        );
        return Err("AI APIキーを読み込めませんでした。設定でAPIキーを保存し直してください".into());
    }
    Ok(cfg)
}

fn live_summary_interval_minutes() -> i64 {
    crate::ai::load_ai_config()
        .live_summary_interval_minutes
        .max(5) as i64
}

fn should_skip_ai_summarization(started_at: DateTime<Local>, now: DateTime<Local>) -> bool {
    now.signed_duration_since(started_at).num_seconds() < MIN_AI_SUMMARIZATION_DURATION_SECS
}

fn should_run_finish_ai(
    provider: &str,
    started_at: DateTime<Local>,
    ended_at: DateTime<Local>,
) -> bool {
    provider != "local" && !should_skip_ai_summarization(started_at, ended_at)
}

fn should_require_finish_chunk_ai(
    started_at: DateTime<Local>,
    ended_at: DateTime<Local>,
    pending_line_count: usize,
) -> bool {
    pending_line_count > 0 && !should_skip_ai_summarization(started_at, ended_at)
}

fn format_recent_summary_context(summaries: &[LiveSummaryChunk], limit: usize) -> String {
    if summaries.is_empty() || limit == 0 {
        return "なし".to_string();
    }

    summaries
        .iter()
        .rev()
        .take(limit)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|chunk| format!("## {}\n{}\n{}", chunk.title, chunk.range_label, chunk.body))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Emit the full prior-chunk history (summary bodies + term explanations) so
/// the whiteboard-only call can build the cumulative board from the already
/// distilled record instead of re-parsing every raw transcript. Used as the
/// auxiliary "前面的所有总结和词条" context for the whiteboard call.
fn format_full_history_for_whiteboard(summaries: &[LiveSummaryChunk]) -> String {
    if summaries.is_empty() {
        return "なし".to_string();
    }
    // Every chunk keeps its segment summary (body) so coverage/ordering and the
    // running narrative stay intact. Only the heavier per-term notes are trimmed
    // for older chunks — those terms are already present as nodes in the
    // cumulative board JSON passed alongside, so re-sending their explanations
    // each time just grows the prompt linearly over a long session.
    const TERMS_DETAIL_RECENT: usize = 4;
    let cutoff = summaries.len().saturating_sub(TERMS_DETAIL_RECENT);
    let mut out = String::new();
    for (idx, chunk) in summaries.iter().enumerate() {
        if idx > 0 {
            out.push_str("\n\n");
        }
        out.push_str(&format!(
            "## Chunk {:02} | {}\n題: {}\n{}",
            idx + 1,
            chunk.range_label,
            chunk.title,
            chunk.body
        ));
        if !chunk.terms.is_empty() {
            if idx < cutoff {
                // Older chunk: list term names only (explanations live on the board).
                let names = chunk
                    .terms
                    .iter()
                    .map(|term| term.term.as_str())
                    .collect::<Vec<_>>()
                    .join("、");
                out.push_str(&format!("\n用語: {}", names));
            } else {
                out.push_str("\n用語:\n");
                for term in &chunk.terms {
                    out.push_str(&format!("- {}: {}", term.term, term.explanation));
                    if !term.external_source.is_empty() {
                        out.push_str(&format!("（出典: {}）", term.external_source));
                    }
                    out.push('\n');
                }
            }
        }
    }
    out
}

/// Emit the just-generated current-chunk summary + terms in the same shape as
/// the historical entries. Fed to the whiteboard call so it knows what this
/// segment introduced.
fn format_current_chunk_for_whiteboard(
    body: &str,
    terms: &[LiveTermExplanation],
    range_label: &str,
) -> String {
    let mut out = format!("範囲: {}\n要約:\n{}", range_label, body);
    if !terms.is_empty() {
        out.push_str("\n用語:\n");
        for term in terms {
            out.push_str(&format!("- {}: {}", term.term, term.explanation));
            if !term.external_source.is_empty() {
                out.push_str(&format!("（出典: {}）", term.external_source));
            }
            out.push('\n');
        }
    }
    out
}

fn normalized_excerpt_match_text(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase()
}

fn whiteboard_excerpt_terms(node: &LiveWhiteboardNode) -> Vec<String> {
    let mut terms = Vec::new();
    for source in [&node.label, &node.detail] {
        for part in source.split(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    'の' | 'と'
                        | 'や'
                        | '・'
                        | '、'
                        | '。'
                        | '，'
                        | ','
                        | '.'
                        | ':'
                        | '：'
                        | ';'
                        | '；'
                        | '('
                        | ')'
                        | '（'
                        | '）'
                        | '['
                        | ']'
                        | '【'
                        | '】'
                        | '/'
                        | '／'
                        | '-'
                        | '_'
                        | '+'
                        | '＋'
                        | '='
                )
        }) {
            let normalized = normalized_excerpt_match_text(part);
            let char_count = normalized.chars().count();
            if char_count >= 3 || (char_count >= 2 && normalized.is_ascii()) {
                terms.push(normalized);
            }
        }
    }
    terms.sort_by_key(|term| std::cmp::Reverse(term.chars().count()));
    terms.dedup();
    terms.truncate(8);
    terms
}

fn best_transcript_excerpt_for_node(
    node: &LiveWhiteboardNode,
    lines: &[LiveTranscriptLine],
) -> Option<String> {
    let terms = whiteboard_excerpt_terms(node);
    if terms.is_empty() {
        return None;
    }

    lines
        .iter()
        .filter_map(|line| {
            let normalized = normalized_excerpt_match_text(&line.text);
            if normalized.is_empty() {
                return None;
            }
            let score = terms
                .iter()
                .filter(|term| normalized.contains(term.as_str()))
                .map(|term| term.chars().count())
                .sum::<usize>();
            if score == 0 {
                None
            } else {
                Some((score, line.text.as_str()))
            }
        })
        .max_by_key(|(score, text)| (*score, text.chars().count()))
        .map(|(_, text)| clamp_chars(text, 80))
}

fn enrich_whiteboard_source_excerpts(
    mut board: Option<LiveWhiteboard>,
    previous: Option<&LiveWhiteboard>,
    terms: &[LiveTermExplanation],
    lines: &[LiveTranscriptLine],
) -> Option<LiveWhiteboard> {
    let whiteboard = board.as_mut()?;
    let previous_by_id = previous
        .map(|prev| {
            prev.nodes
                .iter()
                .filter(|node| !node.source_excerpt.trim().is_empty())
                .map(|node| (node.id.as_str(), node.source_excerpt.as_str()))
                .collect::<std::collections::HashMap<_, _>>()
        })
        .unwrap_or_default();
    let previous_by_label = previous
        .map(|prev| {
            prev.nodes
                .iter()
                .filter(|node| !node.source_excerpt.trim().is_empty())
                .map(|node| (node.label.as_str(), node.source_excerpt.as_str()))
                .collect::<std::collections::HashMap<_, _>>()
        })
        .unwrap_or_default();

    for node in &mut whiteboard.nodes {
        if node.source_type != "lecture" || !node.source_excerpt.trim().is_empty() {
            continue;
        }
        if let Some(excerpt) = previous_by_id
            .get(node.id.as_str())
            .or_else(|| previous_by_label.get(node.label.as_str()))
        {
            node.source_excerpt = clamp_chars(excerpt, 80);
            continue;
        }
        if node.node_type == "term" {
            if let Some(term) = terms.iter().find(|term| {
                !term.source_excerpt.trim().is_empty()
                    && (term.term == node.label
                        || node.label.contains(term.term.as_str())
                        || term.term.contains(node.label.as_str()))
            }) {
                node.source_excerpt = clamp_chars(&term.source_excerpt, 80);
                continue;
            }
        }
        if let Some(excerpt) = best_transcript_excerpt_for_node(node, lines) {
            node.source_excerpt = excerpt;
        }
    }

    board
}

fn live_whiteboard_language_instruction(reply_language: &str) -> &'static str {
    match reply_language {
        "zh" => {
            "whiteboard 的 title、node.label、node.detail 必须全部使用简体中文；非空 edge.label 也必须使用简体中文。node_type、kind、role、source_type、id、parent_id 等结构字段仍使用指定英文枚举值。关系标签要使用中文具体词，例如「具体例」「条件」「导出」「确认点」「并列」「参考」。"
        }
        "en" => {
            "All whiteboard title, node.label, and node.detail values must be written in English; non-empty edge.label values must also be written in English. Structural fields such as node_type, kind, role, source_type, id, and parent_id must keep the specified enum values. Edge labels should be concrete English relationship words such as \"example\", \"condition\", \"leads to\", \"check\", \"parallel\", or \"reference\"."
        }
        "ko" => {
            "whiteboard 의 title, node.label, node.detail 은 모두 한국어로 작성하고, 비어 있지 않은 edge.label 도 한국어로 작성하세요. node_type, kind, role, source_type, id, parent_id 같은 구조 필드는 지정된 영어 enum 값을 유지하세요. 관계 라벨은 「구체예」「조건」「도출」「확인점」「병렬」「참고」처럼 구체적인 한국어 관계어를 사용하세요."
        }
        _ => {
            "whiteboard の title、node.label、node.detail はすべて日本語で書き、空でない edge.label も日本語で書く。node_type、kind、role、source_type、id、parent_id などの構造フィールドは指定された英語 enum 値のままにする。edge label は「具体例」「条件」「導く」「確認点」「並列」「参考」など、具体的な日本語の関係語にする。"
        }
    }
}

fn live_reply_language_hint(reply_language: &str) -> &'static str {
    crate::ai::reply_language_hint(
        reply_language,
        "\n\n重要: 输出全文的自然语言内容必须使用简体中文。JSON 字段名和枚举值保持指定格式。",
        "\n\nIMPORTANT: Write all natural-language output in English. Keep JSON field names and enum values in the specified format.",
        "\n\n중요: 자연어 출력 전체를 한국어로 작성하세요. JSON 필드명과 enum 값은 지정된 형식을 유지하세요.",
    )
}

fn live_overall_output_format(reply_language: &str) -> &'static str {
    match reply_language {
        "zh" => "### 整体总结\n用简洁段落概括整场内容。\n### 本次论点\n- 列出主要论点，每个论点保持简洁",
        "en" => "### Overall Summary\nSummarize the whole session in a concise paragraph.\n### Key Points\n- List the main points from the session concisely",
        "ko" => "### 전체 요약\n전체 내용을 간결한 문단으로 요약한다.\n### 이번 논점\n- 주요 논점을 간결하게 나열한다",
        _ => "### 全体要約\n講義全体の主旨を簡潔な段落にまとめる。\n### 今回の論点\n- 講義で取り上げられた主要論点を簡潔な箇条書きで列挙",
    }
}

fn short_session_overall_summary(
    course: &LiveCourseInfo,
    transcript_line_count: usize,
    reply_language: &str,
) -> String {
    let heading = match reply_language {
        "zh" => "### 整体总结",
        "en" => "### Overall Summary",
        "ko" => "### 전체 요약",
        _ => "### 全体要約",
    };
    match (reply_language, course.is_free_note) {
        ("zh", true) => format!(
            "{}\n由于自由笔记少于2分钟，未进行AI总结，已直接保存全文转写（{}行）。",
            heading, transcript_line_count
        ),
        ("zh", false) => format!(
            "{}\n由于LIVE少于2分钟，未进行AI总结，已直接保存{}的全文转写（{}行）。",
            heading, course.course_name, transcript_line_count
        ),
        ("en", true) => format!(
            "{}\nBecause this free note was under 2 minutes, AI summarization was skipped and the full transcript ({} lines) was saved as-is.",
            heading, transcript_line_count
        ),
        ("en", false) => format!(
            "{}\nBecause this LIVE session was under 2 minutes, AI summarization was skipped and the full transcript for {} ({} lines) was saved as-is.",
            heading, course.course_name, transcript_line_count
        ),
        ("ko", true) => format!(
            "{}\n자유 노트가 2분 미만이어서 AI 요약을 실행하지 않고 전체 전사({}줄)를 그대로 저장했습니다.",
            heading, transcript_line_count
        ),
        ("ko", false) => format!(
            "{}\nLIVE가 2분 미만이어서 AI 요약을 실행하지 않고 {}의 전체 전사({}줄)를 그대로 저장했습니다.",
            heading, course.course_name, transcript_line_count
        ),
        (_, true) => format!(
            "{}\n2分未満の自由ノートのためAI要約は行わず、全文転写（{}行）をそのまま保存しました。",
            heading, transcript_line_count
        ),
        (_, false) => format!(
            "{}\n2分未満のLIVEのためAI要約は行わず、{}の全文転写（{}行）をそのまま保存しました。",
            heading, course.course_name, transcript_line_count
        ),
    }
}

fn fallback_overall_summary(
    course: &LiveCourseInfo,
    transcript_line_count: usize,
    summary_count: usize,
    reply_language: &str,
) -> String {
    let heading = match reply_language {
        "zh" => "### 整体总结",
        "en" => "### Overall Summary",
        "ko" => "### 전체 요약",
        _ => "### 全体要約",
    };
    match (reply_language, course.is_free_note) {
        ("zh", true) => format!(
            "{}\n已保存包含 {} 行转写和 {} 条分段总结的自由笔记。",
            heading, transcript_line_count, summary_count
        ),
        ("zh", false) => format!(
            "{}\n已保存 {} 的课堂笔记，包含 {} 行转写和 {} 条分段总结。",
            heading, course.course_name, transcript_line_count, summary_count
        ),
        ("en", true) => format!(
            "{}\nSaved a free note containing {} transcript lines and {} chunk summaries.",
            heading, transcript_line_count, summary_count
        ),
        ("en", false) => format!(
            "{}\nSaved lecture notes for {} with {} transcript lines and {} chunk summaries.",
            heading, course.course_name, transcript_line_count, summary_count
        ),
        ("ko", true) => format!(
            "{}\n전사 {}줄과 분할 요약 {}개를 포함한 자유 노트를 저장했습니다.",
            heading, transcript_line_count, summary_count
        ),
        ("ko", false) => format!(
            "{}\n{} 강의 메모를 저장했습니다. 전사 {}줄과 분할 요약 {}개가 포함되어 있습니다.",
            heading, course.course_name, transcript_line_count, summary_count
        ),
        (_, true) => format!(
            "{}\n{} 件の転写行と {} 件の分割要約を含む自由ノートを保存しました。",
            heading, transcript_line_count, summary_count
        ),
        (_, false) => format!(
            "{}\n{} の講義メモ。{}件の転写行と{}件の分割要約を保存しました。",
            heading, course.course_name, transcript_line_count, summary_count
        ),
    }
}

fn live_chunk_system_prompt(language_hint: &str, is_free_note: bool) -> String {
    // Call 1 of the per-chunk pipeline: produces summary_markdown + terms only.
    // The whiteboard is generated by a separate downstream call, so this
    // prompt intentionally omits any whiteboard schema/rules.
    let mut prompt = if is_free_note {
        r#"あなたは自由ノート録音の整理アシスタントです。音声認識（STT）による文字起こしを基に、直近の録音内容を要約し、同じ区間で出た重要な人物・概念・出来事・ルール・固有名詞だけを注釈してください。

共通方針:
- 文字起こしには誤認識（同音異義語の取り違え、聞き取り不良による文字化け）が含まれる場合があります。文脈から正しい意味を推測し、明らかな誤認識は自然な範囲で修正してください。
- 原文が断片的でも、文脈上ほぼ確実な内容は読みやすい表現に補って構いません。
- 具体的な数字・年号・割合・固有名詞・順位・因果関係などの高リスク事実は、文字起こしまたは直近文脈から十分に確認できる場合だけ書いてください。確信が弱い場合は一般化するか削除してください。
- 外部知識は、用語理解に必要な標準的定義・短い例・一般的背景を補う場合だけ使えます。使った場合は external_source に確認可能な出典名とURL、公式文書名、書籍名などを書いてください。出典を示せない外部知識は使わないでください。
- 自由ノートは講義とは限りません。会話、会議、メディア音声、自習メモ、アイデアメモでも、録音された内容そのものを整理対象にしてください。非学術的という理由だけで「整理対象外」にしないでください。
- 明らかな無音・相槌・聞き取り不能な断片は省略してよいですが、会話の展開、人物関係、固有のルール、出来事の流れは整理対象にしてください。
- summary_markdown と terms は今回新しく話された内容を中心にし、過去2区間を重複して要約し直さないでください。
- 内容が少ない区間では無理に情報量を増やさず、確認できた範囲だけを簡潔にまとめてください。
- 文体は、あとから見返せる録音メモのように簡潔で具体的にしてください。

出力形式（JSONのみ、厳守。Markdownフェンスや説明文を付けない。whiteboard 等のフィールドは出力しない）:
{"summary_markdown":"- 重点見出し（名詞句または短文）\n- 重点見出し\n\n---\n\n**重点見出し**: 補足説明（具体的に）\n\n**重点見出し**: 補足説明（具体的に）","terms":[{"term":"専門用語または固有概念","explanation":"録音文脈での意味に加え、論点との関係・注意点・短い例のいずれかを補う。","source_excerpt":"録音内の根拠になる短い発話断片","external_source":"外部知識を使った場合の正確な出典名とURL。使っていない場合は空文字"}]}

summary_markdown のルール:
- 上半分: 箇条書きタイトルのみ。録音の核心概念やキーワードを、理解に必要な分だけ含める。
- 下半分(---以降): 各重点の補足を段落形式で記述。箇条書き(- )は使わない。
- 見出し(###等)は使わない。
- 不明瞭な部分を無理に解釈せず、確信できる情報のみ記載する。

terms のルール:
- 今回の区間で出た重要な人物名・作品名・ルール名・概念・出来事・固有名詞・略語だけを選ぶ。
- 注釈対象は「その語や人物・ルールを知らないと録音内容の理解が止まりやすいもの」に限定する。
- 人物名・地名・道具名・一度だけ出た固有名詞を、出現したという理由だけで terms にしない。summary_markdown の文中で十分読めるもの、または白板の構造ラベルを読む妨げにならないものは省く。
- 自由ノートでは、繰り返し話題の軸になる人物/場所/仕組み、複数の論点をつなぐ名前、または知らないと出来事の意味が分からない語だけを選ぶ。
- 一般常識、日常語、単なる相槌、意味の薄い断片は注釈しない。
- explanation は簡潔にする。語の意味だけで終わらせず、録音内の話題との関係、混同しやすい点、短い例、または見返す観点を補う。
- source_excerpt は必ず録音内の根拠だけを書く。external_source は外部知識を使った場合だけ書く。録音内だけで十分理解できる名前や固有設定に、公式サイト・百科事典などの外部出典を無理に付けない。
- 該当語が少ない場合は terms を空配列にする。
"#
    } else {
        r#"あなたは大学講義メモの整理アシスタントです。音声認識（STT）による文字起こしを基に、直近の講義内容を要約し、同じ区間で出た重要な専門用語・固有概念だけを注釈してください。

共通方針:
- 文字起こしには誤認識（同音異義語の取り違え、聞き取り不良による文字化け）が含まれる場合があります。文脈から正しい意味を推測し、明らかな誤認識は自然な範囲で修正してください。
- 原文が断片的でも、文脈上ほぼ確実な内容は読みやすい表現に補って構いません。
- 具体的な数字・年号・割合・固有名詞・順位・因果関係などの高リスク事実は、文字起こしまたは直近文脈から十分に確認できる場合だけ書いてください。確信が弱い場合は一般化するか削除してください。
- 外部知識は、用語理解に必要な標準的定義・短い例・一般的背景を補う場合だけ使えます。使った場合は external_source に確認可能な出典名とURL、公式文書名、書籍名などを書いてください。出典を示せない外部知識は使わないでください。
- 雑談や教室管理の発言（出席確認、マイク調整等）は省略し、学術的内容に集中してください。
- summary_markdown と terms は今回新しく話された内容を中心にし、過去2区間を重複して要約し直さないでください。
- 内容が少ない区間では無理に情報量を増やさず、確認できた範囲だけを簡潔にまとめてください。
- 文体は、信頼できる講義ノートのように簡潔で具体的にしてください。

出力形式（JSONのみ、厳守。Markdownフェンスや説明文を付けない。whiteboard 等のフィールドは出力しない）:
{"summary_markdown":"- 重点見出し（名詞句または短文）\n- 重点見出し\n\n---\n\n**重点見出し**: 補足説明（具体的に）\n\n**重点見出し**: 補足説明（具体的に）","terms":[{"term":"専門用語または固有概念","explanation":"講義文脈での意味に加え、論点との関係・注意点・短い例のいずれかを補う。","source_excerpt":"講義内の根拠になる短い発話断片","external_source":"外部知識を使った場合の正確な出典名とURL。使っていない場合は空文字"}]}

summary_markdown のルール:
- 上半分: 箇条書きタイトルのみ。講義の核心概念やキーワードを、理解に必要な分だけ含める。
- 下半分(---以降): 各重点の補足を段落形式で記述。箇条書き(- )は使わない。
- 見出し(###等)は使わない。
- 不明瞭な部分を無理に解釈せず、確信できる情報のみ記載する。

terms のルール:
- 今回の区間で出た専門用語・理論名・手法名・制度名・固有概念・略語だけを選ぶ。
- 注釈対象は「その語を知らないと講義の理解が止まりやすいもの」に限定する。
- 一般常識、日常語、教室運営語、授業一般の語、辞書的に自明な普通名詞は注釈しない。例: 授業、講義、先生、学生、教室、出席、課題、レポート、資料、今日、次回。
- その科目で専門的な意味を持つ場合を除き、単に有名・一般的という理由で語を選ばない。
- explanation は簡潔にする。語の意味だけで終わらせず、講義内の論点との関係、混同しやすい点、短い例、または復習時に見る観点を補う。
- source_excerpt は必ず講義内の根拠だけを書く。external_source は外部知識を使った場合だけ書く。
- 該当語が少ない場合は terms を空配列にする。
"#
    }
    .to_string();
    if is_free_note {
        prompt = prompt
            .replace("講義文脈", "録音文脈")
            .replace("講義内の根拠", "録音内の根拠")
            .replace("講義内なら", "録音内なら")
            .replace(
                "講義の核心概念やキーワード",
                "録音内容の中心話題やキーワード",
            );
    }
    prompt.push_str(language_hint);
    prompt
}

fn live_whiteboard_system_prompt(language_instruction: &str, is_free_note: bool) -> String {
    // Call 2 of the per-chunk pipeline: produces ONLY the cumulative whiteboard.
    // Input includes prior summaries+terms, the current cumulative board, the
    // just-generated current-chunk summary+terms, and the raw transcript.
    let mut prompt = r#"あなたは知識整理ボード（whiteboard）を作る専門アシスタントです。分割要約・用語注釈・現在の累積ボード・今回の文字起こしを総合し、講義開始から現在までの累積 whiteboard JSON だけを返してください。summary_markdown / terms / 説明文は返さないでください。

出力形式（JSONのみ、厳守。Markdownフェンスや説明文を付けない）:
{"whiteboard":{"title":"短い題名","layout":"flow|hub|compare|cycle|grid","nodes":[{"id":"stable-id","label":"短い概念名","detail":"白板内で理解できる短い説明","node_type":"structure|term","kind":"core|support|question|result","role":"main|branch","parent_id":"branch の親 main id、または term の親 structure id。全体用語は空文字","source_type":"lecture|external","source_excerpt":"講義内根拠。外部なら空文字","external_source":"外部補足の出典。講義内なら空文字"}],"edges":[{"from":"n1","to":"n2","label":"具体的な関係語"}]}}

目的:
- whiteboard は本文の代替ではなく、扱われた課題・観点・展開・関係を素早く掴むための概念図です。
- 正しい出力は「課題のまとまりが見える」「関係が読める」「用語が主構造を邪魔しない」状態です。
- ノードや edge の量は固定目標ではなく、理解に必要かどうかで決める。必要なものを削らず、不要なものを増やさない。

実行順序:
1. 既存ボードの情報を読む。削除前提ではなく、更新・移動・追加・置換を考える。
2. 今回までの録音全体について、内容本体・活動内容・方法論・運営連絡を分ける。
3. 各まとまりに合う構造パターンを選ぶ。パターン名自体を node にしない。
4. main は「何について整理しているか」を示す上位テーマ、branch は構成要素・立場・理由・手順・事例・結果にする。
5. parent_id / edge / term を自検し、意味上の所属・関係・補助説明だけを残す。

内容構造の再編:
- 時系列メモにしない。分割要約の区間や発話順をそのまま main にしない。
- main は時間チャンクではなく、概念領域・中核制度・主要素材・問題領域・発展段階を束ねる上位カテゴリにする。
- 「基礎概念 → 中核メカニズム/歴史的展開 → 現代的課題」「背景/問題 → 解決策 → 実装/応用 → 帰結/限界」のような骨格が見える場合はそれを優先する。
- 同一の発展脈絡にある A/B/C を有名語だからと平行 main にしない。共通 main 配下の branch とし、edge で「基礎」「応用」「完成」「帰結」などを示す。
- schema は main/branch の二層が基本。branch の下位関係は parent_id で入れ子にせず、branch 間 edge.label で読む。

内容タイプ別の骨格選択（構造パターン庫）:
- まず主要タイプを判定し、下の骨格を自然な label に言い換える。すべてを同じ「話題一覧」にしない。
- 討論・ディベート・賛否検討: 論題 main -> 肯定側/否定側/立場A/B、理由、根拠、質疑、反駁、判定。ルール・採点・態度は方法論 main。論題の理由ノードを方法論 main 配下に置かない。
- 比較・対照: 比較軸/問い main -> 対象A/B、基準、共通点、差異、結論。
- ケース分析・事例紹介: 事例/中心問題 main -> 背景、関係者、出来事、原因、対応、結果、教訓。
- 問題解決・政策/制度検討: 問題/政策課題 main -> 現状、原因、制約、選択肢、評価基準、提案、リスク、残課題。
- 因果メカニズム・理論説明: 理論/メカニズム main -> 前提、要因、媒介過程、結果、反例、適用範囲。
- 分類・体系整理: 分類軸/体系 main -> カテゴリ、基準、代表例、境界例、例外。
- 手順・プロセス・歴史展開: 全体プロセス main -> 段階、条件、分岐、成果、課題。
- 資料・文献・テキスト読解: 資料/読解上の問い main -> 主張、根拠、キーワード、解釈、引用箇所、批判点、確認事項。
- データ・統計・図表解釈: データが答える問い main -> 指標、観察結果、傾向、比較、解釈、留保、次の確認点。
- Q&A・相談・個別指導: 質問/相談テーマ main -> 質問、回答、理由、追加確認、次アクション。
- 発表・作品・提出物への講評: 成果物/評価観点 main -> 良い点、改善点、根拠、修正方針、提出条件。人格評価と混ぜない。
- 研究指導・レポート相談: 指導対象/研究テーマ群 main -> テーマ、論点の絞り込み、文献可否、方法、次の作業。
- 実習・演習・ワークショップ: 作業/技能 main -> 目的、手順、観察、つまずき、フィードバック、改善方法。
- 語学・表現練習: 技能/表現課題 main -> 語彙、文法、発音、例文、誤用、訂正、使い分け。
- 意思決定・計画立案: 決めること main -> 候補、条件、制約、判断基準、決定、担当、期限、未決事項。
- ブレインストーミング・アイデア整理: 中心テーマ main -> アイデア群、目的、制約、採用候補、保留案、次の検証。
- 物語・出来事の整理: 出来事/展開 main -> 背景、登場要素、転換点、結果、解釈、反応。
- 連絡・運営・予定調整: 運営事項 main -> 決定事項、変更理由、期限、担当、次アクション。
- 概念講義: 上位概念 main -> 定義、背景、メカニズム、具体例、応用、限界。
- 雑談・反応・メタコメント: 内容理解に必要な反応だけ branch 化。内容本体を説明しない雑談は main にしない。

混在タイプの分離:
- 一区間に正文内容（概念説明、資料読解、制度説明、事例分析など）と活動内容（討論、質疑、発表講評、研究相談、運営連絡など）が混ざる場合、「何について学んでいるか」と「どう扱っているか」を分ける。
- 正文説明が独立して十分あるなら正文 main を作り、討論・講評・相談は活動 main、ルール・採点・やり方は方法論 main、予定・締切・担当は運営 main にする。
- 討論内の事実や制度が立場の根拠にすぎないなら討論 main 配下。独立した説明に発展した時だけ正文 main へ分ける。
- 所属に迷う branch は「答えている問い」で決める。中身の説明なら正文、立場/反駁/判定なら活動、やり方なら方法論。

判断手順:
1. 直前ボードの全 nodes / edges / id を保持対象として読み、既に表現された情報を失わない前提を置く。
2. 全履歴と今回区間から、時系列ではなく内容タイプと内容上の骨格（概念領域、問題、解決策、応用、帰結、現代課題など）を見直す。
3. 今回区間の各材料を、既存 main の続き、既存 main の branch/term、上位 main を新設すべき新領域、または全体用語に分類する。
4. parent_id が意味上の所属先と合っているかを点検する。ルール・方法論・講評の main が、内容本体の理由・根拠・事例を親として吸い込んでいないか必ず確認する。
5. 既存ノードの更新で十分な材料は label/detail/edge/parent_id を更新する。既存ノードでは表せない新材料だけ node として追加する。
6. 最後に nodes の順序と edge.label を整え、読解順が「前提→展開→帰結」または「問題→解決策→結果」になるようにする。

累積更新（最重要）:
- whiteboard は差分ではなく、録音開始から現在までの累積ボード全体を毎回返す。ノード総数に上限はない。区間が進むほど表現すべき情報は増える前提で設計する。
- 既存情報は原則すべて引き継ぐ。話題が変わっても既出の具体論点を消さない。
- 今回返す nodes 配列の長さは、直前ボードの長さ「以上」を基本とする。新材料があれば新規 structure / branch / term を追加する。
- 累積増加は「既存ノードを更新しない」という意味ではない。label / detail / source_excerpt / kind / edge label / parent_id はより正確に更新してよい。
- 旧ノードは、情報を明示的に引き継げるなら、より正確な上位概念・分割・統合へアップグレードしてよい。古い id 固執より情報を保った置換を優先する。
- 禁止: 既出論点を消す、旧 branch / term を main detail にだけ押し込む、別話題へ曖昧に吸収する、「前区間の内容」のような要約表現で隠す。
- 重複・STT 誤認識・意味不明ノードは訂正/統合/分割/置換してよい。旧情報の引き継ぎは detail または edge で分かるようにする。
- 既存 edge は両端ノードが残る限り維持し、ノード置換時は意味を新 edge に移す。最初期以外で空配列や極端な縮小にしない。

話題境界:
- 今回区間が既存 main の続きか、新しい話題・章・素材・論点かを判定してから追加する。
- 新話題の目安: 主語/対象/人物/制度/問題設定が大きく変わる、因果関係が薄い、締め/導入がある、別素材/会話/教材/議題へ切り替わる、用語集合がほぼ重ならない。
- 新話題でも、散らばった点を同じ素材・人物群・制度・事例・問題設定・説明目的で束ね、合理的な上位 main を合成する。細かい名前や一言コメントを main に乱立させない。
- 既存話題の続きなら新 main を増やさず、該当 main 配下の branch / term / edge として追加・更新する。
- 複数話題が混ざる場合は「同じ上位テーマで説明できる散点群」ごとに振り分ける。別 main 間 edge は明確な因果・比較・前提・反論・同一対象だけ。

ノード:
- 主次を必ず分ける。role="main" は講義の主要課題・章・観点を代表するノードにし、少数の冒頭ノードだけに固定し続けない。
- role="branch" の分岐ノードは必ず parent_id で最も近い主ノードに接続し、主ノードなしの孤立分岐を作らない。
- parent_id は「その branch が何についての構成要素か」で決める。発話中に教師が評価・講評・ルール説明をしたからといって、内容本体の理由や事例をルール/講評 main の配下へ移さない。内容本体の所属は内容本体の main に置き、講評やルールとの関係は edge で表す。
- branch が別 branch の下位要素に見える場合でも、schema 上は最も近い上位 main を parent_id にする。branch 同士の疑似階層は edge.label で「構成」「理由」「根拠」「反論」「例」「結果」などを明示する。
- 新しい語・固有名詞・出来事・数値・属性だけを理由に main を増やさない。大きな論点、主要素材、説明対象そのものが変わった時だけ main を増やす。
- 複数 branch が同じ「何について」の答えになるなら、それらを包む上位 main を作る。
- 構造ノード: 外すと流れ・対比・因果・制度/人物関係が分かりにくくなる概念。用語ノード: 構造ノードを読むための短い定義・別名・属性・背景語。
- 用語ノードは node_type="term"、role="branch"、kind="support"。最も近い構造ノードを parent_id にし、親が明確でない全体用語だけ parent_id=""。用語同士や別グループへの edge は作らない。
- 用語は「知らないと理解が止まる語」「何度も出る語」「構造ラベル理解に必要な語」に限る。出た語をすべて term にしない。
- 人物名・地名・組織名・道具名は、主語/結節点になる時だけ構造ノード。単発の登場名や属性は detail/source_excerpt に含める。
- 各 node の detail は白板内だけでも最低限理解できるように、講義文脈での役割・条件・注意点を短く具体的に書く。
- 講義内に出た概念は source_type="lecture" とし、source_excerpt に根拠となる短い発話断片を可能な限り入れる。外部補足以外で根拠がある node の source_excerpt をまとめて空にしない。
- 理解に役立つ標準的な背景知識・関連概念は必要に応じて少数追加してよいが、必ず source_type="external" とし、external_source に確認可能な出典を書く。外部補足ノードは原則 branch にし、detail の末尾にも外部補足だと分かる表現を入れる。
- 出典を示せない外部補足、具体値や固有事実の断定、講義から離れすぎた発展は追加しない。

レイアウト:
- layout は内容で選ぶ。flow=時系列/手順/因果/継承/発展、compare=明確な対比、cycle=反復循環、grid=独立並列、hub=単一中心から自然に放射する場合だけ。
- 中心放射に見せるためだけに hub を選ばない。複数課題が並ぶだけなら無理に一本道の flow にしない。
- nodes 配列は読解順。main を先に、branch は所属 main の直後に置き、発話順より概念上の前提→展開→帰結を優先する。

エッジ:
- edge は因果・流れ・対比・包含・条件など、外すと構造理解が悪くなる関係だけ。弱い関連、隣接、連想、知識追加目的のリンクは作らない。
- 強い関連は同じ main 配下へまとめ、横断 edge は重要な因果・対比・条件・制度接続だけ。
- parent_id だけで主従が十分なら同じ関係を edge で重複しない。用語ノードの edge は親構造ノードとだけ、label=""。
- label は具体的な関係語にする。単に「関連」「説明」「補足」だけにしない。
- core→support は「具体例」「条件」「手順」「背景」。support/core→result は「導く」「結論」「効果」「適用」。question は「確認点」「未解決」「答え」。result 同士は強い推論がなければ「並列」「比較」「まとめ」。
- title には「復習」という語を避け、知識整理・概念整理として自然な短い題名を付ける。

最終セルフチェック:
- main が時間区間や発話順ではなく、内容上のまとまりになっている。
- 混在内容では、正文 main / 活動 main / 方法論 main / 運営 main が必要に応じて分かれている。
- 討論では、論題 main の下に立場 branch があり、理由・根拠・反駁・判定が方法論 main に吸い込まれていない。
- parent_id は意味上の所属先で、branch 同士の下位関係は edge.label で読める。
- 用語ノードが主構造を圧迫せず、出た名前・語をすべて term にしていない。
- 既出情報は消えていない。旧ノードのアップグレード・統合・分割時も情報の引き継ぎが分かる。

"#
    .to_string();
    if is_free_note {
        prompt = prompt
            .replace("講義開始", "録音開始")
            .replace("講義内容", "録音内容")
            .replace("講義の流れ", "録音の流れ")
            .replace("講義・録音全体", "録音全体")
            .replace("講義の主要課題・章・観点", "録音の主要話題・場面・観点")
            .replace("講義内", "録音内")
            .replace("講義文脈", "録音文脈")
            .replace("講義の大きな論点", "録音の大きな話題");
        prompt.push_str(
            "\n自由ノートでは source_type=\"lecture\" は互換性のための列挙値であり、「録音内に出た内容」という意味で使う。UI と保存 Markdown では録音内根拠として表示される前提で、source_excerpt も録音内の短い根拠を書く。録音内容が非学術的でも、人物関係・出来事・ルール・話題の構造がある場合は whiteboard を作り、整理対象外にしない。\n",
        );
    }
    prompt.push_str(language_instruction);
    prompt
}

fn live_overall_system_prompt(
    reply_language: &str,
    language_hint: &str,
    is_free_note: bool,
) -> String {
    let prompt = if is_free_note {
        "あなたは自由ノート録音を仕上げるアシスタントです。分割要約と末尾の文字起こしを基に、録音全体を俯瞰する要約をMarkdownで返してください。\n\n注意事項:\n- 各分割要約を単純に繋げるのではなく、録音全体を貫く話題、出来事の流れ、人物・概念の関係を抽出してください。\n- 自由ノートは講義とは限りません。会話、会議、メディア音声、自習メモ、アイデアメモでも録音内容そのものを整理対象にし、非学術的という理由だけで除外しないでください。\n- 文字起こしには音声認識の誤りが含まれる可能性があります。文脈から意味を推測し、明らかな誤認識は自然な範囲で補正して構いません。\n- 原文が断片的でも、文脈上ほぼ確実な内容は読みやすく整理して構いません。\n- 具体的な数字・年号・割合・固有名詞・順位・因果関係などの高リスク事実は、分割要約または文字起こしから十分に確認できる場合だけ書いてください。\n- 高リスク事実について確信が弱い場合は、一般化するか削除してください。外部知識だけで具体値や詳細を補ってはいけません。\n- 文体は、あとから見返せる録音メモのように簡潔で具体的にしてください。"
    } else {
        "あなたは大学講義ノートを仕上げるアシスタントです。分割要約と末尾の文字起こしを基に、講義全体を俯瞰する要約をMarkdownで返してください。\n\n注意事項:\n- 各分割要約を単純に繋げるのではなく、講義全体を貫くテーマや論理の流れを抽出してください。\n- 文字起こしには音声認識の誤りが含まれる可能性があります。文脈から意味を推測し、明らかな誤認識は自然な範囲で補正して構いません。\n- 原文が断片的でも、文脈上ほぼ確実な内容は読みやすく整理して構いません。\n- 具体的な数字・年号・割合・固有名詞・順位・因果関係などの高リスク事実は、分割要約または文字起こしから十分に確認できる場合だけ書いてください。\n- 高リスク事実について確信が弱い場合は、一般化するか削除してください。外部知識だけで具体値や詳細を補ってはいけません。\n- 講義全体の理解を助ける整理はしてよいですが、補った背景知識を講義で明示された事実のように書いてはいけません。\n- 文体は、信頼できる講義ノートのように簡潔で具体的にしてください。"
    };
    format!(
        "{}\n\n出力形式（厳守）:\n{}\n\nルール:\n- 指定形式以外のセクションや見出しを追加しない。\n- 抽象的すぎる表現を避け、{}固有の具体的概念やキーワードを含める。{}",
        prompt,
        live_overall_output_format(reply_language),
        if is_free_note { "録音" } else { "講義" },
        language_hint
    )
}

fn live_todo_language_instruction(reply_language: &str) -> &'static str {
    match reply_language {
        "zh" => "title、note、source_excerpt 使用简体中文；content_type 必须保持日语枚举值。",
        "en" => "Write title, note, and source_excerpt in English; keep content_type as one of the Japanese enum values.",
        "ko" => "title, note, source_excerpt 는 한국어로 작성하고, content_type 은 일본어 enum 값으로 유지하세요.",
        _ => "title、note、source_excerpt は日本語で書き、content_type は指定された日本語 enum 値を使う。",
    }
}

fn live_todo_system_prompt(reply_language: &str) -> String {
    format!(
        "あなたは大学講義ノートから学生のTODO候補だけを抽出するアシスタントです。先生が明確に課題、提出物、宿題、レポート、事前準備、復習タスク、小テスト準備として指示したものだけを抽出してください。講義内容そのもの、一般的な学習アドバイス、AIが勝手に作った復習案は含めません。締切は発話中の具体日付/時刻を最優先し、「次回まで」「来週の授業まで」「授業計画の該当回まで」など相対的に判断できる場合は、現在日時・次回授業候補・授業計画から YYYY-MM-DD HH:mm 形式で推定してください。推定した場合は note に根拠を短く含めてください。どうしても判断できない場合だけ deadline を空文字にします。{}\n\n出力はJSONのみで、説明文やMarkdownを付けないでください。形式: {{\"todos\":[{{\"title\":\"課題名\",\"content_type\":\"課題|レポート|予習|復習|テスト準備|その他\",\"deadline\":\"YYYY-MM-DD HH:mm または 空文字\",\"note\":\"学生が次にすることを短く。締切推定時は根拠も短く\",\"source_excerpt\":\"根拠になる発話を短く\"}}]}}。候補がなければ {{\"todos\":[]}}。",
        live_todo_language_instruction(reply_language)
    )
}

/// Two-pass chunk pipeline:
///   Call 1 → summary_markdown + terms (sees raw transcript only)
///   Call 2 → whiteboard JSON only (sees all prior summaries+terms, the
///            current cumulative board, the just-produced summary+terms, and
///            the raw transcript for completeness)
///
/// Splitting the calls lets each output budget breathe (the whiteboard JSON no
/// longer competes with summary tokens) and lets the whiteboard call work from
/// already-distilled prior material rather than re-parsing every transcript.
/// If Call 2 fails, we surface `whiteboard = None` and let `reconcile_whiteboard`
/// carry the previous board forward.
async fn summarize_chunk(
    course: &LiveCourseInfo,
    lines: &[LiveTranscriptLine],
    recent_summaries: &[LiveSummaryChunk],
    range_label: &str,
) -> Result<LiveChunkAiResult, String> {
    let cfg = live_ai_config()?;
    let language_hint = live_reply_language_hint(&cfg.reply_language);
    let whiteboard_language_instruction = live_whiteboard_language_instruction(&cfg.reply_language);
    let transcript = lines
        .iter()
        .map(|line| format!("- [{}] {}", line.at, line.text))
        .collect::<Vec<_>>()
        .join("\n");
    // Whiteboard (Call 2) gets a transcript trimmed to its tail to bound token
    // cost when a chunk window contains an unusually large number of STT lines.
    // Call 1 still sees the full transcript because summary+terms accuracy
    // depends on covering every line.
    const WHITEBOARD_TRANSCRIPT_LINE_CAP: usize = 500;
    let transcript_for_whiteboard = if lines.len() > WHITEBOARD_TRANSCRIPT_LINE_CAP {
        let elided = lines.len() - WHITEBOARD_TRANSCRIPT_LINE_CAP;
        let mut out = format!("(... 古い文字起こし {} 行を省略 ...)\n", elided);
        out.push_str(
            &lines
                .iter()
                .skip(elided)
                .map(|line| format!("- [{}] {}", line.at, line.text))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        out
    } else {
        transcript.clone()
    };
    let course_block = if course.is_free_note {
        format!("記録種別: 自由ノート\n題名: {}", course.course_name)
    } else {
        format!(
            "講義: {}\n授業コード: {}\n教員: {}\n教室: {}\n時間帯: {}",
            course.course_name,
            course.course_code,
            if course.teacher.is_empty() {
                "不明"
            } else {
                &course.teacher
            },
            if course.room.is_empty() {
                "未設定"
            } else {
                &course.room
            },
            course.time_label,
        )
    };
    let trailing_note = if course.is_free_note {
        "注記: 自由ノートは講義とは限りません。録音内容そのものを対象に、人物・出来事・ルール・話題の流れを整理してください。文字起こしの固有名詞には STT の誤認識が混ざる可能性があります。".to_string()
    } else {
        format!(
            "注記: 文字起こしの専門用語・固有名詞は STT の誤認識が混ざる可能性があります。講義名「{}」の分野脈絡を手がかりに、明らかな誤りは自然に補正してください。",
            course.course_name
        )
    };

    // === Call 1: summary + terms ===
    let recent_summary_context = format_recent_summary_context(recent_summaries, 2);
    let messages_1 = vec![
        crate::ai::ChatMessage {
            role: "system".into(),
            content: live_chunk_system_prompt(language_hint, course.is_free_note),
            images: Vec::new(),
        },
        crate::ai::ChatMessage {
            role: "user".into(),
            content: format!(
                "{}\n\n直前の分割要約:\n{}\n\n今回の文字起こし:\n{}\n\n{}",
                course_block, recent_summary_context, transcript, trailing_note,
            ),
            images: Vec::new(),
        },
    ];
    let raw_1 = crate::ai::chat_completion_public(&cfg, messages_1).await?;
    let parsed_1 = parse_chunk_ai_result(&raw_1);
    // An empty body means the model returned nothing usable (e.g. a reasoning
    // model that emitted only a <think> block stripped by sanitize). Treat it as
    // a failure rather than pushing a blank chunk: the caller's error path keeps
    // the pending lines and does not advance batch_started_at, so the retry
    // layers (driver loop mid-session, final-flush retry on stop) re-attempt it.
    if parsed_1.body.trim().is_empty() {
        return Err("AI要約の本文が空でした（再試行します）".to_string());
    }

    // === Call 2: whiteboard only ===
    let whiteboard_context = format_latest_whiteboard_context(recent_summaries);
    let full_history = format_full_history_for_whiteboard(recent_summaries);
    let current_chunk_brief =
        format_current_chunk_for_whiteboard(&parsed_1.body, &parsed_1.terms, range_label);
    let messages_2 = vec![
        crate::ai::ChatMessage {
            role: "system".into(),
            content: live_whiteboard_system_prompt(
                whiteboard_language_instruction,
                course.is_free_note,
            ),
            images: Vec::new(),
        },
        crate::ai::ChatMessage {
            role: "user".into(),
            content: format!(
                "{}\n\nこれまでの全分割要約と用語注釈（累積素材）:\n{}\n\n現在の累積知識整理ボード:\n{}\n\n今回新しく生成された区間の要約と用語:\n{}\n\n今回の文字起こし（補助参考、必要に応じて細部を拾う。長すぎる場合は末尾のみ表示）:\n{}\n\n指示: system の実行順序と構造パターン庫に従い、録音開始から現在までの累積 whiteboard JSON を返す。既出情報を失わず、必要なら既存ノードを更新・移動・アップグレード・統合・分割する。新しい具体材料は追加する。最後に parent_id、edge、term、混在タイプ分離をセルフチェックする。",
                course_block,
                full_history,
                whiteboard_context,
                current_chunk_brief,
                transcript_for_whiteboard,
            ),
            images: Vec::new(),
        },
    ];
    let whiteboard = match crate::ai::chat_completion_public(&cfg, messages_2).await {
        Ok(raw_2) => parse_chunk_ai_result(&raw_2).whiteboard,
        Err(err) => {
            eprintln!(
                "[Live whiteboard] secondary call failed: {err}; carrying previous board forward"
            );
            None
        }
    };
    let whiteboard = enrich_whiteboard_source_excerpts(
        whiteboard,
        latest_whiteboard(recent_summaries),
        &parsed_1.terms,
        lines,
    );

    Ok(LiveChunkAiResult {
        body: parsed_1.body,
        terms: parsed_1.terms,
        whiteboard,
    })
}

async fn summarize_overall(
    course: &LiveCourseInfo,
    summaries: &[LiveSummaryChunk],
    transcript_lines: &[LiveTranscriptLine],
) -> Result<String, String> {
    let cfg = live_ai_config()?;
    let language_hint = live_reply_language_hint(&cfg.reply_language);
    let summary_text = summaries
        .iter()
        .map(|chunk| format!("## {}\n{}\n{}", chunk.title, chunk.range_label, chunk.body))
        .collect::<Vec<_>>()
        .join("\n\n");
    let recent_transcript = transcript_lines
        .iter()
        .rev()
        .take(24)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|line| format!("- [{}] {}", line.at, line.text))
        .collect::<Vec<_>>()
        .join("\n");
    let user_content = if course.is_free_note {
        format!(
            "記録種別: 自由ノート\n題名: {}\n\n分割要約:\n{}\n\n終盤の文字起こし:\n{}\n\n注記: 文字起こしには STT 誤認識が含まれる可能性があります。録音内容の文脈から、明らかな誤りは自然に補正してください。自由ノートは講義とは限らないため、会話・素材記録・自習メモなど実際の内容に合わせて整理してください。",
            course.course_name, summary_text, recent_transcript,
        )
    } else {
        format!(
            "講義: {}\n授業コード: {}\n教員: {}\n\n分割要約:\n{}\n\n終盤の文字起こし:\n{}\n\n注記: 文字起こしには STT 誤認識が含まれる可能性があります。講義名「{}」の分野脈絡から、明らかな誤りは自然に補正してください。",
            course.course_name,
            course.course_code,
            if course.teacher.is_empty() {
                "不明"
            } else {
                &course.teacher
            },
            summary_text,
            recent_transcript,
            course.course_name,
        )
    };
    let messages = vec![
        crate::ai::ChatMessage {
            role: "system".into(),
            content: live_overall_system_prompt(
                &cfg.reply_language,
                language_hint,
                course.is_free_note,
            ),
            images: Vec::new(),
        },
        crate::ai::ChatMessage {
            role: "user".into(),
            content: user_content,
            images: Vec::new(),
        },
    ];
    let raw = crate::ai::chat_completion_public(&cfg, messages).await?;
    Ok(sanitize_model_output(&raw))
}

async fn generate_overall_summary(
    course: &LiveCourseInfo,
    started_at: DateTime<Local>,
    ended_at: DateTime<Local>,
    summaries: &[LiveSummaryChunk],
    transcript_lines: &[LiveTranscriptLine],
) -> String {
    let ai_config = crate::ai::load_ai_config();
    let reply_language = ai_config.reply_language.clone();
    if should_skip_ai_summarization(started_at, ended_at) {
        return short_session_overall_summary(course, transcript_lines.len(), &reply_language);
    }
    if !should_run_finish_ai(&ai_config.provider, started_at, ended_at) {
        return fallback_overall_summary(
            course,
            transcript_lines.len(),
            summaries.len(),
            &reply_language,
        );
    }
    summarize_overall(course, summaries, transcript_lines)
        .await
        .unwrap_or_else(|_| {
            fallback_overall_summary(
                course,
                transcript_lines.len(),
                summaries.len(),
                &reply_language,
            )
        })
}

async fn extract_todo_suggestions(
    app: &tauri::AppHandle,
    course: &LiveCourseInfo,
    summaries: &[LiveSummaryChunk],
    transcript_lines: &[LiveTranscriptLine],
    ended_at: DateTime<Local>,
) -> Vec<LiveTodoSuggestion> {
    if course.is_free_note || transcript_lines.is_empty() {
        return Vec::new();
    }
    let Ok(cfg) = live_ai_config() else {
        return Vec::new();
    };
    let summary_text = summaries
        .iter()
        .map(|chunk| format!("## {}\n{}\n{}", chunk.title, chunk.range_label, chunk.body))
        .collect::<Vec<_>>()
        .join("\n\n");
    let transcript = transcript_lines
        .iter()
        .rev()
        .take(80)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|line| format!("- [{}] {}", line.at, line.text))
        .collect::<Vec<_>>()
        .join("\n");
    let course_plan_context = live_todo_course_plan_context(app, course, ended_at);
    let messages = vec![
        crate::ai::ChatMessage {
            role: "system".into(),
            content: live_todo_system_prompt(&cfg.reply_language),
            images: Vec::new(),
        },
        crate::ai::ChatMessage {
            role: "user".into(),
            content: format!(
                "講義: {}\n授業コード: {}\n曜日/時限: {} {}\n教員: {}\n\n締切推定の参考情報:\n{}\n\nAIレポート/分割要約:\n{}\n\n文字起こし（終盤中心）:\n{}\n\nこの講義内で明確に指示されたTODO/課題候補だけを抽出し、必要なDDLをできるだけ補ってください。",
                course.course_name,
                course.course_code,
                course.day,
                course.period,
                if course.teacher.is_empty() { "不明" } else { &course.teacher },
                course_plan_context,
                summary_text,
                transcript,
            ),
            images: Vec::new(),
        },
    ];
    let Ok(raw) = crate::ai::chat_completion_public(&cfg, messages).await else {
        return Vec::new();
    };
    let Some(json_text) = extract_json_object(&raw) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json_text) else {
        return Vec::new();
    };
    let Some(items) = value.get("todos").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items.iter().take(6) {
        let title = value_to_trimmed_string(item.get("title"));
        if title.is_empty() {
            continue;
        }
        let content_type = value_to_trimmed_string(item.get("content_type"));
        out.push(LiveTodoSuggestion {
            title,
            course_name: course.course_name.clone(),
            content_type: if content_type.is_empty() {
                "課題".to_string()
            } else {
                content_type
            },
            deadline: value_to_trimmed_string(item.get("deadline")),
            note: value_to_trimmed_string(item.get("note")),
            source_excerpt: value_to_trimmed_string(item.get("source_excerpt")),
            day: course.day,
            period: course.period,
        });
    }
    out
}

fn live_todo_course_plan_context(
    app: &tauri::AppHandle,
    course: &LiveCourseInfo,
    ended_at: DateTime<Local>,
) -> String {
    let mut lines = vec![
        format!("現在日時: {}", ended_at.format("%Y-%m-%d %H:%M")),
        format!(
            "次回授業候補: {}",
            next_course_meeting_hint(course, ended_at).unwrap_or_else(|| "不明".to_string())
        ),
    ];
    let course_code = course.course_code.trim();
    if course_code.is_empty() {
        lines.push("授業計画: 授業コードなし".to_string());
        return lines.join("\n");
    }

    let db = app.state::<crate::db::Database>();
    match db.get_all_session_plans() {
        Ok(plans) => {
            if let Some((_, course_plans)) =
                plans.iter().find(|(code, _)| code.trim() == course_code)
            {
                lines.push("授業計画:".to_string());
                for plan in course_plans.iter().take(18) {
                    let mut parts = Vec::new();
                    if !plan.th_header.trim().is_empty() {
                        parts.push(clamp_chars(&plan.th_header, 80));
                    }
                    if !plan.topic.trim().is_empty() {
                        parts.push(clamp_chars(&plan.topic, 160));
                    }
                    if !plan.study_outside.trim().is_empty() {
                        parts.push(format!(
                            "授業外学修: {}",
                            clamp_chars(&plan.study_outside, 180)
                        ));
                    }
                    if !parts.is_empty() {
                        lines.push(format!("第{}回: {}", plan.session_num, parts.join(" / ")));
                    }
                }
            } else {
                lines.push("授業計画: キャッシュなし".to_string());
            }
        }
        Err(_) => lines.push("授業計画: 読み込み失敗".to_string()),
    }

    if let Ok(Some(detail)) = db.get_kgc_course_detail(course_code) {
        let detail_lines = detail
            .fields
            .iter()
            .filter(|(label, value)| {
                let label = label.as_str();
                !value.trim().is_empty()
                    && (label.contains("授業外")
                        || label.contains("課題")
                        || label.contains("評価")
                        || label.contains("試験"))
            })
            .take(4)
            .map(|(label, value)| format!("{}: {}", label, clamp_chars(value, 160)))
            .collect::<Vec<_>>();
        if !detail_lines.is_empty() {
            lines.push("シラバス補足:".to_string());
            lines.extend(detail_lines);
        }
    }

    lines.join("\n")
}

fn next_course_meeting_hint(course: &LiveCourseInfo, ended_at: DateTime<Local>) -> Option<String> {
    if !(1..=7).contains(&course.day) {
        return None;
    }
    let today = ended_at.weekday().number_from_monday() as i32;
    let mut days_until = (course.day - today + 7) % 7;
    if days_until == 0 {
        days_until = 7;
    }
    let date = ended_at.date_naive() + ChronoDuration::days(days_until as i64);
    let time = course_period_start_time(course.period);
    Some(match time {
        Some((hour, minute)) => format!("{} {:02}:{:02}", date.format("%Y-%m-%d"), hour, minute),
        None => date.format("%Y-%m-%d").to_string(),
    })
}

fn course_period_start_time(period: i32) -> Option<(u32, u32)> {
    if period < 1 {
        return None;
    }
    crate::config::PERIOD_TIMES
        .get((period - 1) as usize)
        .map(|(start_h, start_m, _, _)| (*start_h, *start_m))
}

fn build_chunk_title(index: usize, start: DateTime<Local>, end: DateTime<Local>) -> String {
    format!(
        "Chunk {:02} | {}-{}",
        index,
        format_time(start),
        format_time(end)
    )
}

async fn flush_session_summary(
    state: &LiveState,
    force: bool,
) -> Result<LiveSessionSnapshot, String> {
    let mut wait_attempts = 0usize;
    let summary_interval_minutes = live_summary_interval_minutes();
    let (session_id, course, lines, recent_summaries, range_start, range_end, chunk_index) = loop {
        let captured = {
            let now = Local::now();
            let mut guard = state
                .0
                .lock()
                .map_err(|_| "Live state lock failed".to_string())?;
            let session = guard
                .as_mut()
                .ok_or_else(|| "Liveセッションが開始されていません".to_string())?;
            if session.flush_in_flight {
                let snapshot = session.snapshot();
                if !force || wait_attempts >= LIVE_FLUSH_FORCE_WAIT_ATTEMPTS {
                    return Ok(snapshot);
                }
                None
            } else {
                if session.pending_lines.is_empty() {
                    return Ok(session.snapshot());
                }
                if should_skip_ai_summarization(session.started_at, now) {
                    return Ok(session.snapshot());
                }
                let batch_started_at = effective_batch_started_at(session);
                if !force
                    && now.signed_duration_since(batch_started_at).num_minutes()
                        < summary_interval_minutes
                {
                    return Ok(session.snapshot());
                }
                // Scheduled summaries follow the original noise guard: wait
                // until at least a few finalized STT segments accumulated.
                // Forced flushes on stop still include any remaining content.
                if !force && session.pending_lines.len() < 3 {
                    return Ok(session.snapshot());
                }
                let lines = session.pending_lines.clone();
                let range_end =
                    last_transcript_line_datetime(session.started_at, lines.as_ref(), now);
                session.flush_in_flight = true;
                Some((
                    session.session_id.clone(),
                    session.course.clone(),
                    lines,
                    session.summaries.clone(),
                    batch_started_at,
                    range_end,
                    session.summaries.len() + 1,
                ))
            }
        };
        if let Some(captured) = captured {
            break captured;
        }
        wait_attempts += 1;
        tokio::time::sleep(std::time::Duration::from_millis(LIVE_FLUSH_FORCE_WAIT_MS)).await;
    };

    let range_label = format!("{}-{}", format_time(range_start), format_time(range_end));
    let chunk_ai_result = summarize_chunk(&course, &lines, &recent_summaries, &range_label).await;
    let summarized_line_count = lines.len();
    let chunk_ai = match chunk_ai_result {
        Ok(chunk_ai) => chunk_ai,
        Err(err) => {
            let mut guard = state
                .0
                .lock()
                .map_err(|_| "Live state lock failed".to_string())?;
            if let Some(session) = guard.as_mut() {
                if session.session_id == session_id {
                    session.flush_in_flight = false;
                }
            }
            return Err(err);
        }
    };
    {
        let mut guard = state
            .0
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        let Some(session) = guard.as_mut() else {
            return Ok(empty_snapshot());
        };
        if session.session_id != session_id {
            return Ok(session.snapshot());
        }
    }
    let reconciled_board =
        reconcile_whiteboard(latest_whiteboard(&recent_summaries), chunk_ai.whiteboard);

    let mut guard = state
        .0
        .lock()
        .map_err(|_| "Live state lock failed".to_string())?;
    let Some(session) = guard.as_mut() else {
        return Ok(empty_snapshot());
    };
    if session.session_id != session_id {
        return Ok(session.snapshot());
    }
    session.flush_in_flight = false;
    if session.pending_lines.is_empty() {
        return Ok(session.snapshot());
    }
    let summary = LiveSummaryChunk {
        title: build_chunk_title(chunk_index, range_start, range_end),
        range_label: range_label.clone(),
        body: chunk_ai.body,
        line_count: lines.len(),
        terms: chunk_ai.terms,
        whiteboard: reconciled_board,
    };
    Arc::make_mut(&mut session.summaries).push(summary);
    let pending = Arc::make_mut(&mut session.pending_lines);
    let drain_count = summarized_line_count.min(pending.len());
    pending.drain(0..drain_count);
    session.batch_started_at = range_end;
    Ok(session.snapshot())
}

/// Final-flush retry on stop. Unlike scheduled chunks, the closing segment has
/// no driver loop to retry it, so a single transient AI failure would lose the
/// last segment permanently. Retry a few times with backoff before giving up.
const FINAL_FLUSH_RETRY_ATTEMPTS: usize = 3;

async fn flush_final_summary_with_retry(state: &LiveState) -> Result<LiveSessionSnapshot, String> {
    let mut last_err = "final flush failed".to_string();
    for attempt in 1..=FINAL_FLUSH_RETRY_ATTEMPTS {
        match flush_session_summary(state, true).await {
            Ok(snapshot) => return Ok(snapshot),
            Err(err) => {
                log::warn!(
                    "[Live] final chunk flush failed on attempt {}/{}: {}",
                    attempt,
                    FINAL_FLUSH_RETRY_ATTEMPTS,
                    err
                );
                last_err = err;
                if attempt < FINAL_FLUSH_RETRY_ATTEMPTS {
                    tokio::time::sleep(std::time::Duration::from_secs(2 * attempt as u64)).await;
                }
            }
        }
    }
    Err(last_err)
}

fn live_session_matches(state: &LiveState, session_id: &str) -> bool {
    state
        .0
        .lock()
        .ok()
        .and_then(|guard| {
            guard
                .as_ref()
                .map(|session| session.session_id == session_id)
        })
        .unwrap_or(false)
}

fn live_next_scheduled_flush_delay(
    state: &LiveState,
    session_id: &str,
) -> Option<std::time::Duration> {
    let now = Local::now();
    let summary_interval_minutes = live_summary_interval_minutes();
    let guard = state.0.lock().ok()?;
    let session = guard.as_ref()?;
    if session.session_id != session_id {
        return None;
    }
    if session.pending_lines.is_empty() || session.pending_lines.len() < 3 {
        return Some(std::time::Duration::from_secs(
            LIVE_FLUSH_DRIVER_IDLE_SLEEP_SECS,
        ));
    }
    let interval_due_at =
        effective_batch_started_at(session) + ChronoDuration::minutes(summary_interval_minutes);
    let min_ai_due_at =
        session.started_at + ChronoDuration::seconds(MIN_AI_SUMMARIZATION_DURATION_SECS);
    let due_at = if interval_due_at > min_ai_due_at {
        interval_due_at
    } else {
        min_ai_due_at
    };
    let wait_ms = due_at.signed_duration_since(now).num_milliseconds();
    if wait_ms <= 0 {
        return Some(std::time::Duration::from_secs(0));
    }
    let wait = std::time::Duration::from_millis(wait_ms as u64);
    let max_wait = std::time::Duration::from_secs(LIVE_FLUSH_DRIVER_MAX_SLEEP_SECS);
    Some(if wait > max_wait { max_wait } else { wait })
}

async fn live_flush_summary_with_side_effects(
    app: &tauri::AppHandle,
    state: &LiveState,
    force: bool,
) -> Result<LiveSessionSnapshot, String> {
    let summary_count_before = {
        let guard = state
            .0
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        guard.as_ref().map(|s| s.summaries.len()).unwrap_or(0)
    };
    let snapshot = flush_session_summary(state, force).await?;
    auto_save_day_cache(state, true);

    // Whenever the AI flush actually produced a new summary chunk, also persist
    // the formal .md file. Cheap insurance: a crash before stop now leaves a
    // real markdown on disk, not just the hidden day_cache sidecar.
    if snapshot.summaries.len() > summary_count_before {
        let info = {
            let guard = state
                .0
                .lock()
                .map_err(|_| "Live state lock failed".to_string())?;
            guard.as_ref().map(|s| {
                (
                    s.course.clone(),
                    s.started_at,
                    s.transcript_lines.clone(),
                    s.summaries.clone(),
                )
            })
        };
        if let Some((course, started_at, transcript_lines, summaries)) = info {
            write_partial_markdown_file(&course, started_at, &transcript_lines, &summaries);
        }
    }

    emit_live_update(app, state);
    Ok(snapshot)
}

fn start_live_flush_driver(app: tauri::AppHandle, session_id: String) {
    tauri::async_runtime::spawn(async move {
        loop {
            let state = app.state::<LiveState>();
            let Some(wait) = live_next_scheduled_flush_delay(state.inner(), &session_id) else {
                break;
            };
            if !wait.is_zero() {
                let notify = state.inner().flush_notify();
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = notify.notified() => continue,
                }
            } else {
                tokio::time::sleep(std::time::Duration::from_secs(
                    LIVE_FLUSH_DRIVER_MIN_SLEEP_SECS,
                ))
                .await;
            }
            let state = app.state::<LiveState>();
            if !live_session_matches(state.inner(), &session_id) {
                break;
            }
            match live_flush_summary_with_side_effects(&app, state.inner(), false).await {
                Ok(snapshot) => {
                    if !snapshot.active {
                        break;
                    }
                }
                Err(err) => {
                    log::warn!("[Live] backend scheduled flush failed: {err}");
                    // The flush already cleared `flush_in_flight` on failure, but
                    // without pushing a fresh snapshot the UI stays stuck on
                    // "要約を生成中…". Emit the cleared state plus an error event so
                    // Live can surface the failure instead of hanging silently.
                    emit_live_update(&app, state.inner());
                    let _ = app.emit("live-summary-error", serde_json::json!({ "message": err }));
                    tokio::time::sleep(std::time::Duration::from_secs(
                        LIVE_FLUSH_DRIVER_IDLE_SLEEP_SECS,
                    ))
                    .await;
                }
            }
        }
    });
}

#[tauri::command]
pub fn live_get_session(state: tauri::State<'_, LiveState>) -> LiveSessionSnapshot {
    current_snapshot(&state)
}

/// Peek at the day cache for a course without starting a session.
/// Returns an inactive snapshot with the cached transcript/summaries, or empty if no cache.
#[tauri::command]
pub fn live_peek_day_cache(course: LiveCourseInfo) -> LiveSessionSnapshot {
    match load_day_cache(&course) {
        Some(cache) => LiveSessionSnapshot {
            active: false,
            course: Some(course),
            started_at: Some(cache.started_at),
            transcript_lines: Arc::new(cache.transcript_lines),
            pending_lines: Arc::new(Vec::new()),
            summaries: Arc::new(cache.summaries),
            next_summary_at_ms: None,
            summarizing: false,
        },
        None => empty_snapshot(),
    }
}

#[tauri::command]
pub fn live_start_session(
    app: tauri::AppHandle,
    state: tauri::State<'_, LiveState>,
    mut course: LiveCourseInfo,
) -> Result<LiveSessionSnapshot, String> {
    if !course.is_free_note && course.course_name.trim().is_empty() {
        return Err("講義名が空です".into());
    }
    if course.is_free_note {
        course.course_name = FREE_NOTE_FOLDER_NAME.to_string();
        course.course_code.clear();
        course.room.clear();
        course.teacher.clear();
        course.day = 0;
        course.period = 0;
        course.time_label.clear();
    } else {
        course.course_name = course.course_name.trim().to_string();
        course.course_code = course.course_code.trim().to_string();
        course.room = course.room.trim().to_string();
        course.teacher = course.teacher.trim().to_string();
        course.time_label = course.time_label.trim().to_string();
    }

    let now = Local::now();

    // Load accumulated data from earlier in the same course today
    let cached = load_day_cache(&course);
    let is_fresh_start = cached.is_none();
    let (prev_transcript, prev_summaries, original_start) = match cached {
        Some(cache) => (cache.transcript_lines, cache.summaries, cache.started_at),
        None => (Vec::new(), Vec::new(), format_datetime(now)),
    };
    let started_at = chrono::NaiveDateTime::parse_from_str(&original_start, "%Y-%m-%d %H:%M:%S")
        .map(|naive| naive.and_local_timezone(Local).unwrap())
        .unwrap_or(now);
    let batch_started_at = latest_summary_end_datetime(started_at, &prev_summaries)
        .or_else(|| {
            if prev_summaries.is_empty() {
                None
            } else {
                Some(last_transcript_line_datetime(
                    started_at,
                    &prev_transcript,
                    now,
                ))
            }
        })
        .unwrap_or(now);

    let persisted_line_count = prev_transcript.len();
    let session_id = uuid::Uuid::new_v4().to_string();
    let session = LiveSession {
        session_id: session_id.clone(),
        course,
        started_at,
        transcript_lines: Arc::new(prev_transcript),
        pending_lines: Arc::new(Vec::new()),
        summaries: Arc::new(prev_summaries),
        batch_started_at,
        flush_in_flight: false,
        is_fresh_start,
        persisted_line_count,
    };
    let snapshot = session.snapshot();
    let mut guard = state
        .0
        .lock()
        .map_err(|_| "Live state lock failed".to_string())?;
    *guard = Some(session);
    drop(guard);
    emit_live_update(&app, &state);
    start_live_flush_driver(app.clone(), session_id);
    Ok(snapshot)
}

#[tauri::command]
pub fn live_append_transcript(
    app: tauri::AppHandle,
    state: tauri::State<'_, LiveState>,
    text: String,
) -> Result<LiveSessionSnapshot, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(current_snapshot(&state));
    }
    let line = LiveTranscriptLine {
        text: text.to_string(),
        at: Local::now().format("%H:%M:%S").to_string(),
    };
    let snapshot = {
        let mut guard = state
            .0
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        let session = guard
            .as_mut()
            .ok_or_else(|| "Liveセッションが開始されていません".to_string())?;
        // make_mut is in-place when no other Arc holders exist; if a
        // previously-emitted snapshot is still being serialized it copies once
        // — bounded and rare. Either way, no per-append deep clone of the Vec.
        Arc::make_mut(&mut session.transcript_lines).push(line.clone());
        Arc::make_mut(&mut session.pending_lines).push(line.clone());
        session.snapshot()
    };
    auto_save_day_cache(&state, false);
    state.inner().notify_flush_driver();
    // Slim delta event for the subtitle overlay and any cheap subscriber.
    // Emitting the full snapshot per final line grew O(N) in payload size —
    // a 2-hour lecture was serialising hundreds of KB on every append.
    let _ = app.emit("live-line-appended", &line);
    Ok(snapshot)
}

#[tauri::command]
pub async fn live_flush_summary(
    app: tauri::AppHandle,
    state: tauri::State<'_, LiveState>,
    force: bool,
) -> Result<LiveSessionSnapshot, String> {
    live_flush_summary_with_side_effects(&app, state.inner(), force).await
}

#[tauri::command]
pub async fn live_generate_overall_summary(
    state: tauri::State<'_, LiveState>,
) -> Result<String, String> {
    // The overall summary is a quick "so far" snapshot: it reads the existing
    // summary chunks plus the tail of the live transcript (which already holds
    // every appended line, pending ones included). It must NOT force a chunk
    // flush first — that ran the full per-chunk pipeline (summary + terms +
    // whiteboard generation, several AI calls) and could block up to
    // LIVE_FLUSH_FORCE_WAIT_ATTEMPTS while a scheduled flush was in flight,
    // which is what made this button take abnormally long.
    let (course, started_at, transcript_lines, summaries) = {
        let guard = state
            .0
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        let session = guard
            .as_ref()
            .ok_or_else(|| "Liveセッションが開始されていません".to_string())?;
        if session.transcript_lines.is_empty() {
            return Err("全体要約を生成できる文字起こしがまだありません".to_string());
        }
        (
            session.course.clone(),
            session.started_at,
            session.transcript_lines.clone(),
            session.summaries.clone(),
        )
    };

    Ok(generate_overall_summary(
        &course,
        started_at,
        Local::now(),
        &summaries,
        &transcript_lines,
    )
    .await)
}

#[tauri::command]
pub fn live_cancel_session(
    app: tauri::AppHandle,
    state: tauri::State<'_, LiveState>,
) -> Result<(), String> {
    let mut guard = state
        .0
        .lock()
        .map_err(|_| "Live state lock failed".to_string())?;
    // Grab info we need to scrub on-disk artifacts before dropping the session.
    // The flush path may have written a partial .md (and recorded it in the
    // downloads history) — leaving those behind would contradict the UI's
    // "破棄" message. But only scrub when this session was a fresh start;
    // a resumed session shares its .md and day_cache with earlier completed
    // recordings today, and we must not destroy that prior content.
    let cleanup = guard.as_ref().map(|s| {
        (
            s.course.clone(),
            s.started_at,
            !s.transcript_lines.is_empty(),
            s.is_fresh_start,
        )
    });
    *guard = None;
    drop(guard);

    if let Some((course, started_at, had_transcript, is_fresh_start)) = cleanup {
        if is_fresh_start {
            if had_transcript {
                let partial_path =
                    live_storage_dir(&course).join(formal_markdown_filename(&course, started_at));
                if partial_path.exists() {
                    let _ = std::fs::remove_file(&partial_path);
                }
                crate::commands::remove_download_records_by_path(&partial_path.to_string_lossy());
            }
            if !course.is_free_note {
                remove_day_cache(&course);
            }
        }
    }

    state.inner().notify_flush_driver();
    emit_live_update(&app, &state);
    Ok(())
}

/// Clear the day cache for a specific course, removing all accumulated transcript/summary data.
#[tauri::command]
pub fn live_clear_day_cache(course: LiveCourseInfo) -> Result<(), String> {
    if course.is_free_note {
        return Ok(());
    }
    if course.course_name.trim().is_empty() {
        return Err("講義名が空です".into());
    }
    remove_day_cache(&course);
    Ok(())
}

#[tauri::command]
pub async fn live_finish_session(
    app: tauri::AppHandle,
    state: tauri::State<'_, LiveState>,
) -> Result<LiveSaveResult, String> {
    let (course, started_at, transcript_lines, summaries, pending_line_count) = {
        let guard = state
            .0
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        let Some(session) = guard.as_ref() else {
            return Err("Liveセッションが開始されていません".to_string());
        };
        if session.transcript_lines.is_empty() {
            let course = session.course.clone();
            drop(guard);
            if !course.is_free_note {
                remove_day_cache(&course);
            }
            let snapshot = {
                let mut guard = state
                    .0
                    .lock()
                    .map_err(|_| "Live state lock failed".to_string())?;
                let session = guard
                    .as_ref()
                    .ok_or_else(|| "Liveセッションが開始されていません".to_string())?;
                let snapshot = session.snapshot();
                *guard = None;
                snapshot
            };
            state.inner().notify_flush_driver();
            let result = LiveSaveResult {
                saved: false,
                path: String::new(),
                markdown: String::new(),
                snapshot,
                suggested_todos: Vec::new(),
                todos_pending: false,
            };
            emit_live_update(&app, &state);
            return Ok(result);
        }
        (
            session.course.clone(),
            session.started_at,
            session.transcript_lines.clone(),
            session.summaries.clone(),
            session.pending_lines.len(),
        )
    };

    let pre_ai_saved_at = Local::now();
    let pre_ai_markdown = build_markdown(
        &course,
        started_at,
        pre_ai_saved_at,
        PRE_AI_OVERALL_SUMMARY,
        &summaries,
        &transcript_lines,
    );
    write_formal_markdown_file(&course, started_at, &pre_ai_markdown)?;
    save_day_cache_full(&course, started_at, &transcript_lines, &summaries);
    emit_live_finish_progress(&app, "record_saved");

    let ai_config = crate::ai::load_ai_config();
    let will_run_finish_ai = should_run_finish_ai(&ai_config.provider, started_at, pre_ai_saved_at);
    let needs_final_chunk_ai =
        should_require_finish_chunk_ai(started_at, pre_ai_saved_at, pending_line_count);
    if needs_final_chunk_ai || will_run_finish_ai {
        emit_live_finish_progress(&app, "ai");
    }
    if needs_final_chunk_ai {
        // Retry the closing segment a few times before giving up. If it still
        // fails, degrade to saving the transcript plus the segments we already
        // have rather than failing the whole stop and losing everything — the
        // markdown build below works from whatever summaries exist.
        if let Err(err) = flush_final_summary_with_retry(&state).await {
            log::warn!(
                "[Live] final chunk summary gave up after retries: {err}; \
                 saving transcript without the last segment"
            );
        }
    } else {
        // Non-fatal for short sessions: they intentionally skip AI and save the transcript as-is.
        let _ = flush_session_summary(&state, true).await;
    }

    let (course, started_at, transcript_lines, summaries) = {
        let guard = state
            .0
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        let session = guard
            .as_ref()
            .ok_or_else(|| "Liveセッションが開始されていません".to_string())?;
        (
            session.course.clone(),
            session.started_at,
            session.transcript_lines.clone(),
            session.summaries.clone(),
        )
    };

    let ended_at = Local::now();
    let should_run_finish_ai = should_run_finish_ai(&ai_config.provider, started_at, ended_at);
    let overall_summary =
        generate_overall_summary(&course, started_at, ended_at, &summaries, &transcript_lines)
            .await;
    let markdown = build_markdown(
        &course,
        started_at,
        ended_at,
        &overall_summary,
        &summaries,
        &transcript_lines,
    );
    // TODO/DDL judgment is the slowest AI step. It must NOT block saving — we
    // run it in the background (after the file is written and the session is
    // cleared) and push the result through the `live-todo-suggestions` event so
    // the UI can jump straight to the TODO page and add them there.
    let run_todos_in_background = !course.is_free_note
        && !transcript_lines.is_empty()
        && !should_skip_ai_summarization(started_at, ended_at)
        && should_run_finish_ai;

    emit_live_finish_progress(&app, "final_save");
    let path = write_formal_markdown_file(&course, started_at, &markdown)?;

    // Save day cache so next session for same course today can resume
    save_day_cache_full(&course, started_at, &transcript_lines, &summaries);

    let path_str = path.to_string_lossy().to_string();

    let snapshot = {
        let mut guard = state
            .0
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        let session = guard
            .as_ref()
            .ok_or_else(|| "Liveセッションが開始されていません".to_string())?;
        let snapshot = session.snapshot();
        *guard = None;
        snapshot
    };
    state.inner().notify_flush_driver();

    // Kick off TODO/DDL judgment without blocking the return. Suggestions arrive
    // via `live-todo-suggestions`; the frontend moves to the TODO page meanwhile.
    if run_todos_in_background {
        let app_bg = app.clone();
        let course_bg = course.clone();
        let summaries_bg = summaries.clone();
        let transcript_bg = transcript_lines.clone();
        let source_path = path_str.clone();
        tauri::async_runtime::spawn(async move {
            let suggestions = extract_todo_suggestions(
                &app_bg,
                &course_bg,
                &summaries_bg,
                &transcript_bg,
                ended_at,
            )
            .await;
            let payload = LiveTodoSuggestionsEvent {
                suggestions,
                source_path,
            };
            let _ = app_bg.emit("live-todo-suggestions", &payload);
        });
    }

    let result = LiveSaveResult {
        saved: true,
        path: path_str.clone(),
        markdown,
        snapshot,
        suggested_todos: Vec::new(),
        todos_pending: run_todos_in_background,
    };
    let _ = app.emit("live-session-saved", &result);
    emit_live_update(&app, &state);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn skip_ai_summarization_for_sessions_under_two_minutes() {
        let now = Local::now();
        assert!(should_skip_ai_summarization(
            now - chrono::Duration::seconds(119),
            now
        ));
        assert!(!should_skip_ai_summarization(
            now - chrono::Duration::seconds(120),
            now
        ));
    }

    #[test]
    fn transcript_line_datetime_uses_session_date() {
        let started_at = Local
            .with_ymd_and_hms(2026, 5, 13, 10, 0, 0)
            .single()
            .unwrap();
        let line = LiveTranscriptLine {
            text: "topic".into(),
            at: "10:05:30".into(),
        };

        let parsed = transcript_line_datetime(started_at, &line).unwrap();

        assert_eq!(
            parsed.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-05-13 10:05:30"
        );
    }

    #[test]
    fn transcript_line_datetime_handles_midnight_rollover() {
        let started_at = Local
            .with_ymd_and_hms(2026, 5, 13, 23, 50, 0)
            .single()
            .unwrap();
        let line = LiveTranscriptLine {
            text: "after midnight".into(),
            at: "00:05:00".into(),
        };

        let parsed = transcript_line_datetime(started_at, &line).unwrap();

        assert_eq!(
            parsed.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-05-14 00:05:00"
        );
    }

    #[test]
    fn effective_batch_start_uses_first_pending_line_for_first_chunk() {
        let started_at = Local
            .with_ymd_and_hms(2026, 5, 13, 10, 0, 0)
            .single()
            .unwrap();
        let pending = vec![
            LiveTranscriptLine {
                text: "first".into(),
                at: "10:03:00".into(),
            },
            LiveTranscriptLine {
                text: "second".into(),
                at: "10:04:00".into(),
            },
        ];
        let session = LiveSession {
            session_id: "test".into(),
            course: LiveCourseInfo {
                course_name: "テスト".into(),
                course_code: String::new(),
                room: String::new(),
                teacher: String::new(),
                day: 1,
                period: 1,
                time_label: String::new(),
                is_free_note: false,
            },
            started_at,
            transcript_lines: Arc::new(pending.clone()),
            pending_lines: Arc::new(pending),
            summaries: Arc::new(Vec::new()),
            batch_started_at: started_at,
            flush_in_flight: false,
            is_fresh_start: true,
            persisted_line_count: 0,
        };

        let effective = effective_batch_started_at(&session);

        assert_eq!(
            effective.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-05-13 10:03:00"
        );
    }

    #[test]
    fn effective_batch_start_uses_latest_summary_end_after_resume() {
        let started_at = Local
            .with_ymd_and_hms(2026, 5, 13, 10, 0, 0)
            .single()
            .unwrap();
        let resumed_batch_started_at = latest_summary_end_datetime(
            started_at,
            &[LiveSummaryChunk {
                title: "Chunk 01 | 10:03-10:10".into(),
                range_label: "10:03-10:10".into(),
                body: "summary".into(),
                line_count: 3,
                terms: Vec::new(),
                whiteboard: None,
            }],
        )
        .unwrap();
        let session = LiveSession {
            session_id: "test".into(),
            course: LiveCourseInfo {
                course_name: "テスト".into(),
                course_code: String::new(),
                room: String::new(),
                teacher: String::new(),
                day: 1,
                period: 1,
                time_label: String::new(),
                is_free_note: false,
            },
            started_at,
            transcript_lines: Arc::new(vec![LiveTranscriptLine {
                text: "covered".into(),
                at: "10:09:30".into(),
            }]),
            pending_lines: Arc::new(vec![LiveTranscriptLine {
                text: "new".into(),
                at: "10:20:00".into(),
            }]),
            summaries: Arc::new(vec![LiveSummaryChunk {
                title: "Chunk 01 | 10:03-10:10".into(),
                range_label: "10:03-10:10".into(),
                body: "summary".into(),
                line_count: 3,
                terms: Vec::new(),
                whiteboard: None,
            }]),
            batch_started_at: resumed_batch_started_at,
            flush_in_flight: false,
            is_fresh_start: false,
            persisted_line_count: 1,
        };

        let effective = effective_batch_started_at(&session);

        assert_eq!(
            effective.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-05-13 10:10:00"
        );
    }

    #[test]
    fn effective_batch_start_keeps_second_precision_after_current_flush() {
        let started_at = Local
            .with_ymd_and_hms(2026, 5, 13, 10, 0, 0)
            .single()
            .unwrap();
        let last_subtitle_at = Local
            .with_ymd_and_hms(2026, 5, 13, 10, 10, 45)
            .single()
            .unwrap();
        let session = LiveSession {
            session_id: "test".into(),
            course: LiveCourseInfo {
                course_name: "テスト".into(),
                course_code: String::new(),
                room: String::new(),
                teacher: String::new(),
                day: 1,
                period: 1,
                time_label: String::new(),
                is_free_note: false,
            },
            started_at,
            transcript_lines: Arc::new(Vec::new()),
            pending_lines: Arc::new(vec![LiveTranscriptLine {
                text: "new".into(),
                at: "10:20:00".into(),
            }]),
            summaries: Arc::new(vec![LiveSummaryChunk {
                title: "Chunk 01 | 10:03-10:10".into(),
                range_label: "10:03-10:10".into(),
                body: "summary".into(),
                line_count: 3,
                terms: Vec::new(),
                whiteboard: None,
            }]),
            batch_started_at: last_subtitle_at,
            flush_in_flight: false,
            is_fresh_start: true,
            persisted_line_count: 0,
        };

        let effective = effective_batch_started_at(&session);

        assert_eq!(
            effective.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-05-13 10:10:45"
        );
    }

    #[test]
    fn finish_ai_runs_only_for_non_local_provider_after_minimum_duration() {
        let now = Local::now();
        let long_session = now - chrono::Duration::seconds(120);
        let short_session = now - chrono::Duration::seconds(119);

        assert!(!should_run_finish_ai("local", long_session, now));
        assert!(should_run_finish_ai("openai", long_session, now));
        assert!(!should_run_finish_ai("openai", short_session, now));
    }

    #[test]
    fn finish_requires_pending_chunk_ai_after_minimum_duration() {
        let now = Local::now();
        let long_session = now - chrono::Duration::seconds(120);
        let short_session = now - chrono::Duration::seconds(119);

        assert!(should_require_finish_chunk_ai(long_session, now, 3));
        assert!(!should_require_finish_chunk_ai(long_session, now, 0));
        assert!(!should_require_finish_chunk_ai(short_session, now, 3));
    }

    #[test]
    fn parse_chunk_ai_result_extracts_terms() {
        let raw = r#"{
          "summary_markdown": "- MVC\n\n---\n\n**MVC**: 画面と処理を分ける考え方。",
	          "terms": [
	            {
	              "term": "MVC",
	              "explanation": "Model、View、Controllerに責務を分ける設計パターン。画面変更とデータ処理の責任範囲を見直す観点になる。",
	              "source_excerpt": "MVCという設計",
	              "external_source": "MDN Web Docs: MVC architecture"
	            }
	          ],
	          "whiteboard": {
	            "title": "MVCの責務分離",
	            "layout": "flow",
	            "nodes": [
	              { "id": "model", "label": "Model", "detail": "データ", "kind": "core" },
	              { "id": "view", "label": "View", "detail": "表示", "kind": "support" },
	              { "id": "controller", "label": "Controller", "detail": "制御", "kind": "result" },
	              { "id": "observer", "label": "Observer", "detail": "変更通知の関連パターン", "kind": "support", "source_type": "external", "external_source": "Gamma et al., Design Patterns" }
	            ],
	            "edges": [
	              { "from": "model", "to": "view", "label": "反映" },
	              { "from": "view", "to": "missing", "label": "無効" }
	            ]
	          }
	        }"#;
        let parsed = parse_chunk_ai_result(raw);
        assert!(parsed.body.contains("MVC"));
        assert_eq!(parsed.terms.len(), 1);
        assert_eq!(parsed.terms[0].term, "MVC");
        assert!(parsed.terms[0].external_source.contains("MDN"));
        let board = parsed.whiteboard.expect("whiteboard should parse");
        assert_eq!(board.title, "MVCの責務分離");
        assert_eq!(board.layout, "flow");
        assert_eq!(board.nodes.len(), 4);
        assert_eq!(board.nodes[0].kind, "core");
        assert_eq!(board.nodes[0].role, "main");
        assert_eq!(board.nodes[3].source_type, "external");
        assert!(board.nodes[3].external_source.contains("Design Patterns"));
        assert_eq!(board.edges.len(), 1);
    }

    #[test]
    fn parse_chunk_ai_result_filters_low_value_terms() {
        let raw = r#"{
          "summary_markdown": "- 重点\n\n---\n\n**重点**: 説明",
          "terms": [
            {
              "term": "授業",
              "explanation": "大学で行われる講義のこと。",
              "source_excerpt": "今日の授業"
            },
            {
              "term": "認知的不協和",
              "explanation": "矛盾する認知を同時に持つことで生じる不快感。講義では態度変容の説明に使われる。",
              "source_excerpt": "認知的不協和が起きる"
            }
          ]
        }"#;
        let parsed = parse_chunk_ai_result(raw);
        assert_eq!(parsed.terms.len(), 1);
        assert_eq!(parsed.terms[0].term, "認知的不協和");
    }

    #[test]
    fn parse_chunk_ai_result_falls_back_to_markdown() {
        let parsed = parse_chunk_ai_result("- 重点\n\n---\n\n**重点**: 説明");
        assert!(parsed.body.starts_with("- 重点"));
        assert!(parsed.terms.is_empty());
        assert!(parsed.whiteboard.is_none());
    }

    #[test]
    fn latest_whiteboard_context_uses_most_recent_cumulative_board() {
        let summaries = vec![
            LiveSummaryChunk {
                title: "前半".to_string(),
                range_label: "10:00-10:05".to_string(),
                body: "古い内容".to_string(),
                line_count: 3,
                terms: Vec::new(),
                whiteboard: Some(LiveWhiteboard {
                    title: "古いボード".to_string(),
                    layout: "grid".to_string(),
                    nodes: vec![
                        LiveWhiteboardNode {
                            id: "old".to_string(),
                            label: "旧概念".to_string(),
                            detail: String::new(),
                            node_type: "structure".to_string(),
                            kind: "core".to_string(),
                            role: "main".to_string(),
                            parent_id: String::new(),
                            source_type: "lecture".to_string(),
                            source_excerpt: String::new(),
                            external_source: String::new(),
                        },
                        LiveWhiteboardNode {
                            id: "old-2".to_string(),
                            label: "旧補足".to_string(),
                            detail: String::new(),
                            node_type: "structure".to_string(),
                            kind: "support".to_string(),
                            role: "branch".to_string(),
                            parent_id: "old".to_string(),
                            source_type: "lecture".to_string(),
                            source_excerpt: String::new(),
                            external_source: String::new(),
                        },
                    ],
                    edges: Vec::new(),
                    schema_version: 0,
                    normalized_by: String::new(),
                }),
            },
            LiveSummaryChunk {
                title: "後半".to_string(),
                range_label: "10:05-10:10".to_string(),
                body: "新しい内容".to_string(),
                line_count: 4,
                terms: Vec::new(),
                whiteboard: Some(LiveWhiteboard {
                    title: "更新後ボード".to_string(),
                    layout: "flow".to_string(),
                    nodes: vec![
                        LiveWhiteboardNode {
                            id: "old".to_string(),
                            label: "旧概念".to_string(),
                            detail: String::new(),
                            node_type: "structure".to_string(),
                            kind: "core".to_string(),
                            role: "main".to_string(),
                            parent_id: String::new(),
                            source_type: "lecture".to_string(),
                            source_excerpt: String::new(),
                            external_source: String::new(),
                        },
                        LiveWhiteboardNode {
                            id: "new".to_string(),
                            label: "新概念".to_string(),
                            detail: "追加".to_string(),
                            node_type: "structure".to_string(),
                            kind: "result".to_string(),
                            role: "branch".to_string(),
                            parent_id: "old".to_string(),
                            source_type: "lecture".to_string(),
                            source_excerpt: String::new(),
                            external_source: String::new(),
                        },
                    ],
                    edges: vec![LiveWhiteboardEdge {
                        from: "old".to_string(),
                        to: "new".to_string(),
                        label: "発展".to_string(),
                    }],
                    schema_version: 0,
                    normalized_by: String::new(),
                }),
            },
        ];

        let context = format_latest_whiteboard_context(&summaries);
        assert!(context.contains("更新後ボード"));
        assert!(context.contains("新概念"));
        assert!(!context.contains("古いボード"));
    }

    fn test_whiteboard_node(
        id: &str,
        label: &str,
        node_type: &str,
        source_type: &str,
        source_excerpt: &str,
    ) -> LiveWhiteboardNode {
        LiveWhiteboardNode {
            id: id.to_string(),
            label: label.to_string(),
            detail: String::new(),
            node_type: node_type.to_string(),
            kind: if node_type == "term" {
                "support".to_string()
            } else {
                "core".to_string()
            },
            role: if node_type == "term" {
                "branch".to_string()
            } else {
                "main".to_string()
            },
            parent_id: String::new(),
            source_type: source_type.to_string(),
            source_excerpt: source_excerpt.to_string(),
            external_source: String::new(),
        }
    }

    fn test_whiteboard(nodes: Vec<LiveWhiteboardNode>) -> LiveWhiteboard {
        LiveWhiteboard {
            title: "テスト白板".to_string(),
            layout: "grid".to_string(),
            nodes,
            edges: Vec::new(),
            schema_version: 1,
            normalized_by: "backend".to_string(),
        }
    }

    #[test]
    fn enrich_whiteboard_source_excerpts_uses_previous_terms_and_transcript() {
        let previous = test_whiteboard(vec![test_whiteboard_node(
            "old",
            "既存ノード",
            "structure",
            "lecture",
            "既存の根拠",
        )]);
        let mut term_node = test_whiteboard_node("term-source", "一次資料", "term", "lecture", "");
        term_node.parent_id = "old".to_string();
        let mut external_node =
            test_whiteboard_node("external", "外部補足", "structure", "external", "");
        external_node.external_source = "外部資料".to_string();
        let board = test_whiteboard(vec![
            test_whiteboard_node("old", "既存ノード", "structure", "lecture", ""),
            term_node,
            test_whiteboard_node("theme", "個人発表のテーマ選定", "structure", "lecture", ""),
            external_node,
        ]);
        let terms = vec![LiveTermExplanation {
            term: "一次資料".to_string(),
            explanation: "大元の資料".to_string(),
            source_excerpt: "一次資料まで遡る必要があります".to_string(),
            external_source: String::new(),
        }];
        let lines = vec![LiveTranscriptLine {
            at: "12:00:00".to_string(),
            text: "個人発表のテーマ選定では、賛否が分かれる問いを選んでください。".to_string(),
        }];

        let enriched =
            enrich_whiteboard_source_excerpts(Some(board), Some(&previous), &terms, &lines)
                .expect("whiteboard should remain available");
        let source_by_id = enriched
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node.source_excerpt.as_str()))
            .collect::<std::collections::HashMap<_, _>>();

        assert_eq!(source_by_id.get("old"), Some(&"既存の根拠"));
        assert_eq!(
            source_by_id.get("term-source"),
            Some(&"一次資料まで遡る必要があります")
        );
        assert_eq!(
            source_by_id.get("theme"),
            Some(&"個人発表のテーマ選定では、賛否が分かれる問いを選んでください。")
        );
        assert_eq!(source_by_id.get("external"), Some(&""));
    }

    #[test]
    fn live_prompts_keep_language_policy_consistent() {
        let language_hint = live_reply_language_hint("zh");
        assert!(language_hint.contains("简体中文"));

        let chunk_prompt = live_chunk_system_prompt(language_hint, false);
        // Call 1 prompt produces summary + terms only; it must NOT mention the
        // whiteboard schema (that lives in the standalone whiteboard prompt).
        assert!(chunk_prompt.contains("\"summary_markdown\""));
        assert!(chunk_prompt.contains("\"terms\""));
        assert!(!chunk_prompt.contains("\"whiteboard\""));
        assert!(!chunk_prompt.contains("\"role\":\"main|branch\""));
        assert!(!chunk_prompt.contains("\"node_type\":\"structure|term\""));

        let board_prompt =
            live_whiteboard_system_prompt(live_whiteboard_language_instruction("zh"), false);
        // Call 2 prompt owns the whiteboard JSON schema now.
        assert!(
            board_prompt.chars().count() < 16_000,
            "whiteboard prompt grew too large: {} chars",
            board_prompt.chars().count()
        );
        assert!(board_prompt.contains("\"role\":\"main|branch\""));
        assert!(board_prompt.contains("\"node_type\":\"structure|term\""));
        assert!(board_prompt.contains("\"source_type\":\"lecture|external\""));
        assert!(board_prompt.contains("whiteboard JSON だけ"));
        assert!(board_prompt.contains("目的"));
        assert!(board_prompt.contains("ノードや edge の量は固定目標ではなく"));
        assert!(board_prompt.contains("実行順序"));
        assert!(board_prompt.contains("内容本体・活動内容・方法論・運営連絡"));
        assert!(board_prompt.contains("パターン名自体を node にしない"));
        assert!(board_prompt.contains("内容構造の再編"));
        assert!(board_prompt.contains("時系列メモにしない"));
        assert!(board_prompt.contains("分割要約の区間や発話順をそのまま main にしない"));
        assert!(board_prompt.contains("基礎概念 → 中核メカニズム/歴史的展開 → 現代的課題"));
        assert!(board_prompt.contains("背景/問題 → 解決策 → 実装/応用 → 帰結/限界"));
        assert!(board_prompt.contains("同一の発展脈絡"));
        assert!(board_prompt.contains("branch 間 edge"));
        assert!(board_prompt.contains("内容タイプ別の骨格選択（構造パターン庫）"));
        for marker in [
            "討論・ディベート・賛否検討",
            "論題 main -> 肯定側/否定側",
            "理由ノードを方法論 main 配下に置かない",
            "比較・対照",
            "ケース分析・事例紹介",
            "問題解決・政策/制度検討",
            "因果メカニズム・理論説明",
            "分類・体系整理",
            "手順・プロセス・歴史展開",
            "資料・文献・テキスト読解",
            "データ・統計・図表解釈",
            "Q&A・相談・個別指導",
            "発表・作品・提出物への講評",
            "研究指導・レポート相談",
            "実習・演習・ワークショップ",
            "語学・表現練習",
            "意思決定・計画立案",
            "ブレインストーミング・アイデア整理",
            "物語・出来事の整理",
            "連絡・運営・予定調整",
            "雑談・反応・メタコメント",
        ] {
            assert!(board_prompt.contains(marker), "missing marker: {marker}");
        }
        assert!(board_prompt.contains("混在タイプの分離"));
        assert!(board_prompt.contains("正文内容"));
        assert!(board_prompt.contains("何について学んでいるか"));
        assert!(board_prompt.contains("どう扱っているか"));
        assert!(board_prompt.contains("正文 main"));
        assert!(board_prompt.contains("活動 main"));
        assert!(board_prompt.contains("判断手順"));
        assert!(board_prompt.contains("既存ノードの更新で十分な材料"));
        assert!(board_prompt.contains("既存ノードでは表せない新材料だけ"));
        assert!(board_prompt.contains("parent_id が意味上の所属先"));
        assert!(board_prompt.contains("内容本体の理由・根拠・事例"));
        assert!(board_prompt.contains("ノード総数に上限はない"));
        assert!(board_prompt.contains("既存情報は原則すべて引き継ぐ"));
        assert!(board_prompt.contains("nodes 配列の長さは、直前ボードの長さ「以上」を基本"));
        assert!(board_prompt.contains("重複・STT 誤認識"));
        assert!(board_prompt.contains("既存ノードを更新しない"));
        assert!(board_prompt.contains("アップグレードしてよい"));
        assert!(board_prompt.contains("古い id 固執より"));
        assert!(board_prompt.contains("前区間の内容"));
        assert!(board_prompt.contains("話題境界"));
        assert!(board_prompt.contains("別素材/会話/教材/議題"));
        assert!(board_prompt.contains("既存話題の続きなら"));
        assert!(board_prompt.contains("散らばった点"));
        assert!(board_prompt.contains("合理的な上位 main を合成"));
        assert!(board_prompt
            .contains("新しい語・固有名詞・出来事・数値・属性だけを理由に main を増やさない"));
        assert!(board_prompt.contains("branch が別 branch の下位要素に見える場合"));
        assert!(board_prompt.contains("中心放射に見せるためだけに hub を選ばない"));
        assert!(board_prompt.contains("発話順より概念上の前提→展開→帰結を優先する"));
        assert!(board_prompt.contains("構造ノード"));
        assert!(board_prompt.contains("node_type=\"term\""));
        assert!(board_prompt.contains("用語ノード"));
        assert!(board_prompt.contains("source_excerpt に根拠となる短い発話断片"));
        assert!(board_prompt.contains("source_excerpt をまとめて空にしない"));
        assert!(board_prompt.contains("人物名・地名・組織名・道具名"));
        assert!(board_prompt.contains("parent_id=\"\""));
        assert!(board_prompt.contains("用語ノードの edge は親構造ノードとだけ"));
        assert!(board_prompt.contains("外すと構造理解が悪くなる関係だけ"));
        assert!(board_prompt.contains("最終セルフチェック"));
        assert!(board_prompt.contains("混在内容では"));
        assert!(board_prompt.contains("branch 同士の下位関係は edge.label"));
        assert!(board_prompt.contains("result 同士"));
        assert!(board_prompt.contains("非空 edge.label 也必须使用简体中文"));
        assert!(!board_prompt.contains("ゲーム"));
        assert!(!board_prompt.contains("動画"));
        assert!(!board_prompt.contains("実況"));

        let overall_prompt = live_overall_system_prompt("zh", language_hint, false);
        assert!(overall_prompt.contains("### 整体总结"));
        assert!(overall_prompt.contains("### 本次论点"));

        let todo_prompt = live_todo_system_prompt("zh");
        assert!(todo_prompt.contains("title、note、source_excerpt 使用简体中文"));
        assert!(todo_prompt.contains("content_type\":\"課題|レポート|予習|復習|テスト準備|その他"));
    }

    #[test]
    fn free_note_prompts_do_not_dismiss_non_lecture_content() {
        let language_hint = live_reply_language_hint("zh");
        let chunk_prompt = live_chunk_system_prompt(language_hint, true);
        assert!(chunk_prompt.contains("自由ノートは講義とは限りません"));
        assert!(chunk_prompt.contains("非学術的という理由だけで「整理対象外」にしない"));
        assert!(chunk_prompt.contains("人物関係"));
        assert!(chunk_prompt.contains("一度だけ出た固有名詞"));
        assert!(chunk_prompt.contains("録音内だけで十分理解できる名前や固有設定"));
        assert!(!chunk_prompt.contains("ゲーム"));
        assert!(!chunk_prompt.contains("動画"));
        assert!(!chunk_prompt.contains("実況"));

        let board_prompt =
            live_whiteboard_system_prompt(live_whiteboard_language_instruction("zh"), true);
        assert!(board_prompt.contains("録音内容"));
        assert!(board_prompt.contains("録音開始から現在まで"));
        assert!(board_prompt.contains("基礎概念 → 中核メカニズム/歴史的展開 → 現代的課題"));
        assert!(board_prompt.contains("録音の主要話題・場面・観点"));
        assert!(board_prompt.contains("整理対象外にしない"));
        assert!(board_prompt.contains("source_type=\"lecture\" は互換性のための列挙値"));
        assert!(board_prompt.contains("散らばった点"));
        assert!(board_prompt.contains("反応"));
        assert!(!board_prompt.contains("講義開始"));
        assert!(!board_prompt.contains("講義の流れ"));
        assert!(!board_prompt.contains("講義・録音全体"));
        assert!(!board_prompt.contains("講義の主要課題・章・観点"));

        let overall_prompt = live_overall_system_prompt("zh", language_hint, true);
        assert!(overall_prompt.contains("自由ノート録音"));
        assert!(overall_prompt.contains("非学術的という理由だけで除外しない"));
    }

    fn fixture_cache(transcript: Vec<(&str, &str)>) -> LiveDayCache {
        LiveDayCache {
            date: "2026-05-13".to_string(),
            course_name: "テスト".to_string(),
            started_at: "2026-05-13 10:00:00".to_string(),
            transcript_lines: transcript
                .into_iter()
                .map(|(text, at)| LiveTranscriptLine {
                    text: text.to_string(),
                    at: at.to_string(),
                })
                .collect(),
            summaries: Vec::new(),
        }
    }

    fn delta_line(i: usize, text: &str, at: &str) -> String {
        serde_json::to_string(&LiveLineDeltaRef { i, t: text, a: at }).unwrap()
    }

    #[test]
    fn replay_appends_new_deltas_in_order() {
        let mut cache = fixture_cache(vec![("hello", "10:00:01")]);
        let deltas = format!(
            "{}\n{}\n",
            delta_line(1, "world", "10:00:02"),
            delta_line(2, "again", "10:00:03"),
        );
        replay_deltas_into(&mut cache, &deltas);
        assert_eq!(cache.transcript_lines.len(), 3);
        assert_eq!(cache.transcript_lines[1].text, "world");
        assert_eq!(cache.transcript_lines[2].at, "10:00:03");
    }

    #[test]
    fn replay_skips_stale_entries_already_in_snapshot() {
        // Snapshot already has 2 lines (e.g. last flush wrote both into cache.json),
        // but deltas still contains those entries because the truncation didn't run.
        let mut cache = fixture_cache(vec![("a", "10:00:01"), ("b", "10:00:02")]);
        let deltas = format!(
            "{}\n{}\n{}\n",
            delta_line(0, "a", "10:00:01"), // stale
            delta_line(1, "b", "10:00:02"), // stale
            delta_line(2, "c", "10:00:03"), // new
        );
        replay_deltas_into(&mut cache, &deltas);
        assert_eq!(cache.transcript_lines.len(), 3);
        assert_eq!(cache.transcript_lines[2].text, "c");
    }

    #[test]
    fn replay_stops_on_gap_to_avoid_reorder() {
        let mut cache = fixture_cache(vec![("a", "10:00:01")]);
        // Missing index 1; should stop before applying index 2.
        let deltas = format!(
            "{}\n{}\n",
            delta_line(2, "c", "10:00:03"),
            delta_line(3, "d", "10:00:04"),
        );
        replay_deltas_into(&mut cache, &deltas);
        assert_eq!(cache.transcript_lines.len(), 1);
    }

    #[test]
    fn replay_tolerates_blank_and_corrupt_lines() {
        let mut cache = fixture_cache(vec![("a", "10:00:01")]);
        let deltas = format!(
            "\n{}\nnot-json\n{}\n",
            delta_line(1, "b", "10:00:02"),
            delta_line(2, "c", "10:00:03"),
        );
        replay_deltas_into(&mut cache, &deltas);
        // The "not-json" between two valid entries is skipped (`continue`), and
        // replay keeps going — `b` at index 1 lands, then `c` at index 2 lands.
        assert_eq!(cache.transcript_lines.len(), 3);
        assert_eq!(cache.transcript_lines[2].text, "c");
    }

    #[test]
    fn replay_noop_on_empty_deltas() {
        let mut cache = fixture_cache(vec![("a", "10:00:01")]);
        replay_deltas_into(&mut cache, "");
        assert_eq!(cache.transcript_lines.len(), 1);
    }

    #[test]
    fn delta_roundtrips_preserve_escapes() {
        // Newlines / quotes in transcript text must survive NDJSON encoding so a
        // single delta entry stays on one line.
        let line = LiveTranscriptLine {
            text: "first\nsecond \"quoted\"".to_string(),
            at: "10:00:01".to_string(),
        };
        let serialized = serde_json::to_string(&LiveLineDeltaRef {
            i: 0,
            t: &line.text,
            a: &line.at,
        })
        .unwrap();
        // Must not contain a raw newline; deltas file splits by '\n'.
        assert!(!serialized.contains('\n'));
        // Roundtrip
        let parsed: LiveLineDeltaOwned = serde_json::from_str(&serialized).unwrap();
        assert_eq!(parsed.t, line.text);
        assert_eq!(parsed.a, line.at);
    }

    #[test]
    fn formal_filename_anchors_to_started_at_date() {
        // started_at on 2026-05-12 23:50; "now" doesn't matter — filename uses
        // the start date so partial mid-session and final on the next calendar
        // day land on the same path.
        let course = LiveCourseInfo {
            course_name: "高等数学".into(),
            course_code: "M101".into(),
            room: "".into(),
            teacher: "".into(),
            day: 1,
            period: 1,
            time_label: "".into(),
            is_free_note: false,
        };
        let dt = Local
            .with_ymd_and_hms(2026, 5, 12, 23, 50, 0)
            .single()
            .unwrap();
        let name = formal_markdown_filename(&course, dt);
        assert!(name.starts_with("20260512_"));
        assert!(name.ends_with("_live.md"));
    }

    #[test]
    fn free_note_formal_filename_uses_started_at_time() {
        let course = LiveCourseInfo {
            course_name: FREE_NOTE_FOLDER_NAME.into(),
            course_code: "".into(),
            room: "".into(),
            teacher: "".into(),
            day: 0,
            period: 0,
            time_label: "".into(),
            is_free_note: true,
        };
        let dt = Local
            .with_ymd_and_hms(2026, 5, 13, 14, 30, 45)
            .single()
            .unwrap();
        let name = formal_markdown_filename(&course, dt);
        assert_eq!(name, "20260513_143045_live.md");
    }

    #[test]
    fn snapshot_serialization_does_not_clone_vec() {
        // The serialized JSON must round-trip back into a LiveDayCache with the
        // original transcript_lines/summaries. This ensures LiveDayCacheRef
        // (the borrow-only serializer) is wire-compatible with LiveDayCache
        // (the owned deserializer).
        let lines = vec![
            LiveTranscriptLine {
                text: "one".into(),
                at: "10:00:01".into(),
            },
            LiveTranscriptLine {
                text: "two".into(),
                at: "10:00:02".into(),
            },
        ];
        let summaries: Vec<LiveSummaryChunk> = vec![];
        let cache_ref = LiveDayCacheRef {
            date: "2026-05-13".into(),
            course_name: "テスト",
            started_at: "2026-05-13 10:00:00".into(),
            transcript_lines: &lines,
            summaries: &summaries,
        };
        let json = serde_json::to_string(&cache_ref).unwrap();
        let parsed: LiveDayCache = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.transcript_lines.len(), 2);
        assert_eq!(parsed.transcript_lines[1].text, "two");
        assert_eq!(parsed.course_name, "テスト");
    }
}
