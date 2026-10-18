//! Shared caption boundaries: punctuation belongs to its sentence, including
//! closing quotes. A decimal, abbreviation or hostname is not a sentence end.

pub(super) fn period_ends_sentence(chars: &[char], index: usize) -> bool {
    if chars
        .get(index + 1)
        .is_some_and(|ch| ch.is_ascii_alphanumeric())
    {
        return false;
    }
    let start = chars[..index]
        .iter()
        .rposition(|ch| !ch.is_ascii_alphabetic())
        .map_or(0, |i| i + 1);
    let word: String = chars[start..index].iter().collect();
    !(word.len() == 1 && word.chars().all(|ch| ch.is_ascii_uppercase()))
        && ![
            "mr", "mrs", "ms", "dr", "prof", "jr", "sr", "vs", "etc", "e", "g", "i",
        ]
        .contains(&word.to_ascii_lowercase().as_str())
}

pub(super) fn ends_sentence(chars: &[char], index: usize) -> bool {
    matches!(chars[index], '。' | '！' | '？' | '!' | '?')
        || (chars[index] == '.' && period_ends_sentence(chars, index))
}

pub(super) fn closing(ch: char) -> bool {
    matches!(
        ch,
        '。' | '！'
            | '？'
            | '!'
            | '?'
            | '.'
            | '」'
            | '』'
            | '”'
            | '’'
            | '\''
            | '"'
            | ')'
            | '）'
            | ']'
            | '】'
    )
}

/// End offsets in characters, retaining all closing punctuation together.
pub(super) fn boundaries(chars: &[char], newlines: bool) -> Vec<usize> {
    let mut ends = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let boundary = ends_sentence(chars, index) || newlines && chars[index] == '\n';
        index += 1;
        if boundary {
            while index < chars.len() && closing(chars[index]) {
                index += 1;
            }
            ends.push(index);
        }
    }
    ends
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abbreviations_decimals_domains_and_quotes_keep_their_sentences() {
        let text = "Dr. Smith used 3.14 at example.com. He said, \"Value is 3.\" She said 'Stop.' 次は「分散です。」最後です！？";
        let chars: Vec<_> = text.chars().collect();
        let mut start = 0;
        let pieces: Vec<String> = boundaries(&chars, false)
            .into_iter()
            .map(|end| {
                let piece = chars[start..end].iter().collect::<String>();
                start = end;
                piece.trim().to_string()
            })
            .collect();
        assert_eq!(
            pieces,
            [
                "Dr. Smith used 3.14 at example.com.",
                "He said, \"Value is 3.\"",
                "She said 'Stop.'",
                "次は「分散です。」",
                "最後です！？"
            ]
        );
    }
}
