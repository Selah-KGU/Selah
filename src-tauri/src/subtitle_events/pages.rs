//! Fit complete translated batches into the shared single-line overlay.

use std::time::Duration;

use unicode_segmentation::UnicodeSegmentation;

use super::Caption;

// Both overlays cap at 620px. Reserve their padding and a font-measurement
// margin; graphemes keep combining characters and emoji on the same page.
const PAGE_WIDTH: usize = 520;

fn width(grapheme: &str) -> usize {
    if grapheme.chars().all(char::is_whitespace) {
        8
    } else if grapheme.is_ascii() {
        18
    } else {
        22
    }
}

pub(crate) fn split_pages(text: &str) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut pages = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let mut end = 0;
        let mut measured = 0;
        let mut boundary = None;
        for (index, grapheme) in rest.grapheme_indices(true) {
            let next = measured + width(grapheme);
            if next > PAGE_WIDTH && end > 0 {
                break;
            }
            measured = next;
            end = index + grapheme.len();
            if grapheme.contains('\n') {
                boundary = Some(end);
                break;
            }
            if measured >= PAGE_WIDTH / 2
                && (grapheme.chars().all(char::is_whitespace)
                    || matches!(grapheme, "。" | "！" | "？" | "." | "!" | "?"))
            {
                boundary = Some(end);
            }
        }
        if end < rest.len() {
            end = boundary.unwrap_or(end);
        }
        pages.push(rest[..end].to_string());
        rest = &rest[end..];
    }
    pages
}

pub(super) fn page_delay(text: &str) -> Duration {
    Duration::from_millis((text.graphemes(true).count() as u64 * 90).clamp(1_800, 3_000))
}

pub(super) async fn show_pages(
    caption: Caption,
    current: impl Fn() -> bool,
    mut show: impl FnMut(Caption),
    delay: fn(&str) -> Duration,
) {
    let pages = split_pages(&caption.text);
    let count = pages.len();
    for (index, text) in pages.into_iter().enumerate() {
        if !caption.is_current() || !current() {
            return;
        }
        let pause = delay(&text);
        show(caption.page(text, index + 1 == count));
        if index + 1 < count {
            tokio::time::sleep(pause).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    fn caption(text: &str) -> Caption {
        Caption {
            text: text.into(),
            session_id: "a".into(),
            is_final: true,
            revision: 1,
            current_revision: Arc::new(AtomicU64::new(1)),
        }
    }

    fn immediate(_: &str) -> Duration {
        Duration::ZERO
    }

    #[tokio::test]
    async fn every_character_of_a_long_batch_reaches_a_fitting_page() {
        let text = "第一句讲期望值的定义以及如何计算。第二句解释方差和标准差的区别。第三句把这些量应用到正态分布。";
        let mut shown = Vec::new();
        show_pages(caption(text), || true, |page| shown.push(page), immediate).await;
        assert!(shown.len() > 1);
        assert_eq!(
            shown
                .iter()
                .map(|page| page.text.as_str())
                .collect::<String>(),
            text
        );
        assert!(shown
            .iter()
            .all(|page| page.text.graphemes(true).map(width).sum::<usize>() <= PAGE_WIDTH));
        assert!(shown[..shown.len() - 1].iter().all(|page| !page.is_final));
        assert!(shown.last().unwrap().is_final);
    }

    #[test]
    fn pagination_preserves_words_whitespace_and_unicode_graphemes() {
        for text in [
            "Three sentences about probability. Keep every word and every space. Include the final sentence too.",
            "家族👨‍👩‍👧‍👦 と cafe\u{301} の話です。\n次の行も表示します。",
            &"👨‍👩‍👧‍👦e\u{301}🇯🇵".repeat(40),
        ] {
            let pages = split_pages(text);
            assert_eq!(pages.concat(), text);
            let original: Vec<_> = text.graphemes(true).collect();
            let displayed: Vec<_> = pages.iter().flat_map(|page| page.graphemes(true)).collect();
            assert_eq!(displayed, original);
        }
    }

    #[tokio::test]
    async fn a_replacement_caption_cancels_all_remaining_pages() {
        let caption = caption(&"長い字幕です。".repeat(30));
        let revision = caption.current_revision.clone();
        let mut shown = Vec::new();
        show_pages(
            caption,
            || true,
            |page| {
                shown.push(page.text);
                revision.fetch_add(1, Ordering::Relaxed);
            },
            immediate,
        )
        .await;
        assert_eq!(shown.len(), 1);
    }

    #[tokio::test]
    async fn disabled_or_finished_overlays_do_not_display_a_page() {
        let mut shown = Vec::new();
        show_pages(
            caption("字幕"),
            || false,
            |page| shown.push(page.text),
            immediate,
        )
        .await;
        assert!(shown.is_empty());
    }

    #[tokio::test]
    async fn an_empty_clear_caption_still_clears_the_visible_label() {
        let mut shown = Vec::new();
        show_pages(
            caption(""),
            || true,
            |page| shown.push(page.text),
            immediate,
        )
        .await;
        assert_eq!(shown, [""]);
    }
}
