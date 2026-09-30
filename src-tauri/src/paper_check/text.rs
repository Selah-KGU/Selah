//! Text utilities shared by the feature, judge and similarity layers:
//! sentence segmentation and tokenisation that behave sensibly for both
//! Japanese (CJK, no spaces) and Latin-script writing.

/// Re-join soft-wrapped lines. PDF extraction (and some docx flows) hard-wraps
/// prose at the visual line width; `split_sentences` treats every newline as a
/// boundary, so without this step a paragraph shatters into near-uniform "line
/// sentences" — which wrecks burstiness (uniform lengths read as AI-like) and
/// feeds the judge meaningless fragments.
///
/// A newline is treated as a soft wrap (merged) when the line carries no
/// sentence-terminal punctuation at its end AND it is long enough to look like
/// wrapped prose rather than a heading or list item. Lines ending mid-clause
/// (、 or ,) merge regardless of length. CJK joins seamlessly; Latin joins with
/// a space. Blank lines (paragraph breaks) are always preserved.
pub fn reflow_soft_wraps(text: &str) -> String {
    /// Sentence-terminal punctuation, incl. closers that follow a terminal.
    fn ends_hard(line: &str) -> bool {
        for c in line.chars().rev() {
            match c {
                '。' | '！' | '？' | '．' | '.' | '!' | '?' | ':' | '：' | ';' | '；' => {
                    return true
                }
                '」' | '』' | ')' | '\u{FF09}' | '】' | '"' | '\'' | ']' => continue,
                _ => return false,
            }
        }
        false
    }
    fn ends_mid_clause(line: &str) -> bool {
        matches!(line.chars().last(), Some('、') | Some(','))
    }
    /// Minimum visible length for a terminal-less line to count as wrapped
    /// prose; anything shorter is kept as its own line (heading, list item).
    const MIN_WRAP_LEN: usize = 30;

    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    let mut out = String::with_capacity(text.len());
    for (i, line) in lines.iter().enumerate() {
        out.push_str(line);
        if i + 1 >= lines.len() {
            break;
        }
        let next_blank = lines[i + 1].trim().is_empty();
        let trimmed = line.trim();
        let soft = !trimmed.is_empty()
            && !next_blank
            && !ends_hard(trimmed)
            && (ends_mid_clause(trimmed) || visible_len(trimmed) >= MIN_WRAP_LEN);
        if soft {
            // Merge with the next line: CJK↔CJK joins without a space.
            let last = trimmed.chars().last().unwrap_or(' ');
            let next_first = lines[i + 1].trim_start().chars().next().unwrap_or(' ');
            if !is_cjk(last) && !is_cjk(next_first) {
                out.push(' ');
            }
        } else {
            out.push('\n');
        }
    }
    out
}

