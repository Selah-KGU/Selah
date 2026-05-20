use chrono::{DateTime, Datelike, Duration as ChronoDuration, Local};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

mod ai_output;
mod cache;
mod markdown;
mod types;

use self::cache::{
    auto_save_day_cache, formal_markdown_filename, live_storage_dir, load_day_cache,
    remove_day_cache, save_day_cache_full, write_partial_markdown_file,
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
    LiveTodoSuggestion, LiveTranscriptLine, LiveWhiteboard, LiveWhiteboardEdge, LiveWhiteboardNode,
};

const MIN_AI_SUMMARIZATION_DURATION_SECS: i64 = 120;
const MAX_LIVE_TERM_EXPLANATION_CHARS: usize = 220;
// Whiteboard nodes/edges are intentionally uncapped: the board must accumulate
// the full course/recording as it grows, so a hard ceiling silently forces the
// model to compress earlier branches. Per-field length and the relationship
// guards in `parse_live_whiteboard` are the remaining safety nets.
const FREE_NOTE_FOLDER_NAME: &str = "自由ノート";

pub struct LiveState(Mutex<Option<LiveSession>>);

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
- 明らかな無音・相槌・聞き取り不能な断片は省略してよいですが、会話の展開、人物関係、ゲームや作品内ルール、出来事の流れは整理対象にしてください。
- summary_markdown と terms は今回新しく話された内容を中心にし、過去2区間を重複して要約し直さないでください。
- 内容が少ない区間では無理に情報量を増やさず、確認できた範囲だけを簡潔にまとめてください。
- 文体は、あとから見返せる録音メモのように簡潔で具体的にしてください。

出力形式（JSONのみ、厳守。Markdownフェンスや説明文を付けない。whiteboard 等のフィールドは出力しない）:
{"summary_markdown":"- 重点見出し（名詞句または短文）\n- 重点見出し\n\n---\n\n**重点見出し**: 補足説明（具体的に）\n\n**重点見出し**: 補足説明（具体的に）","terms":[{"term":"専門用語または固有概念","explanation":"講義文脈での意味に加え、論点との関係・注意点・短い例のいずれかを補う。","source_excerpt":"講義内の根拠になる短い発話断片","external_source":"外部知識を使った場合の正確な出典名とURL。使っていない場合は空文字"}]}

