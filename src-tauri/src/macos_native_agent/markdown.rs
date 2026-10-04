//! Markdown rendering for the native agent result capsule.

use super::*;

// ─ Markdown → NSAttributedString ─────────────────────────────────────────────
enum MdBlock {
    Paragraph(Vec<MdInline>),
    Heading(u8, Vec<MdInline>),
    Bullet(Vec<MdInline>),
    Ordered(u32, Vec<MdInline>),
    HRule,
    Blank,
}

#[derive(Clone)]
enum MdInline {
    Text(String),
    Bold(String),
    Italic(String),
    BoldItalic(String),
    Code(String),
}

fn parse_markdown(text: &str) -> Vec<MdBlock> {
    let mut blocks = Vec::new();
    for raw in text.split('\n') {
        let trimmed = raw.trim_end_matches('\r');
        if trimmed.trim().is_empty() {
            blocks.push(MdBlock::Blank);
            continue;
        }
        if trimmed.trim() == "---" || trimmed.trim() == "***" || trimmed.trim() == "___" {
            blocks.push(MdBlock::HRule);
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("### ") {
            blocks.push(MdBlock::Heading(3, parse_inlines(rest)));
        } else if let Some(rest) = trimmed.strip_prefix("## ") {
            blocks.push(MdBlock::Heading(2, parse_inlines(rest)));
        } else if let Some(rest) = trimmed.strip_prefix("# ") {
            blocks.push(MdBlock::Heading(1, parse_inlines(rest)));
        } else if let Some(rest) = trimmed
            .trim_start()
            .strip_prefix("- ")
            .or_else(|| trimmed.trim_start().strip_prefix("* "))
            .or_else(|| trimmed.trim_start().strip_prefix("• "))
        {
            blocks.push(MdBlock::Bullet(parse_inlines(rest)));
        } else if let Some((num, rest)) = parse_ordered_prefix(trimmed.trim_start()) {
            blocks.push(MdBlock::Ordered(num, parse_inlines(rest)));
        } else {
            blocks.push(MdBlock::Paragraph(parse_inlines(trimmed)));
        }
    }
    blocks
}

fn parse_ordered_prefix(line: &str) -> Option<(u32, &str)> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 || i >= bytes.len() {
        return None;
    }
    if bytes[i] != b'.' && bytes[i] != b')' {
        return None;
    }
    if i + 1 >= bytes.len() || bytes[i + 1] != b' ' {
        return None;
    }
    let n: u32 = line[..i].parse().ok()?;
    Some((n, &line[i + 2..]))
}

fn parse_inlines(line: &str) -> Vec<MdInline> {
    let chars: Vec<char> = line.chars().collect();
    let mut out: Vec<MdInline> = Vec::new();
    let mut buf = String::new();
    let mut i = 0;
    let n = chars.len();

    let flush_buf = |buf: &mut String, out: &mut Vec<MdInline>| {
        if !buf.is_empty() {
            out.push(MdInline::Text(std::mem::take(buf)));
        }
    };

    while i < n {
        let c = chars[i];
        if c == '`' {
            let mut j = i + 1;
            while j < n && chars[j] != '`' {
                j += 1;
            }
            if j < n {
                flush_buf(&mut buf, &mut out);
                out.push(MdInline::Code(chars[i + 1..j].iter().collect()));
                i = j + 1;
                continue;
            }
        }
        if c == '*' && i + 2 < n && chars[i + 1] == '*' && chars[i + 2] == '*' {
            let start = i + 3;
            let mut j = start;
            while j + 2 < n && !(chars[j] == '*' && chars[j + 1] == '*' && chars[j + 2] == '*') {
                j += 1;
            }
            if j + 2 < n && chars[j] == '*' && chars[j + 1] == '*' && chars[j + 2] == '*' {
                flush_buf(&mut buf, &mut out);
                out.push(MdInline::BoldItalic(chars[start..j].iter().collect()));
                i = j + 3;
                continue;
            }
        }
        if c == '*' && i + 1 < n && chars[i + 1] == '*' {
            let start = i + 2;
            let mut j = start;
            while j + 1 < n && !(chars[j] == '*' && chars[j + 1] == '*') {
                j += 1;
            }
            if j + 1 < n && chars[j] == '*' && chars[j + 1] == '*' {
                flush_buf(&mut buf, &mut out);
                out.push(MdInline::Bold(chars[start..j].iter().collect()));
                i = j + 2;
                continue;
            }
        }
        if c == '*' && i + 1 < n {
            let start = i + 1;
            let mut j = start;
            while j < n && chars[j] != '*' {
                j += 1;
            }
            if j < n && chars[j] == '*' {
                flush_buf(&mut buf, &mut out);
                out.push(MdInline::Italic(chars[start..j].iter().collect()));
                i = j + 1;
                continue;
            }
        }
        buf.push(c);
        i += 1;
    }
    flush_buf(&mut buf, &mut out);
    out
}

