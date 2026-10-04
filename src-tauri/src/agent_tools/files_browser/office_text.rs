//! PDF and Office text/image extraction for downloaded course files.

use super::*;
use std::sync::LazyLock;

static DOCX_PARA_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"</w:p>").unwrap());
static DOCX_BREAK_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"<w:br\s*/?>").unwrap());
static DOCX_TAB_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"<w:tab\s*/?>").unwrap());
static DOCX_TAG_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"<[^>]+>").unwrap());
static OFFICE_BLOCK_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"</(?:a:p|w:p|row|si|sst|worksheet|slide)>").unwrap());

/// `max_pages == usize::MAX` reads the whole document (paper-check needs the
/// full text; silently truncating a thesis would skip everything past the cap).
fn extract_pdf_text_pages(path: &Path, max_pages: usize) -> Result<String, String> {
    let doc = lopdf::Document::load(path).map_err(|e| format!("PDF読み込み失敗: {}", e))?;
    let pages = doc.get_pages();
    if pages.is_empty() {
        return Err("PDFにページがありません".into());
    }
    let mut out = String::new();
    for page_num in pages.keys().take(max_pages) {
        match doc.extract_text(&[*page_num]) {
            Ok(text) => {
                if !text.trim().is_empty() {
                    if !out.is_empty() {
                        out.push_str("\n\n");
                    }
                    out.push_str(&text);
                }
            }
            Err(e) => {
                log::warn!("pdf text extraction failed for page {}: {}", page_num, e);
            }
        }
    }
    let text = normalize_extracted_text(&out);
    if text.is_empty() {
        Err("PDFからテキストを抽出できませんでした".into())
    } else {
        Ok(text)
    }
}

/// Maximum number of embedded page images forwarded to the vision model.
const MAX_PDF_IMAGES: usize = 8;
/// Skip any single embedded image larger than this (raw bytes) to bound cost.
const MAX_PDF_IMAGE_BYTES: usize = 6 * 1024 * 1024;

/// Pull embedded JPEG (DCTDecode) image XObjects out of a PDF. Scanned course
/// PDFs have no text layer, but each page is typically a single JPEG image, so
/// these can be sent to a vision model when `extract_pdf_text` yields nothing.
/// Other codecs (Flate/CCITT/JBIG2/JPX) need a decoder we do not bundle and are
/// skipped.
pub fn extract_pdf_images(path: &Path) -> Result<Vec<crate::ai::ImagePart>, String> {
    use base64::Engine;
    let doc = lopdf::Document::load(path).map_err(|e| format!("PDF読み込み失敗: {}", e))?;
    let mut images = Vec::new();
    for object in doc.objects.values() {
        if images.len() >= MAX_PDF_IMAGES {
            break;
        }
        let lopdf::Object::Stream(stream) = object else {
            continue;
        };
        let is_image = stream
            .dict
            .get(b"Subtype")
            .ok()
            .and_then(|value| value.as_name().ok())
            .is_some_and(|name| name == &b"Image"[..]);
        if !is_image || !filter_is_dct(&stream.dict) {
            continue;
        }
        if stream.content.is_empty() || stream.content.len() > MAX_PDF_IMAGE_BYTES {
            continue;
        }
        images.push(crate::ai::ImagePart {
            mime: "image/jpeg".into(),
            data_base64: base64::engine::general_purpose::STANDARD.encode(&stream.content),
        });
    }
    if images.is_empty() {
        Err("PDFから画像も取得できませんでした".into())
    } else {
        Ok(images)
    }
}

/// Locate libpdfium and bind to it. Dev uses the crate's bundled copy via the
/// compile-time manifest path; a packaged app finds it next to the executable
/// or in the macOS .app Frameworks/Resources folder. `SELAH_PDFIUM_DIR` overrides all.
fn bind_pdfium() -> Result<pdfium_render::prelude::Pdfium, String> {
    use pdfium_render::prelude::Pdfium;
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(dir) = std::env::var("SELAH_PDFIUM_DIR") {
        dirs.push(std::path::PathBuf::from(dir));
    }
    dirs.push(std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/lib"
    )));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.join("../Frameworks"));
            dirs.push(parent.join("../Resources"));
            dirs.push(parent.to_path_buf());
        }
    }
    for dir in &dirs {
        let name = Pdfium::pdfium_platform_library_name_at_path(dir);
        if let Ok(bindings) = Pdfium::bind_to_library(&name) {
            return Ok(Pdfium::new(bindings));
        }
    }
    Pdfium::bind_to_system_library()
        .map(Pdfium::new)
        .map_err(|error| format!("pdfium ライブラリを読み込めません: {}", error))
}

