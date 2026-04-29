use super::super::*;

fn normalize_notice_body_lines(s: &str) -> Vec<String> {
    static RE_TAGS: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?is)<[^>]+>").expect("valid regex"));

    let normalized = s
        .replace("<br />", "\n")
        .replace("<br/>", "\n")
        .replace("<br>", "\n")
        .replace("</p>", "\n")
        .replace("</div>", "\n")
        .replace("</li>", "\n")
        .replace("&nbsp;", " ");
    let plain = RE_TAGS.replace_all(&normalized, "");
    plain
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect()
}

pub(crate) fn is_blacklisted_system_notice_text(s: &str) -> bool {
    let text = s.trim();
    if text.is_empty() {
        return false;
    }

    // Luna occasionally returns university-wide support/maintenance notices in place
    // of the actual activity body on the first request. These are stable enough to
    // blacklist explicitly instead of further tightening the structural parser.
    const STRONG_PATTERNS: &[&str] = &[
        "ゲストアクセス」と「履修登録」は違います",
        "履修データ連携に関する補足",
        "このセッションの表示アクセス権がありません。",
        "学生キャビネット ＞ 教務機構 ＞ LUNA・ポートフォリオ",
        "macを利用されている学生は注意ください",
        "LUNAサポートへお問い合わせいただく前に",
        "メンテナンス時間帯において、接続が切れる場合があります。",
    ];
    if STRONG_PATTERNS.iter().any(|pattern| text.contains(pattern)) {
        return true;
    }

    let grouped_patterns: &[&[&str]] = &[
        &["時間割", "ゲストアクセス", "履修登録"],
        &["KG Chatbot", "学生向け動画マニュアル"],
        &["Panoptoボタン", "アクセス権をリクエスト"],
        &[
            "Panoptoボタン",
            "このセッションの表示アクセス権がありません。",
        ],
        &["教務連携スケジュール", "履修データ連携"],
        &["LUNAの定期メンテナンスについて", "AM2:00 - AM2:30"],
        &["学生キャビネット", "LUNA・ポートフォリオ"],
    ];
    grouped_patterns
        .iter()
        .any(|group| group.iter().all(|pattern| text.contains(pattern)))
}

fn is_system_notice_line(line: &str) -> bool {
    const LINE_PATTERNS: &[&str] = &[
        "時間割",
        "ゲストアクセス",
        "履修登録",
        "履修データ連携",
        "教務連携スケジュール",
        "このセッションの表示アクセス権がありません。",
        "アクセス権をリクエスト",
        "学生キャビネット",
        "LUNA・ポートフォリオ",
        "KG Chatbot",
        "学生向け動画マニュアル",
        "LUNAの定期メンテナンスについて",
        "メンテナンス時間帯において、接続が切れる場合があります。",
        "LUNAサポートへお問い合わせいただく前に",
        "macを利用されている学生は注意ください",
        "AM2:00 - AM2:30",
    ];
    LINE_PATTERNS.iter().any(|pattern| line.contains(pattern))
}

pub(super) fn sanitize_blacklisted_notice_body(body: &str) -> Option<String> {
    let body = body.trim();
    if body.is_empty() {
        return None;
    }
    if !is_blacklisted_system_notice_text(body) {
        return Some(body.to_string());
    }

    let kept_lines: Vec<String> = normalize_notice_body_lines(body)
        .into_iter()
        .filter(|line| !is_system_notice_line(line))
        .collect();
    if kept_lines.is_empty() {
        return None;
    }

    let candidate = kept_lines.join("\n").trim().to_string();
    if candidate.is_empty() || is_blacklisted_system_notice_text(&candidate) {
        return None;
    }
    Some(candidate)
}
