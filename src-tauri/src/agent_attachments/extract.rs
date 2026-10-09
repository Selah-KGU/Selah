use quick_xml::{
    events::{BytesStart, Event},
    Reader,
};
use std::io::{Cursor, Read};

type Archive<'a> = zip::ZipArchive<Cursor<&'a [u8]>>;
const XML_LIMIT: u64 = 2 * 1024 * 1024;

fn xml(archive: &mut Archive<'_>, name: &str, remaining: &mut u64) -> Result<String, String> {
    let mut entry = archive
        .by_name(name)
        .map_err(|_| format!("本文が見つかりません: {name}"))?;
    if entry.size() > XML_LIMIT || entry.size() > *remaining {
        return Err("文書の展開後の本文が大きすぎます".into());
    }
    let mut bytes = Vec::new();
    entry
        .by_ref()
        .take(XML_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "文書の本文を読み取れません")?;
    if bytes.len() as u64 > XML_LIMIT || bytes.len() as u64 > *remaining {
        return Err("文書の展開後の本文が大きすぎます".into());
    }
    *remaining -= bytes.len() as u64;
    String::from_utf8(bytes).map_err(|_| "文書のXML文字コードが正しくありません".into())
}

fn attribute(
    element: &BytesStart<'_>,
    key: &[u8],
    reader: &Reader<&[u8]>,
) -> Result<String, String> {
    for attr in element.attributes() {
        let attr = attr.map_err(|_| "文書のXML属性が正しくありません")?;
        if attr.key.as_ref() == key {
            return attr
                .decode_and_unescape_value(reader.decoder())
                .map(|s| s.into_owned())
                .map_err(|_| "文書のXML属性を読めません".into());
        }
    }
    Ok(String::new())
}

fn text(event: &quick_xml::events::BytesText<'_>) -> Result<String, String> {
    let decoded = event
        .decode()
        .map_err(|_| "文書の文字コードが正しくありません")?;
    quick_xml::escape::unescape(&decoded)
        .map(|s| s.into_owned())
        .map_err(|_| "文書のXML文字参照が正しくありません".into())
}

fn reference(event: &quick_xml::events::BytesRef<'_>) -> Result<String, String> {
    let name = event
        .decode()
        .map_err(|_| "文書のXML文字参照が正しくありません")?;
    quick_xml::escape::unescape(&format!("&{name};"))
        .map(|s| s.into_owned())
        .map_err(|_| "文書のXML文字参照が正しくありません".into())
}

