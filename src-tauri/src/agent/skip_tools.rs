//! Decides when a turn can answer without tools.
//!
//! Smalltalk and contextual follow-ups skip planning. Live-note lookups can
//! still chain into reading the matching markdown.

use super::*;

// ─────────────────────── Skip-Tool Detection ───────────────────────

pub(super) fn should_skip_tools(history: &[crate::db::AgentMessageRow], user_text: &str) -> bool {
    let norm = normalize_planner_text(user_text);
    is_smalltalk_or_identity(&norm) || is_follow_up_with_context(history, &norm)
}

fn is_smalltalk_or_identity(norm: &str) -> bool {
    if norm.is_empty() {
        return true;
    }
    // Pure greetings / acknowledgements — never need a tool.
    const SMALLTALK: &[&str] = &[
        "こんにちは",
        "こんばんは",
        "おはよう",
        "ありがと",
        "ありがとう",
        "thanks",
        "thankyou",
        "你好",
        "您好",
        "谢谢",
        "嗨",
        "hello",
        "hi",
        "hey",
        "元気",
        "howareyou",
    ];
    // "Who are you / introduce yourself" style — answer comes from persona only.
    const IDENTITY: &[&str] = &[
        "あなたは誰",
        "君は誰",
        "是谁",
        "你是谁",
        "whoareyou",
        "自己紹介",
        "介绍一下自己",
    ];
    // Pure opinion / feeling questions about the assistant. Kept very short and
    // generic so utterances like "経済学が好き" with concrete subjects still
    // fall through to the planner.
    const OPINION: &[&str] = &["どう思う", "怎么看", "意见", "意見"];
    let short = norm.chars().count() <= 24;
    let very_short = norm.chars().count() <= 10;
    if short && contains_any(norm, SMALLTALK) {
        return true;
    }
    if short && contains_any(norm, IDENTITY) {
        return true;
    }
    if very_short && contains_any(norm, OPINION) {
        return true;
    }
    false
}

