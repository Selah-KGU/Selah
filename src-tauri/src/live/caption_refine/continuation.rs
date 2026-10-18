//! An acoustic final is not necessarily a finished sentence. SenseVoice adds
//! punctuation even after a particle at a bounded audio-window cut. Defer only
//! clear dependent tails; keep every recognized word and restore the original
//! punctuation if a pause or the bounded wait ends the fragment.

pub(super) fn unfinished(text: &str) -> bool {
    let text = text.trim_end_matches([' ', '\t', '\n', '.', '。']);
    if text
        .chars()
        .any(|ch| ('\u{3040}'..='\u{30ff}').contains(&ch))
    {
        // Nominalizers and explanatory endings can finish an utterance. Their
        // final kana are not the particles と/の/も. Likewise a bare て/って can
        // be a complete request, and や can be a Kansai copula. A period alone
        // cannot justify removing those endings' sentence boundary.
        if ["こと", "もの", "なの", "こども", "子ども"]
            .iter()
            .any(|tail| text.ends_with(tail))
        {
            return false;
        }
        return [
            "を",
            "の",
            "が",
            "に",
            "は",
            "と",
            "で",
            "へ",
            "も",
            "から",
            "まで",
            "より",
            "について",
            "として",
            "例えば",
            "たとえば",
            "しかし",
            "ただし",
            "なので",
            "ため",
            "すると",
            "なら",
            "ながら",
            "けれど",
            "けど",
        ]
        .iter()
        .any(|tail| text.ends_with(tail));
    }
    let last = text.split_whitespace().next_back().unwrap_or("");
    if [
        "a", "an", "the", "and", "or", "of", "to", "for", "with", "from", "into", "which", "whose",
        "because", "although", "than",
    ]
    .contains(&last.to_ascii_lowercase().as_str())
    {
        return true;
    }
    [
        "关于",
        "以及",
        "为了",
        "对于",
        "也就是",
        "称为",
        "그리고",
        "또는",
        "위한",
        "대한",
    ]
    .iter()
    .any(|tail| text.ends_with(tail))
}

/// Only a weak terminal mark at the capture edge may be removed when joining.
/// Closing quotes and real question/exclamation marks remain evidence of an end.
pub(super) fn without_provisional_end(text: &str) -> Option<&str> {
    let text = text.trim_end();
    let body = text.strip_suffix('。').or_else(|| text.strip_suffix('.'))?;
    // A repeated period is an ellipsis, not a single capture-edge marker.
    (!body.ends_with(['.', '。']) && unfinished(body)).then_some(body.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_dependent_tail_at_a_weak_capture_edge_is_joined() {
        for text in [
            "関係を。",
            "今日は確率の。",
            "the probability of.",
            "下面介绍关于。",
        ] {
            assert!(without_provisional_end(text).is_some(), "{text}");
        }
        for text in [
            "整理してください。",
            "確率です。",
            "Finished.",
            "関係を？",
            "「関係を。」",
            "待って。",
            "気をつけて。",
            "関係を...",
        ] {
            assert!(without_provisional_end(text).is_none(), "{text}");
        }
    }
}