summary_markdown のルール:
- 上半分: 箇条書きタイトルのみ。録音の核心概念やキーワードを、理解に必要な分だけ含める。
- 下半分(---以降): 各重点の補足を段落形式で記述。箇条書き(- )は使わない。
- 見出し(###等)は使わない。
- 不明瞭な部分を無理に解釈せず、確信できる情報のみ記載する。

terms のルール:
- 今回の区間で出た重要な人物名・作品名・ルール名・概念・出来事・固有名詞・略語だけを選ぶ。
- 注釈対象は「その語や人物・ルールを知らないと録音内容の理解が止まりやすいもの」に限定する。
- 一般常識、日常語、単なる相槌、意味の薄い断片は注釈しない。
- explanation は簡潔にする。語の意味だけで終わらせず、録音内の話題との関係、混同しやすい点、短い例、または見返す観点を補う。
- source_excerpt は必ず録音内の根拠だけを書く。external_source は外部知識を使った場合だけ書く。
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
    let mut prompt = r#"あなたは知識整理ボード（whiteboard）を作る専門アシスタントです。これまでの分割要約・用語注釈・現在の累積ボード・今回の文字起こしを総合し、講義開始から現在までの累積知識整理ボード全体を JSON で返してください。出力は whiteboard JSON のみで、summary_markdown / terms 等は一切返さないでください。

出力形式（JSONのみ、厳守。Markdownフェンスや説明文を付けない）:
{"whiteboard":{"title":"短い題名","layout":"flow|hub|compare|cycle|grid","nodes":[{"id":"stable-id","label":"短い概念名","detail":"白板内で理解できる短い説明","node_type":"structure|term","kind":"core|support|question|result","role":"main|branch","parent_id":"branch の親 main id、または term の親 structure id。全体用語は空文字","source_type":"lecture|external","source_excerpt":"講義内根拠。外部なら空文字","external_source":"外部補足の出典。講義内なら空文字"}],"edges":[{"from":"n1","to":"n2","label":"具体的な関係語"}]}}

whiteboard は講義内容を中心に関連知識を整理する知識整理ボードとして作る。本文の代替ではなく、右側で関係を素早く掴むための概念図です。

正しい出力:
- その時点までの講義内容を見返した人が、扱われた課題・観点・展開を漏れなく追える。
- 新しい課題は新しい課題として見える。冒頭の話題に無理に従属させない。
- 既存課題の深掘りは、既存ノードの detail 更新、branch 追加、関係 edge の更新で表す。
- 構造ノードは講義の流れを支える骨格、用語ノードは骨格を読むための補助説明として分かれる。
- ノードや edge の量は固定目標ではなく、理解に必要かどうかで決める。必要なものを削らず、不要なものを増やさない。
- 正しいボードは「大きい/小さい」ではなく、「課題のまとまりが見える」「関係が読める」「用語が主構造を邪魔しない」状態です。

累積更新（最重要）:
- whiteboard は差分ではなく、録音開始から現在までの累積ボード全体を毎回返す。ノード総数に上限はない。区間が進むほど確実に増える前提で設計する。
- 既存ボードに含まれていた nodes は、原則すべてそのまま今回の出力にも含める。具体的には、既存の main / branch / term ノードを今回返さなかったら、それは「削除した」と解釈される。録音内で既に話された具体トピックは、その後に話題が変わっても削除しない。
- 強い原則: 今回返す nodes 配列の長さは、直前ボードの長さ「以上」でなければならない。減らしてはいけない。新しい区間に少しでも具体材料があれば、既存ノードを 1 つも消さずに、新規 structure node / branch / term を追加して必ず純増させる。
- 累積増加は「既存ノードを更新しない」という意味ではない。後続区間で理解が深まった場合、既存ノードの label / detail / source_excerpt / kind / edge label / 親子関係は、内容をより正確にするために更新してよい。
- ただし更新は情報の精密化・訂正・関係整理のために行う。既に話された具体論点を消す、旧 branch / term を main の detail に押し込む、別の話題に吸収する、またはノードを返さない形で見えなくすることは更新ではなく削除なので禁止。
- 例外（縮小・統合を許す唯一の条件）:
    1) 既存ノードが明らかな重複（同じ概念が別 id で 2 つある）。この場合は 1 つに統合し、保持する側の id を残す。
    2) 既存ノードが STT 誤認識・意味不明で、後続区間で正体が判明した。判明した内容に detail / label を書き換え、id は維持する。
    3) ごく断片的な独り言で、ノードとして残す価値がそもそも無かった単独 term。
  これら以外の理由（「構造を綺麗にしたい」「話題が変わった」「直近の関心に絞りたい」）では既存ノードを絶対に消さない。
- 旧話題の圧縮禁止: 既存 main の branch / term を消して、その内容を main の detail に文章で詰め込む書き方をしない。「（前区間の内容）」「録音前半の内容」のような要約表現を detail / source_excerpt に入れない。各論点は具体ノードのまま残し、detail はそのノード単体での説明にする。
- id 安定性: 同じ概念には毎回まったく同じ id を使う。意味は同じでも別 id（例: social-gaming → social-interaction、game-activities → survival-management のような言い換え）は禁止。label を更新したくなった場合も id は据え置く。
- 既存 edge は対応する両端ノードが残る限り維持する。両端が残っているのに edge だけ消えるのは情報損失なので避ける。新しい関係が見えた場合だけ追加する。
- 一回の録音/講義には複数の課題・章・観点が自然に並ぶ。新しい main 課題は、既存 main クラスターを保ったまま、別の main クラスターとして追加する。冒頭の話題に従属させない。
- まだ構造が不十分な最初期だけ nodes を空配列にしてよい。一度ノードを返した後の区間で空にしたり、極端に縮めたりしない。

