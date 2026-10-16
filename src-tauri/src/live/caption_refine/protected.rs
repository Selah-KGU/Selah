//! Conservative checks for explicit polarity and numeric facts, not semantic NLI.
use regex::Regex;
use std::sync::LazyLock;

pub(super) fn preserved(source: &str, corrected: &str) -> bool {
    negations(source) == negations(corrected)
        && quantities(source) == quantities(corrected)
        && honorifics(source) == honorifics(corrected)
}

fn honorifics(text: &str) -> Vec<String> {
    static NAME_SUFFIX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?:[\p{Han}]{2,}|[A-Z][a-z]+)(さん|さま|様|くん|君|氏|先生|博士)").unwrap()
    });
    NAME_SUFFIX
        .captures_iter(text)
        .map(|capture| capture[1].to_string())
        .collect()
}

fn negations(text: &str) -> usize {
    static NEGATIVE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r"(?i)\b(?:not|never|no|without|cannot)\b|n['’]t\b|ません|なかった|なけれ|なく|ない|不|没|沒|無|无|未能|未曾|未完成|未提交|未确认|未確認|않|없|못(?:\s|$)|(?:^|\s)안(?:\s|$)"
    ).unwrap()
    });
    NEGATIVE
        .find_iter(text)
        .filter(|found| {
            // “No. 5” labels an item; it is not a negative statement.
            !(found.as_str().eq_ignore_ascii_case("no")
                && text[found.end()..]
                    .trim_start()
                    .strip_prefix('.')
                    .is_some_and(|tail| {
                        tail.trim_start()
                            .starts_with(|ch: char| ch.is_ascii_digit())
                    }))
        })
        .count()
}

fn quantities(text: &str) -> Vec<(String, String, String)> {
    static NUMBER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"[+-]?[0-9]+(?:\.[0-9]+)?").unwrap());
    static BEFORE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r"(?i)(>=|<=|>|<|≥|≤|至少|不少于|不低于|至多|不超过|不高于|超过|大于|高于|低于|小于|at\s+least|at\s+most|more\s+than|less\s+than|no\s+less\s+than|no\s+more\s+than)\s*$"
    ).unwrap()
    });
    let text: String = text
        .chars()
        .map(super::quality::normalize_numeric_char)
        .collect();
    NUMBER
        .find_iter(&text)
        .map(|number| {
            let tail = text[number.end()..].trim_start();
            let (unit, remaining) = unit(tail);
            let suffix = [
                ("以上", "ge"),
                ("以下", "le"),
                ("未満", "lt"),
                ("より大き", "gt"),
                ("より小さ", "lt"),
                ("超", "gt"),
            ]
            .iter()
            .find_map(|(prefix, value)| {
                remaining.trim_start().starts_with(prefix).then_some(*value)
            });
            let prefix = BEFORE.captures(&text[..number.start()]).map(|capture| {
                let phrase = capture[1]
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_ascii_lowercase();
                match phrase.as_str() {
                    ">=" | "≥" | "至少" | "不少于" | "不低于" | "at least" | "no less than" => {
                        "ge"
                    }
                    "<=" | "≤" | "至多" | "不超过" | "不高于" | "at most" | "no more than" => {
                        "le"
                    }
                    ">" | "超过" | "大于" | "高于" | "more than" => "gt",
                    _ => "lt",
                }
            });
            (
                number.as_str().to_string(),
                unit.to_string(),
                suffix.or(prefix).unwrap_or("").to_string(),
            )
        })
        .collect()
}

