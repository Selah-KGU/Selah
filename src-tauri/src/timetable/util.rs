use crate::config;

/// Char-boundary-safe string preview for logging/error messages.
pub(super) fn safe_preview(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// KGC day letter -> integer (1=Mon .. 6=Sat)
pub(super) fn day_str_to_int(d: &str) -> i32 {
    match d {
        "月" => 1,
        "火" => 2,
        "水" => 3,
        "木" => 4,
        "金" => 5,
        "土" => 6,
        _ => 0,
    }
}

pub(super) fn day_int_to_str(d: i32) -> &'static str {
    if (1..=6).contains(&d) {
        config::DAY_SHORT[d as usize]
    } else {
        "?"
    }
}