話題境界:
- 今回区間が既存 main クラスターの続きか、新しい話題・章・素材・動画・論点かを必ず判定してからノードを追加する。
- 新しい話題と判断する目安: 主語・対象・人物・作品・制度・問題設定が大きく変わる、前区間との因果や説明関係が薄い、締めの発話や導入文がある、別の動画/会話/教材に切り替わった形跡がある、または用語集合がほぼ重ならない。
- 新しい話題なら、既存ノードを残したまま新しい role="main" の topic/cluster ノードを作り、その区間の branch / term はその新 main 配下へ置く。旧 main の detail に無理に混ぜない。
- 既存話題の続きなら、新しい main を増やさず、該当 main 配下の branch / term / edge として追加・更新する。
- 複数話題が同じ区間に混ざる場合は、それぞれ最も近い main クラスターに振り分ける。どの既存 main にも自然に属さない内容だけ新しい main にする。
- 別 main クラスター間の edge は、因果・比較・前提・反論・同一人物/同一作品など、明確な意味がある場合だけ作る。単に録音内で隣り合っただけの話題を edge で結ばない。

ノード:
- 主次を必ず分ける。role="main" は講義の主要課題・章・観点を代表するノードにし、内容が増えても少数の冒頭ノードだけに固定し続けない。
- role="branch" の分岐ノードは必ず parent_id で最も近い主ノードに接続し、主ノードなしの孤立分岐を作らない。
- 新しい語が出ても、既存主ノードの detail や分岐に収まるなら新しい主ノードにしない。講義の大きな論点が変わった時だけ主ノードを増やす。
- ノード数は固定目標で決めない。大きな論点・人物・出来事・制度・因果上の転換点は構造ノードとして扱い、単なる用語や属性は必要な場合だけ小さな用語ノードにする。
- 構造ノードか用語ノードかは次で判断する。
  - 構造ノード: node_type="structure"。それを外すと流れ・対比・因果・制度関係・人物関係が分かりにくくなる概念。複数の関係を持つ、話題の段階を作る、論点の主語になる、結果や転換点になるもの。
  - 用語ノード: node_type="term"。既存構造ノードを読むための短い定義・別名・属性・背景語。外しても白板の主な流れは壊れないが、知らないとラベルや発言の意味が分かりにくいもの。
  - 判断に迷う場合、今回の区間で関係や展開を担っているなら構造ノード、名前の説明だけなら用語ノードにする。
- 用語ノードは node_type="term"、role="branch"、kind="support" とし、最も近い構造ノード（node_type="structure"）の parent_id を持つ。用語ノードは短い定義・注意点・言い換えだけを書き、別グループにまたがる中心概念にしない。
- 親構造ノードを明確に選べないが全体理解に必要な用語は、parent_id を空文字にした全体用語ノードにする。全体用語ノードは白板の固定端に表示され、edge は持たない。
- 用語ノードは関係を広げるためのノードではない。親構造ノードとだけ接続し、用語ノード同士や別グループへの横断 edge は作らない。
- 用語ノードは「知らないと理解が止まる語」「何度も出る語」「既存構造のラベル理解に必要な語」に限る。出た語をすべてノード化しない。
- 各 node の detail は白板内だけでも最低限理解できるように、講義文脈での役割・条件・注意点を短く具体的に書く。
- 講義内に出た概念は source_type="lecture" とし、source_excerpt に根拠となる短い発話断片を書く。
- 理解に役立つ標準的な背景知識・関連概念は必要に応じて少数追加してよいが、必ず source_type="external" とし、external_source に確認可能な出典を書く。外部補足ノードは原則 branch にし、detail の末尾にも外部補足だと分かる表現を入れる。
- 出典を示せない外部補足、具体値や固有事実の断定、講義から離れすぎた発展は追加しない。

