//! Sentence-sized correction inputs from confirmed recognition only.
//!
//! Offline ASR punctuates each temporary window, then revises it on the next
//! decode. Those windows remain ephemeral on the raw display path. Only final
//! FIFO results may append source text to correction/translation batches.

use unicode_segmentation::UnicodeSegmentation;

const JOIN_AFTER_FINAL_MS: u64 = 2_000;
// A dependent tail must survive the next bounded (<= 10 s) audio window and
// its final decode. The raw partial display continues throughout this wait.
const HARD_BUDGET_MS: u64 = 12_000;
const MAX_SEGMENT_CHARS: usize = 800;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CommittedSegment {
    pub id: u64,
    pub source_seq: u64,
    pub text: String,
}

#[derive(Debug, Default)]
pub(super) struct Segmenter {
    pending: String,
    pending_since_ms: Option<u64>,
    closed_at_ms: Option<u64>,
    pending_seq: u64,
    last_final_seq: Option<u64>,
    next_id: u64,
}

impl Segmenter {
    pub(super) fn activity(&mut self, seq: u64, now: u64) {
        // Activity may defer a pause boundary, but never contributes tentative
        // words or punctuation to the confirmed source.
        // An input can win the race against the interval tick. New speech
        // must not reopen a pause which already ended the previous fragment.
        if seq > self.pending_seq && !self.pending.is_empty() && !self.expired(now) {
            self.closed_at_ms = Some(now);
        }
    }

    pub(super) fn push_final_ordered(
        &mut self,
        text: &str,
        now: u64,
        seq: Option<u64>,
    ) -> Vec<CommittedSegment> {
        if let Some(seq) = seq {
            if self.last_final_seq.is_some_and(|last| seq <= last) {
                return Vec::new();
            }
            self.last_final_seq = Some(seq);
        }
        if !meaningful(text) {
            return self.on_tick(now);
        }
        // Enforce the same deadlines regardless of which select! branch runs
        // first, and commit the old capture with its own source sequence.
        let mut output = self.on_tick(now);
        if self.pending.is_empty() {
            self.pending_since_ms = Some(now);
        }
        let left =
            super::continuation::without_provisional_end(&self.pending).unwrap_or(&self.pending);
        self.pending = join_caption(left, text);
        self.pending_seq = seq.unwrap_or(0);
        self.closed_at_ms = Some(now);
        let (done, rest) = split_complete_sentences(&self.pending);
        self.pending = rest;
        if self.pending.is_empty() {
            self.pending_since_ms = None;
            self.closed_at_ms = None;
        } else if meaningful(&done) {
            self.pending_since_ms = Some(now);
        }
        output.extend(self.commit(done));
        output
    }

    fn expired(&self, now: u64) -> bool {
        self.closed_at_ms
            .is_some_and(|at| now.saturating_sub(at) >= JOIN_AFTER_FINAL_MS)
            || self
                .pending_since_ms
                .is_some_and(|at| now.saturating_sub(at) >= HARD_BUDGET_MS)
    }

    pub(super) fn on_tick(&mut self, now: u64) -> Vec<CommittedSegment> {
        if self.expired(now) {
            return self.finish(now);
        }
        Vec::new()
    }

    pub(super) fn finish(&mut self, _now: u64) -> Vec<CommittedSegment> {
        let text = std::mem::take(&mut self.pending);
        self.pending_since_ms = None;
        self.closed_at_ms = None;
        self.commit(text)
    }

    fn commit(&mut self, text: String) -> Vec<CommittedSegment> {
        split_for_model(&text)
            .into_iter()
            .filter(|text| meaningful(text))
            .map(|text| {
                self.next_id += 1;
                CommittedSegment {
                    id: self.next_id,
                    source_seq: self.pending_seq,
                    text,
                }
            })
            .collect()
    }
}