fn office_text(source: &str) -> Result<String, String> {
    let mut reader = Reader::from_str(source);
    let mut in_text = false;
    let mut out = String::new();
    loop {
        match reader
            .read_event()
            .map_err(|_| "文書のXMLが正しくありません")?
        {
            Event::Start(e) if e.local_name().as_ref() == b"t" => in_text = true,
            Event::Text(e) if in_text => out.push_str(&text(&e)?),
            Event::GeneralRef(e) if in_text => out.push_str(&reference(&e)?),
            Event::CData(e) if in_text => out.push_str(
                &e.decode()
                    .map_err(|_| "文書の文字コードが正しくありません")?,
            ),
            Event::End(e) => match e.local_name().as_ref() {
                b"t" => in_text = false,
                b"p" => out.push('\n'),
                _ => {}
            },
            Event::Empty(e) => match e.local_name().as_ref() {
                b"br" => out.push('\n'),
                b"tab" => out.push('\t'),
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

fn shared_strings(source: &str) -> Result<Vec<String>, String> {
    let mut reader = Reader::from_str(source);
    let mut strings = Vec::new();
    let mut current = String::new();
    let mut in_text = false;
    loop {
        match reader
            .read_event()
            .map_err(|_| "Excelの共有文字列が正しくありません")?
        {
            Event::Start(e) => match e.local_name().as_ref() {
                b"si" => current.clear(),
                b"t" => in_text = true,
                _ => {}
            },
            Event::Text(e) if in_text => current.push_str(&text(&e)?),
            Event::GeneralRef(e) if in_text => current.push_str(&reference(&e)?),
            Event::CData(e) if in_text => current.push_str(
                &e.decode()
                    .map_err(|_| "文書の文字コードが正しくありません")?,
            ),
            Event::End(e) => match e.local_name().as_ref() {
                b"t" => in_text = false,
                b"si" => strings.push(current.clone()),
                _ => {}
            },
            Event::Empty(e) if e.local_name().as_ref() == b"si" => strings.push(String::new()),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(strings)
}

fn worksheet(source: &str, strings: &[String]) -> Result<String, String> {
    let mut reader = Reader::from_str(source);
    let (mut address, mut kind, mut value, mut formula) =
        (String::new(), String::new(), String::new(), String::new());
    let mut capturing = Vec::new();
    let mut out = String::new();
    loop {
        match reader
            .read_event()
            .map_err(|_| "Excelのセル情報が正しくありません")?
        {
            Event::Start(e) => match e.local_name().as_ref() {
                b"c" => {
                    address = attribute(&e, b"r", &reader)?;
                    kind = attribute(&e, b"t", &reader)?;
                    value.clear();
                    formula.clear();
                }
                b"v" | b"t" | b"f" => capturing = e.local_name().as_ref().to_vec(),
                _ => {}
            },
            Event::Text(e) if !capturing.is_empty() => {
                if capturing == b"f" {
                    formula.push_str(&text(&e)?);
                } else {
                    value.push_str(&text(&e)?);
                }
            }
            Event::GeneralRef(e) if !capturing.is_empty() => {
                if capturing == b"f" {
                    formula.push_str(&reference(&e)?);
                } else {
                    value.push_str(&reference(&e)?);
                }
            }
            Event::CData(e) if !capturing.is_empty() => {
                let data = e
                    .decode()
                    .map_err(|_| "文書の文字コードが正しくありません")?;
                if capturing == b"f" {
                    formula.push_str(&data);
                } else {
                    value.push_str(&data);
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"v" | b"t" | b"f" => capturing.clear(),
                b"c" => {
                    let displayed = if kind == "s" {
                        strings
                            .get(
                                value
                                    .parse::<usize>()
                                    .map_err(|_| "Excelの文字列参照が正しくありません")?,
                            )
                            .ok_or("Excelの文字列参照が見つかりません")?
                            .clone()
                    } else if value.is_empty() && !formula.is_empty() {
                        format!("={formula}")
                    } else {
                        value.clone()
                    };
                    if !displayed.is_empty() {
                        out.push_str(&format!("{address}: {displayed}\n"));
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

fn office(extension: &str, bytes: &[u8]) -> Result<String, String> {
    let mut archive = Archive::new(Cursor::new(bytes)).map_err(|_| "Officeファイルを開けません")?;
    if archive.len() > 4000 {
        return Err("文書内のファイル数が多すぎます".into());
    }
    let mut remaining = 8 * 1024 * 1024;
    if extension == "docx" {
        return office_text(&xml(&mut archive, "word/document.xml", &mut remaining)?);
    }
    let strings = if extension == "xlsx"
        && archive
            .file_names()
            .any(|name| name == "xl/sharedStrings.xml")
    {
        shared_strings(&xml(&mut archive, "xl/sharedStrings.xml", &mut remaining)?)?
    } else {
        Vec::new()
    };
    let prefix = if extension == "pptx" {
        "ppt/slides/slide"
    } else {
        "xl/worksheets/sheet"
    };
    let mut names: Vec<_> = archive
        .file_names()
        .filter_map(|name| {
            let number = name
                .strip_prefix(prefix)?
                .strip_suffix(".xml")?
                .parse::<usize>()
                .ok()?;
            Some((number, name.to_owned()))
        })
        .collect();
    names.sort();
    let mut out = String::new();
    for (number, name) in names {
        let source = xml(&mut archive, &name, &mut remaining)?;
        let body = if extension == "xlsx" {
            worksheet(&source, &strings)?
        } else {
            office_text(&source)?
        };
        out.push_str(&format!(
            "\n[{} {number}]\n{body}",
            if extension == "xlsx" {
                "Sheet"
            } else {
                "Slide"
            }
        ));
    }
    Ok(out)
}

fn plain_text(bytes: &[u8]) -> Result<String, String> {
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        if bytes.len() % 2 != 0 {
            return Err("UTF-16ファイルが正しくありません".into());
        }
        let little = bytes[0] == 0xff;
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|p| {
                if little {
                    u16::from_le_bytes([p[0], p[1]])
                } else {
                    u16::from_be_bytes([p[0], p[1]])
                }
            })
            .collect();
        String::from_utf16(&units).map_err(|_| "UTF-16ファイルが正しくありません".into())
    } else {
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
        let text = std::str::from_utf8(bytes)
            .map_err(|_| "テキストはUTF-8またはUTF-16で保存してください")?;
        if text.contains('\0') {
            return Err("バイナリファイルはテキストとして添付できません".into());
        }
        Ok(text.to_owned())
    }
}

pub(super) fn extract(
    extension: &str,
    bytes: &[u8],
) -> Result<(&'static str, String, bool), String> {
    match extension {
        "pdf" => {
            let document = lopdf::Document::load_mem(bytes)
                .map_err(|_| "PDFを開けません。破損やパスワード保護を確認してください")?;
            let pages = document.get_pages();
            let mut out = String::new();
            let mut truncated = pages.len() > 100;
            for page in pages.keys().take(100) {
                match document.extract_text(&[*page]) {
                    Ok(text) => {
                        out.push_str(&text);
                        out.push('\n');
                    }
                    Err(_) => truncated = true,
                }
                if out.chars().count() > super::MAX_CHARS {
                    truncated = true;
                    break;
                }
            }
            if out.trim().is_empty() {
                return Err("PDFに読み取れる文字がありません。スキャン画像は画像ファイルとして添付してください".into());
            }
            Ok(("application/pdf", out, truncated))
        }
        "docx" => Ok((
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            office(extension, bytes)?,
            false,
        )),
        "pptx" => Ok((
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            office(extension, bytes)?,
            false,
        )),
        "xlsx" => Ok((
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            office(extension, bytes)?,
            false,
        )),
        "txt" | "md" | "markdown" | "csv" | "tsv" | "json" | "log" | "yaml" | "yml" => {
            Ok(("text/plain", plain_text(bytes)?, false))
        }
        _ => Err(
            "対応形式は画像、PDF、DOCX、PPTX、XLSX、TXT、Markdown、CSV、TSV、JSON、LOG、YAMLです"
                .into(),
        ),
    }
}