レイアウト:
- layout は内容に合わせて選ぶ。中心放射に見せるためだけに hub を選ばない。
- 明確な時系列・手順・因果・継承・発展の流れがある場合は flow を優先する。
- hub は、単一の中心概念を軸に複数の主ノードが放射する構造が本当に自然な場合だけ使う。
- 二項以上の対比が講義の中心の場合だけ compare。
- 反復循環が明示された場合だけ cycle。
- 主ノード同士が独立した並列論点の場合だけ grid。
- layout を変えるために無理にノードや edge を増やさない。
- nodes 配列の順序は視覚上の読解順序として扱われる。main ノードを先に、学習・叙事・因果の自然な順序で並べ、branch ノードはできるだけ所属する main の直後に置く。
- 主ノード同士の相互関係が薄い場合は grid、比較軸が明確なら compare を選ぶ。複数課題が並ぶだけなら、無理に一本道の flow にしない。

エッジ:
- edges は因果、流れ、対比、包含、条件など、見れば理解が早くなる関係だけを入れる。
- 強い関連を持つ概念はできるだけ同じ主ノード配下へまとめ、別主ノード配下の横断 edges は重要な因果・対比・条件・制度上の接続に限定する。
- 弱い関連、単なる連想、知識を増やすためだけのリンクは作らず summary_markdown/terms に回す。
- parent_id だけで主従関係が十分分かる場合は、同じ関係を edge で重複表現しない。
- 用語ノードに edge を入れる場合は親構造ノードとだけ接続し、label は空文字にする。
- edge の label は、関係を明示した方が読みやすい場合だけ具体的な関係語を書く。構造維持用の単純な edge は label を空文字にしてよい。単に「関連」「説明」「補足」だけにしない。
- edge は読み取りを助ける関係だけにする。横断 edge は、別グループをつなぐ意味が明確な因果・対比・条件・制度上の接続に限る。
- core→support は「具体例」「条件」「手順」「背景」など展開の種類を書く。
- support→result / core→result は「導く」「結論」「効果」「適用」など結果へのつながりを書く。
- question を含む edge は「確認点」「未解決」「答え」など疑問の扱いが分かる語を書く。
- result 同士は強い推論でなければ「並列」「比較」「まとめ」など中立的に書き、「導く」を安易に使わない。
- 外部補足を含む edge は「背景」「参考」「比較」など、講義内事実と外部補足の役割差が分かる語にする。
- title には「復習」という語を避け、知識整理・概念整理として自然な短い題名を付ける。