/// Rasterize the first pages of a PDF to JPEG images. Used as the last resort
/// for a vector PDF (e.g. slide decks) that has neither a text layer nor
/// extractable image XObjects, so a vision model can still read it.
pub fn render_pdf_to_images(path: &Path) -> Result<Vec<crate::ai::ImagePart>, String> {
    use base64::Engine;
    use pdfium_render::prelude::PdfRenderConfig;
    use std::io::Cursor;

    let pdfium = bind_pdfium()?;
    let path_str = path.to_str().ok_or("PDF パスが不正です")?;
    let document = pdfium
        .load_pdf_from_file(path_str, None)
        .map_err(|error| format!("PDF を開けません: {}", error))?;
    let config = PdfRenderConfig::new()
        .set_target_width(1500)
        .set_maximum_height(2100);

    let mut images = Vec::new();
    for page in document.pages().iter().take(MAX_PDF_IMAGES) {
        let bitmap = page
            .render_with_config(&config)
            .map_err(|error| format!("ページ描画失敗: {}", error))?;
        let mut buffer = Vec::new();
        image::DynamicImage::ImageRgb8(bitmap.as_image().into_rgb8())
            .write_to(&mut Cursor::new(&mut buffer), image::ImageFormat::Jpeg)
            .map_err(|error| format!("JPEG 変換失敗: {}", error))?;
        if buffer.is_empty() || buffer.len() > MAX_PDF_IMAGE_BYTES {
            continue;
        }
        images.push(crate::ai::ImagePart {
            mime: "image/jpeg".into(),
            data_base64: base64::engine::general_purpose::STANDARD.encode(&buffer),
        });
    }
    if images.is_empty() {
        Err("PDF をレンダリングできませんでした".into())
    } else {
        Ok(images)
    }
}

/// True when the stream's only filter is DCTDecode, i.e. its raw bytes are a
/// standalone JPEG that can be forwarded without decoding.
fn filter_is_dct(dict: &lopdf::Dictionary) -> bool {
    match dict.get(b"Filter") {
        Ok(lopdf::Object::Name(name)) => name == &b"DCTDecode"[..],
        Ok(lopdf::Object::Array(filters)) => {
            filters.len() == 1
                && matches!(
                    filters.first(),
                    Some(lopdf::Object::Name(name)) if name == &b"DCTDecode"[..]
                )
        }
        _ => false,
    }
}

fn extract_docx_text(path: &Path) -> Result<String, String> {
    let file = File::open(path).map_err(|e| format!("DOCX読み込み失敗: {}", e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("DOCX展開失敗: {}", e))?;
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .map_err(|e| format!("DOCX本文が見つかりません: {}", e))?
        .read_to_string(&mut xml)
        .map_err(|e| format!("DOCX本文読み込み失敗: {}", e))?;

    let xml = DOCX_PARA_RE.replace_all(&xml, "\n");
    let xml = DOCX_BREAK_RE.replace_all(&xml, "\n");
    let xml = DOCX_TAB_RE.replace_all(&xml, "\t");
    let text = DOCX_TAG_RE.replace_all(&xml, " ");
    let text = decode_xml_entities(&text);
    let text = normalize_extracted_text(&text);
    if text.is_empty() {
        Err("DOCXからテキストを抽出できませんでした".into())
    } else {
        Ok(text)
    }
}

fn extract_zipped_office_text(
    path: &Path,
    prefixes: &[&str],
    document_label: &str,
) -> Result<String, String> {
    let file = File::open(path).map_err(|e| format!("{}読み込み失敗: {}", document_label, e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("{}展開失敗: {}", document_label, e))?;
    let mut names = (0..archive.len())
        .filter_map(|index| {
            archive
                .by_index(index)
                .ok()
                .map(|entry| entry.name().to_string())
        })
        .filter(|name| {
            prefixes.iter().any(|prefix| name.starts_with(prefix)) && name.ends_with(".xml")
        })
        .collect::<Vec<_>>();
    names.sort();

    let mut out = String::new();
    for name in names {
        let mut xml = String::new();
        let Ok(mut entry) = archive.by_name(&name) else {
            continue;
        };
        if entry.read_to_string(&mut xml).is_err() {
            continue;
        }
        let xml = OFFICE_BLOCK_RE.replace_all(&xml, "\n");
        let text = DOCX_TAG_RE.replace_all(&xml, " ");
        let text = decode_xml_entities(&text);
        let text = normalize_extracted_text(&text);
        if !text.is_empty() {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(&text);
        }
    }
    if out.is_empty() {
        Err(format!(
            "{}からテキストを抽出できませんでした",
            document_label
        ))
    } else {
        Ok(out)
    }
}

pub fn read_supported_download_file(path: &Path) -> Result<String, String> {
    read_supported_download_file_impl(path, 20)
}

/// Full-document variant (no PDF page cap) for surfaces that must see the
/// entire file, e.g. the paper checker.
pub fn read_supported_download_file_full(path: &Path) -> Result<String, String> {
    read_supported_download_file_impl(path, usize::MAX)
}

fn read_supported_download_file_impl(path: &Path, pdf_page_cap: usize) -> Result<String, String> {
    let ext = file_extension_lower(path);
    match ext.as_str() {
        "pdf" => extract_pdf_text_pages(path, pdf_page_cap),
        "docx" => extract_docx_text(path),
        "pptx" => extract_zipped_office_text(path, &["ppt/slides/"], "PPTX"),
        "xlsx" => {
            extract_zipped_office_text(path, &["xl/sharedStrings.xml", "xl/worksheets/"], "XLSX")
        }
        "txt" | "md" | "json" | "csv" | "html" | "htm" => {
            read_utf8ish_file(path, 2_000_000).map(|s| normalize_extracted_text(&s))
        }
        "doc" => Err("旧式 .doc は未対応です。.docx か PDF に変換してから試してください".into()),
        _ => Err(format!("未対応の拡張子です: .{}", ext)),
    }
}
