use super::*;
use std::io::{Cursor, Write};

#[test]
fn document_attachment_command_accepts_the_frontend_camel_case_payload() {
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![agent_read_document_attachment])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let window =
        tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::App("index.html".into()))
            .build()
            .unwrap();
    let result = tauri::test::get_ipc_response(&window, tauri::webview::InvokeRequest {
        cmd: "agent_read_document_attachment".into(), callback: tauri::ipc::CallbackFn(0), error: tauri::ipc::CallbackFn(1),
        url: "http://tauri.localhost".parse().unwrap(),
        body: tauri::ipc::InvokeBody::Json(serde_json::json!({"name":"資料.txt","dataBase64":STANDARD.encode("日本語の添付 🌕")})),
        headers: Default::default(), invoke_key: tauri::test::INVOKE_KEY.into(),
    }).unwrap().deserialize::<DocumentPart>().unwrap();
    assert_eq!(result.name, "資料.txt");
    assert_eq!(result.text, "日本語の添付 🌕");
}

fn read(name: &str, bytes: &[u8]) -> Result<DocumentPart, String> {
    read_document(name.into(), &STANDARD.encode(bytes))
}
fn office(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, text) in parts {
        writer
            .start_file(
                *name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
        writer.write_all(text.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn text_uploads_preserve_utf8_utf16_and_unicode_truncation() {
    let text = "日本語の資料 🌕\ncol1\tcol2\n";
    for ext in [
        "txt", "md", "markdown", "csv", "tsv", "json", "log", "yaml", "yml",
    ] {
        let part = read(&format!("資料.{ext}"), text.as_bytes()).unwrap();
        assert_eq!(part.text, text);
        assert!(!part.truncated);
    }
    for little in [true, false] {
        let mut bytes = if little {
            vec![0xff, 0xfe]
        } else {
            vec![0xfe, 0xff]
        };
        for unit in text.encode_utf16() {
            bytes.extend(if little {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            });
        }
        assert_eq!(read("資料.txt", &bytes).unwrap().text, text);
    }
    let big = "🌕".repeat(MAX_CHARS + 1);
    let part = read("long.md", big.as_bytes()).unwrap();
    assert!(part.truncated);
    assert_eq!(part.text.chars().count(), MAX_CHARS);
}

#[test]
fn docx_and_pptx_preserve_text_entities_and_numeric_slide_order() {
    let docx = office(&[("word/document.xml", "<w:document><w:p><w:r><w:t>日本語 &amp; &#x1F315;</w:t><w:tab/><w:t>資料</w:t><w:br/><w:t>次の行</w:t></w:r></w:p></w:document>")]);
    assert_eq!(
        read("notes.DOCX", &docx).unwrap().text,
        "日本語 & 🌕\t資料\n次の行\n"
    );
    let pptx = office(&[
        (
            "ppt/slides/slide10.xml",
            "<p:sld><a:p><a:t>ten</a:t></a:p></p:sld>",
        ),
        (
            "ppt/slides/slide2.xml",
            "<p:sld><a:p><a:t>two</a:t></a:p></p:sld>",
        ),
    ]);
    let part = read("slides.pptx", &pptx).unwrap();
    assert!(part.text.find("two").unwrap() < part.text.find("ten").unwrap());
}

#[test]
fn xlsx_resolves_shared_and_inline_strings_and_preserves_cell_addresses() {
    let bytes = office(&[
        ("xl/sharedStrings.xml", "<sst><si><r><t>講</t></r><r><t>義</t></r></si><si><t>Course &amp; Lab</t></si></sst>"),
        ("xl/worksheets/sheet1.xml", "<worksheet><row><c r='A1' t='s'><v>0</v></c><c r='B1' t='s'><v>1</v></c><c r='C1' t='inlineStr'><is><t>日本語</t></is></c><c r='D1'><v>42.5</v></c><c r='E1'><f>SUM(D1)</f></c></row></worksheet>"),
    ]);
    let text = read("table.xlsx", &bytes).unwrap().text;
    for expected in [
        "A1: 講義",
        "B1: Course & Lab",
        "C1: 日本語",
        "D1: 42.5",
        "E1: =SUM(D1)",
    ] {
        assert!(text.contains(expected), "{text}");
    }
    let broken = office(&[(
        "xl/worksheets/sheet1.xml",
        "<worksheet><c r='A1' t='s'><v>0</v></c></worksheet>",
    )]);
    assert!(read("broken.xlsx", &broken).is_err());
}

#[test]
fn uploaded_pdf_reaches_text_extraction() {
    use lopdf::{
        content::{Content, Operation},
        dictionary, Object, Stream,
    };
    let mut document = lopdf::Document::with_version("1.5");
    let pages_id = document.new_object_id();
    let font = document
        .add_object(dictionary! {"Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica"});
    let operations = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new(
                "Tj",
                vec![Object::string_literal("Uploaded lecture material")],
            ),
            Operation::new("ET", vec![]),
        ],
    };
    let stream = document.add_object(Stream::new(dictionary! {}, operations.encode().unwrap()));
    let page = document.add_object(dictionary! {"Type"=>"Page", "Parent"=>pages_id, "Contents"=>stream, "Resources"=>dictionary! {"Font"=>dictionary! {"F1"=>font}}, "MediaBox"=>vec![0.into(),0.into(),595.into(),842.into()]});
    document.objects.insert(
        pages_id,
        dictionary! {"Type"=>"Pages", "Kids"=>vec![page.into()], "Count"=>1}.into(),
    );
    let catalog = document.add_object(dictionary! {"Type"=>"Catalog", "Pages"=>pages_id});
    document.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    assert!(read("lecture.pdf", &bytes)
        .unwrap()
        .text
        .contains("Uploaded lecture material"));
}

#[test]
fn invalid_and_oversized_uploads_fail_without_guessing_document_content() {
    for (name, bytes) in [
        ("empty.txt", &b""[..]),
        ("binary.txt", &b"A\0B"[..]),
        ("bad.txt", &b"\xff\x80"[..]),
        ("broken.pdf", &b"PDF?"[..]),
        ("legacy.doc", &b"old office"[..]),
        ("legacy.xls", &b"old office"[..]),
    ] {
        assert!(read(name, bytes).is_err(), "{name}");
    }
    assert!(read_document(
        "large.txt".into(),
        &"A".repeat(MAX_BYTES.div_ceil(3) * 4 + 1)
    )
    .is_err());
    let too_large = "A".repeat(2 * 1024 * 1024 + 1);
    assert!(read("large.docx", &office(&[("word/document.xml", &too_large)])).is_err());
    assert!(read_document("invalid.txt".into(), "%%%").is_err());
}
