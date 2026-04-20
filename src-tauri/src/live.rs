use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::Emitter;

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveTranscriptLine {
    pub text: String,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSummaryChunk {
    pub title: String,
    pub range_label: String,
    pub body: String,
    pub line_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSessionSnapshot {
    pub active: bool,
    pub course: Option<LiveCourseInfo>,
    pub started_at: Option<String>,
    pub transcript_lines: Vec<LiveTranscriptLine>,
    pub pending_lines: Vec<LiveTranscriptLine>,
    pub summaries: Vec<LiveSummaryChunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSaveResult {
    pub path: String,
    pub markdown: String,
    pub snapshot: LiveSessionSnapshot,
}

#[derive(Debug, Clone)]
struct LiveSession {
    session_id: String,
    course: LiveCourseInfo,
    started_at: DateTime<Local>,
    transcript_lines: Vec<LiveTranscriptLine>,
    pending_lines: Vec<LiveTranscriptLine>,
    summaries: Vec<LiveSummaryChunk>,
    batch_started_at: DateTime<Local>,
}

impl LiveSession {
    fn snapshot(&self) -> LiveSessionSnapshot {
        LiveSessionSnapshot {
            active: true,
            course: Some(self.course.clone()),
            started_at: Some(format_datetime(self.started_at)),
            transcript_lines: self.transcript_lines.clone(),
            pending_lines: self.pending_lines.clone(),
            summaries: self.summaries.clone(),
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

fn day_cache_path(course: &LiveCourseInfo) -> std::path::PathBuf {
    let dir = crate::commands::resolve_download_dir(Some(&course.course_name));
    let date_str = Local::now().format("%Y%m%d").to_string();
    let safe_name = sanitize_filename_component(&course.course_name);
    dir.join(format!(".{}_{}_live.cache.json", date_str, safe_name))
}

fn load_day_cache(course: &LiveCourseInfo) -> Option<LiveDayCache> {
    let path = day_cache_path(course);
    let data = std::fs::read_to_string(&path).ok()?;
    let cache: LiveDayCache = serde_json::from_str(&data).ok()?;
    let today = Local::now().format("%Y-%m-%d").to_string();
    if cache.date == today && cache.course_name == course.course_name {
        Some(cache)
    } else {
        // stale cache from a different day
        let _ = std::fs::remove_file(&path);
        None
    }
}

fn save_day_cache(
    course: &LiveCourseInfo,
    started_at: DateTime<Local>,
    transcript_lines: &[LiveTranscriptLine],
    summaries: &[LiveSummaryChunk],
) {
    let cache = LiveDayCache {
        date: Local::now().format("%Y-%m-%d").to_string(),
        course_name: course.course_name.clone(),
        started_at: format_datetime(started_at),
        transcript_lines: transcript_lines.to_vec(),
        summaries: summaries.to_vec(),
    };
    let path = day_cache_path(course);
    if let Ok(json) = serde_json::to_string(&cache) {
        let _ = std::fs::write(&path, json);
    }
}

fn remove_day_cache(course: &LiveCourseInfo) {
    let _ = std::fs::remove_file(day_cache_path(course));
}

/// Auto-save session state to day cache (non-fatal on error).
fn auto_save_day_cache(state: &LiveState) {
    let guard = state.0.lock().ok();
    if let Some(Some(session)) = guard.as_deref() {
        save_day_cache(
            &session.course,
            session.started_at,
            &session.transcript_lines,
            &session.summaries,
        );
    }
}

fn empty_snapshot() -> LiveSessionSnapshot {
    LiveSessionSnapshot {
        active: false,
        course: None,
        started_at: None,
        transcript_lines: Vec::new(),
        pending_lines: Vec::new(),
        summaries: Vec::new(),
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

fn live_summary_language_hint(reply_language: &str) -> &'static str {
    match reply_language {
        "zh" => "\n\n重要: 输出全文必须使用中文（简体）。标题、要点、补充说明、整体总结都使用中文。",
        "en" => "\n\nIMPORTANT: Write the entire output in English, including headings, bullet points, and explanations.",
        "ko" => "\n\n중요: 출력 전체를 한국어로 작성하세요. 제목, 핵심 포인트, 보충 설명, 전체 요약을 모두 한국어로 작성합니다.",
        _ => "",
    }
}

async fn summarize_chunk(
    course: &LiveCourseInfo,
    lines: &[LiveTranscriptLine],
    recent_summaries: &[LiveSummaryChunk],
) -> Result<String, String> {
    let cfg = live_ai_config()?;
    let language_hint = live_summary_language_hint(&cfg.reply_language);
    let transcript = lines
        .iter()
        .map(|line| format!("- [{}] {}", line.at, line.text))
        .collect::<Vec<_>>()
        .join("\n");
    let recent_summary_context = format_recent_summary_context(recent_summaries, 2);
    let messages = vec![
        crate::ai::ChatMessage {
            role: "system".into(),
            content: format!("あなたは大学講義メモの整理アシスタントです。音声認識（STT）による文字起こしを基に、直近の講義内容を要約してください。\n\n注意事項:\n- 文字起こしには誤認識（同音異義語の取り違え、聞き取り不良による文字化け）が含まれる場合があります。文脈から正しい意味を推測し、明らかな誤認識は自然な範囲で修正して、本来の講義内容を復元してください。\n- 原文が断片的でも、文脈上ほぼ確実な内容は読みやすい表現に補って構いません。\n- ただし、具体的な数字・年号・割合・固有名詞・順位・因果関係などの高リスク事実は、文字起こしまたは直近文脈から十分に確認できる場合に限って書いてください。\n- 高リスク事実について確信が弱い場合は、より一般化した安全な表現に言い換えてください。外部知識だけで具体値や詳細を補ってはいけません。\n- 要約を書いたあと、自分で高リスク事実を見直し、根拠が弱い箇所は削除または表現を弱めてください。\n- 雑談や教室管理の発言（出席確認、マイク調整等）は省略し、学術的内容に集中してください。\n- 直前までの分割要約は講義の流れを把握するための参考情報です。今回の出力は必ず「今回新しく話された内容」を中心に書き、過去2区間の内容を重複して要約し直さないでください。\n- 前区間とのつながりがある場合のみ、その接続関係を短く反映して構いません。\n- 内容が少ない区間では無理に情報量を増やさず、確認できた範囲だけを簡潔にまとめてください。\n- 文体は過度に書き言葉へ寄せず、信頼できる講義ノートのように簡潔で具体的にしてください。\n\n出力形式（Markdownのみ、厳守）:\n\n- 重点1（1行、名詞句または短文）\n- 重点2\n- 重点3\n\n---\n\n**重点1**: 補足説明（1〜2文で具体的に）\n\n**重点2**: 補足説明（1〜2文で具体的に）\n\n**重点3**: 補足説明（1〜2文で具体的に）\n\nルール:\n- 上半分: 箇条書きタイトルのみ（2〜4個）。講義の核心概念やキーワードを含める。\n- 下半分(---以降): 各重点の補足を段落形式で記述。箇条書き(- )は使わない。\n- 見出し(###等)は使わない。\n- 不明瞭な部分を無理に解釈せず、確信できる情報のみ記載する。{}", language_hint),
            images: Vec::new(),
        },
        crate::ai::ChatMessage {
            role: "user".into(),
            content: format!(
                "講義: {}\n授業コード: {}\n時間帯: {}\n\n直前の分割要約（最大2件）:\n{}\n\n今回の文字起こし:\n{}",
                course.course_name,
                course.course_code,
                course.time_label,
                recent_summary_context,
                transcript
            ),
            images: Vec::new(),
        },
    ];
    let raw = crate::ai::chat_completion_public(&cfg, messages).await?;
    Ok(sanitize_model_output(&raw))
}

async fn summarize_overall(
    course: &LiveCourseInfo,
    summaries: &[LiveSummaryChunk],
    transcript_lines: &[LiveTranscriptLine],
) -> Result<String, String> {
    let cfg = live_ai_config()?;
    let language_hint = live_summary_language_hint(&cfg.reply_language);
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
            content: format!("あなたは大学講義ノートを仕上げるアシスタントです。分割要約と末尾の文字起こしを基に、講義全体を俯瞰する要約をMarkdownで返してください。\n\n注意事項:\n- 各分割要約を単純に繋げるのではなく、講義全体を貫くテーマや論理の流れを抽出してください。\n- 文字起こしには音声認識の誤りが含まれる可能性があります。文脈から意味を推測し、明らかな誤認識は自然な範囲で補正して構いません。\n- 原文が断片的でも、文脈上ほぼ確実な内容は読みやすく整理して構いません。\n- ただし、具体的な数字・年号・割合・固有名詞・順位・因果関係などの高リスク事実は、分割要約または文字起こしから十分に確認できる場合に限って書いてください。\n- 高リスク事実について確信が弱い場合は、より一般化した安全な表現に言い換えてください。外部知識だけで具体値や詳細を補ってはいけません。\n- 要約を書いたあと、自分で高リスク事実を見直し、根拠が弱い箇所は削除または表現を弱めてください。\n- 講義全体の理解を助ける整理はしてよいですが、補った背景知識を講義で明示された事実のように書いてはいけません。\n- 文体は過度に書き言葉へ寄せず、信頼できる講義ノートのように簡潔で具体的にしてください。\n\n出力形式（厳守）:\n### 全体要約\n講義全体の主旨を1段落にまとめる。\n### 今回の論点\n- 講義で取り上げられた主要論点を3〜5個、各1行の箇条書きで列挙\n\nルール:\n- 指定形式以外のセクションや見出しを追加しない。\n- 抽象的すぎる表現を避け、講義固有の具体的概念やキーワードを含める。{}", language_hint),
            images: Vec::new(),
        },
        crate::ai::ChatMessage {
            role: "user".into(),
            content: format!(
                "講義: {}\n授業コード: {}\n\n分割要約:\n{}\n\n終盤の文字起こし:\n{}",
                course.course_name, course.course_code, summary_text, recent_transcript
            ),
            images: Vec::new(),
        },
    ];
    let raw = crate::ai::chat_completion_public(&cfg, messages).await?;
    Ok(sanitize_model_output(&raw))
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
        if !force
            && now
                .signed_duration_since(session.batch_started_at)
                .num_minutes()
                < summary_interval_minutes
        {
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

    let body = summarize_chunk(&course, &lines, &recent_summaries).await?;
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
        body,
        line_count: lines.len(),
    };
    session.summaries.push(summary);
    session.pending_lines.clear();
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
                "## {}\n{}\n\n{}",
                chunk.title, chunk.range_label, chunk.body
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

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
            transcript_lines: cache.transcript_lines,
            pending_lines: Vec::new(),
            summaries: cache.summaries,
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
    if course.course_name.trim().is_empty() {
        return Err("講義名が空です".into());
    }
    course.course_name = course.course_name.trim().to_string();
    course.course_code = course.course_code.trim().to_string();
    course.room = course.room.trim().to_string();
    course.teacher = course.teacher.trim().to_string();
    course.time_label = course.time_label.trim().to_string();

    let now = Local::now();

    // Load accumulated data from earlier in the same course today
    let (prev_transcript, prev_summaries, original_start) = match load_day_cache(&course) {
        Some(cache) => (cache.transcript_lines, cache.summaries, cache.started_at),
        None => (Vec::new(), Vec::new(), format_datetime(now)),
    };
    let started_at = chrono::NaiveDateTime::parse_from_str(&original_start, "%Y-%m-%d %H:%M:%S")
        .map(|naive| naive.and_local_timezone(Local).unwrap())
        .unwrap_or(now);

    let session = LiveSession {
        session_id: uuid::Uuid::new_v4().to_string(),
        course,
        started_at,
        transcript_lines: prev_transcript,
        pending_lines: Vec::new(),
        summaries: prev_summaries,
        batch_started_at: now,
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
        session.transcript_lines.push(line.clone());
        session.pending_lines.push(line);
        session.snapshot()
    };
    auto_save_day_cache(&state);
    emit_live_update(&app, &state);
    Ok(snapshot)
}

#[tauri::command]
pub async fn live_flush_summary(
    app: tauri::AppHandle,
    state: tauri::State<'_, LiveState>,
    force: bool,
) -> Result<LiveSessionSnapshot, String> {
    let snapshot = flush_session_summary(&state, force).await?;
    auto_save_day_cache(&state);
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
    *guard = None;
    drop(guard);
    emit_live_update(&app, &state);
    Ok(())
}

/// Clear the day cache for a specific course, removing all accumulated transcript/summary data.
#[tauri::command]
pub fn live_clear_day_cache(course: LiveCourseInfo) -> Result<(), String> {
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
            return Err("文字起こしがまだありません".into());
        }
        (
            session.course.clone(),
            session.started_at,
            session.transcript_lines.clone(),
            session.summaries.clone(),
        )
    };

    let ended_at = Local::now();
    let overall_summary = summarize_overall(&course, &summaries, &transcript_lines)
        .await
        .unwrap_or_else(|_| {
            format!(
                "### 全体要約\n{} の講義メモ。{}件の転写行と{}件の分割要約を保存しました。",
                course.course_name,
                transcript_lines.len(),
                summaries.len()
            )
        });
    let markdown = build_markdown(
        &course,
        started_at,
        ended_at,
        &overall_summary,
        &summaries,
        &transcript_lines,
    );

    let dir = crate::commands::resolve_download_dir(Some(&course.course_name));
    // Deterministic filename: one file per course per day
    let base_name = format!(
        "{}_{}_live.md",
        ended_at.format("%Y%m%d"),
        sanitize_filename_component(&course.course_name)
    );
    let path = dir.join(&base_name);
    std::fs::write(&path, markdown.as_bytes()).map_err(|e| format!("Markdown保存失敗: {}", e))?;

    // Save day cache so next session for same course today can resume
    save_day_cache(&course, started_at, &transcript_lines, &summaries);

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
        path: path_str.clone(),
        markdown,
        snapshot,
    };
    let _ = app.emit("live-session-saved", &result);
    emit_live_update(&app, &state);
    Ok(result)
}
