use super::*;
use std::path::PathBuf;

fn old_text(raw: &str) -> Option<String> {
    let joined = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let text: String = joined.chars().take(700).collect();
    (!text.trim().is_empty()).then_some(text)
}

#[test]
fn text_prefix_matches_original_unicode_whitespace_and_all_700_character_boundaries() {
    let cases = [
        "",
        "\r\n \t\u{feff}\r\n",
        " \t\n ",
        "\r\n 日本語 👩🏽‍💻\t\r\n\r\n 🧑🏾‍💻引用 \n",
        " a\nb\r\n",
        "  \u{0085}\u{00a0}\u{2003} \n",
    ];
    for raw in cases {
        assert_eq!(preview_text(raw), old_text(raw));
    }
    for count in 0..=710 {
        for chunk in ["a", "授", "👩🏽‍💻", "x\n", "\r\n\t", "日本語 \u{0085} \r\n"] {
            let raw = format!("{}\n   最後の行  ", chunk.repeat(count));
            assert_eq!(
                preview_text(&raw),
                old_text(&raw),
                "count={count}, chunk={chunk:?}"
            );
        }
    }
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("selah-preview-fixture-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.0.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn image_preview_preserves_full_bytes_and_mime_without_resizing() {
    let fixture = Fixture::new();
    let bytes = b"fixture\0all\xffbytes";
    for (ext, mime) in [
        ("png", "image/png"),
        ("JPG", "image/jpeg"),
        ("jpeg", "image/jpeg"),
        ("gif", "image/gif"),
        ("webp", "image/webp"),
        ("svg", "image/svg+xml"),
    ] {
        let path = fixture.file(&format!("image.{ext}"), bytes);
        let preview = preview_for_file(&path).unwrap().unwrap();
        assert_eq!(preview.kind, "image");
        assert_eq!(preview.mime, mime);
        assert!(preview.text.is_none());
        let data = preview.data_url.unwrap();
        let (prefix, encoded) = data.split_once(',').unwrap();
        assert_eq!(prefix, format!("data:{mime};base64"));
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .unwrap(),
            bytes
        );
    }
}

#[test]
fn text_previews_keep_lossy_utf8_and_the_existing_size_caps_and_fallbacks() {
    let fixture = Fixture::new();
    for ext in ["md", "markdown", "TXT", "csv", "json", "log"] {
        let bytes = b"\r\n  \xfffixture\t\n\n next \r\n";
        let path = fixture.file(&format!("text.{ext}"), bytes);
        let preview = preview_for_file(&path).unwrap().unwrap();
        assert_eq!(preview.text, old_text(&String::from_utf8_lossy(bytes)));
        assert_eq!(preview.mime, "text/plain");
        assert!(preview.data_url.is_none());
    }
    assert!(preview_for_file(&fixture.0).unwrap().is_none());
    assert!(preview_for_file(&fixture.file("blank.txt", b" \t\n"))
        .unwrap()
        .is_none());
    assert!(preview_for_file(&fixture.file("unsupported.pdf", b"pdf"))
        .unwrap()
        .is_none());
    assert!(preview_for_file(&fixture.0.join("missing.txt")).is_err());
    for (name, cap) in [("image.png", 10 * 1024 * 1024), ("text.txt", 512 * 1024)] {
        let path = fixture.file(name, b"x");
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(cap + 1)
            .unwrap();
        assert!(preview_for_file(&path).unwrap().is_none());
    }
}

#[test]
fn full_text_cap_retains_the_same_preview_and_only_prefix_capacity() {
    let raw = "授業 👩🏽‍💻\n".repeat(512 * 1024 / "授業 👩🏽‍💻\n".len());
    let new = preview_text(&raw).unwrap();
    assert_eq!(Some(new.clone()), old_text(&raw));
    assert_eq!(new.chars().count(), 700);
    assert!(new.capacity() <= 700 * 4);
}
