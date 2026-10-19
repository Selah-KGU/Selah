//! Preserve each source sentence independently, including short exceptions.
use serde_json::Value;

pub(super) struct CheckedCorrection {
    pub text: String,
    pub parts: Vec<String>,
    pub reverted: usize,
}

pub(super) fn parts(text: &str) -> Vec<String> {
    let chars: Vec<_> = text.chars().collect();
    let mut output = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < chars.len() {
        let boundary = super::sentences::ends_sentence(&chars, index) || chars[index] == '\n';
        index += 1;
        if boundary {
            while index < chars.len() && super::sentences::closing(chars[index]) {
                index += 1;
            }
            push(&mut output, &chars[start..index]);
            start = index;
        }
    }
    push(&mut output, &chars[start..]);
    output
}

fn push(output: &mut Vec<String>, chars: &[char]) {
    let text: String = chars.iter().collect();
    if text.chars().any(char::is_alphanumeric) {
        output.push(text.trim().to_string());
    }
}

pub(super) fn reordered(source: &[String], corrected: &[String]) -> bool {
    let normalize = |text: &str| {
        text.chars()
            .filter(|ch| ch.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let before: Vec<_> = source.iter().map(|text| normalize(text)).collect();
    corrected.iter().enumerate().any(|(index, text)| {
        let text = normalize(text);
        before.get(index) != Some(&text) && before.contains(&text)
    })
}

pub(super) fn check(value: &Value, source: &str) -> Option<CheckedCorrection> {
    let source = parts(source);
    if source.is_empty() {
        return None;
    }
    let mut rejected = vec![false; source.len()];
    let candidates = if let Some(edits) = value.get("edits") {
        // A patch response never replaces or drops the surrounding speech.
        if value.get("sentences").is_some() || value.get("corrected").is_some() {
            return None;
        }
        let patched = edit_candidates(edits, &source)?;
        rejected = patched.1;
        patched.0
    } else if let Some(sentences) = value.get("sentences") {
        let sentences = sentences.as_array()?;
        if sentences.len() != source.len() {
            return None;
        }
        let mut candidates = Vec::with_capacity(source.len());
        for (index, sentence) in sentences.iter().enumerate() {
            // Explicit order, rather than silently sorting missing/duplicate IDs.
            if sentence.get("id")?.as_u64()? != index as u64 + 1 {
                return None;
            }
            candidates.push(super::one_line(sentence.get("corrected")?.as_str()?));
        }
        candidates
    } else {
        let corrected = super::one_line(value.get("corrected")?.as_str()?);
        let candidates = parts(&corrected);
        if source.len() == 1 {
            vec![corrected]
        } else if candidates.len() == source.len() {
            candidates
        } else {
            return None;
        }
    };
    if reordered(&source, &candidates) {
        return None;
    }
    let mut reverted = 0;
    let parts: Vec<String> = source
        .iter()
        .zip(&candidates)
        .enumerate()
        .map(|(index, (original, candidate))| {
            if !rejected[index]
                && self::parts(candidate).len() == 1
                && super::quality::acceptable_correction(original, candidate)
            {
                candidate.clone()
            } else {
                reverted += 1;
                original.clone()
            }
        })
        .collect();
    Some(CheckedCorrection {
        text: parts.join(" "),
        parts,
        reverted,
    })
}

pub(super) fn edit_candidates(
    edits: &Value,
    source: &[String],
) -> Option<(Vec<String>, Vec<bool>)> {
    let edits = edits.as_array()?;
    if edits.len() > 8 {
        return None;
    }
    let mut spans = vec![Vec::new(); source.len()];
    let mut rejected = vec![false; source.len()];
    for edit in edits {
        let index = usize::try_from(edit.get("id")?.as_u64()?)
            .ok()?
            .checked_sub(1)?;
        let original = source.get(index)?;
        // Keep a mistakenly included sentence terminator outside the edit.
        let from = edit
            .get("from")?
            .as_str()?
            .trim()
            .trim_end_matches(['。', '！', '？', '!', '?']);
        let to = edit
            .get("to")?
            .as_str()?
            .trim()
            .trim_end_matches(['。', '！', '？', '!', '?']);
        if from == to {
            continue;
        }
        if !super::quality::learnable(from) || !super::quality::learnable(to) {
            rejected[index] = true;
            continue;
        }
        // Kana-only inflection changes are paraphrases, not term repairs.
        let left: Vec<_> = from.chars().collect();
        let right: Vec<_> = to.chars().collect();
        let han = |ch: &char| ('\u{4e00}'..='\u{9fff}').contains(ch);
        let hiragana = |ch: &char| ('\u{3040}'..='\u{309f}').contains(ch);
        if left.iter().any(han)
            && right.iter().any(han)
            && !left
                .iter()
                .filter(|ch| hiragana(ch))
                .eq(right.iter().filter(|ch| hiragana(ch)))
        {
            // A term repair may change kanji, but must not delete particles
            // or turn Japanese verb endings into Chinese grammar.
            rejected[index] = true;
            continue;
        }
        let prefix = left.iter().zip(&right).take_while(|(a, b)| a == b).count();
        let suffix = left[prefix..]
            .iter()
            .rev()
            .zip(right[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        if left[prefix..left.len() - suffix]
            .iter()
            .chain(&right[prefix..right.len() - suffix])
            .all(|ch| ('\u{3040}'..='\u{309f}').contains(ch))
        {
            rejected[index] = true;
            continue;
        }
        // Word-boundary repairs are fine; inserting an English phrase is not
        // a term correction (e.g. cash -> "with a card").
        let letters = |text: &str| {
            text.chars()
                .filter(|ch| ch.is_alphanumeric())
                .collect::<String>()
        };
        if from.is_ascii()
            && to.is_ascii()
            && from.split_whitespace().count() != to.split_whitespace().count()
            && !letters(from).eq_ignore_ascii_case(&letters(to))
        {
            rejected[index] = true;
            continue;
        }
        let word = |ch: char| ch.is_ascii_alphanumeric() || ch == '_';
        let matches = original
            .match_indices(from)
            .filter(|(start, _)| {
                (!from.starts_with(word) || !original[..*start].ends_with(word))
                    && (!from.ends_with(word) || !original[start + from.len()..].starts_with(word))
            })
            .collect::<Vec<_>>();
        // Ambiguous repeats need another model response, not a guessed target.
        let [(start, _)] = matches.as_slice() else {
            rejected[index] = true;
            continue;
        };
        spans[index].push((*start, start + from.len(), to));
    }
    let candidates = source
        .iter()
        .zip(spans)
        .enumerate()
        .map(|(index, (original, mut spans))| {
            spans.sort_by_key(|(start, _, _)| *start);
            let mut result = String::new();
            let mut cursor = 0;
            for (start, end, replacement) in spans {
                if start < cursor {
                    rejected[index] = true;
                    break;
                }
                result.push_str(&original[cursor..start]);
                result.push_str(replacement);
                cursor = end;
            }
            if rejected[index] {
                return original.clone();
            }
            result.push_str(&original[cursor..]);
            result
        })
        .collect();
    Some((candidates, rejected))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn actual_model_clause_rewrites_cannot_translate_a_japanese_correction_into_chinese() {
        let source = "た時に平均が近づいていく値です。";
        let value = json!({"edits":[{"id":1,"from":"平均が近づいていく値です","to":"平均近づいていく值是"}]});
        let checked = check(&value, source).unwrap();
        assert_eq!(checked.text, source);
        assert_eq!(checked.reverted, 1);
        let checked = check(
            &json!({"edits":[{"id":1,"from":"確立変数です","to":"確率変数です"}]}),
            "確立変数です。",
        )
        .unwrap();
        assert_eq!(checked.text, "確率変数です。");
        assert_eq!(checked.reverted, 0);
    }

    #[test]
    fn boundaries_preserve_quotes_decimals_abbreviations_and_unpunctuated_batch_lines() {
        assert_eq!(
            parts("Dr. Smith measured 1.5 mg. He said \"No!\"\n次は別の話\n続きです。"),
            vec![
                "Dr. Smith measured 1.5 mg.",
                "He said \"No!\"",
                "次は別の話",
                "続きです。"
            ]
        );
    }

    #[test]
    fn a_term_edit_cannot_create_extra_translation_sentence_ids() {
        let source = "Use the data base.";
        let value = json!({"edits":[{"id":1,"from":"data base","to":"data. base"}]});
        let result = check(&value, source).unwrap();
        assert_eq!(result.text, source);
        assert_eq!(result.reverted, 1);
        assert_eq!(result.parts.len(), 1);
    }

    #[test]
    fn missing_duplicate_or_reordered_sentence_ids_cannot_drop_an_exception() {
        let source = "確立変数を定義します。例外があります。期待地を求めます。";
        for ids in [vec![1, 3], vec![1, 1, 3], vec![1, 3, 2]] {
            let value = json!({"sentences":ids.iter().map(|id|json!({"id":id,"corrected":"文です。"})).collect::<Vec<_>>()});
            assert!(check(&value, source).is_none());
        }
    }

    #[test]
    fn one_unsafe_sentence_does_not_discard_another_terms_correction() {
        let value = json!({"sentences":[{"id":1,"corrected":"確率変数を定義します。"},{"id":2,"corrected":"変化率は1.5%です。"},{"id":3,"corrected":"期待値を求めます。"}]});
        let result = check(
            &value,
            "確立変数を定義します。変化率は-1.5%です。期待地を求めます。",
        )
        .unwrap();
        assert_eq!(result.reverted, 1);
        assert_eq!(
            result.text,
            "確率変数を定義します。 変化率は-1.5%です。 期待値を求めます。"
        );
    }

    #[test]
    fn legacy_complete_outputs_work_but_omitted_and_moved_sentences_do_not() {
        let source = "まず確立変数を定義します。例外があります。次に期待地を求めます。";
        assert!(check(
            &json!({"corrected":"まず確率変数を定義します。次に期待値を求めます。"}),
            source
        )
        .is_none());
        assert!(check(&json!({"corrected":"次に期待地を求めます。例外があります。まず確立変数を定義します。"}),source).is_none());
        assert_eq!(check(&json!({"corrected":"まず確率変数を定義します。例外があります。次に期待値を求めます。"}),source).unwrap().reverted,0);
    }

    #[test]
    fn local_edits_leave_every_other_word_and_short_exception_intact() {
        let result = check(&json!({"edits":[{"id":1,"from":"確立変数","to":"確率変数"},{"id":3,"from":"期待地","to":"期待値"}]}),
            "確立変数を定義します。例外があります。期待地を求めます。").unwrap();
        assert_eq!(
            result.text,
            "確率変数を定義します。 例外があります。 期待値を求めます。"
        );
        assert_eq!(result.reverted, 0);
        assert_eq!(
            check(&json!({"edits":[]}), "担当者は田中晴人さんです。")
                .unwrap()
                .text,
            "担当者は田中晴人さんです。"
        );
    }

    #[test]
    fn invalid_ambiguous_overlapping_and_phrase_edits_cannot_rewrite_speech() {
        let source = "The customer paid cash. Cash is accepted.";
        for edit in [
            json!({"id":0,"from":"cash","to":"cache"}),
            json!({"id":1,"from":"ash","to":"ache"}),
            json!({"id":1,"from":"cash","to":"with a card"}),
            json!({"id":3,"from":"cash","to":"cache"}),
            json!({"id":1,"from":"cash","to":""}),
        ] {
            assert!(check(&json!({"edits":[edit]}), source)
                .is_none_or(|checked| checked.text == source));
        }
        assert!(check(
            &json!({"edits":[{"id":1,"from":"確立","to":"確率"}]}),
            "確立と確立を比べます。"
        )
        .is_some_and(|checked| checked.reverted == 1));
        assert!(check(&json!({"edits":[{"id":1,"from":"cash","to":"cache"},{"id":1,"from":"cash","to":"cache"}]}),source).is_some_and(|checked| checked.text == source));
        assert!(check(&json!({"edits":[],"corrected":"different"}), source).is_none());
        assert_eq!(
            check(
                &json!({"edits":[{"id":1,"from":"data base","to":"database"}]}),
                "Use the data base."
            )
            .unwrap()
            .text,
            "Use the database."
        );
    }

    #[test]
    fn a_bad_patch_is_isolated_and_term_spans_may_include_unchanged_context() {
        let value = json!({"edits":[
            {"id":1,"from":"性器分布です。","to":"正規分布です。"},
            {"id":2,"from":"標準変差が大きいと横に広がります。","to":"標準偏差が大きいと横に広がります。"},
            {"id":3,"from":"平均は変えません。","to":"平均は変わりません。"}
        ]});
        let result = check(
            &value,
            "この曲線は性器分布です。標準変差が大きいと横に広がります。平均は変えません。",
        )
        .unwrap();
        assert_eq!(
            result.text,
            "この曲線は正規分布です。 標準偏差が大きいと横に広がります。 平均は変えません。"
        );
        assert_eq!(result.reverted, 1);
    }
}
