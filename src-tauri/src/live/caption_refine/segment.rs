//! Sentence-sized caption segments for correction and translation.
//!
//! Partials are a sliding recognition window, not a transcript. Text stays
//! together until a sentence ends, the speaker actually stops, or the delay
//! budget reaches a clause boundary. Stored transcript lines are never rewritten.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

const JOIN_AFTER_FINAL_MS: u64 = 2_000;
const STABLE_MS: u64 = 400;
const CLAUSE_BUDGET_MS: u64 = 8_000;
const HARD_BUDGET_MS: u64 = 9_000;
const MAX_SEGMENT_CHARS: usize = 800;
const COMMITTED_TAIL_CHARS: usize = 480;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CommittedSegment {
    pub id: u64,
    pub text: String,
}

#[derive(Debug, Default)]
pub(super) struct Segmenter {
    pending: String,
    window: String,
    window_ms: Option<u64>,
    pending_since_ms: Option<u64>,
    closed_at_ms: Option<u64>,
    next_id: u64,
    committed_tail: String,
    // Only partial-derived output can be acknowledged by a later final.
    // Final events already represent distinct, admitted transcript lines.
    provisional_committed: String,
    provisional_spans: VecDeque<(u64, usize)>,
    finalized_pending: String,
    window_seq: u64,
    last_final_seq: u64,
    decoding_finals: BTreeSet<u64>,
    waiting_partials: BTreeMap<u64, (String, u64)>,
    new_utterance: bool,
}

impl Segmenter {
    pub(super) fn begin_final(&mut self, seq: u64) {
        if seq > self.last_final_seq {
            self.decoding_finals.insert(seq);
        }
    }

    pub(super) fn end_final(&mut self, seq: u64, now_ms: u64) -> Vec<CommittedSegment> {
        self.decoding_finals.remove(&seq);
        if seq >= self.last_final_seq {
            self.last_final_seq = seq;
            let bytes = self.provisional_bytes_through(seq);
            self.consume_provisional(bytes);
            // The next partial belongs to a different VAD utterance. Repeated
            // words there are speech, not a rolling-window echo of this final.
            self.committed_tail.clear();
            self.window.clear();
            self.window_ms = None;
            self.new_utterance = true;
        }
        self.release_partials(now_ms)
    }

    pub(super) fn audio_drained(&mut self, now_ms: u64) -> Vec<CommittedSegment> {
        // A decoder failure can discard jobs before they get an end marker.
        // Called only after both recognition lanes have joined.
        let mut out = Vec::new();
        while let Some(seq) = self.decoding_finals.first().copied() {
            out.extend(self.end_final(seq, now_ms));
        }
        out.extend(self.release_partials(now_ms));
        out
    }

    fn release_partials(&mut self, now_ms: u64) -> Vec<CommittedSegment> {
        let mut out = Vec::new();
        while let Some((&seq, _)) = self.waiting_partials.first_key_value() {
            if self
                .decoding_finals
                .first()
                .is_some_and(|older| *older < seq)
            {
                break;
            }
            let (text, observed_at) = self.waiting_partials.pop_first().unwrap().1;
            // Replay stability at the observed times. A later sliding window
            // may no longer contain a sentence that stabilized while waiting.
            out.extend(self.on_tick(observed_at));
            out.extend(self.push_partial_ordered(&text, observed_at, seq));
        }
        out.extend(self.on_tick(now_ms));
        out
    }

    #[cfg(test)]
    pub(super) fn push_partial(&mut self, text: &str, now_ms: u64) -> Vec<CommittedSegment> {
        self.push_partial_ordered(text, now_ms, self.window_seq.max(self.last_final_seq) + 1)
    }

    pub(super) fn push_partial_ordered(
        &mut self,
        text: &str,
        now_ms: u64,
        seq: u64,
    ) -> Vec<CommittedSegment> {
        if seq <= self.last_final_seq || seq <= self.window_seq {
            return Vec::new();
        }
        if self
            .decoding_finals
            .first()
            .is_some_and(|older| *older < seq)
        {
            self.waiting_partials
                .insert(seq, (text.to_string(), now_ms));
            return Vec::new();
        }
        self.window_seq = seq;
        let text = strip_committed_prefix(&self.committed_tail, text);
        let text = text.as_str();
        if text.is_empty() {
            return self.on_tick(now_ms);
        }
        let before = self.pending.clone();
        let new_utterance = std::mem::take(&mut self.new_utterance);
        self.pending = if new_utterance {
            join_caption(&self.pending, text)
        } else {
            integrate(&self.pending, &self.window, text)
        };
        self.window = text.to_string();
        self.window_ms = Some(now_ms);
        if self.pending_since_ms.is_none() && meaningful(&self.pending) {
            self.pending_since_ms = Some(now_ms);
        }
        if new_utterance || continues_after_close(&before, &self.pending, text) {
            self.closed_at_ms = None;
        }
        self.commit_stable(now_ms, false)
    }

    #[cfg(test)]
    pub(super) fn push_final(&mut self, text: &str, now_ms: u64) -> Vec<CommittedSegment> {
        self.push_final_ordered(
            text,
            now_ms,
            Some(self.window_seq.max(self.last_final_seq) + 1),
        )
    }

