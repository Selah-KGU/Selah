use chrono::{DateTime, Datelike, Duration as ChronoDuration, Local};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

const CACHE_DEBOUNCE: Duration = Duration::from_secs(30);
const MIN_AI_SUMMARIZATION_DURATION_SECS: i64 = 120;
const MAX_LIVE_TERM_EXPLANATION_CHARS: usize = 220;
static LAST_CACHE_WRITE: AtomicU64 = AtomicU64::new(0);
const FREE_NOTE_FOLDER_NAME: &str = "自由ノート";

fn instant_now_ms() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let origin = *START.get_or_init(Instant::now);
    Instant::now().saturating_duration_since(origin).as_millis() as u64
}

pub struct LiveState(Mutex<Option<LiveSession>>);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveCourseInfo {
    pub course_name: String,
    #[serde(default)]
    pub course_code: String,
    #[serde(default)]
    pub room: String,
    #[serde(default)]
    pub teacher: String,
    pub day: i32,
    pub period: i32,
    #[serde(default)]
    pub time_label: String,
    #[serde(default)]
    pub is_free_note: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveTranscriptLine {
    pub text: String,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveTermExplanation {
    pub term: String,
    pub explanation: String,
    #[serde(default)]
    pub source_excerpt: String,
    #[serde(default)]
    pub external_source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSummaryChunk {
    pub title: String,
    pub range_label: String,
    pub body: String,
    pub line_count: usize,
    #[serde(default)]
    pub terms: Vec<LiveTermExplanation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSessionSnapshot {
    pub active: bool,
    pub course: Option<LiveCourseInfo>,
    pub started_at: Option<String>,
    // Wrapped in Arc so building a snapshot is a refcount bump rather than
    // a deep clone of three potentially large Vec<...>. The wire format is
    // unchanged because serde serializes Arc<T> transparently as T.
    pub transcript_lines: Arc<Vec<LiveTranscriptLine>>,
    pub pending_lines: Arc<Vec<LiveTranscriptLine>>,
    pub summaries: Arc<Vec<LiveSummaryChunk>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSaveResult {
    pub saved: bool,
    pub path: String,
    pub markdown: String,
    pub snapshot: LiveSessionSnapshot,
    #[serde(default)]
    pub suggested_todos: Vec<LiveTodoSuggestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveTodoSuggestion {
    pub title: String,
    pub course_name: String,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub deadline: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub source_excerpt: String,
    pub day: i32,
    pub period: i32,
}

#[derive(Debug, Clone)]
struct LiveSession {
    session_id: String,
    course: LiveCourseInfo,
    started_at: DateTime<Local>,
    transcript_lines: Arc<Vec<LiveTranscriptLine>>,
    pending_lines: Arc<Vec<LiveTranscriptLine>>,
    summaries: Arc<Vec<LiveSummaryChunk>>,
    batch_started_at: DateTime<Local>,
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

struct LiveChunkAiResult {
    body: String,
    terms: Vec<LiveTermExplanation>,
}

impl LiveSession {
    fn snapshot(&self) -> LiveSessionSnapshot {
        // All three Vec<...> are Arc-wrapped, so cloning is a refcount bump.
        LiveSessionSnapshot {
            active: true,
            course: Some(self.course.clone()),
            started_at: Some(format_datetime(self.started_at)),
            transcript_lines: Arc::clone(&self.transcript_lines),
            pending_lines: Arc::clone(&self.pending_lines),
            summaries: Arc::clone(&self.summaries),
        }
    }
}

impl LiveState {
    pub fn new() -> Self {
        Self(Mutex::new(None))
    }
}

/// Sidecar JSON that persists accumulated session data across stop/start within the same course day.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LiveDayCache {
    date: String, // YYYY-MM-DD
    course_name: String,
    started_at: String,
    transcript_lines: Vec<LiveTranscriptLine>,
    summaries: Vec<LiveSummaryChunk>,
}

fn live_storage_dir(course: &LiveCourseInfo) -> std::path::PathBuf {
    if course.is_free_note {
        let dir = crate::commands::resolve_download_dir(None).join(FREE_NOTE_FOLDER_NAME);
        let _ = std::fs::create_dir_all(&dir);
        dir
    } else {
        crate::commands::resolve_download_dir(Some(&course.course_name))
    }
}

/// Single transcript line appended to the deltas log. Field names are short
/// (`i`/`t`/`a`) because we write one of these per spoken line — saves bytes
/// over a session.
#[derive(Debug, Serialize)]
struct LiveLineDeltaRef<'a> {
    i: usize,
    t: &'a str,
    a: &'a str,
}

#[derive(Debug, Deserialize)]
struct LiveLineDeltaOwned {
    i: usize,
    t: String,
    a: String,
}

/// Borrowing view of `LiveDayCache` used only for serialization, so we don't
/// have to deep-clone the transcript Vec every rewrite.
#[derive(Debug, Serialize)]
struct LiveDayCacheRef<'a> {
    date: String,
    course_name: &'a str,
    started_at: String,
    transcript_lines: &'a [LiveTranscriptLine],
    summaries: &'a [LiveSummaryChunk],
}

fn day_cache_path(course: &LiveCourseInfo) -> Option<std::path::PathBuf> {
    if course.is_free_note {
        return None;
    }
    let dir = live_storage_dir(course);
    let date_str = Local::now().format("%Y%m%d").to_string();
    let safe_name = sanitize_filename_component(&course.course_name);
    Some(dir.join(format!(".{}_{}_live.cache.json", date_str, safe_name)))
}

/// Append-only NDJSON log of transcript lines not yet folded into the main
/// snapshot. Lets us avoid rewriting the full cache every 30s.
fn day_cache_deltas_path(course: &LiveCourseInfo) -> Option<std::path::PathBuf> {
    if course.is_free_note {
        return None;
    }
    let dir = live_storage_dir(course);
    let date_str = Local::now().format("%Y%m%d").to_string();
    let safe_name = sanitize_filename_component(&course.course_name);
    Some(dir.join(format!(".{}_{}_live.lines.ndjson", date_str, safe_name)))
}

fn load_day_cache(course: &LiveCourseInfo) -> Option<LiveDayCache> {
    let path = day_cache_path(course)?;
    let data = std::fs::read_to_string(&path).ok()?;
    let mut cache: LiveDayCache = serde_json::from_str(&data).ok()?;
    let today = Local::now().format("%Y-%m-%d").to_string();
    if cache.date != today || cache.course_name != course.course_name {
        // stale cache from a different day; nuke both sides.
        let _ = std::fs::remove_file(&path);
        if let Some(d) = day_cache_deltas_path(course) {
            let _ = std::fs::remove_file(d);
        }
        return None;
    }
    // Replay any deltas not yet folded into the snapshot. A crash between cache
    // rewrite and deltas truncation can leave stale entries with `i` less than
    // the snapshot's line count — we filter those out.
    if let Some(deltas_path) = day_cache_deltas_path(course) {
        if let Ok(deltas_data) = std::fs::read_to_string(&deltas_path) {
            replay_deltas_into(&mut cache, &deltas_data);
        }
    }
    Some(cache)
}

/// Append entries from a deltas NDJSON blob into `cache.transcript_lines`.
/// - Entries with `i < cache.transcript_lines.len()` are skipped as stale
///   (already in the snapshot, e.g. after a crash between snapshot rewrite
///   and deltas truncation).
/// - A gap (`i > expected`) stops the replay so out-of-order entries can't
///   silently reorder transcripts.
fn replay_deltas_into(cache: &mut LiveDayCache, deltas_text: &str) {
    for raw in deltas_text.lines() {
        if raw.trim().is_empty() {
            continue;
        }
        let Ok(delta) = serde_json::from_str::<LiveLineDeltaOwned>(raw) else {
            continue;
        };
        let expected = cache.transcript_lines.len();
        if delta.i < expected {
            continue;
        }
        if delta.i != expected {
            break;
        }
        cache.transcript_lines.push(LiveTranscriptLine {
            text: delta.t,
            at: delta.a,
        });
    }
}

/// Full snapshot rewrite. Also truncates the deltas log since the snapshot now
/// includes everything. Called on flush/finish, not per line.
fn save_day_cache_full(
    course: &LiveCourseInfo,
    started_at: DateTime<Local>,
    transcript_lines: &[LiveTranscriptLine],
    summaries: &[LiveSummaryChunk],
) {
    let cache_ref = LiveDayCacheRef {
        date: Local::now().format("%Y-%m-%d").to_string(),
        course_name: &course.course_name,
        started_at: format_datetime(started_at),
        transcript_lines,
        summaries,
    };
    let Some(path) = day_cache_path(course) else {
        return;
    };
    let Ok(json) = serde_json::to_string(&cache_ref) else {
        return;
    };
    if std::fs::write(&path, json).is_ok() {
        if let Some(deltas) = day_cache_deltas_path(course) {
            let _ = std::fs::remove_file(deltas);
        }
    }
}

/// Append new transcript lines `[start..]` to the deltas log. Cheap incremental
/// write — typically a few hundred bytes vs the tens-to-hundreds of KB a full
/// snapshot rewrite would cost.
fn append_day_cache_deltas(
    course: &LiveCourseInfo,
    transcript_lines: &[LiveTranscriptLine],
    start: usize,
) {
    if start >= transcript_lines.len() {
        return;
    }
    let Some(path) = day_cache_deltas_path(course) else {
        return;
    };
    use std::io::Write;
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let mut buf = String::with_capacity((transcript_lines.len() - start) * 64);
    for (offset, line) in transcript_lines[start..].iter().enumerate() {
        let delta = LiveLineDeltaRef {
            i: start + offset,
            t: &line.text,
            a: &line.at,
        };
        if let Ok(json) = serde_json::to_string(&delta) {
            buf.push_str(&json);
            buf.push('\n');
        }
    }
    let _ = file.write_all(buf.as_bytes());
}

fn remove_day_cache(course: &LiveCourseInfo) {
    if let Some(path) = day_cache_path(course) {
        let _ = std::fs::remove_file(path);
    }
    if let Some(deltas) = day_cache_deltas_path(course) {
        let _ = std::fs::remove_file(deltas);
    }
}

/// Stable filename for a session's formal markdown. Anchored to `started_at` so a
/// mid-session save and the final save land at the same path — finish overwrites
/// the partial file rather than leaving an orphan.
fn formal_markdown_filename(course: &LiveCourseInfo, started_at: DateTime<Local>) -> String {
    if course.is_free_note {
        format!(
            "{}_{}_live.md",
            started_at.format("%Y%m%d"),
            started_at.format("%H%M%S")
        )
    } else {
        format!(
            "{}_{}_live.md",
            started_at.format("%Y%m%d"),
            sanitize_filename_component(&course.course_name)
        )
    }
}

/// Write a partial formal markdown file mid-session so a crash before stop still
/// leaves recoverable content on disk. The overall summary is a placeholder —
/// `live_finish_session` overwrites with the AI-generated overall summary at stop.
fn write_partial_markdown_file(
    course: &LiveCourseInfo,
    started_at: DateTime<Local>,
    transcript_lines: &[LiveTranscriptLine],
    summaries: &[LiveSummaryChunk],
) {
    if transcript_lines.is_empty() {
        return;
    }
    let overall_summary = "### 全体要約\n_(セッション継続中…保存時に確定します)_".to_string();
    let markdown = build_markdown(
        course,
        started_at,
        Local::now(),
        &overall_summary,
        summaries,
        transcript_lines,
    );
    let dir = live_storage_dir(course);
    let path = dir.join(formal_markdown_filename(course, started_at));
    if std::fs::write(&path, markdown.as_bytes()).is_err() {
        return;
    }
    let path_str = path.to_string_lossy().to_string();
    let file_name = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("live.md");
    // record_download dedupes by path, so repeated mid-session calls just update
    // the size/timestamp of a single download entry rather than spawning dupes.
    crate::commands::record_download(
        file_name,
        &path_str,
        Some(&course.course_name),
        "live",
        markdown.len() as u64,
    );
}

/// Auto-save session state to day cache (non-fatal on error).
/// - `force=false` (per-line trigger, debounced): append only newly-added lines
///   to the deltas log. Tiny write, no full re-serialization.
/// - `force=true` (per-flush / finish trigger): full snapshot rewrite and
///   truncate the deltas log. Catches the latest summaries too.
fn auto_save_day_cache(state: &LiveState, force: bool) {
    if !force {
        let now = instant_now_ms();
        let last = LAST_CACHE_WRITE.load(Ordering::Relaxed);
        if last > 0 && now.saturating_sub(last) < CACHE_DEBOUNCE.as_millis() as u64 {
            return;
        }
    }
    let Ok(mut guard) = state.0.lock() else {
        return;
    };
    let Some(session) = guard.as_mut() else {
        return;
    };
    if session.course.is_free_note {
        return;
    }

    if force {
        save_day_cache_full(
            &session.course,
            session.started_at,
            &session.transcript_lines,
            &session.summaries,
        );
        session.persisted_line_count = session.transcript_lines.len();
    } else {
        let total = session.transcript_lines.len();
        let start = session.persisted_line_count;
        if start >= total {
            return;
        }
        append_day_cache_deltas(&session.course, &session.transcript_lines, start);
        session.persisted_line_count = total;
    }
    LAST_CACHE_WRITE.store(instant_now_ms(), Ordering::Relaxed);
}

fn empty_snapshot() -> LiveSessionSnapshot {
    LiveSessionSnapshot {
        active: false,
        course: None,
        started_at: None,
        transcript_lines: Arc::new(Vec::new()),
        pending_lines: Arc::new(Vec::new()),
        summaries: Arc::new(Vec::new()),
    }
}

fn format_datetime(dt: DateTime<Local>) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn format_time(dt: DateTime<Local>) -> String {
    dt.format("%H:%M").to_string()
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
        let model = crate::local_ai::model_catalog()
            .iter()
            .find(|model| model.id == cfg.local_model)
            .ok_or_else(|| "Live要約用のローカルモデルが見つかりません".to_string())?;
        if !crate::local_ai::is_model_downloaded(&model.file_name) {
            return Err("Live要約用のローカルモデルを先にダウンロードしてください".into());
        }
    }
    Ok(cfg)
}

fn live_summary_interval_minutes() -> i64 {
    crate::ai::load_ai_config()
        .live_summary_interval_minutes
        .clamp(5, 30) as i64
}

fn should_skip_ai_summarization(started_at: DateTime<Local>, now: DateTime<Local>) -> bool {
    now.signed_duration_since(started_at).num_seconds() < MIN_AI_SUMMARIZATION_DURATION_SECS
}

fn short_session_overall_summary(course: &LiveCourseInfo, transcript_line_count: usize) -> String {
    if course.is_free_note {
        format!(
            "### 全体要約\n2分未満の自由ノートのためAI要約は行わず、全文転写（{}行）をそのまま保存しました。",
            transcript_line_count
        )
    } else {
        format!(
            "### 全体要約\n2分未満のLIVEのためAI要約は行わず、{}の全文転写（{}行）をそのまま保存しました。",
            course.course_name, transcript_line_count
        )
    }
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

async fn summarize_chunk(
    course: &LiveCourseInfo,
    lines: &[LiveTranscriptLine],
    recent_summaries: &[LiveSummaryChunk],
) -> Result<LiveChunkAiResult, String> {
    let cfg = live_ai_config()?;
    let language_hint = crate::ai::reply_language_hint(
        &cfg.reply_language,
        "\n\n重要: 输出全文必须使用中文（简体）。标题、要点、补充说明、整体总结都使用中文。",
        "\n\nIMPORTANT: Write the entire output in English, including headings, bullet points, and explanations.",
        "\n\n중요: 출력 전체를 한국어로 작성하세요. 제목, 핵심 포인트, 보충 설명, 전체 요약을 모두 한국어로 작성합니다.",
    );
    let transcript = lines
        .iter()
        .map(|line| format!("- [{}] {}", line.at, line.text))
        .collect::<Vec<_>>()
        .join("\n");
    let recent_summary_context = format_recent_summary_context(recent_summaries, 2);
    let messages = vec![
        crate::ai::ChatMessage {
            role: "system".into(),
            content: format!(
                "あなたは大学講義メモの整理アシスタントです。音声認識（STT）による文字起こしを基に、直近の講義内容を要約し、同じ区間で出た重要な専門用語・固有概念だけを注釈してください。\n\n注意事項:\n- 文字起こしには誤認識（同音異義語の取り違え、聞き取り不良による文字化け）が含まれる場合があります。文脈から正しい意味を推測し、明らかな誤認識は自然な範囲で修正して、本来の講義内容を復元してください。\n- 原文が断片的でも、文脈上ほぼ確実な内容は読みやすい表現に補って構いません。\n- ただし、具体的な数字・年号・割合・固有名詞・順位・因果関係などの高リスク事実は、文字起こしまたは直近文脈から十分に確認できる場合に限って書いてください。\n- 高リスク事実について確信が弱い場合は、より一般化した安全な表現に言い換えてください。\n- 外部知識は、用語の理解に必要な一般的背景・標準的定義・短い例を補う場合のみ使って構いません。ただし外部知識を使った場合は external_source に正確な出典名とURL、または公式文書名・書籍名など確認可能な出典を必ず書いてください。\n- 正確な出典を示せない外部知識は使わず、講義内で確認できる範囲に留めてください。\n- 講義で確認できない固有の数字・年号・人物関係・統計値などを外部知識で断定的に補ってはいけません。\n- 要約を書いたあと、自分で高リスク事実を見直し、根拠が弱い箇所は削除または表現を弱めてください。\n- 雑談や教室管理の発言（出席確認、マイク調整等）は省略し、学術的内容に集中してください。\n- 直前までの分割要約は講義の流れを把握するための参考情報です。今回の出力は必ず「今回新しく話された内容」を中心に書き、過去2区間の内容を重複して要約し直さないでください。\n- 前区間とのつながりがある場合のみ、その接続関係を短く反映して構いません。\n- 内容が少ない区間では無理に情報量を増やさず、確認できた範囲だけを簡潔にまとめてください。\n- 文体は過度に書き言葉へ寄せず、信頼できる講義ノートのように簡潔で具体的にしてください。\n\n出力形式（JSONのみ、厳守。Markdownフェンスや説明文を付けない）:\n{{\"summary_markdown\":\"- 重点1（1行、名詞句または短文）\\n- 重点2\\n- 重点3\\n\\n---\\n\\n**重点1**: 補足説明（1〜2文で具体的に）\\n\\n**重点2**: 補足説明（1〜2文で具体的に）\\n\\n**重点3**: 補足説明（1〜2文で具体的に）\",\"terms\":[{{\"term\":\"専門用語または固有概念\",\"explanation\":\"講義文脈での意味に加え、論点との関係・注意点・短い例のいずれかを補う。\",\"source_excerpt\":\"講義内の根拠になる短い発話断片\",\"external_source\":\"外部知識を使った場合の正確な出典名とURL。使っていない場合は空文字\"}}]}}\n\nsummary_markdown のルール:\n- 上半分: 箇条書きタイトルのみ（2〜4個）。講義の核心概念やキーワードを含める。\n- 下半分(---以降): 各重点の補足を段落形式で記述。箇条書き(- )は使わない。\n- 見出し(###等)は使わない。\n- 不明瞭な部分を無理に解釈せず、確信できる情報のみ記載する。\n\nterms のルール:\n- 今回の区間で出た専門用語・理論名・手法名・制度名・固有概念・略語だけを最大5件。\n- 注釈対象は「その語を知らないと講義の理解が止まりやすいもの」に限定する。\n- 一般常識、日常語、教室運営語、授業一般の語、辞書的に自明な普通名詞は注釈しない。例: 授業、講義、先生、学生、教室、出席、課題、レポート、資料、今日、次回。\n- その科目で専門的な意味を持つ場合を除き、単に有名・一般的という理由で語を選ばない。\n- explanation は1〜2文。語の意味だけで終わらせず、講義内の論点との関係、混同しやすい点、短い例、または復習時に見る観点を1つ補う。\n- source_excerpt は必ず講義内の根拠だけを書く。external_source は外部知識を使った場合だけ書く。\n- 該当語が少ない場合は terms を空配列にする。{}",
                language_hint
            ),
            images: Vec::new(),
        },
        crate::ai::ChatMessage {
            role: "user".into(),
            content: format!(
                "講義: {}\n授業コード: {}\n教員: {}\n教室: {}\n時間帯: {}\n\n直前の分割要約（最大2件）:\n{}\n\n今回の文字起こし:\n{}\n\n注記: 文字起こしの専門用語・固有名詞は STT の誤認識が混ざる可能性があります。講義名「{}」の分野脈絡を手がかりに、明らかな誤りは自然に補正してください。",
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
                recent_summary_context,
                transcript,
                course.course_name,
            ),
            images: Vec::new(),
        },
    ];
    let raw = crate::ai::chat_completion_public(&cfg, messages).await?;
    Ok(parse_chunk_ai_result(&raw))
}

async fn summarize_overall(
    course: &LiveCourseInfo,
    summaries: &[LiveSummaryChunk],
    transcript_lines: &[LiveTranscriptLine],
) -> Result<String, String> {
    let cfg = live_ai_config()?;
    let language_hint = crate::ai::reply_language_hint(
        &cfg.reply_language,
        "\n\n重要: 输出全文必须使用中文（简体）。标题、要点、补充说明、整体总结都使用中文。",
        "\n\nIMPORTANT: Write the entire output in English, including headings, bullet points, and explanations.",
        "\n\n중요: 출력 전체를 한국어로 작성하세요. 제목, 핵심 포인트, 보충 설명, 전체 요약을 모두 한국어로 작성합니다.",
    );
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
    let messages = vec![
        crate::ai::ChatMessage {
            role: "system".into(),
            content: format!(
                "あなたは大学講義ノートを仕上げるアシスタントです。分割要約と末尾の文字起こしを基に、講義全体を俯瞰する要約をMarkdownで返してください。\n\n注意事項:\n- 各分割要約を単純に繋げるのではなく、講義全体を貫くテーマや論理の流れを抽出してください。\n- 文字起こしには音声認識の誤りが含まれる可能性があります。文脈から意味を推測し、明らかな誤認識は自然な範囲で補正して構いません。\n- 原文が断片的でも、文脈上ほぼ確実な内容は読みやすく整理して構いません。\n- ただし、具体的な数字・年号・割合・固有名詞・順位・因果関係などの高リスク事実は、分割要約または文字起こしから十分に確認できる場合に限って書いてください。\n- 高リスク事実について確信が弱い場合は、より一般化した安全な表現に言い換えてください。外部知識だけで具体値や詳細を補ってはいけません。\n- 要約を書いたあと、自分で高リスク事実を見直し、根拠が弱い箇所は削除または表現を弱めてください。\n- 講義全体の理解を助ける整理はしてよいですが、補った背景知識を講義で明示された事実のように書いてはいけません。\n- 文体は過度に書き言葉へ寄せず、信頼できる講義ノートのように簡潔で具体的にしてください。\n\n出力形式（厳守）:\n### 全体要約\n講義全体の主旨を1段落にまとめる。\n### 今回の論点\n- 講義で取り上げられた主要論点を3〜5個、各1行の箇条書きで列挙\n\nルール:\n- 指定形式以外のセクションや見出しを追加しない。\n- 抽象的すぎる表現を避け、講義固有の具体的概念やキーワードを含める。{}",
                language_hint
            ),
            images: Vec::new(),
        },
        crate::ai::ChatMessage {
            role: "user".into(),
            content: format!(
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
            ),
            images: Vec::new(),
        },
    ];
    let raw = crate::ai::chat_completion_public(&cfg, messages).await?;
    Ok(sanitize_model_output(&raw))
}

fn extract_json_object(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let start = bytes.iter().position(|b| *b == b'{')?;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (idx, b) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if *b == b'\\' {
                escaped = true;
            } else if *b == b'"' {
                in_string = false;
            }
            continue;
        }
        match *b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return text.get(start..=idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn value_to_trimmed_string(value: Option<&serde_json::Value>) -> String {
    match value {
        Some(serde_json::Value::String(s)) => s.trim().to_string(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn clamp_chars(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out = trimmed.chars().take(max_chars).collect::<String>();
    out.push('…');
    out
}

fn is_low_value_live_term(term: &str) -> bool {
    let normalized = term
        .trim()
        .trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\''
                    | '`'
                    | '「'
                    | '」'
                    | '『'
                    | '』'
                    | '（'
                    | '）'
                    | '('
                    | ')'
                    | '【'
                    | '】'
                    | '['
                    | ']'
            )
        })
        .to_lowercase();
    if normalized.chars().count() <= 1 {
        return true;
    }
    const LOW_VALUE_TERMS: &[&str] = &[
        "授業",
        "講義",
        "先生",
        "教員",
        "教授",
        "学生",
        "大学",
        "教室",
        "出席",
        "欠席",
        "課題",
        "宿題",
        "レポート",
        "資料",
        "教科書",
        "スライド",
        "今日",
        "次回",
        "明日",
        "来週",
        "学校",
        "勉強",
        "学習",
        "考试",
        "作业",
        "报告",
        "老师",
        "学生",
        "大学",
        "教室",
        "今天",
        "下次",
        "tomorrow",
        "today",
        "class",
        "lecture",
        "teacher",
        "student",
        "assignment",
        "report",
        "homework",
        "textbook",
        "slides",
    ];
    LOW_VALUE_TERMS
        .iter()
        .any(|candidate| normalized == *candidate)
}

fn parse_chunk_ai_result(raw: &str) -> LiveChunkAiResult {
    let sanitized = sanitize_model_output(raw);
    let Some(json_text) = extract_json_object(&sanitized) else {
        return LiveChunkAiResult {
            body: sanitized,
            terms: Vec::new(),
        };
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json_text) else {
        return LiveChunkAiResult {
            body: sanitized,
            terms: Vec::new(),
        };
    };

    let body = value_to_trimmed_string(
        value
            .get("summary_markdown")
            .or_else(|| value.get("summary"))
            .or_else(|| value.get("body")),
    );
    let mut terms = Vec::new();
    if let Some(items) = value.get("terms").and_then(|v| v.as_array()) {
        for item in items.iter().take(5) {
            let term = clamp_chars(&value_to_trimmed_string(item.get("term")), 40);
            let explanation = clamp_chars(
                &value_to_trimmed_string(item.get("explanation")),
                MAX_LIVE_TERM_EXPLANATION_CHARS,
            );
            if term.is_empty() || explanation.is_empty() || is_low_value_live_term(&term) {
                continue;
            }
            terms.push(LiveTermExplanation {
                term,
                explanation,
                source_excerpt: clamp_chars(
                    &value_to_trimmed_string(item.get("source_excerpt")),
                    80,
                ),
                external_source: clamp_chars(
                    &value_to_trimmed_string(item.get("external_source")),
                    180,
                ),
            });
        }
    }

    LiveChunkAiResult {
        body: if body.is_empty() { sanitized } else { body },
        terms,
    }
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
            content: "あなたは大学講義ノートから学生のTODO候補だけを抽出するアシスタントです。先生が明確に課題、提出物、宿題、レポート、事前準備、復習タスク、小テスト準備として指示したものだけを抽出してください。講義内容そのもの、一般的な学習アドバイス、AIが勝手に作った復習案は含めません。締切は発話中の具体日付/時刻を最優先し、「次回まで」「来週の授業まで」「授業計画の該当回まで」など相対的に判断できる場合は、現在日時・次回授業候補・授業計画から YYYY-MM-DD HH:mm 形式で推定してください。推定した場合は note に根拠を短く含めてください。どうしても判断できない場合だけ deadline を空文字にします。出力はJSONのみで、説明文やMarkdownを付けないでください。形式: {\"todos\":[{\"title\":\"課題名\",\"content_type\":\"課題|レポート|予習|復習|テスト準備|その他\",\"deadline\":\"YYYY-MM-DD HH:mm または 空文字\",\"note\":\"学生が次にすることを短く。締切推定時は根拠も短く\",\"source_excerpt\":\"根拠になる発話を短く\"}]}。候補がなければ {\"todos\":[]}。".into(),
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

fn format_terms_markdown(terms: &[LiveTermExplanation]) -> String {
    if terms.is_empty() {
        return String::new();
    }
    let lines = terms
        .iter()
        .map(|term| {
            let mut detail = String::new();
            if !term.source_excerpt.is_empty() {
                detail.push_str(&format!("（講義内根拠: {}）", term.source_excerpt));
            }
            if !term.external_source.is_empty() {
                detail.push_str(&format!("（外部出典: {}）", term.external_source));
            }
            format!("- **{}**: {}{}", term.term, term.explanation, detail)
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("\n\n### 用語注釈\n{}", lines)
}

async fn flush_session_summary(
    state: &LiveState,
    force: bool,
) -> Result<LiveSessionSnapshot, String> {
    let now = Local::now();
    let summary_interval_minutes = live_summary_interval_minutes();
    let (session_id, course, lines, recent_summaries, range_start, range_end, chunk_index) = {
        let guard = state
            .0
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        let session = guard
            .as_ref()
            .ok_or_else(|| "Liveセッションが開始されていません".to_string())?;
        if session.pending_lines.is_empty() {
            return Ok(session.snapshot());
        }
        if should_skip_ai_summarization(session.started_at, now) {
            return Ok(session.snapshot());
        }
        if !force
            && now
                .signed_duration_since(session.batch_started_at)
                .num_minutes()
                < summary_interval_minutes
        {
            return Ok(session.snapshot());
        }
        // Skip the scheduled summary when almost nothing has been said this
        // interval — spending an AI call on 1-2 stray lines wastes power and
        // yields a useless summary. We leave batch_started_at untouched, so
        // the next tick still considers this content and will fire once more
        // lines have accumulated (or immediately if forced on stop).
        if !force && session.pending_lines.len() < 3 {
            return Ok(session.snapshot());
        }
        (
            session.session_id.clone(),
            session.course.clone(),
            session.pending_lines.clone(),
            session.summaries.clone(),
            session.batch_started_at,
            now,
            session.summaries.len() + 1,
        )
    };

    let chunk_ai = summarize_chunk(&course, &lines, &recent_summaries).await?;
    let mut guard = state
        .0
        .lock()
        .map_err(|_| "Live state lock failed".to_string())?;
    let session = guard
        .as_mut()
        .ok_or_else(|| "Liveセッションが開始されていません".to_string())?;
    if session.session_id != session_id {
        return Ok(session.snapshot());
    }
    if session.pending_lines.is_empty() {
        return Ok(session.snapshot());
    }
    let summary = LiveSummaryChunk {
        title: build_chunk_title(chunk_index, range_start, range_end),
        range_label: format!("{}-{}", format_time(range_start), format_time(range_end)),
        body: chunk_ai.body,
        line_count: lines.len(),
        terms: chunk_ai.terms,
    };
    Arc::make_mut(&mut session.summaries).push(summary);
    Arc::make_mut(&mut session.pending_lines).clear();
    session.batch_started_at = now;
    Ok(session.snapshot())
}

fn build_markdown(
    course: &LiveCourseInfo,
    started_at: DateTime<Local>,
    ended_at: DateTime<Local>,
    overall_summary: &str,
    summaries: &[LiveSummaryChunk],
    transcript_lines: &[LiveTranscriptLine],
) -> String {
    let transcript = transcript_lines
        .iter()
        .map(|line| format!("- [{}] {}", line.at, line.text))
        .collect::<Vec<_>>()
        .join("\n");
    let chunk_markdown = summaries
        .iter()
        .map(|chunk| {
            format!(
                "## {}\n{}\n\n{}{}",
                chunk.title,
                chunk.range_label,
                chunk.body,
                format_terms_markdown(&chunk.terms)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    if course.is_free_note {
        format!(
            "# {title}\n\n- 開始: {started}\n- 終了: {ended}\n\n{overall_summary}\n\n## 区間ごとの要約\n\n{chunk_markdown}\n\n## 全文転写\n\n{transcript}\n",
            title = FREE_NOTE_FOLDER_NAME,
            started = format_datetime(started_at),
            ended = format_datetime(ended_at),
        )
    } else {
        format!(
            "# {course_name}\n\n- 授業コード: {course_code}\n- 教員: {teacher}\n- 教室: {room}\n- 時間帯: {time_label}\n- 開始: {started}\n- 終了: {ended}\n\n{overall_summary}\n\n## 区間ごとの要約\n\n{chunk_markdown}\n\n## 全文転写\n\n{transcript}\n",
            course_name = course.course_name,
            course_code = if course.course_code.is_empty() {
                "不明"
            } else {
                &course.course_code
            },
            teacher = if course.teacher.is_empty() {
                "不明"
            } else {
                &course.teacher
            },
            room = if course.room.is_empty() {
                "未設定"
            } else {
                &course.room
            },
            time_label = if course.time_label.is_empty() {
                "未設定"
            } else {
                &course.time_label
            },
            started = format_datetime(started_at),
            ended = format_datetime(ended_at),
        )
    }
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

    let persisted_line_count = prev_transcript.len();
    let session = LiveSession {
        session_id: uuid::Uuid::new_v4().to_string(),
        course,
        started_at,
        transcript_lines: Arc::new(prev_transcript),
        pending_lines: Arc::new(Vec::new()),
        summaries: Arc::new(prev_summaries),
        batch_started_at: now,
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
    let summary_count_before = {
        let guard = state
            .0
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        guard.as_ref().map(|s| s.summaries.len()).unwrap_or(0)
    };
    let snapshot = flush_session_summary(&state, force).await?;
    auto_save_day_cache(&state, true);

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

    emit_live_update(&app, &state);
    Ok(snapshot)
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
    // Non-fatal: try to flush remaining pending lines, but don't abort if AI fails
    let _ = flush_session_summary(&state, true).await;

    let (course, started_at, transcript_lines, summaries) = {
        let guard = state
            .0
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        let session = guard
            .as_ref()
            .ok_or_else(|| "Liveセッションが開始されていません".to_string())?;
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
            let result = LiveSaveResult {
                saved: false,
                path: String::new(),
                markdown: String::new(),
                snapshot,
                suggested_todos: Vec::new(),
            };
            emit_live_update(&app, &state);
            return Ok(result);
        }
        (
            session.course.clone(),
            session.started_at,
            session.transcript_lines.clone(),
            session.summaries.clone(),
        )
    };

    let ended_at = Local::now();
    let overall_summary = if should_skip_ai_summarization(started_at, ended_at) {
        short_session_overall_summary(&course, transcript_lines.len())
    } else {
        summarize_overall(&course, &summaries, &transcript_lines)
            .await
            .unwrap_or_else(|_| {
                if course.is_free_note {
                    format!(
                        "### 全体要約\n{} 件の転写行と {} 件の分割要約を含む自由ノートを保存しました。",
                        transcript_lines.len(),
                        summaries.len()
                    )
                } else {
                    format!(
                        "### 全体要約\n{} の講義メモ。{}件の転写行と{}件の分割要約を保存しました。",
                        course.course_name,
                        transcript_lines.len(),
                        summaries.len()
                    )
                }
            })
    };
    let markdown = build_markdown(
        &course,
        started_at,
        ended_at,
        &overall_summary,
        &summaries,
        &transcript_lines,
    );
    let suggested_todos = if should_skip_ai_summarization(started_at, ended_at) {
        Vec::new()
    } else {
        extract_todo_suggestions(&app, &course, &summaries, &transcript_lines, ended_at).await
    };

    let dir = live_storage_dir(&course);
    let path = dir.join(formal_markdown_filename(&course, started_at));
    std::fs::write(&path, markdown.as_bytes()).map_err(|e| format!("Markdown保存失敗: {}", e))?;

    // Save day cache so next session for same course today can resume
    save_day_cache_full(&course, started_at, &transcript_lines, &summaries);

    let path_str = path.to_string_lossy().to_string();
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("live.md");
    crate::commands::record_download(
        file_name,
        &path_str,
        Some(&course.course_name),
        "live",
        markdown.len() as u64,
    );

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

    let result = LiveSaveResult {
        saved: true,
        path: path_str.clone(),
        markdown,
        snapshot,
        suggested_todos,
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
          ]
        }"#;
        let parsed = parse_chunk_ai_result(raw);
        assert!(parsed.body.contains("MVC"));
        assert_eq!(parsed.terms.len(), 1);
        assert_eq!(parsed.terms[0].term, "MVC");
        assert!(parsed.terms[0].external_source.contains("MDN"));
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