fn join_caption(left: &str, right: &str) -> String {
    let left = left.trim();
    let right = right.trim();
    if left.is_empty() {
        return right.to_string();
    }
    if right.is_empty() {
        return left.to_string();
    }
    let spaced_word = |ch: char| {
        ch.is_alphanumeric()
            && !matches!(ch as u32,
                0x3040..=0x30ff | 0x3400..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x323af)
    };
    // Hangul and non-ASCII alphabetic words need the same separation as
    // English. Inspect graphemes so a trailing combining accent is preserved.
    let space = (left
        .graphemes(true)
        .next_back()
        .is_some_and(|grapheme| grapheme.chars().any(spaced_word))
        || left.ends_with(['.', ',', '!', '?', ';', ':', ')', ']']))
        && right.starts_with(spaced_word);
    format!("{left}{}{right}", if space { " " } else { "" })
}

fn meaningful(text: &str) -> bool {
    text.chars().any(|ch| {
        ch.is_alphanumeric()
            || ('\u{3040}'..='\u{30ff}').contains(&ch)
            || ('\u{4e00}'..='\u{9fff}').contains(&ch)
    })
}

fn split_complete_sentences(text: &str) -> (String, String) {
    let chars: Vec<char> = text.chars().collect();
    let mut ends = super::sentences::boundaries(&chars, false);
    if ends.last() == Some(&chars.len())
        && super::continuation::without_provisional_end(text).is_some()
    {
        ends.pop();
    }
    let Some(end) = ends.last().copied() else {
        return (String::new(), text.trim().to_string());
    };
    let done: String = chars[..end].iter().collect();
    let rest: String = chars[end..].iter().collect();
    (done.trim().to_string(), rest.trim().to_string())
}

pub(super) fn sentence_units(text: &str) -> usize {
    super::protocol::parts(text).len()
}