fn unit(tail: &str) -> (&'static str, &str) {
    // SI case is intentional: mW and MW must not collapse into the same unit.
    const UNITS: &[(&str, &str)] = &[
        ("パーセント", "%"),
        ("percent", "%"),
        ("%", "%"),
        ("kilograms", "kg"),
        ("kilogram", "kg"),
        ("milligrams", "mg"),
        ("milligram", "mg"),
        ("キログラム", "kg"),
        ("ミリグラム", "mg"),
        ("グラム", "g"),
        ("kg", "kg"),
        ("mg", "mg"),
        ("µg", "ug"),
        ("μg", "ug"),
        ("ug", "ug"),
        ("g", "g"),
        ("kilometers", "km"),
        ("kilometer", "km"),
        ("centimeters", "cm"),
        ("centimeter", "cm"),
        ("millimeters", "mm"),
        ("millimeter", "mm"),
        ("meters", "m"),
        ("meter", "m"),
        ("キロメートル", "km"),
        ("センチメートル", "cm"),
        ("ミリメートル", "mm"),
        ("メートル", "m"),
        ("km", "km"),
        ("cm", "cm"),
        ("mm", "mm"),
        ("m", "m"),
        ("milliseconds", "ms"),
        ("millisecond", "ms"),
        ("seconds", "s"),
        ("second", "s"),
        ("minutes", "min"),
        ("minute", "min"),
        ("hours", "h"),
        ("hour", "h"),
        ("ms", "ms"),
        ("min", "min"),
        ("s", "s"),
        ("h", "h"),
        ("kHz", "kHz"),
        ("MHz", "MHz"),
        ("Hz", "Hz"),
        ("mW", "mW"),
        ("MW", "MW"),
        ("kW", "kW"),
        ("W", "W"),
        ("mV", "mV"),
        ("V", "V"),
        ("mA", "mA"),
        ("A", "A"),
        ("GB", "GB"),
        ("MB", "MB"),
        ("KB", "KB"),
        ("B", "B"),
        ("℃", "C"),
        ("°C", "C"),
        ("度", "度"),
        ("万円", "万円"),
        ("円", "円"),
        ("点", "点"),
        ("人", "人"),
        ("個", "個"),
        ("回", "回"),
        ("時間", "h"),
        ("年", "年"),
        ("月", "月"),
        ("日", "日"),
        ("時", "時"),
        ("分", "min"),
        ("秒", "s"),
    ];
    for (spelling, canonical) in UNITS {
        if let Some(rest) = tail.strip_prefix(spelling) {
            if spelling.ends_with(|ch: char| ch.is_ascii_alphabetic())
                && rest.starts_with(|ch: char| ch.is_ascii_alphabetic())
            {
                continue;
            }
            return (canonical, rest);
        }
    }
    ("", tail)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_polarity_units_and_comparison_directions_are_protected() {
        for (before, after) in [
            ("この方法は推奨していません。", "この方法は推奨しています。"),
            ("这个结论不成立。", "这个结论成立。"),
            ("We do not publish the key.", "We publish the key."),
            ("여기서는 저장하지 않습니다.", "여기서는 저장합니다."),
            ("60点以上なら合格です。", "60点以下なら合格です。"),
            ("Values must be at least 60.", "Values must be at most 60."),
            ("温度超过30度才开始。", "温度低于30度才开始。"),
            ("質量は10mgです。", "質量は10gです。"),
            ("The distance is 5km.", "The distance is 5m."),
            ("Power is 5mW.", "Power is 5MW."),
            ("担当者は田中晴人さんです。", "担当者は田中晴人です。"),
        ] {
            assert!(!preserved(before, after), "{before} -> {after}");
        }
    }
    #[test]
    fn equivalent_spellings_do_not_look_like_fact_changes() {
        for (before, after) in [
            (
                "この方法は推奨していません。",
                "この方法は推奨していません。",
            ),
            ("This cannot happen.", "This can not happen."),
            ("No. 5 is ready.", "Item 5 is ready."),
            ("距離は5キロメートルです。", "距離は5kmです。"),
            ("変化率は−１．５％です。", "変化率は-1.5%です。"),
            ("Dr. Smith waited 1.5 seconds.", "Dr. Smith waited 1.5 s."),
        ] {
            assert!(preserved(before, after), "{before} -> {after}");
        }
    }
}