    pub(super) fn push_final_ordered(
        &mut self,
        text: &str,
        now_ms: u64,
        seq: Option<u64>,
    ) -> Vec<CommittedSegment> {
        if let Some(seq) = seq {
            if seq <= self.last_final_seq {
                return Vec::new();
            }
            self.last_final_seq = seq;
        }
        let text = text.trim();
        if text.is_empty() {
            return self.on_tick(now_ms);
        }
        let newer_window = seq.is_none_or(|seq| self.window_seq > seq);
        let future = newer_window.then(|| {
            (
                self.pending
                    .strip_prefix(&self.finalized_pending)
                    .unwrap_or(&self.pending)
                    .to_string(),
                self.window.clone(),
                self.window_ms,
                self.pending_since_ms,
            )
        });
        let fresh = self.confirm_provisional(text, seq);
        // Replace speculative recognition with this final, keeping only the
        // unfinished text from earlier finals. Do not deduplicate distinct finals.
        self.pending = join_caption(&self.finalized_pending, &fresh);
        self.finalized_pending = self.pending.clone();
        self.window.clear();
        self.window_ms = None;
        if meaningful(&self.pending) {
            if self.pending_since_ms.is_none() {
                self.pending_since_ms = Some(now_ms);
            }
            self.closed_at_ms = Some(now_ms);
        } else {
            self.pending_since_ms = None;
            self.closed_at_ms = None;
        }
        let out = self.commit_stable(now_ms, true);
        if let Some((pending, window, window_ms, since)) = future {
            let pending = strip_committed_prefix(&self.committed_tail, &pending);
            self.pending = overlap_join(&self.pending, &pending, 4);
            self.window = strip_committed_prefix(&self.committed_tail, &window);
            self.window_ms = window_ms;
            if meaningful(&pending) {
                self.pending_since_ms = self.pending_since_ms.or(since).or(Some(now_ms));
                self.closed_at_ms = None;
            }
        }
        out
    }

    fn confirm_provisional(&mut self, text: &str, seq: Option<u64>) -> String {
        let Some(seq) = seq else {
            // Manual transcript lines do not acknowledge captured audio.
            return text.to_string();
        };
        let older_bytes = self.provisional_bytes_through(seq);
        let eligible = &self.provisional_committed[..older_bytes];
        if let Some(bytes) = caption_prefix(eligible, text) {
            self.consume_provisional(bytes);
            return String::new();
        }
        if let Some(bytes) = caption_prefix(text, eligible) {
            let fresh = text[bytes..].trim().to_string();
            self.consume_provisional(older_bytes);
            return fresh;
        }
        // Recognition changed. Retire only predictions captured before this
        // final, preserving a newer utterance decoded on the parallel lane.
        self.consume_provisional(older_bytes);
        text.to_string()
    }

    fn provisional_bytes_through(&self, seq: u64) -> usize {
        self.provisional_spans
            .iter()
            .take_while(|(capture, _)| *capture <= seq)
            .map(|(_, bytes)| bytes)
            .sum()
    }

    fn consume_provisional(&mut self, mut bytes: usize) {
        self.provisional_committed.drain(..bytes);
        while bytes > 0 {
            let Some((_, remaining)) = self.provisional_spans.front_mut() else {
                break;
            };
            if *remaining > bytes {
                *remaining -= bytes;
                break;
            }
            bytes -= *remaining;
            self.provisional_spans.pop_front();
        }
    }

    pub(super) fn finish(&mut self, now_ms: u64) -> Vec<CommittedSegment> {
        let mut out = self.audio_drained(now_ms);
        out.extend(self.commit_stable(now_ms, true));
        if meaningful(&self.pending) {
            out.extend(self.commit_all());
        }
        out
    }

    pub(super) fn on_tick(&mut self, now_ms: u64) -> Vec<CommittedSegment> {
        let mut out = self.commit_stable(now_ms, false);
        if let Some(closed) = self.closed_at_ms {
            if now_ms.saturating_sub(closed) >= JOIN_AFTER_FINAL_MS && meaningful(&self.pending) {
                out.extend(self.commit_all());
                return out;
            }
        }
        out.extend(self.budget_cut(now_ms));
        out
    }

    fn commit_stable(&mut self, now_ms: u64, from_final: bool) -> Vec<CommittedSegment> {
        let (done, rest) = split_complete_sentences(&self.pending);
        if !meaningful(&done) {
            return Vec::new();
        }
        if !from_final && boundary_unstable(&self.window, &done, self.window_ms, now_ms) {
            return Vec::new();
        }
        self.pending = rest;
        if self.pending.is_empty() {
            self.pending_since_ms = None;
            self.closed_at_ms = None;
            self.window.clear();
            self.window_ms = None;
        } else {
            self.pending_since_ms = Some(now_ms);
        }
        let out = self.commit_owned(done);
        self.window = strip_committed_prefix(&self.committed_tail, &self.window);
        out
    }

