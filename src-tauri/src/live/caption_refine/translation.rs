//! Check translation coverage separately from local correction edits.
use serde_json::Value;

#[cfg(test)]
pub(super) fn complete(value: &Value, corrected: &str, target: Option<&str>) -> Option<String> {
    check(value, corrected, target)?.complete
}

pub(super) struct CheckedTranslations {
    pub parts: Vec<Option<String>>,
    pub complete: Option<String>,
}

pub(super) fn check(
    value: &Value,
    corrected: &str,
    target: Option<&str>,
) -> Option<CheckedTranslations> {
    let sentences = super::protocol::parts(corrected);
    let parts =
        if let Some(items) = value
            .get("source_translations")
            .or_else(|| value.get("translations"))
        {
            let items = items.as_array()?;
            if items.len() != sentences.len() {
                return None;
            }
            // Count and identity are structural: never guess how to align a bad
            // response. Content rejection, however, belongs to its own sentence.
            if items.iter().enumerate().any(|(index, item)| {
                item.get("id").and_then(Value::as_u64) != Some(index as u64 + 1)
            }) {
                return None;
            }
            items
                .iter()
                .zip(&sentences)
                .map(|(item, source)| {
                    let text = super::one_line(item.get("text")?.as_str()?);
                    (valid_text(source, &text)
                        && target.is_none_or(|target| matches_target(source, &text, target)))
                    .then_some(text)
                })
                .collect::<Vec<_>>()
        } else {
            // Older providers may still return one string. It cannot prove that
            // every sentence was covered when the batch contains multiple ids.
            if sentences.len() != 1 {
                return None;
            }
            let text = super::one_line(value.get("translation")?.as_str()?);
            vec![(valid_text(corrected, &text)
                && target.is_none_or(|target| matches_target(corrected, &text, target)))
            .then_some(text)]
        };
    let complete = parts
        .iter()
        .cloned()
        .collect::<Option<Vec<_>>>()
        .map(|parts| parts.join(" "))
        .filter(|text| {
            valid_text(corrected, text)
                && target.is_none_or(|target| matches_target(corrected, text, target))
        });
    Some(CheckedTranslations { parts, complete })
}

fn valid_text(source: &str, text: &str) -> bool {
    let numbers = |text: &str| {
        let mut values: Vec<_> = super::quality::numbers(text)
            .into_iter()
            .map(|value| value.trim_end_matches('%').to_string())
            .collect();
        // Fractions can reverse their written order across languages. Keep
        // each numeric token, sign and decimal intact without imposing the
        // source language's word order or percent spelling on the translation.
        values.sort_unstable();
        values
    };
    text.chars().any(char::is_alphanumeric)
        && super::acceptable_translation(source, text)
        && numbers(source) == numbers(text)
}