pub(super) fn build_markdown_attributed(
    text: &str,
    theme: Theme,
) -> Retained<NSMutableAttributedString> {
    let blocks = parse_markdown(text);
    let out = NSMutableAttributedString::new();

    let body_color = theme.label_color();
    let muted_color = theme.muted_label();
    let accent_color = theme.accent();
    let code_fg = theme.code_fg();
    let code_bg = theme.code_bg();

    let mut first_block = true;
    let mut prev_blank = false;
    for block in &blocks {
        if let MdBlock::Blank = block {
            prev_blank = true;
            continue;
        }

        if !first_block {
            append_plain(&out, "\n", &body_color, RESULT_BODY_FONT, None, 0.0);
        }
        first_block = false;

        match block {
            MdBlock::Heading(level, inlines) => {
                let (font_size, spacing_before) = match level {
                    1 => (RESULT_H1_FONT, 10.0),
                    2 => (RESULT_H2_FONT, 8.0),
                    _ => (RESULT_H3_FONT, 6.0),
                };
                let color = if *level == 1 {
                    body_color.clone()
                } else {
                    accent_color.clone()
                };
                let style = make_paragraph_style(
                    RESULT_LINE_HEIGHT_MUL,
                    RESULT_PARAGRAPH_SPACING,
                    if first_block { 0.0 } else { spacing_before },
                    0.0,
                    0.0,
                    NSTextAlignment::Left,
                );
                append_inlines(
                    &out,
                    inlines,
                    &BlockCtx {
                        base_font_size: font_size,
                        bold: true,
                        color: &color,
                        accent: &accent_color,
                        code_fg: &code_fg,
                        code_bg: &code_bg,
                        paragraph: &style,
                    },
                );
            }
            MdBlock::Paragraph(inlines) => {
                let style = make_paragraph_style(
                    RESULT_LINE_HEIGHT_MUL,
                    if prev_blank {
                        RESULT_PARAGRAPH_SPACING
                    } else {
                        2.0
                    },
                    0.0,
                    0.0,
                    0.0,
                    NSTextAlignment::Left,
                );
                append_inlines(
                    &out,
                    inlines,
                    &BlockCtx {
                        base_font_size: RESULT_BODY_FONT,
                        bold: false,
                        color: &body_color,
                        accent: &accent_color,
                        code_fg: &code_fg,
                        code_bg: &code_bg,
                        paragraph: &style,
                    },
                );
            }
            MdBlock::Bullet(inlines) => {
                let style = make_paragraph_style(
                    RESULT_LINE_HEIGHT_MUL,
                    2.0,
                    0.0,
                    14.0,
                    14.0,
                    NSTextAlignment::Left,
                );
                append_plain(
                    &out,
                    "•  ",
                    &accent_color,
                    RESULT_BODY_FONT,
                    Some(&style),
                    0.0,
                );
                append_inlines(
                    &out,
                    inlines,
                    &BlockCtx {
                        base_font_size: RESULT_BODY_FONT,
                        bold: false,
                        color: &body_color,
                        accent: &accent_color,
                        code_fg: &code_fg,
                        code_bg: &code_bg,
                        paragraph: &style,
                    },
                );
            }
            MdBlock::Ordered(n, inlines) => {
                let style = make_paragraph_style(
                    RESULT_LINE_HEIGHT_MUL,
                    2.0,
                    0.0,
                    18.0,
                    18.0,
                    NSTextAlignment::Left,
                );
                let marker = format!("{n}.  ");
                append_plain(
                    &out,
                    &marker,
                    &accent_color,
                    RESULT_BODY_FONT,
                    Some(&style),
                    0.0,
                );
                append_inlines(
                    &out,
                    inlines,
                    &BlockCtx {
                        base_font_size: RESULT_BODY_FONT,
                        bold: false,
                        color: &body_color,
                        accent: &accent_color,
                        code_fg: &code_fg,
                        code_bg: &code_bg,
                        paragraph: &style,
                    },
                );
            }
            MdBlock::HRule => {
                let style = make_paragraph_style(0.9, 6.0, 6.0, 0.0, 0.0, NSTextAlignment::Left);
                let rule: String = "─".repeat(48);
                append_plain(
                    &out,
                    &rule,
                    &muted_color,
                    RESULT_BODY_FONT * 0.7,
                    Some(&style),
                    0.0,
                );
            }
            MdBlock::Blank => {}
        }
        prev_blank = false;
    }
    out
}

struct BlockCtx<'a> {
    base_font_size: f64,
    bold: bool,
    color: &'a NSColor,
    accent: &'a NSColor,
    code_fg: &'a NSColor,
    code_bg: &'a NSColor,
    paragraph: &'a NSMutableParagraphStyle,
}