    fn budget_cut(&mut self, now_ms: u64) -> Vec<CommittedSegment> {
        let Some(since) = self.pending_since_ms else {
            return Vec::new();
        };
        let age = now_ms.saturating_sub(since);
        if age < CLAUSE_BUDGET_MS || !meaningful(&self.pending) {
            return Vec::new();
        }
        if let Some((done, rest)) = split_clause(&self.pending) {
            self.pending = rest;
            self.pending_since_ms = if self.pending.is_empty() {
                None
            } else {
                Some(now_ms)
            };
            self.closed_at_ms = None;
            let out = self.commit_owned(done);
            self.window = strip_committed_prefix(&self.committed_tail, &self.window);
            return out;
        }
        if age < HARD_BUDGET_MS {
            return Vec::new();
        }
        self.commit_all()
    }

    fn commit_all(&mut self) -> Vec<CommittedSegment> {
        let text = std::mem::take(&mut self.pending);
        self.window.clear();
        self.window_ms = None;
        self.pending_since_ms = None;
        self.closed_at_ms = None;
        self.commit_owned(text)
    }

    fn commit_owned(&mut self, text: String) -> Vec<CommittedSegment> {
        let mut out = Vec::new();
        for chunk in split_for_model(&text) {
            if !meaningful(&chunk) {
                continue;
            }
            if let Some(rest) = self.finalized_pending.strip_prefix(&chunk) {
                self.finalized_pending = rest.trim_start().to_string();
            } else {
                let provisional = chunk
                    .strip_prefix(&self.finalized_pending)
                    .unwrap_or(&chunk);
                let before = self.provisional_committed.len();
                self.provisional_committed = join_caption(&self.provisional_committed, provisional);
                let bytes = self.provisional_committed.len() - before;
                if bytes > 0 {
                    self.provisional_spans.push_back((self.window_seq, bytes));
                }
                self.finalized_pending.clear();
            }
            self.remember(&chunk);
            self.next_id += 1;
            out.push(CommittedSegment {
                id: self.next_id,
                text: chunk,
            });
        }
        out
    }

    fn remember(&mut self, text: &str) {
        self.committed_tail = join_caption(&self.committed_tail, text);
        let count = self.committed_tail.chars().count();
        if count > COMMITTED_TAIL_CHARS {
            self.committed_tail = self
                .committed_tail
                .chars()
                .skip(count - COMMITTED_TAIL_CHARS)
                .collect();
        }
    }
}

// Recognition windows and final lines can disagree only about spaces. Return
// the original byte offset so acknowledgements still retire the right captures.
fn caption_prefix(text: &str, prefix: &str) -> Option<usize> {
    let mut source = text.char_indices().filter(|(_, ch)| !ch.is_whitespace());
    let mut end = 0;
    for expected in prefix.chars().filter(|ch| !ch.is_whitespace()) {
        let (offset, actual) = source.next()?;
        if actual != expected {
            return None;
        }
        end = offset + actual.len_utf8();
    }
    Some(end + text[end..].len() - text[end..].trim_start().len())
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
    let space = left.chars().last().is_some_and(|ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '.' | ',' | '!' | '?' | ';' | ':' | ')' | ']')
    }) && right.starts_with(|ch: char| ch.is_ascii_alphanumeric());
    format!("{left}{}{right}", if space { " " } else { "" })
}

fn meaningful(text: &str) -> bool {
    text.chars().any(|ch| {
        ch.is_alphanumeric()
            || ('\u{3040}'..='\u{30ff}').contains(&ch)
            || ('\u{4e00}'..='\u{9fff}').contains(&ch)
    })
}

fn continues_after_close(before: &str, after: &str, partial: &str) -> bool {
    partial.chars().count() >= 4
        && !before.contains(partial)
        && after.chars().count() > before.chars().count()
}

fn boundary_unstable(window: &str, done: &str, window_ms: Option<u64>, now_ms: u64) -> bool {
    let tail: String = done
        .chars()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if tail.chars().count() < 2 || !window.contains(&tail) {
        return false;
    }
    window_ms.is_none_or(|at| now_ms.saturating_sub(at) < STABLE_MS)
}

fn integrate(pending: &str, old_window: &str, new_window: &str) -> String {
    if new_window.is_empty() {
        return pending.to_string();
    }
    if old_window.is_empty() {
        if pending.is_empty() || new_window.contains(pending) {
            return new_window.to_string();
        }
        if new_window.chars().count() >= 4 && pending.contains(new_window) {
            return pending.to_string();
        }
        return overlap_join(pending, new_window, 4);
    }
    let scrolled = scrolled_out(old_window, new_window);
    if let Some(before_window) = strip_suffix(pending, old_window) {
        let base = overlap_join(&before_window, &scrolled, 4);
        return overlap_join(&base, new_window, 4);
    }
    if new_window.chars().count() >= 4 && pending.contains(new_window) {
        return pending.to_string();
    }
    overlap_join(pending, new_window, 4)
}

fn split_complete_sentences(text: &str) -> (String, String) {
    let chars: Vec<char> = text.chars().collect();
    let mut last = None;
    for (index, _) in chars.iter().enumerate() {
        if is_sentence_end(&chars, index) {
            last = Some(index);
        }
    }
    let Some(end) = last else {
        return (String::new(), text.trim().to_string());
    };
    let done: String = chars[..=end].iter().collect();
    let rest: String = chars[end + 1..].iter().collect();
    (done.trim().to_string(), rest.trim().to_string())
}