fn split_for_model(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // A recognizer can return many sentences in one window. Keep the same
    // boundaries as the model protocol instead of treating that window as a
    // single paragraph which can bypass the batch's four-sentence limit.
    for sentence in super::protocol::parts(text) {
        let graphemes: Vec<_> = sentence.graphemes(true).collect();
        let mut start = 0;
        let mut carried = String::new();
        while start < graphemes.len() {
            let mut end = start;
            let mut chars = 0;
            let mut boundary = None;
            while end < graphemes.len() {
                let next = chars + graphemes[end].chars().count();
                if next > MAX_SEGMENT_CHARS && end > start {
                    break;
                }
                chars = next;
                if chars >= MAX_SEGMENT_CHARS / 2
                    && (graphemes[end].chars().all(char::is_whitespace)
                        || matches!(graphemes[end], "、" | "，" | "," | ";" | "；"))
                {
                    boundary = Some(end + 1);
                }
                end += 1;
            }
            if end < graphemes.len() {
                end = boundary.unwrap_or(end);
            }
            carried.push_str(&graphemes[start..end].concat());
            if meaningful(&carried) {
                out.push(carried.trim().to_string());
                carried.clear();
            }
            start = end;
        }
        // A size boundary before closing punctuation must not discard it as
        // a standalone non-speech chunk. Graphemes also keep accents and emoji
        // intact when the preferred size lands inside a Unicode sequence.
        if !carried.is_empty() {
            if let Some(last) = out.last_mut() {
                last.push_str(&carried);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(items: &[CommittedSegment]) -> String {
        items.iter().map(|item| item.text.as_str()).collect()
    }

    #[test]
    fn confirmed_sentences_keep_quotes_abbreviations_domains_and_numbers() {
        let mut segmenter = Segmenter::default();
        let source = "Dr. Smith used 3.14 at example.com. He said 'Value is 3.' 次は「分散です。」最後です！？";
        let output = segmenter.push_final_ordered(source, 0, Some(10));
        assert_eq!(output.len(), 4);
        assert_eq!(
            output.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
            [
                "Dr. Smith used 3.14 at example.com.",
                "He said 'Value is 3.'",
                "次は「分散です。」",
                "最後です！？"
            ]
        );
        assert!(output.iter().all(|item| item.source_seq == 10));
        assert!(segmenter.finish(5_000).is_empty());
    }

    #[test]
    fn finals_join_an_unfinished_sentence_across_a_short_breath() {
        let mut segmenter = Segmenter::default();
        assert!(segmenter
            .push_final_ordered("今日は確率の", 0, Some(10))
            .is_empty());
        assert!(segmenter.on_tick(1_000).is_empty());
        let output = segmenter.push_final_ordered("話をします。", 1_200, Some(20));
        assert_eq!(text(&output), "今日は確率の話をします。");
        assert_eq!(output[0].source_seq, 20);
    }

    #[test]
    fn english_finals_keep_word_boundaries_across_a_breath() {
        let mut segmenter = Segmenter::default();
        segmenter.push_final_ordered("This is", 0, Some(10));
        assert_eq!(
            text(&segmenter.push_final_ordered("the expectation.", 500, Some(20))),
            "This is the expectation."
        );
    }

    #[test]
    fn unicode_words_keep_spaces_when_confirmed_windows_join() {
        for (first, next, expected) in [
            ("확률은", "사건의 비율입니다.", "확률은 사건의 비율입니다."),
            ("The café", "is open.", "The café is open."),
            ("Это", "вероятность.", "Это вероятность."),
            ("The cafe\u{301}", "is open.", "The cafe\u{301} is open."),
        ] {
            let mut segmenter = Segmenter::default();
            assert!(segmenter.push_final_ordered(first, 0, Some(10)).is_empty());
            assert_eq!(
                text(&segmenter.push_final_ordered(next, 1_000, Some(20))),
                expected
            );
        }
    }

    #[test]
    fn a_new_final_cannot_reopen_an_expired_pause_before_the_next_tick() {
        let mut segmenter = Segmenter::default();
        segmenter.push_final_ordered("関係を。", 0, Some(10));
        let output = segmenter.push_final_ordered("次に分散を説明します。", 2_001, Some(20));
        assert_eq!(
            output.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
            ["関係を。", "次に分散を説明します。"]
        );
        assert_eq!(
            output.iter().map(|s| s.source_seq).collect::<Vec<_>>(),
            [10, 20]
        );
    }

    #[test]
    fn new_activity_cannot_reopen_a_pause_while_the_timer_is_waiting() {
        let mut segmenter = Segmenter::default();
        segmenter.push_final_ordered("関係を。", 0, Some(10));
        segmenter.activity(11, 2_001);
        assert_eq!(text(&segmenter.on_tick(2_100)), "関係を。");
        assert_eq!(
            text(&segmenter.push_final_ordered("次の話題です。", 2_200, Some(20))),
            "次の話題です。"
        );
    }

    #[test]
    fn a_new_final_cannot_bypass_the_hard_join_budget_before_a_tick() {
        let mut segmenter = Segmenter::default();
        segmenter.push_final_ordered("確率の", 0, Some(10));
        for second in 1..=12 {
            segmenter.activity(10 + second, second * 1_000);
        }
        let output = segmenter.push_final_ordered("定義です。", 12_001, Some(30));
        assert_eq!(
            output.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
            ["確率の", "定義です。"]
        );
    }

    #[test]
    fn japanese_complete_endings_are_not_glued_to_the_following_statement() {
        for first in [
            "待って。",
            "気をつけて。",
            "説明して。",
            "これを見て。",
            "忘れないこと。",
            "これは私のもの。",
            "あれは子ども。",
            "答えはこうなの。",
            "そうや。",
        ] {
            let mut segmenter = Segmenter::default();
            let output = segmenter.push_final_ordered(first, 0, Some(10));
            assert_eq!(text(&output), first);
            assert_eq!(
                text(&segmenter.push_final_ordered("次は分散です。", 1_000, Some(20))),
                "次は分散です。"
            );
        }
    }

    #[test]
    fn screenshot_object_and_predicate_rejoin_across_an_entire_next_audio_window() {
        let first = "最後に、これまでの説明を振り返り、確率、期待値、分散、条件付き確率の関係を。";
        let next = "整理してください。第一の話題は確率時期待値です。サイコロを一回投げた時、出る目は一から六までの整数です。";
        let mut segmenter = Segmenter::default();
        assert!(segmenter.push_final_ordered(first, 0, Some(10)).is_empty());
        for tick in 1..=10 {
            segmenter.activity(10 + tick, tick * 1_000);
            assert!(segmenter.on_tick(tick * 1_000).is_empty());
        }
        let output = segmenter.push_final_ordered(next, 10_600, Some(100));
        assert_eq!(output.iter().map(|s|s.text.as_str()).collect::<Vec<_>>(), [
            "最後に、これまでの説明を振り返り、確率、期待値、分散、条件付き確率の関係を整理してください。",
            "第一の話題は確率時期待値です。",
            "サイコロを一回投げた時、出る目は一から六までの整数です。",
        ]);
        assert!(output.iter().all(|s| s.source_seq == 100));
        let words = |text: &str| {
            text.chars()
                .filter(|ch| ch.is_alphanumeric())
                .collect::<String>()
        };
        assert_eq!(words(&text(&output)), words(&format!("{first}{next}")));
        assert!(segmenter.finish(20_000).is_empty());
    }

    #[test]
    fn complete_sentences_before_a_dependent_tail_are_not_delayed() {
        let mut segmenter = Segmenter::default();
        assert_eq!(
            text(&segmenter.push_final_ordered("平均です。次に関係を。", 0, Some(10))),
            "平均です。"
        );
        assert_eq!(
            text(&segmenter.push_final_ordered("整理してください。", 1_000, Some(20))),
            "次に関係を整理してください。"
        );
    }

    #[test]
    fn a_real_pause_keeps_the_original_fragment_punctuation() {
        let mut paused = Segmenter::default();
        paused.push_final_ordered("関係を。", 0, Some(10));
        assert_eq!(text(&paused.on_tick(JOIN_AFTER_FINAL_MS)), "関係を。");
        assert_eq!(
            text(&paused.push_final_ordered("次に分散を説明します。", 3_000, Some(20))),
            "次に分散を説明します。"
        );
    }

    #[test]
    fn topic_words_inside_a_continuation_do_not_force_another_split() {
        for (first, next, expected) in [
            (
                "先生は。",
                "第一の話題を説明します。",
                "先生は第一の話題を説明します。",
            ),
            (
                "関係を。",
                "次に整理してください。",
                "関係を次に整理してください。",
            ),
        ] {
            let mut segmenter = Segmenter::default();
            assert!(segmenter.push_final_ordered(first, 0, Some(10)).is_empty());
            assert_eq!(
                text(&segmenter.push_final_ordered(next, 1_000, Some(20))),
                expected
            );
        }
    }

    #[test]
    fn an_english_preposition_with_a_provisional_period_waits_for_its_object() {
        let mut segmenter = Segmenter::default();
        assert!(segmenter
            .push_final_ordered("We define the probability of.", 0, Some(10))
            .is_empty());
        assert_eq!(
            text(&segmenter.push_final_ordered("the event.", 1_000, Some(20))),
            "We define the probability of the event."
        );
    }

    #[test]
    fn a_short_unpunctuated_final_waits_for_a_pause_then_commits_once() {
        let mut segmenter = Segmenter::default();
        assert!(segmenter
            .push_final_ordered("期待値を定義します", 0, Some(10))
            .is_empty());
        assert!(segmenter.on_tick(JOIN_AFTER_FINAL_MS - 1).is_empty());
        assert_eq!(
            text(&segmenter.on_tick(JOIN_AFTER_FINAL_MS)),
            "期待値を定義します"
        );
        assert!(segmenter.on_tick(5_000).is_empty());
    }

    #[test]
    fn ongoing_activity_defers_a_pause_without_appending_any_tentative_words() {
        let mut segmenter = Segmenter::default();
        segmenter.push_final_ordered("今日は確率の", 0, Some(10));
        segmenter.activity(11, 1_500);
        assert!(segmenter.on_tick(2_000).is_empty());
        let output = segmenter.push_final_ordered("話をします。", 2_100, Some(20));
        assert_eq!(text(&output), "今日は確率の話をします。");
        assert!(segmenter.finish(4_000).is_empty());
    }

    #[test]
    fn old_activity_cannot_defer_newer_confirmed_speech() {
        let mut segmenter = Segmenter::default();
        segmenter.push_final_ordered("確定した発話", 0, Some(20));
        segmenter.activity(10, 1_500);
        assert_eq!(
            text(&segmenter.on_tick(JOIN_AFTER_FINAL_MS)),
            "確定した発話"
        );
    }

    #[test]
    fn finish_preserves_a_short_unfinished_tail_once() {
        let mut segmenter = Segmenter::default();
        segmenter.push_final_ordered("次に分散を定義", 0, Some(10));
        assert_eq!(text(&segmenter.finish(100)), "次に分散を定義");
        assert!(segmenter.finish(200).is_empty());
    }

    #[test]
    fn repeated_speech_is_preserved_but_a_replayed_capture_is_rejected() {
        let mut segmenter = Segmenter::default();
        for seq in [10, 20, 30] {
            assert_eq!(
                text(&segmenter.push_final_ordered("同じ定理です。", seq, Some(seq))),
                "同じ定理です。"
            );
        }
        for seq in [30, 20, 10] {
            assert!(segmenter
                .push_final_ordered("遅れた結果です。", 100, Some(seq))
                .is_empty());
        }
    }

    #[test]
    fn continuous_confirmed_fragments_have_a_bounded_join_budget_without_losing_words() {
        let mut segmenter = Segmenter::default();
        let mut output = Vec::new();
        let mut expected = String::new();
        for seq in 1..=14 {
            let part = format!("第{seq}部分");
            expected.push_str(&part);
            output.extend(segmenter.push_final_ordered(&part, (seq - 1) * 1_000, Some(seq)));
            output.extend(segmenter.on_tick((seq - 1) * 1_000));
        }
        assert!(!output.is_empty());
        output.extend(segmenter.finish(14_000));
        assert_eq!(text(&output), expected);
    }

    #[test]
    fn public_asr_finals_are_not_expanded_into_repeated_speculative_paragraphs() {
        // Exact confirmed output from the opt-in 22 s public WAV replay.
        // The old partial path emitted 277 characters/27 segments for these
        // 56 confirmed characters because every hypothesis had a new period.
        let finals = [
            "开放时间早上9点至下午五点开放时间早上9点至下午。",
            "5点开放时间早上9点至下午5点。开放时间早上9点。",
            "至下午5点。",
        ];
        let mut segmenter = Segmenter::default();
        let mut output = Vec::new();
        for (i, source) in finals.iter().enumerate() {
            output.extend(segmenter.push_final_ordered(
                source,
                i as u64 * 10_000,
                Some(i as u64 + 1),
            ));
        }
        output.extend(segmenter.finish(23_000));
        assert_eq!(text(&output), finals.concat());
        assert_eq!(output.len(), 4);
    }

    #[test]
    fn a_large_final_is_split_into_complete_sentences() {
        let source: String = (1..=20).map(|id| format!("第{id}文です。")).collect();
        let mut segmenter = Segmenter::default();
        let output = segmenter.push_final_ordered(&source, 0, Some(10));
        assert_eq!(output.len(), 20);
        assert_eq!(text(&output), source);
    }

    #[test]
    fn size_boundaries_preserve_punctuation_quotes_and_unicode_graphemes() {
        for source in [
            "a".repeat(800) + ".",
            "a".repeat(799) + "e\u{301}。",
            "あ".repeat(795) + "👨‍👩‍👧‍👦" + &"いう".repeat(40) + "。",
            "He said 'Stop.'".into(),
        ] {
            let output = split_for_model(&source);
            assert_eq!(output.concat(), source);
            assert_eq!(
                output
                    .iter()
                    .flat_map(|s| s.graphemes(true))
                    .collect::<Vec<_>>(),
                source.graphemes(true).collect::<Vec<_>>()
            );
        }
    }
}
