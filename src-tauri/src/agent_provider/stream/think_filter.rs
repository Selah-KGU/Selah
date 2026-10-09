use crate::agent_text;

#[cfg(test)]
#[path = "think_filter/tests.rs"]
mod tests;

/// Stateful thinking-tag splitter: routes content inside the block to
/// `on_chunk(text, true)` and everything else to `on_chunk(text, false)`.
/// Tolerates tag boundaries that cross chunks.
pub struct ThinkFilter<F: FnMut(&str, bool) + Send + 'static> {
    inner: F,
    buf: String,
    in_think: bool,
}

impl<F: FnMut(&str, bool) + Send + 'static> ThinkFilter<F> {
    /// Returns `(feed, flush)`. `feed(chunk, is_think)` ingests a chunk;
    /// `flush()` drains any buffered tail (call it once the upstream stream
    /// has ended so a trailing partial thinking block is not silently lost).
    // The return-tuple type expresses exactly what callers consume; abstracting
    // into a `type` alias would just rename it without simplifying the API.
    #[allow(clippy::type_complexity)]
    pub fn wrap_with_flush(
        inner: F,
    ) -> (Box<dyn FnMut(&str, bool) + Send>, Box<dyn FnMut() + Send>) {
        let state = std::sync::Arc::new(std::sync::Mutex::new(ThinkFilter {
            inner,
            buf: String::new(),
            in_think: false,
        }));
        let feed_state = state.clone();
        let feed = Box::new(move |chunk: &str, is_think: bool| {
            let mut guard = match feed_state.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            if is_think {
                (guard.inner)(chunk, true);
                return;
            }
            guard.buf.push_str(chunk);
            guard.drain(false);
        });
        let flush_state = state;
        let flush = Box::new(move || {
            if let Ok(mut guard) = flush_state.lock() {
                guard.drain(true);
            }
        });
        (feed, flush)
    }

    fn drain(&mut self, flush: bool) {
        // The callback consumes a borrowed slice synchronously. Mutate/drain
        // the buffer only after it returns; preserve callback-panic ordering.
        loop {
            if self.in_think {
                if let Some((idx, tag_len)) = agent_text::find_thinking_end_tag(&self.buf) {
                    if idx > 0 {
                        (self.inner)(&self.buf[..idx], true);
                    }
                    self.buf.drain(..idx + tag_len);
                    self.in_think = false;
                    continue;
                }
                let hold = agent_text::holdback(&self.buf, agent_text::THINKING_TAG_HOLDBACK);
                if hold > 0 {
                    (self.inner)(&self.buf[..hold], true);
                    self.buf.drain(..hold);
                }
                if flush && !self.buf.is_empty() {
                    let emit = std::mem::take(&mut self.buf);
                    (self.inner)(&emit, true);
                }
                return;
            } else {
                if let Some((idx, tag_len)) = agent_text::find_thinking_start_tag(&self.buf) {
                    if idx > 0 {
                        (self.inner)(&self.buf[..idx], false);
                    }
                    self.buf.drain(..idx + tag_len);
                    self.in_think = true;
                    continue;
                }
                let hold = agent_text::holdback(&self.buf, agent_text::THINKING_TAG_HOLDBACK);
                if hold > 0 {
                    (self.inner)(&self.buf[..hold], false);
                    self.buf.drain(..hold);
                }
                if flush && !self.buf.is_empty() {
                    let emit = std::mem::take(&mut self.buf);
                    (self.inner)(&emit, false);
                }
                return;
            }
        }
    }
}