fn split_clause(text: &str) -> Option<(String, String)> {
    let chars: Vec<char> = text.chars().collect();
    let mut last = None;
    for (index, ch) in chars.iter().enumerate() {
        if index + 1 < 12 {
            continue;
        }
        if matches!(ch, '、' | '，' | ',')
            && !(index > 0 && chars[index - 1].is_ascii_digit())
            && !chars
                .get(index + 1)
                .is_some_and(|next| next.is_ascii_digit())
        {
            last = Some(index);
        }
    }
    let end = last?;
    let done: String = chars[..=end].iter().collect();
    let rest: String = chars[end + 1..].iter().collect();
    if !meaningful(&done) {
        return None;
    }
    Some((done.trim().to_string(), rest.trim().to_string()))
}

pub(super) fn sentence_units(text: &str) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut count = 0usize;
    let mut last_end = None;
    for index in 0..chars.len() {
        if is_sentence_end(&chars, index) {
            count += 1;
            last_end = Some(index);
        }
    }
    let tail_start = last_end.map(|index| index + 1).unwrap_or(0);
    let tail: String = chars[tail_start..].iter().collect();
    if meaningful(tail.trim()) {
        count += 1;
    }
    count
}

fn is_sentence_end(chars: &[char], index: usize) -> bool {
    match chars[index] {
        '。' | '！' | '？' | '!' | '?' => true,
        '.' => {
            let prev_digit = index > 0 && chars[index - 1].is_ascii_digit();
            let next_digit = chars.get(index + 1).is_some_and(|ch| ch.is_ascii_digit());
            !prev_digit && !next_digit
        }
        _ => false,
    }
}

fn scrolled_out(previous: &str, current: &str) -> String {
    if previous.is_empty() || previous == current {
        return String::new();
    }
    let prev: Vec<char> = previous.chars().collect();
    let curr: Vec<char> = current.chars().collect();
    let max = prev.len().min(curr.len());
    if max >= 4 {
        for len in (4..=max).rev() {
            if prev[prev.len() - len..] == curr[..len] {
                return prev[..prev.len() - len].iter().collect();
            }
        }
    }
    if curr.len() >= 4 {
        let needle: String = curr.iter().take(8.min(curr.len())).collect();
        if previous.contains(&needle) {
            return String::new();
        }
    }
    String::new()
}

fn overlap_join(left: &str, right: &str, min_overlap: usize) -> String {
    if left.is_empty() {
        return right.trim().to_string();
    }
    if right.is_empty() {
        return left.trim().to_string();
    }
    if left.ends_with(right) {
        return left.to_string();
    }
    let left_chars: Vec<char> = left.chars().collect();
    let right_chars: Vec<char> = right.chars().collect();
    let max = left_chars.len().min(right_chars.len());
    if max >= min_overlap {
        for len in (min_overlap..=max).rev() {
            if left_chars[left_chars.len() - len..] == right_chars[..len] {
                let mut out: String = left_chars.iter().collect();
                out.extend(right_chars[len..].iter().copied());
                return out;
            }
        }
    }
    join_caption(left, right)
}

fn strip_suffix(text: &str, suffix: &str) -> Option<String> {
    if suffix.is_empty() {
        return Some(text.to_string());
    }
    let text_chars: Vec<char> = text.chars().collect();
    let suffix_chars: Vec<char> = suffix.chars().collect();
    if text_chars.len() < suffix_chars.len() {
        return None;
    }
    let start = text_chars.len() - suffix_chars.len();
    if text_chars[start..] != suffix_chars[..] {
        return None;
    }
    Some(text_chars[..start].iter().collect())
}

fn strip_committed_prefix(committed: &str, text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }
    if text.chars().count() >= 8 && committed.contains(text) {
        return String::new();
    }
    let committed: Vec<char> = committed.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();
    let max = committed.len().min(text_chars.len());
    if max >= 4 {
        for len in (4..=max).rev() {
            if committed[committed.len() - len..] == text_chars[..len] {
                return text_chars[len..]
                    .iter()
                    .collect::<String>()
                    .trim()
                    .to_string();
            }
        }
    }
    text.to_string()
}

