use super::{SharedSummaryChunk, SharedTranscriptLine};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// Sidecar JSON that persists accumulated session data across stop/start within the same course day.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::live) struct LiveDayCache {
    pub(in crate::live) date: String, // YYYY-MM-DD
    pub(in crate::live) course_name: String,
    pub(in crate::live) started_at: String,
    pub(in crate::live) transcript_lines: Vec<SharedTranscriptLine>,
    pub(in crate::live) summaries: Vec<SharedSummaryChunk>,
}

/// Single transcript line appended to the deltas log. Field names are short
/// (`i`/`t`/`a`) because we write one of these per spoken line — saves bytes
/// over a session.
#[derive(Debug, Serialize)]
pub(in crate::live) struct LiveLineDeltaRef<'a> {
    pub(in crate::live) i: usize,
    pub(in crate::live) t: &'a str,
    pub(in crate::live) a: &'a str,
}

#[derive(Debug, Deserialize)]
#[cfg(test)]
pub(in crate::live) struct LiveLineDeltaOwned {
    pub(in crate::live) i: usize,
    pub(in crate::live) t: String,
    pub(in crate::live) a: String,
}

/// Only newly restored rows need their own strings. Unescaped fields borrow
/// the current input line; escaped JSON strings keep Serde's owned fallback.
#[derive(Debug, Deserialize)]
pub(in crate::live) struct LiveLineDeltaBorrowed<'a> {
    pub(in crate::live) i: usize,
    #[serde(borrow)]
    pub(in crate::live) t: Cow<'a, str>,
    #[serde(borrow)]
    pub(in crate::live) a: Cow<'a, str>,
}

/// Borrowing view of `LiveDayCache` used only for serialization, so we don't
/// have to deep-clone the transcript Vec every rewrite.
#[derive(Debug, Serialize)]
pub(in crate::live) struct LiveDayCacheRef<'a> {
    pub(in crate::live) date: String,
    pub(in crate::live) course_name: &'a str,
    pub(in crate::live) started_at: String,
    pub(in crate::live) transcript_lines: &'a [SharedTranscriptLine],
    pub(in crate::live) summaries: &'a [SharedSummaryChunk],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowed_delta_accepts_exactly_the_previous_schema_including_escapes_and_sequences() {
        let cases = [
            r#"{"i":1,"t":"全文 中文 👩🏽‍💻","a":"10:00:00"}"#,
            r#"{"a":"10:00:00","t":"全文","i":1,"extra":{"ignored":[true,null]}}"#,
            r#"{"i":0,"t":"","a":""}"#,
            r#"{"i":1,"t":"\n\r\t\u0000\"\\\ud83c\udf15","a":"\u0031\n"}"#,
            r#"[1,"全文","10:00:00"]"#,
            r#"{"i":1,"t":null,"a":"a"}"#,
            r#"{"i":1,"t":["text"],"a":"a"}"#,
            r#"{"i":1,"t":5,"a":"a"}"#,
            r#"{"i":1,"t":true,"a":"a"}"#,
            r#"{"i":1,"t":"text","a":null}"#,
            r#"{"i":1,"t":"text","a":{}}"#,
            r#"{"i":1,"t":"text"}"#,
            r#"{"i":1,"a":"a"}"#,
            r#"{"t":"text","a":"a"}"#,
            r#"{"i":-1,"t":"text","a":"a"}"#,
            r#"{"i":1.5,"t":"text","a":"a"}"#,
            r#"{"i":"1","t":"text","a":"a"}"#,
            r#"{"i":18446744073709551616,"t":"text","a":"a"}"#,
            r#"{"i":1,"i":2,"t":"text","a":"a"}"#,
            r#"{"i":1,"t":"text","t":"again","a":"a"}"#,
            r#"{"i":1,"t":"text","a":"a","a":"again"}"#,
            r#"{"i":1,"t":"\ud800","a":"a"}"#,
            r#"{"i":1,"t":"\uXXXX","a":"a"}"#,
            r#"{"i":1,"t":"text","a":"a"} trailing"#,
            r#"{"i":1,"t":"torn"#,
            r#"[1,"text"]"#,
            r#"[1,"text","a",0]"#,
            r#"null"#,
            "",
        ];
        for raw in cases {
            let old = serde_json::from_str::<LiveLineDeltaOwned>(raw);
            let new = serde_json::from_str::<LiveLineDeltaBorrowed<'_>>(raw);
            assert_eq!(old.is_ok(), new.is_ok(), "{raw}");
            match (old, new) {
                (Ok(old), Ok(new)) => {
                    assert_eq!(old.i, new.i);
                    assert_eq!(old.t, new.t);
                    assert_eq!(old.a, new.a);
                }
                (Err(old), Err(new)) => assert_eq!(old.classify(), new.classify()),
                _ => unreachable!(),
            }
            assert_eq!(
                serde_json::from_slice::<LiveLineDeltaOwned>(raw.as_bytes()).is_ok(),
                serde_json::from_slice::<LiveLineDeltaBorrowed<'_>>(raw.as_bytes()).is_ok(),
            );
        }
        let invalid = b"{\"i\":1,\"t\":\"\xff\",\"a\":\"a\"}";
        assert!(serde_json::from_slice::<LiveLineDeltaOwned>(invalid).is_err());
        assert!(serde_json::from_slice::<LiveLineDeltaBorrowed<'_>>(invalid).is_err());
    }

    #[test]
    fn plain_unicode_fields_borrow_input_and_escaped_fields_move_their_owned_fallback() {
        let raw = r#"{"i":1,"t":"全文 中文 👩🏽‍💻","a":"10:00:00"}"#;
        let delta: LiveLineDeltaBorrowed<'_> = serde_json::from_str(raw).unwrap();
        for field in [&delta.t, &delta.a] {
            assert!(matches!(field, Cow::Borrowed(_)));
            assert!((raw.as_ptr() as usize..raw.as_ptr() as usize + raw.len())
                .contains(&(field.as_ptr() as usize)));
        }
        let raw = r#"{"i":1,"t":"完整\n\"🌕\"","a":"10:00:00"}"#;
        let delta: LiveLineDeltaBorrowed<'_> = serde_json::from_str(raw).unwrap();
        assert!(matches!(delta.t, Cow::Owned(_)));
        assert!(matches!(delta.a, Cow::Borrowed(_)));
        let pointer = delta.t.as_ptr();
        let text = delta.t.into_owned();
        assert_eq!(text, "完整\n\"🌕\"");
        assert_eq!(text.as_ptr(), pointer);
        assert_eq!(delta.a.into_owned(), "10:00:00");
    }
}