#[cfg(test)]
pub(super) fn recent_downloaded_file_path(
    history: &[crate::db::AgentMessageRow],
) -> Option<String> {
    history
        .iter()
        .rev()
        .filter(|row| row.role == "tool")
        .find_map(|row| {
            let name = row.tool_name.as_deref()?;
            if name != "list_downloaded_files" {
                return None;
            }
            let raw = row.tool_result_json.as_deref()?;
            let parsed: Value = serde_json::from_str(raw).ok()?;
            parsed
                .get("files")
                .and_then(|v| v.as_array())
                .and_then(|items| items.first())
                .and_then(|file| file.get("path"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
}

pub(super) fn should_auto_read_live_note(user_text: &str, tool_name: &str) -> bool {
    if tool_name != "list_downloaded_files" {
        return false;
    }
    let norm = normalize_planner_text(user_text);
    contains_any(
        &norm,
        &[
            "讲义",
            "講義",
            "讲了什么",
            "講了什麼",
            "说了什么",
            "說了什麼",
            "上课内容",
            "上課內容",
            "这节课",
            "這節課",
            "授業内容",
            "講義内容",
            "ノート",
            "课堂笔记",
            "課堂筆記",
            "内容",
            "要点",
            "重點",
            "live",
        ],
    )
}

pub(super) fn preferred_live_courses(user_text: &str, results: &[(String, Value)]) -> Vec<String> {
    let norm = normalize_planner_text(user_text);
    let wants_afternoon = contains_any(&norm, &["下午", "午後", "afternoon"]);
    let wants_morning = contains_any(&norm, &["上午", "午前", "morning"]);

    results
        .iter()
        .find_map(|(name, value)| {
            if name != "list_today_classes" {
                return None;
            }
            let classes = value.get("classes")?.as_array()?;
            let mut picked: Vec<(i64, String)> = classes
                .iter()
                .filter(|class| {
                    if class
                        .get("cancelled")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        return false;
                    }
                    let period = class.get("period").and_then(|v| v.as_i64()).unwrap_or(0);
                    if wants_afternoon {
                        return period >= 3;
                    }
                    if wants_morning {
                        return period > 0 && period <= 2;
                    }
                    true
                })
                .filter_map(|class| {
                    let period = class.get("period").and_then(|v| v.as_i64()).unwrap_or(0);
                    let name = class.get("name").and_then(|v| v.as_str())?.trim();
                    if name.is_empty() {
                        return None;
                    }
                    Some((period, name.to_string()))
                })
                .collect();

            if wants_afternoon {
                picked.sort_by_key(|(period, _)| *period);
            }

            Some(picked.into_iter().map(|(_, name)| name).collect::<Vec<_>>())
        })
        .unwrap_or_default()
}

pub(super) fn pick_live_markdown_path(
    result: &Value,
    preferred_courses: &[String],
) -> Option<String> {
    let files = result.get("files")?.as_array()?;
    let preferred_norms = preferred_courses
        .iter()
        .map(|name| normalize_planner_text(name))
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();

    fn score(file: &Value, preferred_norms: &[String]) -> i64 {
        let filename = file
            .get("filename")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let path = file
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let source = file
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let joined = normalize_planner_text(&format!("{} {}", filename, path));

        let mut score = 0_i64;
        if source == "live" {
            score += 5;
        }
        if filename.ends_with(".md") {
            score += 2;
        }
        if filename.contains("_live.md") || path.contains("_live.md") {
            score += 6;
        }
        if filename.contains("live") || path.contains("live") {
            score += 2;
        }
        for course in preferred_norms {
            if joined.contains(course) {
                score += 20;
            }
        }
        if let Some(downloaded_at) = file.get("downloaded_at").and_then(|v| v.as_i64()) {
            score += downloaded_at / 1_000_000_000;
        }
        score
    }

    files
        .iter()
        .filter_map(|file| {
            let path = file.get("path").and_then(|v| v.as_str())?;
            let filename = file
                .get("filename")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();
            let path_lower = path.to_lowercase();
            if !filename.ends_with(".md") && !path_lower.ends_with(".md") {
                return None;
            }
            Some((score(file, &preferred_norms), path.to_string()))
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, path)| path)
}

fn is_follow_up_with_context(history: &[crate::db::AgentMessageRow], norm: &str) -> bool {
    if !history.iter().rev().take(6).any(|row| row.role == "tool") {
        return false;
    }
    const DETAIL_MARKERS: &[&str] = &[
        "詳しく",
        "详细",
        "详细一点",
        "もう少し",
        "为什么",
        "為什麼",
        "怎么说",
        "什么意思",
        "哪个",
        "哪個",
        "whichone",
        "why",
        "moredetail",
        "continue",
        "続けて",
        "もっと",
        "具体的に",
        "ほかに",
        "他に",
        "还有",
        "另外",
        "第一",
        "第二",
        "第三",
        "最初",
        "最後",
        "pdf",
        "doc",
        "docx",
        "ファイル",
        "附件",
        "本文",
        "添付",
        // Calendar / action words — a short message that contains both an
        // acknowledgement and a directive (e.g. "了解、日历加一下") must still
        // trigger tool planning, not be silently swallowed.
        "日历",
        "カレンダー",
        "calendar",
        "加进",
        "加入",
        "追加",
        "登録",
        "削除",
        "删除",
        "编辑",
        "修改",
        "変更",
        "更新",
    ];
    if contains_any(norm, DETAIL_MARKERS) {
        return false;
    }
    const ACK_MARKERS: &[&str] = &[
        "ありがと",
        "ありがとう",
        "谢谢",
        "thanks",
        "thankyou",
        "ok",
        "わかった",
        "了解",
        "助かった",
        "收到",
        "明白",
        "なるほど",
        // Short CJK acknowledgements that never start an action sequence.
        // Note: "好", "行", "加", "要", "可以" are intentionally excluded because
        // they frequently serve as directives ("好，加进日历") that should still
        // trigger tool calls.
        "嗯",         // uh-huh / mm-hmm (Chinese)
        "そうですか", // I see / is that so (Japanese)
        "そうか",     // I see (Japanese)
    ];
    norm.chars().count() <= 24 && contains_any(norm, ACK_MARKERS)
}
