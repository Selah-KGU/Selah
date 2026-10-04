use regex::Regex;
use std::sync::LazyLock;

static SESSION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"第\s*(\d+)\s*(回|週|章|講|課)").unwrap());
static LESSON_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(lesson|week|unit|chapter|day)\s*0*(\d+)").unwrap());
static TOPIC_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(小テスト|レポート|報告書|報告|課題|宿題|試験|クイズ|演習|quiz|report|assignment|exam)")
        .unwrap()
});

/// Theme key + display label for a document. A session/lesson marker wins (it
/// groups everything from the same 回 together, with a zero-padded 第NN回 label
/// so it merges with the AI's session folders); otherwise a topic word; failing
/// both, the document kind. The summary is searched too, so a note whose title
/// is just a date still finds its 第N回 when the body mentions it.
pub(in crate::course_automation) fn theme_of(
    title: &str,
    summary: &str,
    filename: &str,
    kind: &str,
) -> (String, String) {
    // Normalize full-width digits (第１０回) to ASCII (第10回) up front: Rust's
    // `\d` matches both, but `str::parse::<u32>` only accepts ASCII, so without
    // this a full-width marker skips zero-padding and forks a separate folder
    // from its half-width twin.
    let text = normalize_fullwidth_digits(&format!("{title} {filename} {summary}"));
    if let Some(caps) = SESSION_RE.captures(&text) {
        let n = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
        let unit = caps.get(2).map(|m| m.as_str()).unwrap_or("回");
        // Zero-pad every unit (第03回 / 第03週) so labels sort and merge uniformly.
        let label = match n.parse::<u32>() {
            Ok(num) => format!("第{num:02}{unit}"),
            _ => format!("第{n}{unit}"),
        };
        return (format!("session:{label}"), label);
    }
    if let Some(caps) = LESSON_RE.captures(&text) {
        let word = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
        let n = caps.get(2).map(|m| m.as_str()).unwrap_or_default();
        let label = format!("{} {}", title_case(word), n);
        return (format!("session:{}", label.to_lowercase()), label);
    }
    if let Some(caps) = TOPIC_RE.captures(&text) {
        let word = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
        let label = canonical_topic(word);
        return (format!("topic:{label}"), label.to_string());
    }
    (format!("kind:{kind}"), kind_label(kind))
}

/// Canonicalizes a session label so the AI planner's grouping merges with the
/// heuristic's: a label that is (or contains) a 第N回 marker becomes the padded,
/// half-width `第NN回` form the heuristic emits; anything else is just trimmed.
/// Keeps `第3回` / `第１０回` / `第3回 オリエン` from forking a second folder next
/// to `第03回` / `第10回`.
pub fn canonical_session_label(label: &str) -> String {
    detect_session(label).unwrap_or_else(|| label.trim().to_string())
}

/// The canonical `第NN回` label if `text` contains a session marker (full-width
/// tolerant), else None. Lets callers cheaply tell whether a notice pins a
/// session without pulling in the regex.
pub fn detect_session(text: &str) -> Option<String> {
    let normalized = normalize_fullwidth_digits(text);
    let caps = SESSION_RE.captures(&normalized)?;
    let n = caps.get(1)?.as_str();
    let unit = caps.get(2).map(|m| m.as_str()).unwrap_or("回");
    let num: u32 = n.parse().ok()?;
    Some(format!("第{num:02}{unit}"))
}

/// Maps full-width digits (０-９) to ASCII so session markers parse and pad the
/// same regardless of which width the source used.
fn normalize_fullwidth_digits(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '０'..='９' => char::from_u32(c as u32 - '０' as u32 + '0' as u32).unwrap_or(c),
            _ => c,
        })
        .collect()
}

/// Canonical Japanese label for a topic match, so an English hit (`report`,
/// `quiz`, `exam`) and its Japanese synonym land in the same folder instead of
/// forking a `report` folder next to a `レポート` one. Unknown words pass through.
fn canonical_topic(word: &str) -> &'static str {
    match word.to_lowercase().as_str() {
        "レポート" | "報告" | "報告書" | "report" => "レポート",
        "課題" | "宿題" | "assignment" => "課題",
        "小テスト" | "クイズ" | "quiz" => "小テスト",
        "試験" | "exam" => "試験",
        "演習" => "演習",
        _ => "課題",
    }
}

fn title_case(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
        None => String::new(),
    }
}

fn kind_label(kind: &str) -> String {
    match kind {
        "material" => "教材".into(),
        "announcement" => "お知らせ".into(),
        "report" => "課題".into(),
        other if !other.is_empty() => other.to_string(),
        _ => "資料".into(),
    }
}

/// Safe single path component: strips separators / reserved characters and bounds
/// the length so a theme label can never escape its folder.
pub(in crate::course_automation) fn sanitize_component(label: &str) -> String {
    let cleaned: String = label
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    trimmed.chars().take(60).collect()
}
