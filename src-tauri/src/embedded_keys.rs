/// Minimal XOR-based obfuscation so OAuth client credentials don't appear
/// as plain-text in the compiled binary.  This is **not** encryption — it only
/// raises the bar above `strings(1)`.  Desktop OAuth client-ids/secrets are
/// considered public per Google / Microsoft documentation, so this is a
/// cosmetic measure, not a security boundary.

const KEY: &[u8] = b"selah-kwic-2026";

/// XOR `data` with a repeating key.
fn xor(data: &[u8]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ KEY[i % KEY.len()])
        .collect()
}

/// Decode a compile-time XOR-encoded byte slice back to a UTF-8 String.
pub fn decode(encoded: &[u8]) -> String {
    String::from_utf8(xor(encoded)).expect("invalid embedded credential")
}

/// Helper executed at build time / dev time to produce the encoded byte arrays.
/// Run: `cargo test -- --nocapture encode_credentials`
#[cfg(test)]
mod tests {
    use super::*;

    fn encode(plain: &str) -> String {
        let bytes = xor(plain.as_bytes());
        format!(
            "&[{}]",
            bytes
                .iter()
                .map(|b| format!("0x{:02X}", b))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }

    #[test]
    fn encode_credentials() {
        // Google Calendar
        let gcal_id = "73896007148-v58haqu83810imt0miarem0299g43uod.apps.googleusercontent.com";
        let gcal_secret = "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";
        // Microsoft Mail
        let ms_id = "9e5f94bc-e8a4-4e73-b8be-63364c29d753";

        println!("--- Google Calendar Client ID ---");
        println!("{}", encode(gcal_id));
        println!("--- Google Calendar Client Secret ---");
        println!("{}", encode(gcal_secret));
        println!("--- Microsoft Mail Client ID ---");
        println!("{}", encode(ms_id));
    }

    /// Pin the embedded byte arrays to the plaintext credentials so a typo or
    /// truncation in the array (we shipped a `client_id` missing its leading
    /// digit once) fails the build instead of OAuth at runtime.
    #[test]
    fn embedded_defaults_match_plaintext() {
        assert_eq!(
            crate::google_calendar::default_client_id_for_test(),
            "73896007148-v58haqu83810imt0miarem0299g43uod.apps.googleusercontent.com"
        );
        assert_eq!(
            crate::google_calendar::default_client_secret_for_test(),
            "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"
        );
    }
}