fn append_inlines(out: &NSMutableAttributedString, inlines: &[MdInline], ctx: &BlockCtx) {
    for inline in inlines {
        match inline {
            MdInline::Text(s) => {
                let font = if ctx.bold {
                    NSFont::boldSystemFontOfSize(ctx.base_font_size)
                } else {
                    NSFont::systemFontOfSize(ctx.base_font_size)
                };
                append_attr(
                    out,
                    s,
                    Some(&font),
                    Some(ctx.color),
                    None,
                    Some(ctx.paragraph),
                );
            }
            MdInline::Bold(s) => {
                let font = NSFont::boldSystemFontOfSize(ctx.base_font_size);
                append_attr(
                    out,
                    s,
                    Some(&font),
                    Some(ctx.color),
                    None,
                    Some(ctx.paragraph),
                );
            }
            MdInline::Italic(s) => {
                let font = italic_system_font(ctx.base_font_size);
                append_attr(
                    out,
                    s,
                    Some(&font),
                    Some(ctx.accent),
                    None,
                    Some(ctx.paragraph),
                );
            }
            MdInline::BoldItalic(s) => {
                let font = bold_italic_system_font(ctx.base_font_size);
                append_attr(
                    out,
                    s,
                    Some(&font),
                    Some(ctx.accent),
                    None,
                    Some(ctx.paragraph),
                );
            }
            MdInline::Code(s) => {
                let font = NSFont::monospacedSystemFontOfSize_weight(RESULT_CODE_FONT, unsafe {
                    objc2_app_kit::NSFontWeightMedium
                });
                // Subtle padding around inline code using hair-space around the text.
                let padded = format!("\u{2009}{s}\u{2009}");
                append_attr(
                    out,
                    &padded,
                    Some(&font),
                    Some(ctx.code_fg),
                    Some(ctx.code_bg),
                    Some(ctx.paragraph),
                );
            }
        }
    }
}

fn append_plain(
    out: &NSMutableAttributedString,
    text: &str,
    color: &NSColor,
    font_size: f64,
    paragraph: Option<&NSMutableParagraphStyle>,
    _kern: f64,
) {
    let font = NSFont::systemFontOfSize(font_size);
    append_attr(out, text, Some(&font), Some(color), None, paragraph);
}

fn append_attr(
    out: &NSMutableAttributedString,
    text: &str,
    font: Option<&NSFont>,
    fg: Option<&NSColor>,
    bg: Option<&NSColor>,
    paragraph: Option<&NSMutableParagraphStyle>,
) {
    if text.is_empty() {
        return;
    }
    let ns = NSString::from_str(text);
    let start = out.length();
    let piece = NSAttributedString::initWithString(NSAttributedString::alloc(), &ns);
    out.appendAttributedString(&piece);
    let end = out.length();
    if end <= start {
        return;
    }
    let range = NSRange::new(start, end - start);

    unsafe {
        if let Some(font) = font {
            out.addAttribute_value_range(NSFontAttributeName, font.as_ref(), range);
        }
        if let Some(fg) = fg {
            out.addAttribute_value_range(NSForegroundColorAttributeName, fg.as_ref(), range);
        }
        if let Some(bg) = bg {
            out.addAttribute_value_range(
                objc2_app_kit::NSBackgroundColorAttributeName,
                bg.as_ref(),
                range,
            );
        }
        if let Some(paragraph) = paragraph {
            out.addAttribute_value_range(NSParagraphStyleAttributeName, paragraph.as_ref(), range);
        }
    }
}

fn make_paragraph_style(
    line_height_mul: f64,
    paragraph_spacing: f64,
    spacing_before: f64,
    head_indent: f64,
    first_head_indent: f64,
    alignment: NSTextAlignment,
) -> Retained<NSMutableParagraphStyle> {
    let style = NSMutableParagraphStyle::new();
    style.setLineHeightMultiple(line_height_mul);
    style.setParagraphSpacing(paragraph_spacing);
    style.setParagraphSpacingBefore(spacing_before);
    style.setHeadIndent(head_indent);
    style.setFirstLineHeadIndent(first_head_indent);
    style.setAlignment(alignment);
    style
}

fn italic_system_font(size: f64) -> Retained<NSFont> {
    unsafe {
        let cls = AnyClass::get(c"NSFontManager").unwrap();
        let shared: *mut AnyObject = msg_send![cls, sharedFontManager];
        let base = NSFont::systemFontOfSize(size);
        let italic_traits: i64 = 1; // NSItalicFontMask
        let result: *mut NSFont = msg_send![
            shared,
            convertFont: &*base,
            toHaveTrait: italic_traits
        ];
        if result.is_null() {
            base
        } else {
            Retained::retain(result).unwrap_or_else(|| NSFont::systemFontOfSize(size))
        }
    }
}

fn bold_italic_system_font(size: f64) -> Retained<NSFont> {
    unsafe {
        let cls = AnyClass::get(c"NSFontManager").unwrap();
        let shared: *mut AnyObject = msg_send![cls, sharedFontManager];
        let base = NSFont::boldSystemFontOfSize(size);
        let italic_traits: i64 = 1; // NSItalicFontMask
        let result: *mut NSFont = msg_send![
            shared,
            convertFont: &*base,
            toHaveTrait: italic_traits
        ];
        if result.is_null() {
            base
        } else {
            Retained::retain(result).unwrap_or_else(|| NSFont::boldSystemFontOfSize(size))
        }
    }
}

pub(super) fn srgb(r: u8, g: u8, b: u8, a: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        r as f64 / 255.0,
        g as f64 / 255.0,
        b as f64 / 255.0,
        a,
    )
}