/// Remove spaces that PDF extraction inserts inside CJK words at text-run
/// boundaries (「思われ る」→「思われる」, 「判 別」→「判別」). Only
/// horizontal whitespace strictly between two CJK characters (or CJK
/// punctuation) is removed; newlines and Latin/mixed spacing are untouched.
pub fn clean_cjk_spaces(text: &str) -> String {
    fn cjk_side(c: char) -> bool {
        is_cjk(c)
            || matches!(
                c,
                '。' | '、' | '「' | '」' | '『' | '』' | '(' | ')' | '・' | '?' | '!' | ':'
            )
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < chars.len() {
        if matches!(chars[i], ' ' | '\t' | '\u{3000}') {
            let mut j = i;
            while j < chars.len() && matches!(chars[j], ' ' | '\t' | '\u{3000}') {
                j += 1;
            }
            let prev_cjk = i > 0 && cjk_side(chars[i - 1]);
            let next_cjk = j < chars.len() && cjk_side(chars[j]);
            if !(prev_cjk && next_cjk) {
                out.extend(&chars[i..j]);
            }
            i = j;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Split raw text into sentences. Breaks on Japanese sentence punctuation
/// (。！？) and Latin `.`/`!`/`?` followed by whitespace, while trying not to
/// shatter on decimals or common abbreviations. Newline-separated lines that
/// carry no terminal punctuation (headings, list items) are kept as their own
/// sentence so they still count toward the stats.
pub fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut start = 0usize;
    let n = chars.len();

    for i in 0..n {
        let c = chars[i];
        let is_cjk_end = matches!(c, '。' | '！' | '？' | '．');
        let is_latin_end = matches!(c, '.' | '!' | '?');
        let mut boundary = false;

        if is_cjk_end {
            boundary = true;
        } else if is_latin_end {
            // Only treat as a boundary when the next non-quote char is
            // whitespace/end — avoids splitting "3.14" or "e.g.".
            let next = chars.get(i + 1).copied();
            let prev = if i > 0 {
                chars.get(i - 1).copied()
            } else {
                None
            };
            let next_is_space = next.map(|x| x.is_whitespace()).unwrap_or(true);
            let both_digits = prev.map(|x| x.is_ascii_digit()).unwrap_or(false)
                && next.map(|x| x.is_ascii_digit()).unwrap_or(false);
            if next_is_space && !both_digits {
                boundary = true;
            }
        } else if c == '\n' {
            // A hard line break with content behind it also ends a "sentence".
            boundary = true;
        }

        if boundary {
            let piece: String = chars[start..=i].iter().collect();
            let trimmed = piece.trim();
            if !trimmed.is_empty() {
                sentences.push(trimmed.to_string());
            }
            start = i + 1;
        }
    }

    if start < n {
        let piece: String = chars[start..n].iter().collect();
        let trimmed = piece.trim();
        if !trimmed.is_empty() {
            sentences.push(trimmed.to_string());
        }
    }

    sentences
}

/// Tokenise into comparable units. Latin runs (letters/digits) become
/// lowercase word tokens; each CJK ideograph/kana becomes its own token so
/// spaceless Japanese still yields a meaningful token stream.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut buf = String::new();
    for c in text.chars() {
        if is_cjk(c) {
            if !buf.is_empty() {
                tokens.push(std::mem::take(&mut buf));
            }
            tokens.push(c.to_string());
        } else if c.is_alphanumeric() {
            buf.extend(c.to_lowercase());
        } else if !buf.is_empty() {
            tokens.push(std::mem::take(&mut buf));
        }
    }
    if !buf.is_empty() {
        tokens.push(buf);
    }
    tokens
}

/// Like [`tokenize`], but also returns each token's character span
/// `[start, end)` in the original text. Used by the similarity channel to map
/// matched word-shingles back to concrete character ranges for coverage.
pub fn tokenize_spans(text: &str) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut buf_start = 0usize;
    let mut count = 0usize;
    for (idx, c) in text.chars().enumerate() {
        count = idx + 1;
        if is_cjk(c) {
            if !buf.is_empty() {
                out.push((std::mem::take(&mut buf), buf_start, idx));
            }
            out.push((c.to_string(), idx, idx + 1));
        } else if c.is_alphanumeric() {
            if buf.is_empty() {
                buf_start = idx;
            }
            buf.extend(c.to_lowercase());
        } else if !buf.is_empty() {
            out.push((std::mem::take(&mut buf), buf_start, idx));
        }
    }
    if !buf.is_empty() {
        out.push((buf, buf_start, count));
    }
    out
}

/// Set of word n-grams (tokens joined by a separator) for a piece of text.
/// Shared by the similarity, DNA-GPT and Raidar channels for overlap measures.
pub fn word_ngrams(text: &str, n: usize) -> std::collections::HashSet<String> {
    let tokens = tokenize(text);
    let mut set = std::collections::HashSet::new();
    if tokens.len() < n {
        if !tokens.is_empty() {
            set.insert(tokens.join("\u{1}"));
        }
        return set;
    }
    for i in 0..=tokens.len() - n {
        set.insert(tokens[i..i + n].join("\u{1}"));
    }
    set
}

/// Character length used for burstiness — counts visible characters, ignoring
/// whitespace, so sentence "length" reflects content not spacing.
pub fn visible_len(sentence: &str) -> usize {
    sentence.chars().filter(|c| !c.is_whitespace()).count()
}

pub(super) fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF   // hiragana + katakana
        | 0x3400..=0x4DBF // CJK ext A
        | 0x4E00..=0x9FFF // CJK unified
        | 0xF900..=0xFAFF // compatibility
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_mixed_punctuation() {
        let s = split_sentences("これはテストです。二文目！ Third one? Yes.");
        assert_eq!(s.len(), 4);
    }

    #[test]
    fn keeps_decimals_intact() {
        let s = split_sentences("The value is 3.14 in total.");
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn tokenizes_cjk_per_char() {
        let t = tokenize("猫 cat");
        assert_eq!(t, vec!["猫", "cat"]);
    }

    #[test]
    fn reflow_merges_hard_wrapped_japanese_paragraph() {
        // PDF-style: one sentence hard-wrapped across three visual lines.
        let wrapped = "本研究では、大学生のレポートにおける生成AIの利用実態を調査し、その\n結果をもとに教育現場での適切な指導方法を検討することを目的として、\n質問紙調査と面接調査を実施した。\n";
        let flowed = reflow_soft_wraps(wrapped);
        assert_eq!(
            split_sentences(&flowed).len(),
            1,
            "should be one sentence: {flowed}"
        );
        assert!(
            !flowed.contains("その 結果"),
            "CJK join must not insert a space"
        );
    }

    #[test]
    fn reflow_merges_wrapped_english_with_space() {
        let wrapped = "The survey was administered to two hundred undergraduate students who\nhad submitted at least one report during the semester.";
        let flowed = reflow_soft_wraps(wrapped);
        assert_eq!(split_sentences(&flowed).len(), 1);
        assert!(
            flowed.contains("who had"),
            "Latin join needs a space: {flowed}"
        );
    }

    #[test]
    fn reflow_keeps_headings_and_blank_lines() {
        let text = "1. はじめに\n\n本章では研究の背景を述べる。\n2. 方法\n調査は二段階で実施した。";
        let flowed = reflow_soft_wraps(text);
        assert!(
            flowed.contains("1. はじめに\n"),
            "short heading keeps its line break"
        );
        assert!(
            flowed.contains("2. 方法\n"),
            "short heading keeps its line break"
        );
        assert!(
            flowed.contains("背景を述べる。\n"),
            "terminal punctuation keeps the break"
        );
    }

    #[test]
    fn clean_cjk_spaces_strips_run_boundary_gaps() {
        assert_eq!(
            clean_cjk_spaces("説明だけでは不十分であるように思われ る。"),
            "説明だけでは不十分であるように思われる。"
        );
        assert_eq!(
            clean_cjk_spaces("どの段階から「依存」と呼べるのかを判 別する"),
            "どの段階から「依存」と呼べるのかを判別する"
        );
        assert_eq!(
            clean_cjk_spaces("判　別"),
            "判別",
            "fullwidth space between CJK"
        );
        // Latin boundaries keep their spacing; newlines untouched.
        assert_eq!(
            clean_cjk_spaces("AI 技術は difficult 難しい"),
            "AI 技術は difficult 難しい"
        );
        assert_eq!(clean_cjk_spaces("一行目\n二行目"), "一行目\n二行目");
    }

    #[test]
    fn reflow_merges_comma_ended_short_line() {
        let text = "しかし、\n実際の運用では課題が残る。";
        let flowed = reflow_soft_wraps(text);
        assert_eq!(
            split_sentences(&flowed).len(),
            1,
            "comma-ended line merges: {flowed}"
        );
    }
}
