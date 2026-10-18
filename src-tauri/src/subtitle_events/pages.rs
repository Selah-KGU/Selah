//! Fit complete translated batches into the shared single-line overlay.

use std::time::{Duration, Instant};

use unicode_segmentation::UnicodeSegmentation;

use super::Caption;

// Both overlays cap at 620px. Reserve their padding and a font-measurement
// margin; graphemes keep combining characters and emoji on the same page.
const PAGE_WIDTH: usize = 520;
const LIVE_HOLD_BUDGET: Duration = Duration::from_secs(5);

fn width(grapheme: &str) -> usize {
    if grapheme.chars().all(char::is_whitespace) {
        8
    } else if grapheme.is_ascii() {
        18
    } else {
        22
    }
}

/// Raw recognition replaces the current line. Keep its newest words visible
/// instead of allowing the single-line native label to ellipsize the tail.
pub(super) fn live_tail(text: &str) -> String {
    let text = text.trim();
    let mut measured = 0;
    let mut start = text.len();
    for (index, grapheme) in text.grapheme_indices(true).rev() {
        measured += width(grapheme);
        if measured > PAGE_WIDTH {
            break;
        }
        start = index;
    }
    if start == 0 {
        return text.to_string();
    }
    // Reserve the ellipsis, then prefer a complete English word. Never trim
    // inside a grapheme, including a family emoji or combining accent.
    while start < text.len()
        && text[start..].graphemes(true).map(width).sum::<usize>() + width("…") > PAGE_WIDTH
    {
        start += text[start..].graphemes(true).next().unwrap().len();
    }
    if text[..start].ends_with(|ch: char| ch.is_ascii_alphanumeric())
        && text[start..].starts_with(|ch: char| ch.is_ascii_alphanumeric())
    {
        if let Some((offset, _)) = text[start..]
            .char_indices()
            .find(|(_, ch)| ch.is_whitespace())
        {
            start += offset;
        }
    }
    format!("…{}", text[start..].trim_start())
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

#[cfg(test)]
pub(super) async fn show_pages(
    caption: Caption,
    current: impl Fn() -> bool,
    show: impl FnMut(Caption),
    delay: fn(&str) -> Duration,
) {
    show_budgeted_pages(caption, current, show, delay, None).await;
}

pub(super) async fn show_live_pages(
    caption: Caption,
    current: impl Fn() -> bool,
    show: impl FnMut(Caption),
    delay: fn(&str) -> Duration,
) {
    show_budgeted_pages(caption, current, show, delay, Some(LIVE_HOLD_BUDGET)).await;
}

async fn show_budgeted_pages(
    caption: Caption,
    current: impl Fn() -> bool,
    mut show: impl FnMut(Caption),
    delay: fn(&str) -> Duration,
    budget: Option<Duration>,
) {
    let deadline = budget.map(|budget| Instant::now() + budget);
    let pages = split_pages(&caption.text);
    let count = pages.len();
    for (index, text) in pages.into_iter().enumerate() {
        if !caption.is_current() || !current() {
            return;
        }
        let pause = delay(&text);
        // Full translations remain in the LIVE history. Do not start another
        // overlay page unless its reading time fits before returning to speech.
        if index > 0 && deadline.is_some_and(|deadline| Instant::now() + pause > deadline) {
            return;
        }
        show(caption.page(text, index + 1 == count));
        // Hold the last translated page long enough to read, then allow the
        // owner to resume current speech. A translation must not freeze the
        // overlay until another model response happens to arrive.
        if index + 1 < count || !caption.is_final {
            let pause = deadline.map_or(pause, |deadline| {
                pause.min(deadline.saturating_duration_since(Instant::now()))
            });
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

    #[test]
    fn raw_captions_show_the_latest_words_instead_of_a_clipped_old_prefix() {
        let short = "最新の短い字幕です。";
        assert_eq!(live_tail(short), short);
        for text in [
            "これは既に話し終えた説明です。".repeat(12) + "今話しているのは条件付き確率です。",
            "These previous words were already spoken. ".repeat(12)
                + "The current topic is conditional probability.",
            "👨‍👩‍👧‍👦e\u{301}🇯🇵".repeat(40),
        ] {
            let tail = live_tail(&text);
            assert!(tail.starts_with('…'));
            assert!(text.ends_with(tail.trim_start_matches('…')));
            assert!(tail.graphemes(true).map(width).sum::<usize>() <= PAGE_WIDTH);
            let graphemes: Vec<_> = text.graphemes(true).collect();
            let tail_graphemes: Vec<_> = tail.graphemes(true).skip(1).collect();
            assert!(graphemes.ends_with(&tail_graphemes));
        }
    }

    #[tokio::test]
    async fn a_long_live_translation_releases_the_overlay_before_all_history_pages_are_played() {
        fn reading_time(_: &str) -> Duration {
            Duration::from_millis(10)
        }
        let text = "長い翻訳です。".repeat(400);
        let full_pages = split_pages(&text).len();
        let mut shown = Vec::new();
        tokio::time::timeout(
            Duration::from_millis(500),
            show_budgeted_pages(
                caption(&text),
                || true,
                |page| shown.push(page.text),
                reading_time,
                Some(Duration::from_millis(25)),
            ),
        )
        .await
        .unwrap();
        assert!(!shown.is_empty() && shown.len() < full_pages);
        assert!(text.starts_with(&shown.concat()));
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
