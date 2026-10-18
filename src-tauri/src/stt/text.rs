//! SenseVoice can put token spaces inside Japanese words. Only join adjacent
//! Japanese/Chinese characters; Latin words, numbers and Korean keep spacing.
pub(super) fn join_cjk_tokens(text: &str) -> String {
    let cjk = |ch: char| {
        ('\u{3040}'..='\u{30ff}').contains(&ch)
            || ('\u{3400}'..='\u{9fff}').contains(&ch)
            || ('\u{f900}'..='\u{faff}').contains(&ch)
    };
    let mut out = String::with_capacity(text.len());
    let mut spaces = String::new();
    for ch in text.chars() {
        if matches!(ch, ' ' | '\t') {
            spaces.push(ch);
            continue;
        }
        if !spaces.is_empty() {
            if !cjk(ch) || !out.chars().next_back().is_some_and(cjk) {
                out.push_str(&spaces);
            }
            spaces.clear();
        }
        out.push(ch);
    }
    out.push_str(&spaces);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actual_japanese_token_spaces_join_without_changing_words_or_numeric_boundaries() {
        let source = "1 とする と、期待 値 は 3 5 に なり ます、期待 値 は 必ず 実際 に 観測 される 値 と は 限り ません。";
        let joined = join_cjk_tokens(source);
        assert_eq!(
            joined,
            "1 とすると、期待値は 3 5 になります、期待値は必ず実際に観測される値とは限りません。"
        );
        assert_eq!(
            source
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect::<String>(),
            joined
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect::<String>()
        );
        assert_eq!(
            join_cjk_tokens("data base と 期待 値 は 3.5 mg です。"),
            "data base と期待値は 3.5 mg です。"
        );
    }

    #[test]
    fn english_korean_newlines_and_unicode_sequences_remain_intact() {
        for text in [
            "data base  is a phrase",
            "확률 분포 입니다",
            "日本\n語",
            "e\u{301} 👨‍👩‍👧‍👦 日本",
            "1 2 3",
            "  日本語  ",
        ] {
            assert_eq!(join_cjk_tokens(text), text);
        }
    }
}
