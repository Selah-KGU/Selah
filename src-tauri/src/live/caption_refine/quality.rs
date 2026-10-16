//! Reject loss of speech content and learn only edits evidenced by both texts.
use std::collections::BTreeMap;

pub(super) fn acceptable_correction(source: &str, corrected: &str) -> bool {
    if !acceptable_fragment(source, corrected) {
        return false;
    }
    let before = super::protocol::parts(source);
    let after = super::protocol::parts(corrected);
    if before.len() > 1 {
        before.len() == after.len()
            && !super::protocol::reordered(&before, &after)
            && before
                .iter()
                .zip(&after)
                .all(|(left, right)| acceptable_fragment(left, right))
    } else {
        true
    }
}

fn acceptable_fragment(source: &str, corrected: &str) -> bool {
    let source_len = source.chars().count();
    let corrected_len = corrected.chars().count();
    if source_len == 0
        || corrected_len == 0
        || corrected_len > 4_000
        || corrected_len > source_len * 2 + 24
        || corrected_len + 16 < source_len / 2
        || numbers(source) != numbers(corrected)
        || !super::protected::preserved(source, corrected)
    {
        return false;
    }
    let before = content_chars(source);
    let after = content_chars(corrected);
    if before.is_empty() || after.is_empty() {
        return false;
    }
    // Allow several recognition errors, while rejecting a fluent summary or
    // an unrelated sentence even when its length happens to match the source.
    let budget = (before.len().max(after.len()) * 2 / 5).max(4);
    within_edit_budget(&before, &after, budget)
}

fn content_chars(text: &str) -> Vec<char> {
    text.chars()
        .filter(|ch| ch.is_alphanumeric())
        .map(normalize_numeric_char)
        .flat_map(char::to_lowercase)
        .collect()
}

fn numbers(text: &str) -> Vec<String> {
    let chars: Vec<_> = text.chars().map(normalize_numeric_char).collect();
    let mut runs = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let start = index;
        if matches!(chars[index], '+' | '-')
            && chars.get(index + 1).is_some_and(char::is_ascii_digit)
        {
            index += 1;
        } else if !chars[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        while index < chars.len() {
            if chars[index].is_ascii_digit()
                || (chars[index] == '.' && chars.get(index + 1).is_some_and(char::is_ascii_digit))
            {
                index += 1;
            } else {
                break;
            }
        }
        if chars.get(index) == Some(&'%') {
            index += 1;
        }
        runs.push(chars[start..index].iter().collect());
    }
    runs
}

pub(super) fn normalize_numeric_char(ch: char) -> char {
    match ch {
        '０'..='９' => char::from_u32(ch as u32 - '０' as u32 + '0' as u32).unwrap(),
        '＋' => '+',
        '－' | '−' => '-',
        '．' => '.',
        '％' => '%',
        _ => ch,
    }
}

fn within_edit_budget(before: &[char], after: &[char], budget: usize) -> bool {
    edit_cost(before, after, budget).is_some()
}