fn split_for_model(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if chars.len() <= MAX_SEGMENT_CHARS {
        return vec![trimmed.to_string()];
    }
    let mut out = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let end_limit = (start + MAX_SEGMENT_CHARS).min(chars.len());
        let mut end = end_limit;
        if end_limit < chars.len() {
            if let Some(rel) = chars[start..end_limit]
                .iter()
                .enumerate()
                .rev()
                .find(|(rel, _)| *rel + 1 >= 12 && is_sentence_end(&chars[start..end_limit], *rel))
                .map(|(rel, _)| rel)
            {
                end = start + rel + 1;
            }
        }
        let piece: String = chars[start..end].iter().collect();
        let piece = piece.trim().to_string();
        if !piece.is_empty() {
            out.push(piece);
        }
        if end == start {
            break;
        }
        start = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentence_units_count_finished_sentences_and_a_trailing_clause() {
        assert_eq!(sentence_units("一です。二です。三です。"), 3);
        assert_eq!(sentence_units("一です。まだ続く"), 2);
        assert_eq!(sentence_units("まだ句点がない"), 1);
        assert_eq!(sentence_units("   "), 0);
    }

    #[test]
    fn finish_commits_the_unfinished_tail_once() {
        let mut segmenter = Segmenter::default();
        assert!(segmenter.push_partial("次に分散を定義", 0).is_empty());
        let committed = segmenter.finish(100);
        assert_eq!(committed.len(), 1);
        assert_eq!(committed[0].text, "次に分散を定義");
        assert!(segmenter.finish(200).is_empty());
    }

    #[test]
    fn short_pause_does_not_cut_an_unfinished_sentence() {
        let mut segmenter = Segmenter::default();
        assert!(segmenter
            .push_partial("今日は確率の話をします", 0)
            .is_empty());
        assert!(segmenter.on_tick(1_200).is_empty());
        assert!(segmenter.on_tick(6_500).is_empty());
        assert!(segmenter.pending.contains("確率"));
    }

    #[test]
    fn sliding_window_keeps_the_scrolled_prefix_until_the_sentence_ends() {
        let mut segmenter = Segmenter::default();
        assert!(segmenter
            .push_partial("今日は確率の話をします", 0)
            .is_empty());
        assert!(segmenter
            .push_partial("確率の話をします次に期待値を定義します。", 600)
            .is_empty());
        let committed = segmenter.on_tick(600 + STABLE_MS);
        assert_eq!(
            committed
                .iter()
                .map(|segment| segment.text.as_str())
                .collect::<Vec<_>>(),
            vec!["今日は確率の話をします次に期待値を定義します。"]
        );
        assert!(segmenter
            .push_final("今日は確率の話をします次に期待値を定義します。", 1_200)
            .is_empty());
    }

    #[test]
    fn same_window_revision_replaces_the_tail_instead_of_appending_it() {
        let mut segmenter = Segmenter::default();
        assert!(segmenter.push_partial("確率の話をします", 0).is_empty());
        assert!(segmenter.push_partial("確立の話をします", 600).is_empty());
        assert!(segmenter.on_tick(1_000).is_empty());
        assert_eq!(segmenter.pending, "確立の話をします");
    }

    #[test]
    fn sentence_end_commits_only_the_finished_sentence() {
        let mut segmenter = Segmenter::default();
        assert!(segmenter
            .push_partial("期待値を定義します。次に", 100)
            .is_empty());
        let committed = segmenter.on_tick(100 + STABLE_MS);
        assert_eq!(committed.len(), 1);
        assert_eq!(committed[0].text, "期待値を定義します。");
        assert_eq!(segmenter.pending, "次に");
    }

    #[test]
    fn a_revised_window_does_not_reinsert_a_committed_sentence() {
        let mut segmenter = Segmenter::default();
        segmenter.push_partial("期待値を定義します。次に", 100);
        let first = segmenter.on_tick(500);
        assert_eq!(first[0].text, "期待値を定義します。");
        segmenter.push_partial("期待値を定義します。次に分散です。", 600);
        let second = segmenter.on_tick(1_000);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].text, "次に分散です。");
        assert!(segmenter
            .push_final("期待値を定義します。次に分散です。", 1_200)
            .is_empty());
    }

    #[test]
    fn a_revised_window_after_a_clause_cut_keeps_only_the_remaining_text() {
        let mut segmenter = Segmenter::default();
        let text = "今日は確率の話をします、次に期待値を定義してから分散に進みます";
        segmenter.push_partial(text, 0);
        assert_eq!(
            segmenter.on_tick(CLAUSE_BUDGET_MS)[0].text,
            "今日は確率の話をします、"
        );
        segmenter.push_partial(&format!("{text}。"), CLAUSE_BUDGET_MS + 100);
        let rest = segmenter.on_tick(CLAUSE_BUDGET_MS + 100 + STABLE_MS);
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].text, "次に期待値を定義してから分散に進みます。");
    }

    #[test]
    fn distinct_finals_preserve_identical_utterances() {
        let mut segmenter = Segmenter::default();
        for index in 0..3 {
            let next = segmenter.push_final("これは重要な定義です。", index * 5_000);
            assert_eq!(next.len(), 1);
            assert_eq!(next[0].id, index + 1);
            assert_eq!(next[0].text, "これは重要な定義です。");
        }
    }

    #[test]
    fn a_final_acknowledges_a_partial_once_without_swallowing_the_next_utterance() {
        let mut segmenter = Segmenter::default();
        segmenter.push_partial("これは重要な定義です。", 0);
        assert_eq!(segmenter.on_tick(STABLE_MS).len(), 1);
        assert!(segmenter
            .push_final("これは重要な定義です。", 500)
            .is_empty());
        let repeated = segmenter.push_final("これは重要な定義です。", 5_000);
        assert_eq!(repeated.len(), 1);
        assert_eq!(repeated[0].text, "これは重要な定義です。");
    }

    #[test]
    fn separately_committed_english_sentences_are_acknowledged_by_one_final() {
        let mut segmenter = Segmenter::default();
        segmenter.push_partial_ordered("First sentence. Second", 0, 5);
        assert_eq!(segmenter.on_tick(STABLE_MS)[0].text, "First sentence.");
        segmenter.push_partial_ordered("First sentence. Second sentence.", 600, 6);
        assert_eq!(segmenter.on_tick(1_000)[0].text, "Second sentence.");
        assert!(segmenter
            .push_final_ordered("First sentence. Second sentence.", 1_100, Some(10))
            .is_empty());
        assert_eq!(
            segmenter.push_final_ordered("Second sentence.", 2_000, Some(20))[0].text,
            "Second sentence."
        );
    }

    #[test]
    fn separate_english_finals_acknowledge_whitespace_revisions_once() {
        let mut segmenter = Segmenter::default();
        segmenter.push_partial_ordered("First sentence.  Second sentence.", 0, 5);
        assert_eq!(segmenter.on_tick(STABLE_MS).len(), 1);
        assert!(segmenter
            .push_final_ordered("First sentence.", 500, Some(10))
            .is_empty());
        assert!(segmenter
            .push_final_ordered("Second sentence.", 600, Some(20))
            .is_empty());
        assert!(segmenter.finish(1_000).is_empty());
    }

    #[test]
    fn english_finals_keep_word_boundaries_across_a_breath() {
        let mut segmenter = Segmenter::default();
        assert!(segmenter
            .push_final_ordered("This is", 0, Some(10))
            .is_empty());
        assert_eq!(
            segmenter.push_final_ordered("the expectation.", 500, Some(20))[0].text,
            "This is the expectation."
        );
    }

    #[test]
    fn one_partial_can_be_acknowledged_by_separate_delayed_finals() {
        let mut segmenter = Segmenter::default();
        segmenter.push_partial("期待値を定義します。次に分散です。", 0);
        assert_eq!(segmenter.on_tick(STABLE_MS).len(), 1);
        assert!(segmenter.push_final("期待値を定義します。", 500).is_empty());
        assert!(segmenter.push_final("次に分散です。", 600).is_empty());
        let repeated = segmenter.push_final("次に分散です。", 5_000);
        assert_eq!(repeated.len(), 1);
        assert_eq!(repeated[0].text, "次に分散です。");
    }

    #[test]
    fn an_older_final_preserves_a_newer_committed_partial() {
        let mut segmenter = Segmenter::default();
        let newer = "次に分散を定義します。";
        segmenter.push_partial_ordered(newer, 0, 20);
        let mut all = segmenter.on_tick(STABLE_MS);
        all.extend(segmenter.push_final_ordered("まず期待値を定義します。", 500, Some(10)));
        assert!(segmenter
            .push_final_ordered(newer, 600, Some(30))
            .is_empty());
        assert_eq!(all.iter().filter(|item| item.text == newer).count(), 1);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn an_older_final_preserves_a_newer_partial_still_waiting_for_stability() {
        let mut segmenter = Segmenter::default();
        let newer = "次に分散を定義します。";
        segmenter.push_partial_ordered(newer, 0, 20);
        let older = segmenter.push_final_ordered("まず期待値を定義します。", 100, Some(10));
        assert_eq!(older.len(), 1);
        assert_eq!(segmenter.on_tick(STABLE_MS)[0].text, newer);
        assert!(segmenter
            .push_final_ordered(newer, 600, Some(30))
            .is_empty());
    }

    #[test]
    fn a_changed_older_final_retires_only_its_own_predictions() {
        let mut segmenter = Segmenter::default();
        segmenter.push_partial_ordered("今日は確立の話です。", 0, 5);
        segmenter.on_tick(STABLE_MS);
        segmenter.push_partial_ordered("次に分散です。", 500, 20);
        segmenter.on_tick(500 + STABLE_MS);
        segmenter.push_final_ordered("今日は確率の話です。", 1_000, Some(10));
        assert!(segmenter
            .push_final_ordered("次に分散です。", 1_100, Some(30))
            .is_empty());
    }

    #[test]
    fn capture_replays_cannot_reinsert_confirmed_speech() {
        let mut segmenter = Segmenter::default();
        assert_eq!(
            segmenter
                .push_final_ordered("期待値を定義します。", 0, Some(10))
                .len(),
            1
        );
        assert!(segmenter
            .push_partial_ordered("古い仮字幕です。", 100, 5)
            .is_empty());
        assert!(segmenter
            .push_final_ordered("期待値を定義します。", 200, Some(10))
            .is_empty());
        assert!(segmenter.on_tick(1_000).is_empty());
        // The same words from a separate VAD segment still count as speech.
        assert_eq!(
            segmenter
                .push_final_ordered("期待値を定義します。", 1_100, Some(30))
                .len(),
            1
        );
    }

    #[test]
    fn an_unordered_manual_line_does_not_acknowledge_newer_audio() {
        let mut segmenter = Segmenter::default();
        segmenter.push_partial_ordered("音声の字幕です。", 0, 20);
        segmenter.on_tick(STABLE_MS);
        assert_eq!(
            segmenter
                .push_final_ordered("手動の行です。", 500, None)
                .len(),
            1
        );
        assert!(segmenter
            .push_final_ordered("音声の字幕です。", 600, Some(30))
            .is_empty());
    }

    #[test]
    fn an_older_final_cannot_acknowledge_a_newer_utterances_matching_prefix() {
        let mut segmenter = Segmenter::default();
        let newer = "期待値を定義します。次に分散です。";
        segmenter.push_partial_ordered(newer, 0, 20);
        assert_eq!(segmenter.on_tick(STABLE_MS)[0].text, newer);
        let older = segmenter.push_final_ordered("期待値を定義します。", 500, Some(10));
        assert_eq!(older[0].text, "期待値を定義します。");
        assert!(segmenter
            .push_final_ordered(newer, 600, Some(30))
            .is_empty());
    }

    #[test]
    fn later_partials_wait_for_captured_finals_and_keep_repeated_sentences() {
        let mut segmenter = Segmenter::default();
        let first = "期待値を定義します。";
        let newer = "期待値を定義します。次に分散です。";
        segmenter.begin_final(10);
        assert!(segmenter.push_partial_ordered(newer, 0, 20).is_empty());
        segmenter.begin_final(30);
        assert!(segmenter.on_tick(STABLE_MS).is_empty());
        let mut out = segmenter.push_final_ordered(first, 500, Some(10));
        out.extend(segmenter.end_final(10, 500));
        assert_eq!(
            out.iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>(),
            [first, newer]
        );
        assert!(segmenter
            .push_final_ordered(newer, 600, Some(30))
            .is_empty());
        assert!(segmenter.end_final(30, 600).is_empty());
    }

    #[test]
    fn waiting_for_a_slow_final_preserves_sentences_that_scroll_out_of_later_windows() {
        let mut segmenter = Segmenter::default();
        let second = "次に分散を定義します。";
        let third = "最後に正規分布です。";
        segmenter.begin_final(10);
        segmenter.push_partial_ordered(second, 100, 20);
        segmenter.push_partial_ordered(third, 6_100, 25);
        assert!(segmenter.on_tick(6_500).is_empty());
        segmenter.push_final_ordered("まず期待値を定義します。", 7_000, Some(10));
        let released = segmenter.end_final(10, 7_000);
        assert_eq!(
            released
                .iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>(),
            [second, third]
        );
        assert!(segmenter
            .push_final_ordered(&format!("{second}{third}"), 7_100, Some(30))
            .is_empty());
    }

    #[test]
    fn capture_boundaries_keep_repeated_partial_sentences_from_being_stripped() {
        let mut segmenter = Segmenter::default();
        let repeated = "これは重要な定義です。";
        assert_eq!(segmenter.push_final_ordered(repeated, 0, Some(10)).len(), 1);
        segmenter.end_final(10, 0);
        segmenter.push_partial_ordered(repeated, 100, 20);
        assert_eq!(segmenter.on_tick(500)[0].text, repeated);
        assert!(segmenter
            .push_final_ordered(repeated, 600, Some(30))
            .is_empty());
    }

    #[test]
    fn a_new_utterance_keeps_a_finalized_unfinished_prefix_even_if_it_repeats() {
        let mut segmenter = Segmenter::default();
        segmenter.push_final_ordered("期待値の", 0, Some(10));
        segmenter.end_final(10, 0);
        segmenter.push_partial_ordered("期待値の定義です。", 100, 20);
        assert_eq!(segmenter.on_tick(500)[0].text, "期待値の期待値の定義です。");
        assert!(segmenter
            .push_final_ordered("期待値の定義です。", 600, Some(30))
            .is_empty());
    }

    #[test]
    fn empty_finals_and_failed_decode_cleanup_do_not_block_later_captions() {
        for cleanup in [false, true] {
            let mut segmenter = Segmenter::default();
            segmenter.begin_final(10);
            segmenter.push_partial_ordered("後の発言です。", 0, 20);
            assert!(segmenter.on_tick(STABLE_MS).is_empty());
            let out = if cleanup {
                segmenter.audio_drained(500)
            } else {
                segmenter.end_final(10, 500)
            };
            assert_eq!(out[0].text, "後の発言です。");
            assert!(segmenter
                .push_final_ordered("後の発言です。", 600, Some(30))
                .is_empty());
            assert!(segmenter.finish(700).is_empty());
        }
    }

    #[test]
    fn failed_decode_cleanup_keeps_boundaries_between_discarded_final_jobs() {
        let mut segmenter = Segmenter::default();
        let second = "同じ定理です。";
        let third = "同じ定理です。追加の説明です。";
        segmenter.begin_final(10);
        segmenter.push_partial_ordered(second, 100, 20);
        segmenter.begin_final(30);
        segmenter.push_partial_ordered(third, 1_000, 40);
        let out = segmenter.audio_drained(1_500);
        assert_eq!(
            out.iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>(),
            [second, third]
        );
        assert!(segmenter.finish(2_000).is_empty());
    }

    #[test]
    fn a_fully_acknowledged_final_does_not_age_the_next_partial() {
        let mut segmenter = Segmenter::default();
        segmenter.push_partial("期待値を定義します。次に", 0);
        assert_eq!(segmenter.on_tick(STABLE_MS).len(), 1);
        assert!(segmenter.push_final("期待値を定義します。", 500).is_empty());
        segmenter.push_partial("分散を定義しています", 8_500);
        assert!(segmenter.on_tick(9_500).is_empty());
        assert_eq!(
            segmenter.on_tick(8_500 + HARD_BUDGET_MS)[0].text,
            "分散を定義しています"
        );
    }

    #[test]
    fn a_finalized_unpunctuated_tail_is_not_acknowledged_by_the_next_final() {
        let mut segmenter = Segmenter::default();
        segmenter.push_final("これは重要な定義です", 0);
        assert_eq!(segmenter.on_tick(JOIN_AFTER_FINAL_MS).len(), 1);
        segmenter.push_partial("これは重要な定義です", 3_000);
        segmenter.push_final("これは重要な定義です", 4_000);
        let repeated = segmenter.on_tick(4_000 + JOIN_AFTER_FINAL_MS);
        assert_eq!(repeated.len(), 1);
        assert_eq!(repeated[0].text, "これは重要な定義です");
    }

    #[test]
    fn model_chunking_keeps_repeated_passages_within_one_final() {
        let mut segmenter = Segmenter::default();
        let passage = "あ".repeat(MAX_SEGMENT_CHARS - 1) + "。";
        let repeated = segmenter.push_final(&passage.repeat(2), 0);
        assert_eq!(repeated.len(), 2);
        assert_eq!(repeated[0].text, passage);
        assert_eq!(repeated[1].text, passage);
    }

    #[test]
    fn finals_join_across_a_breath_until_the_sentence_ends() {
        let mut segmenter = Segmenter::default();
        assert!(segmenter.push_final("今日は確率の", 0).is_empty());
        assert!(segmenter.on_tick(1_000).is_empty());
        let committed = segmenter.push_final("話をします。", 1_200);
        assert_eq!(committed.len(), 1);
        assert_eq!(committed[0].text, "今日は確率の話をします。");
    }

    #[test]
    fn final_without_punctuation_waits_for_the_speaker_to_stop() {
        let mut segmenter = Segmenter::default();
        assert!(segmenter.push_final("期待値を定義します", 0).is_empty());
        assert!(segmenter.on_tick(JOIN_AFTER_FINAL_MS - 1).is_empty());
        let committed = segmenter.on_tick(JOIN_AFTER_FINAL_MS);
        assert_eq!(committed.len(), 1);
        assert_eq!(committed[0].text, "期待値を定義します");
    }

    #[test]
    fn echo_partial_does_not_cancel_the_join_pause() {
        let mut segmenter = Segmenter::default();
        assert!(segmenter.push_final("期待値を定義します", 0).is_empty());
        assert!(segmenter.push_partial("期待値を定義します", 500).is_empty());
        let committed = segmenter.on_tick(JOIN_AFTER_FINAL_MS);
        assert_eq!(committed.len(), 1);
        assert_eq!(committed[0].text, "期待値を定義します");
    }

    #[test]
    fn continued_speech_is_not_flushed_as_a_fragment() {
        let mut segmenter = Segmenter::default();
        assert!(segmenter.push_final("期待値を定義します", 0).is_empty());
        assert!(segmenter
            .push_partial("定義します次の式で表せます", 500)
            .is_empty());
        assert!(segmenter.on_tick(JOIN_AFTER_FINAL_MS).is_empty());
        assert!(segmenter.pending.contains("次の式"));
    }

    #[test]
    fn budget_cuts_at_a_clause_and_keeps_the_rest() {
        let mut segmenter = Segmenter::default();
        let text = "今日は確率の話をします、次に期待値を定義してから分散に進みます";
        assert!(segmenter.push_partial(text, 0).is_empty());
        assert!(segmenter.on_tick(CLAUSE_BUDGET_MS - 1).is_empty());
        let committed = segmenter.on_tick(CLAUSE_BUDGET_MS);
        assert_eq!(committed.len(), 1);
        assert_eq!(committed[0].text, "今日は確率の話をします、");
        assert!(segmenter.pending.starts_with("次に期待値"));
    }

    #[test]
    fn hard_budget_commits_one_passage_without_chopping_a_tail() {
        let mut segmenter = Segmenter::default();
        let text = "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほ";
        assert!(segmenter.push_partial(text, 0).is_empty());
        assert!(segmenter.on_tick(CLAUSE_BUDGET_MS).is_empty());
        let committed = segmenter.on_tick(HARD_BUDGET_MS);
        assert_eq!(committed.len(), 1);
        assert_eq!(committed[0].text, text);
        assert!(segmenter.pending.is_empty());
    }

    #[test]
    fn revisions_do_not_postpone_the_budget() {
        let mut segmenter = Segmenter::default();
        assert!(segmenter
            .push_partial("あいうえおかきくけこさしすせそ", 0)
            .is_empty());
        assert!(segmenter
            .push_partial("あいうえおかきくけこさしすせそたち", 6_000)
            .is_empty());
        assert!(!segmenter.on_tick(HARD_BUDGET_MS).is_empty());
    }

    #[test]
    fn already_committed_sentence_is_not_repeated_by_a_later_final() {
        let mut segmenter = Segmenter::default();
        let first = "あ".repeat(80);
        assert!(segmenter.push_partial(&format!("{first}。"), 0).is_empty());
        assert_eq!(segmenter.on_tick(STABLE_MS).len(), 1);
        let rest = format!("{first}。{}", "か".repeat(80));
        assert!(segmenter.push_final(&rest, 500).is_empty());
        let committed = segmenter.on_tick(500 + JOIN_AFTER_FINAL_MS);
        assert_eq!(committed.len(), 1);
        assert!(committed[0].text.chars().all(|ch| ch == 'か'));
    }
}