"#
    .to_string();
    if is_free_note {
        prompt = prompt
            .replace("講義内容", "録音内容")
            .replace("講義内", "録音内")
            .replace("講義文脈", "録音文脈")
            .replace("講義の大きな論点", "録音の大きな話題");
        prompt.push_str(
            "\n自由ノートでは source_type=\"lecture\" を「録音内に出た内容」という意味で使う。録音内容が非学術的でも、人物関係・出来事・ルール・話題の構造がある場合は whiteboard を作り、整理対象外にしない。\n",
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
                "{}\n\nこれまでの全分割要約と用語注釈（累積素材）:\n{}\n\n現在の累積知識整理ボード:\n{}\n\n今回新しく生成された区間の要約と用語:\n{}\n\n今回の文字起こし（補助参考、必要に応じて細部を拾う。長すぎる場合は末尾のみ表示）:\n{}\n\n指示: 上記の累積素材すべてと今回区間を統合して、現在までの累積 whiteboard を JSON で返す。既存ノードは絶対に削らず、id を維持し、純増させること。追加前に今回区間が既存 main クラスターの続きか、新しい話題・章・素材・動画・論点かを判定し、新しい話題なら新しい main クラスターへ、続きなら既存 main 配下へ配置すること。",
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

    let range_label = format!("{}-{}", format_time(range_start), format_time(range_end));
    let chunk_ai =
        summarize_chunk(&course, &lines, &recent_summaries, &range_label).await?;
    let reconciled_board =
        reconcile_whiteboard(latest_whiteboard(&recent_summaries), chunk_ai.whiteboard);

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
        range_label: range_label.clone(),
        body: chunk_ai.body,
        line_count: lines.len(),
        terms: chunk_ai.terms,
        whiteboard: reconciled_board,
    };
    Arc::make_mut(&mut session.summaries).push(summary);
    Arc::make_mut(&mut session.pending_lines).clear();
    session.batch_started_at = now;
    Ok(session.snapshot())
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
    let (finish_started_at, pending_line_count) = {
        let guard = state
            .0
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?;
        let Some(session) = guard.as_ref() else {
            return Err("Liveセッションが開始されていません".to_string());
        };
        (session.started_at, session.pending_lines.len())
    };
    let finish_started_check_at = Local::now();
    if should_require_finish_chunk_ai(
        finish_started_at,
        finish_started_check_at,
        pending_line_count,
    ) {
        flush_session_summary(&state, true).await?;
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
    let ai_config = crate::ai::load_ai_config();
    let reply_language = ai_config.reply_language.clone();
    let should_run_finish_ai = should_run_finish_ai(&ai_config.provider, started_at, ended_at);
    let overall_summary = if should_skip_ai_summarization(started_at, ended_at) {
        short_session_overall_summary(&course, transcript_lines.len(), &reply_language)
    } else if !should_run_finish_ai {
        fallback_overall_summary(
            &course,
            transcript_lines.len(),
            summaries.len(),
            &reply_language,
        )
    } else {
        summarize_overall(&course, &summaries, &transcript_lines)
            .await
            .unwrap_or_else(|_| {
                fallback_overall_summary(
                    &course,
                    transcript_lines.len(),
                    summaries.len(),
                    &reply_language,
                )
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
    let suggested_todos = if should_skip_ai_summarization(started_at, ended_at)
        || !should_run_finish_ai
    {
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
        assert!(board_prompt.contains("\"role\":\"main|branch\""));
        assert!(board_prompt.contains("\"node_type\":\"structure|term\""));
        assert!(board_prompt.contains("\"source_type\":\"lecture|external\""));
        assert!(board_prompt.contains("whiteboard JSON のみ"));
        assert!(board_prompt.contains("正しい出力"));
        assert!(board_prompt.contains("ノードや edge の量は固定目標ではなく"));
        assert!(board_prompt.contains("ノード総数に上限はない"));
        assert!(board_prompt.contains("原則すべてそのまま今回の出力にも含める"));
        assert!(board_prompt.contains("減らしてはいけない"));
        assert!(board_prompt.contains("既存ノードを更新しない"));
        assert!(board_prompt.contains("情報の精密化・訂正・関係整理"));
        assert!(board_prompt.contains("旧話題の圧縮禁止"));
        assert!(board_prompt.contains("「（前区間の内容）」"));
        assert!(board_prompt.contains("id 安定性"));
        assert!(board_prompt.contains("social-gaming → social-interaction"));
        assert!(board_prompt.contains("別の main クラスターとして追加する"));
        assert!(board_prompt.contains("話題境界"));
        assert!(board_prompt.contains("別の動画/会話/教材に切り替わった形跡"));
        assert!(board_prompt.contains("既存 main クラスターの続き"));
        assert!(board_prompt.contains("中心放射に見せるためだけに hub を選ばない"));
        assert!(board_prompt.contains("小さな用語ノード"));
        assert!(board_prompt.contains("node_type=\"term\""));
        assert!(board_prompt.contains("構造ノードか用語ノードかは次で判断する"));
        assert!(board_prompt.contains("名前の説明だけなら用語ノード"));
        assert!(board_prompt.contains("全体用語ノード"));
        assert!(board_prompt.contains("用語ノードに edge を入れる場合は親構造ノードとだけ接続"));
        assert!(board_prompt.contains("result 同士"));
        assert!(board_prompt.contains("非空 edge.label 也必须使用简体中文"));

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

        let board_prompt =
            live_whiteboard_system_prompt(live_whiteboard_language_instruction("zh"), true);
        assert!(board_prompt.contains("録音内容"));
        assert!(board_prompt.contains("整理対象外にしない"));

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