fn edit_cost(mut before: &[char], mut after: &[char], budget: usize) -> Option<usize> {
    let prefix = before.iter().zip(after).take_while(|(a, b)| a == b).count();
    before = &before[prefix..];
    after = &after[prefix..];
    let suffix = before
        .iter()
        .rev()
        .zip(after.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    before = &before[..before.len() - suffix];
    after = &after[..after.len() - suffix];
    if before.len().abs_diff(after.len()) > budget {
        return None;
    }
    if before.is_empty() || after.is_empty() {
        let cost = before.len().max(after.len());
        return (cost <= budget).then_some(cost);
    }
    let unreachable = budget + 1;
    let mut previous: Vec<_> = (0..=after.len()).map(|i| i.min(unreachable)).collect();
    let mut row = vec![unreachable; after.len() + 1];
    for (i, left) in before.iter().enumerate() {
        row.fill(unreachable);
        row[0] = (i + 1).min(unreachable);
        let start = (i + 1).saturating_sub(budget).max(1);
        let end = (i + 1 + budget).min(after.len());
        let mut best = row[0];
        for j in start..=end {
            row[j] = (previous[j] + 1)
                .min(row[j - 1] + 1)
                .min(previous[j - 1] + usize::from(*left != after[j - 1]));
            best = best.min(row[j]);
        }
        if best > budget {
            return None;
        }
        std::mem::swap(&mut previous, &mut row);
    }
    let cost = previous[after.len()];
    (cost <= budget).then_some(cost)
}

pub(super) fn learnable(text: &str) -> bool {
    (2..=32).contains(&text.chars().count())
        && text.chars().all(|ch| {
            ch.is_alphanumeric() || matches!(ch, ' ' | '-' | '\'' | '’' | '・' | '+' | '#' | '.')
        })
        && text.chars().any(|ch| {
            ('\u{4e00}'..='\u{9fff}').contains(&ch)
                || ('\u{30a0}'..='\u{30ff}').contains(&ch)
                || ('\u{ac00}'..='\u{d7a3}').contains(&ch)
                || ch.is_ascii_alphabetic()
        })
}

pub(super) fn correction_terms(
    value: Option<&serde_json::Value>,
    source: &str,
    corrected: &str,
) -> Vec<(String, String)> {
    let candidates = match value {
        Some(serde_json::Value::Array(values)) => values
            .iter()
            .take(16)
            .filter_map(|value| {
                Some((
                    value.get("from")?.as_str()?.trim().to_string(),
                    value.get("to")?.as_str()?.trim().to_string(),
                ))
            })
            .collect(),
        // Small/local models may still return the original two-field schema.
        None => contiguous_replacement(source, corrected)
            .into_iter()
            .collect(),
        _ => Vec::new(),
    };
    let mut terms: BTreeMap<String, Option<String>> = BTreeMap::new();
    for (from, to) in candidates {
        if from == to
            || !learnable(&from)
            || !learnable(&to)
            || numbers(&from) != numbers(&to)
            || occurrences(source, &from) <= occurrences(corrected, &from)
            || occurrences(corrected, &to) <= occurrences(source, &to)
            || !aligned_replacement(source, corrected, &from, &to)
        {
            continue;
        }
        terms
            .entry(from)
            .and_modify(|existing| {
                if existing.as_deref() != Some(&to) {
                    *existing = None;
                }
            })
            .or_insert(Some(to));
    }
    terms
        .into_iter()
        .filter_map(|(from, to)| Some((from, to?)))
        .take(8)
        .collect()
}

fn aligned_replacement(source: &str, corrected: &str, from: &str, to: &str) -> bool {
    let before = content_chars(source);
    let after = content_chars(corrected);
    let original = edit_cost(&before, &after, before.len().max(after.len())).unwrap_or(0);
    if original == 0 {
        return false;
    }
    source.match_indices(from).take(8).any(|(start, _)| {
        let candidate = format!(
            "{}{}{}",
            &source[..start],
            to,
            &source[start + from.len()..]
        );
        edit_cost(&content_chars(&candidate), &after, original - 1).is_some()
    })
}

fn occurrences(text: &str, term: &str) -> usize {
    let word_char = |ch: char| ch.is_ascii_alphanumeric() || ch == '_';
    text.match_indices(term)
        .filter(|(index, _)| {
            let end = index + term.len();
            let left_ok = !term.starts_with(word_char) || !text[..*index].ends_with(word_char);
            let right_ok = !term.ends_with(word_char) || !text[end..].starts_with(word_char);
            left_ok && right_ok
        })
        .count()
}

fn contiguous_replacement(before: &str, after: &str) -> Option<(String, String)> {
    let before: Vec<_> = before.chars().collect();
    let after: Vec<_> = after.chars().collect();
    let prefix = before
        .iter()
        .zip(&after)
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = before[prefix..]
        .iter()
        .rev()
        .zip(after[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    if prefix + suffix < 2 {
        return None;
    }
    let from_end = before.len() - suffix;
    let to_end = after.len() - suffix;
    let mut start = prefix;
    while (from_end - start < 2 || to_end - start < 2) && start > 0 {
        start -= 1;
    }
    let from = &before[start..from_end];
    let to = &after[start..to_end];
    if from
        .windows(2)
        .any(|run| to.windows(2).any(|other| run == other))
    {
        return None;
    }
    Some((from.iter().collect(), to.iter().collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn term_pairs_must_explain_the_same_edit_not_cross_two_unrelated_edits() {
        let source = "確立変数と期待地を求めます。";
        let corrected = "確率変数と期待値を求めます。";
        let bad = json!([{"from":"確立変数","to":"期待値"},{"from":"期待地","to":"確率変数"}]);
        assert!(correction_terms(Some(&bad), source, corrected).is_empty());
        let good = json!([{"from":"確立変数","to":"確率変数"},{"from":"期待地","to":"期待値"}]);
        assert_eq!(correction_terms(Some(&good), source, corrected).len(), 2);
        let korean = json!([{"from":"표준 편자","to":"표준 편차"}]);
        assert_eq!(
            correction_terms(
                Some(&korean),
                "표준 편자를 계산합니다.",
                "표준 편차를 계산합니다."
            )
            .len(),
            1
        );
    }

    #[test]
    fn several_homophones_can_be_fixed_without_rewriting_the_sentence() {
        assert!(acceptable_correction(
            "確立変数の期待地を求め、標準変差を計算します。",
            "確率変数の期待値を求め、標準偏差を計算します。"
        ));
        assert!(acceptable_correction(
            "This is a neuron network.",
            "This is a neural network."
        ));
        assert!(acceptable_correction(
            "我们用最小二成法计算回归系数。",
            "我们用最小二乘法计算回归系数。"
        ));
    }

    #[test]
    fn fluent_summaries_and_unrelated_same_length_replies_are_rejected() {
        assert!(!acceptable_correction(
            "今日は確率変数を定義します。次に期待値を求めます。最後に分散を計算します。",
            "今日は確率と統計の基本的な概念について学びます。"
        ));
        assert!(!acceptable_correction(
            "今日は確率分布の性質を説明します。",
            "明日は文学作品の歴史を勉強します。"
        ));
    }

    #[test]
    fn numbers_cannot_be_added_removed_reordered_or_guessed() {
        let source = "2026年は3回、1.5倍で計算します。";
        for invalid in [
            "2025年は3回、1.5倍で計算します。",
            "2026年は2回、1.5倍で計算します。",
            "2026年は3回、15倍で計算します。",
            "2026年は3回で計算します。",
            "2026年は3回、1.5倍で計算します。5回ではありません。",
        ] {
            assert!(!acceptable_correction(source, invalid), "{invalid}");
        }
        assert!(acceptable_correction(
            "２０２６年は３回です。",
            "2026年は3回です。"
        ));
        assert!(!acceptable_correction(
            "変化率は-1.5%です。",
            "変化率は1.5%です。"
        ));
        assert!(!acceptable_correction(
            "変化率は1.5%です。",
            "変化率は1.5です。"
        ));
        assert!(acceptable_correction(
            "変化率は−１．５％です。",
            "変化率は-1.5%です。"
        ));
    }

    #[test]
    fn explicit_terms_learn_multiple_edits_but_reject_unsupported_suggestions() {
        let terms = json!([
            {"from":"確立変数", "to":"確率変数"},
            {"from":"期待地", "to":"期待値"},
            {"from":"存在しない語", "to":"分散"},
            {"from":"HTTP", "to":"HTTPS"},
            {"from":"です", "to":"ます"},
            {"from":"3回", "to":"4回"}
        ]);
        assert_eq!(
            correction_terms(
                Some(&terms),
                "確立変数の期待地です。HTTPSURLを3回使います。",
                "確率変数の期待値です。HTTPSURLを3回使います。"
            ),
            vec![
                ("期待地".into(), "期待値".into()),
                ("確立変数".into(), "確率変数".into())
            ]
        );
    }

    #[test]
    fn unchanged_words_and_conflicting_mappings_cannot_enter_the_glossary() {
        let terms = json!([
            {"from":"確立", "to":"確率"},
            {"from":"確立", "to":"確定"},
            {"from":"分散", "to":"期待値"}
        ]);
        assert!(correction_terms(
            Some(&terms),
            "確立と確立と分散です。",
            "確率と確定と分散と期待値です。"
        )
        .is_empty());
    }

    #[test]
    fn legacy_outputs_still_learn_one_term_but_explicitly_empty_terms_are_respected() {
        let source = "今日は確立の定義です";
        let corrected = "今日は確率の定義です";
        assert_eq!(
            correction_terms(None, source, corrected),
            vec![("確立".into(), "確率".into())]
        );
        assert!(correction_terms(Some(&json!([])), source, corrected).is_empty());
        assert!(
            correction_terms(None, "今日は確立の話と分散です", "今日は確率の話と方差です")
                .is_empty()
        );
    }

    #[test]
    fn latin_terms_obey_token_boundaries_including_underscores() {
        let terms = json!([{"from":"HTTP", "to":"HTTPS"}]);
        assert!(
            correction_terms(Some(&terms), "HTTP_CLIENT HTTPS", "HTTPS_CLIENT HTTPS").is_empty()
        );
        assert_eq!(
            correction_terms(Some(&terms), "use HTTP", "use HTTPS").len(),
            1
        );
    }

    #[test]
    fn edit_budget_matches_full_distance_for_short_inputs() {
        let words = [
            "", "a", "ab", "abc", "abcd", "axc", "bcda", "xyzw", "aabbcc",
        ];
        for left in words {
            for right in words {
                let before: Vec<_> = left.chars().collect();
                let after: Vec<_> = right.chars().collect();
                let mut row: Vec<_> = (0..=after.len()).collect();
                for (i, a) in before.iter().enumerate() {
                    let mut diagonal = row[0];
                    row[0] = i + 1;
                    for (j, b) in after.iter().enumerate() {
                        let old = row[j + 1];
                        row[j + 1] = (row[j] + 1)
                            .min(old + 1)
                            .min(diagonal + usize::from(a != b));
                        diagonal = old;
                    }
                }
                for budget in 0..=5 {
                    assert_eq!(
                        within_edit_budget(&before, &after, budget),
                        row[after.len()] <= budget,
                        "{left:?} -> {right:?}, budget {budget}"
                    );
                }
            }
        }
    }
}