pub(super) fn matches_target(source: &str, text: &str, target: &str) -> bool {
    if !keeps_explicit_negation(source, text, target) {
        return false;
    }
    let content = |text: &str| {
        text.chars()
            .filter(|ch| ch.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let source = content(source);
    let text = content(text);
    let kana = |ch: &char| ('\u{3040}'..='\u{30ff}').contains(ch);
    let hangul = |ch: &char| ('\u{ac00}'..='\u{d7af}').contains(ch);
    let kana_count = text.chars().filter(kana).count();
    let hangul_count = text.chars().filter(hangul).count();
    let count = text.chars().count();
    // The system model sometimes fills translation with the complete source.
    // A Japanese/Korean sentence copied into another language is unambiguous.
    if source == text && (target != "ja" && kana_count > 0 || target != "ko" && hangul_count > 0) {
        return false;
    }
    if count < 12 {
        // Short proper names, technical terms and formulae can legitimately
        // keep their writing, but copying a whole kana/Hangul source above
        // is still not a translation merely because the sentence is short.
        return true;
    }
    if target != "ja" && kana_count * 4 > count || target != "ko" && hangul_count * 4 > count {
        return false;
    }
    true
}

fn keeps_explicit_negation(source: &str, text: &str, target: &str) -> bool {
    use regex::Regex;
    use std::sync::LazyLock;
    static NEGATIVE_CLAUSE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)\b(?:not|never|without|cannot)\b|n['’]t\b|ません|ではない|ではなかった|じゃない|とは限らない|않|없",
        )
        .unwrap()
    });
    if !NEGATIVE_CLAUSE.is_match(source) {
        return true;
    }
    // This only catches a missing explicit negative, not semantic equivalence.
    // Do not count every Han 不 as source negation: it can be part of a term.
    match target {
        "zh" => text
            .chars()
            .any(|ch| "不没沒无無未非否莫勿别別".contains(ch)),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_input_sentence_needs_one_translation_in_capture_order() {
        let source = "結果は独立と考えられます。第五の話題は標本と母集団です。全員のデータを集めることが難しい場合、一部の人から。";
        let translations = json!([
            {"id":1,"text":"结果可视为独立。"},
            {"id":2,"text":"第五个话题是样本与总体。"},
            {"id":3,"text":"难以收集所有人的数据时，从部分人中抽样。"}
        ]);
        let value = json!({"translations":translations});
        assert_eq!(
            complete(&value, source, None).unwrap(),
            "结果可视为独立。 第五个话题是样本与总体。 难以收集所有人的数据时，从部分人中抽样。"
        );
        for items in [
            json!([{"id":1,"text":"结果可视为独立。"}]),
            json!([{"id":1,"text":"结果。"},{"id":1,"text":"样本。"},{"id":3,"text":"抽样。"}]),
            json!([{"id":2,"text":"样本。"},{"id":1,"text":"结果。"},{"id":3,"text":"抽样。"}]),
            json!([{"id":1,"text":"结果。"},{"id":2,"text":"..."},{"id":3,"text":"抽样。"}]),
        ] {
            assert!(complete(&json!({"translations":items}), source, None).is_none());
        }
        assert!(complete(&json!({"translation":"结果可视为独立。"}), source, None).is_none());
    }

    #[test]
    fn legacy_single_sentence_translation_still_works_but_ellipsis_is_not_a_translation() {
        assert_eq!(
            complete(
                &json!({"translation":"Define probability."}),
                "確率を定義します。",
                None
            ),
            Some("Define probability.".into())
        );
        for text in ["", "...", "…", "！？"] {
            assert!(complete(&json!({"translation":text}), "確率を定義します。", None).is_none());
        }
    }

    #[test]
    fn copied_japanese_is_not_presented_as_chinese_translation() {
        let source = "思考を何 度も繰り返したときに平均が近づいていく値です 第二の話題は分散と標準偏差です。";
        assert!(!matches_target(source, source, "zh"));
        assert!(!matches_target(source, &format!("上文: {source}"), "zh"));
        assert!(matches_target(
            source,
            "反复进行试验时，平均值会趋近于这个数值。第二个话题是方差和标准差。",
            "zh"
        ));
        assert!(matches_target(source, source, "ja"));
        assert!(matches_target("3.14", "3.14", "zh"));
        assert!(matches_target("OpenAI", "OpenAI", "ja"));
        assert!(matches_target("東京", "東京", "zh"));
        assert!(!matches_target(
            "確率を定義します。",
            "確率を定義します",
            "zh"
        ));
        assert!(!matches_target("확률입니다.", "확률입니다", "en"));
    }

    #[test]
    fn actual_translation_cannot_merge_separate_numbers_or_guess_a_missing_decimal() {
        let source = "1 とすると、期待値は 3 5 になります、期待値は必ず実際に観測される値とは限りません、思考を何度も繰り返し。";
        let changed = "如果这样的话，期望值就是 35，期望值不一定就是实际观测到的值，思考要反复。";
        assert!(complete(&json!({"translation":changed}), source, Some("zh")).is_none());
        for changed in ["期望值为 35。", "期望值为 3.5。", "期望值为 3。"] {
            assert!(complete(
                &json!({"translation":changed}),
                "期待値は 3 5。",
                Some("zh")
            )
            .is_none());
        }
        for (source, text) in [
            ("期待値は 3 5。", "期望值为 3 5。"),
            ("確率は6分の1です。", "The probability is 1/6."),
            ("変化率は−１．５％です。", "The change is -1.5 percent."),
        ] {
            assert_eq!(
                complete(&json!({"translation":text}), source, None).as_deref(),
                Some(text)
            );
        }
    }

    #[test]
    fn actual_chinese_translation_cannot_turn_not_necessarily_into_only() {
        let source = "期待値は必ず実際に観測される値とは限りません、思考を何度も繰り返したときに。";
        let reversed = "期望值仅限于实际观测到的值，在多次思考时。";
        assert!(complete(&json!({"translation":reversed}), source, Some("zh")).is_none());
        for text in [
            "期望值不一定是实际观测到的值，在多次思考时。",
            "期望值未必是实际观测到的值，在多次思考时。",
            "期望值并非总是实际观测到的值，在多次思考时。",
        ] {
            assert_eq!(
                complete(&json!({"translation":text}), source, Some("zh")).as_deref(),
                Some(text)
            );
        }
        assert!(matches_target("標本数は少ないです。", "样本量较少。", "zh"));
        assert!(matches_target("少なくとも1つです。", "至少1个。", "zh"));
        assert!(matches_target(
            "不定積分です。",
            "This is an indefinite integral.",
            "en"
        ));
    }

    #[test]
    fn one_untranslated_sentence_cannot_hide_inside_a_mostly_chinese_batch() {
        let source = "期待値は平均が近づいていく値です。第二の話題は分散と標準偏差です。";
        let response = json!({"translations":[
            {"id":1,"text":"期望值是反复进行试验时平均值逐渐趋近的数值。"},
            {"id":2,"text":"第二の話題は分散と標準偏差です。"}
        ]});
        assert!(complete(&response, source, Some("zh")).is_none());
    }
}
