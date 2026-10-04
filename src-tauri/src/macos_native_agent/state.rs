use std::sync::Mutex;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CapsuleMode {
    Listening,
    Processing,
    Result,
    Notice,
}

#[derive(Default)]
pub(super) struct SharedState {
    pub(super) mode: Option<CapsuleMode>,
    pub(super) stop_requested: bool,
    /// All VAD-finalized segments so far, joined with separators.
    pub(super) finals_accumulated: String,
    /// The current in-flight partial (the latest segment not yet finalized).
    pub(super) current_speech: String,
    pub(super) agent_listener: Option<tauri::EventId>,
    pub(super) result_accumulated: String,
}

pub(super) fn append_final_segment(sh: &mut SharedState, text: &str) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    if !sh.finals_accumulated.is_empty() {
        let needs_space = !ends_with_cjk(&sh.finals_accumulated) && !starts_with_cjk(trimmed);
        if needs_space {
            sh.finals_accumulated.push(' ');
        }
    }
    sh.finals_accumulated.push_str(trimmed);
    sh.current_speech.clear();
}

pub(super) fn listening_display_text(sh: &SharedState) -> String {
    let mut out = sh.finals_accumulated.clone();
    let partial = sh.current_speech.trim();
    if !partial.is_empty() {
        if !out.is_empty() {
            let needs_space = !ends_with_cjk(&out) && !starts_with_cjk(partial);
            if needs_space {
                out.push(' ');
            }
        }
        out.push_str(partial);
    }
    out
}

pub(super) fn consume_all_speech(sh: &mut SharedState) -> String {
    let partial = sh.current_speech.trim().to_string();
    let mut out = std::mem::take(&mut sh.finals_accumulated);
    sh.current_speech.clear();
    if !partial.is_empty() {
        if !out.is_empty() {
            let needs_space = !ends_with_cjk(&out) && !starts_with_cjk(&partial);
            if needs_space {
                out.push(' ');
            }
        }
        out.push_str(&partial);
    }
    out.trim().to_string()
}

fn is_cjk_char(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x309F   // Hiragana
        | 0x30A0..=0x30FF // Katakana
        | 0x3400..=0x4DBF | 0x4E00..=0x9FFF // CJK Unified
        | 0xF900..=0xFAFF // CJK Compat
        | 0xFF00..=0xFFEF // Halfwidth/Fullwidth
    )
}

fn ends_with_cjk(s: &str) -> bool {
    s.chars().next_back().map(is_cjk_char).unwrap_or(false)
}

fn starts_with_cjk(s: &str) -> bool {
    s.chars().next().map(is_cjk_char).unwrap_or(false)
}

pub(super) static SHARED: std::sync::LazyLock<Mutex<SharedState>> =
    std::sync::LazyLock::new(|| Mutex::new(SharedState::default()));
