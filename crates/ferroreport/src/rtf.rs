//! RTF text (`markup="rtf"`) read into styled runs, as JasperReports reads it with the Java
//! runtime's RTF reader: fonts, sizes, bold, italic, underline, strike-through and colors;
//! paragraphs and line breaks. Pictures, tables and page layout are left out.

use crate::model::Color;
use crate::rich::{Run, RunStyle, StyledText};

#[derive(Clone)]
struct State {
    style: RunStyle,
    /// Characters to skip after a `\uN` (`\ucN`).
    uc: usize,
    /// Inside a destination whose text is not shown (`\fonttbl`, `\*`, `\pict`...).
    hidden: bool,
}

/// Reads `src` as RTF; text that is not RTF comes back as it is.
pub fn parse(src: &str, base: &RunStyle) -> StyledText {
    // Like JasperReports (Swing's RTF reader), text that is not RTF reads as nothing.
    if !src.trim_start().starts_with("{\\rtf") {
        return StyledText::plain("", base);
    }
    let chars: Vec<char> = src.chars().collect();
    let mut fonts: Vec<(i32, String)> = Vec::new();
    let mut colors: Vec<Option<Color>> = Vec::new();
    let mut stack: Vec<State> = Vec::new();
    let mut st = State { style: base.clone(), uc: 1, hidden: false };
    let mut text = String::new();
    let mut runs: Vec<Run> = Vec::new();
    let push = |text: &mut String, runs: &mut Vec<Run>, s: &str, style: &RunStyle| {
        if s.is_empty() {
            return;
        }
        let start = text.len();
        text.push_str(s);
        match runs.last_mut() {
            Some(r) if r.style == *style && r.end == start => r.end = text.len(),
            _ => runs.push(Run { start, end: text.len(), style: style.clone() }),
        }
    };
    // The destination being read: font table (font number being named) or color table.
    let mut in_fonttbl = 0usize;
    let mut font_num: Option<i32> = None;
    let mut font_name = String::new();
    let mut in_colortbl = 0usize;
    let mut rgb = [0u8; 3];
    let mut skip = 0usize;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '{' => {
                stack.push(st.clone());
                i += 1;
            }
            '}' => {
                if in_fonttbl > 0 && stack.len() == in_fonttbl + 1 {
                    if let Some(n) = font_num.take() {
                        fonts.push((n, font_name.trim().trim_end_matches(';').trim().to_string()));
                    }
                    font_name.clear();
                }
                if stack.len() == in_fonttbl {
                    in_fonttbl = 0;
                }
                if stack.len() == in_colortbl {
                    in_colortbl = 0;
                }
                st = stack.pop().unwrap_or_else(|| st.clone());
                i += 1;
            }
            '\\' => {
                i += 1;
                let Some(&n) = chars.get(i) else { break };
                if !n.is_ascii_alphabetic() {
                    // Control symbols.
                    i += 1;
                    let out = match n {
                        '\\' | '{' | '}' => Some(n.to_string()),
                        '~' => Some('\u{A0}'.to_string()),
                        '_' => Some('-'.to_string()),
                        '\'' => {
                            let hex: String = chars[i..chars.len().min(i + 2)].iter().collect();
                            i += 2;
                            u8::from_str_radix(&hex, 16).ok().map(|b| cp1252(b).to_string())
                        }
                        '*' => {
                            st.hidden = true;
                            None
                        }
                        '\n' | '\r' => Some("\n".to_string()),
                        _ => None,
                    };
                    if let Some(s) = out {
                        if skip > 0 {
                            skip -= 1;
                        } else if in_fonttbl > 0 {
                            font_name.push_str(&s);
                        } else if !st.hidden {
                            push(&mut text, &mut runs, &s, &st.style);
                        }
                    }
                    continue;
                }
                let start = i;
                while i < chars.len() && chars[i].is_ascii_alphabetic() {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                let num_start = i;
                if i < chars.len() && (chars[i] == '-' || chars[i].is_ascii_digit()) {
                    i += 1;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let param: Option<i32> = chars[num_start..i].iter().collect::<String>().parse().ok();
                if i < chars.len() && chars[i] == ' ' {
                    i += 1;
                }
                let on = param != Some(0);
                match word.as_str() {
                    "fonttbl" => {
                        in_fonttbl = stack.len();
                        st.hidden = true;
                    }
                    "colortbl" => {
                        in_colortbl = stack.len();
                        st.hidden = true;
                        rgb = [0; 3];
                    }
                    "stylesheet" | "info" | "pict" | "header" | "footer" | "headerl" | "headerr" | "footerl" | "footerr"
                    | "object" | "listtable" | "listoverridetable" | "rsidtbl" | "generator" | "xmlnstbl" | "themedata"
                    | "colorschememapping" | "latentstyles" | "datastore" | "filetbl" | "revtbl" => st.hidden = true,
                    "f" if in_fonttbl > 0 => font_num = param,
                    "f" => {
                        if let Some(name) = fonts.iter().find(|(n, _)| Some(*n) == param).map(|(_, n)| n.clone()) {
                            st.style.font.name = name;
                        }
                    }
                    "red" if in_colortbl > 0 => rgb[0] = param.unwrap_or(0) as u8,
                    "green" if in_colortbl > 0 => rgb[1] = param.unwrap_or(0) as u8,
                    "blue" if in_colortbl > 0 => rgb[2] = param.unwrap_or(0) as u8,
                    "fs" => {
                        if let Some(p) = param.filter(|p| *p > 0 && *p < 20_000) {
                            st.style.font.size = p as f32 / 2.0;
                        }
                    }
                    "b" => st.style.font.bold = on,
                    "i" => st.style.font.italic = on,
                    "ul" => st.style.font.underline = on,
                    "ulnone" => st.style.font.underline = false,
                    "strike" => st.style.font.strike = on,
                    "cf" => {
                        st.style.color = colors.get(param.unwrap_or(0) as usize).copied().flatten().unwrap_or(base.color);
                    }
                    "plain" => {
                        st.style.font.bold = false;
                        st.style.font.italic = false;
                        st.style.font.underline = false;
                        st.style.font.strike = false;
                        st.style.color = base.color;
                        st.style.font.size = base.font.size;
                    }
                    "uc" => st.uc = param.unwrap_or(1).max(0) as usize,
                    "u" => {
                        if let Some(p) = param {
                            let code = if p < 0 { p + 65536 } else { p } as u32;
                            if let (false, Some(ch)) = (st.hidden, char::from_u32(code)) {
                                push(&mut text, &mut runs, &ch.to_string(), &st.style);
                            }
                            skip = st.uc;
                        }
                    }
                    "par" | "line" | "sect" | "page" if !st.hidden => push(&mut text, &mut runs, "\n", &st.style),
                    "tab" if !st.hidden => push(&mut text, &mut runs, "\t", &st.style),
                    "emdash" if !st.hidden => push(&mut text, &mut runs, "\u{2014}", &st.style),
                    "endash" if !st.hidden => push(&mut text, &mut runs, "\u{2013}", &st.style),
                    "bullet" if !st.hidden => push(&mut text, &mut runs, "\u{2022}", &st.style),
                    "lquote" if !st.hidden => push(&mut text, &mut runs, "\u{2018}", &st.style),
                    "rquote" if !st.hidden => push(&mut text, &mut runs, "\u{2019}", &st.style),
                    "ldblquote" if !st.hidden => push(&mut text, &mut runs, "\u{201C}", &st.style),
                    "rdblquote" if !st.hidden => push(&mut text, &mut runs, "\u{201D}", &st.style),
                    _ => {}
                }
            }
            '\r' | '\n' => i += 1,
            _ => {
                i += 1;
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                if in_colortbl > 0 && c == ';' {
                    let first = colors.is_empty() && rgb == [0; 3];
                    colors.push(if first { None } else { Some(Color { r: rgb[0], g: rgb[1], b: rgb[2] }) });
                    rgb = [0; 3];
                } else if in_fonttbl > 0 {
                    font_name.push(c);
                } else if !st.hidden {
                    push(&mut text, &mut runs, &c.to_string(), &st.style);
                }
            }
        }
    }
    // The reader ends the document with a paragraph mark: drop the last one.
    while text.ends_with('\n') {
        text.pop();
    }
    let len = text.len();
    runs.retain_mut(|r| {
        r.end = r.end.min(len);
        r.start < r.end
    });
    if runs.is_empty() {
        runs.push(Run { start: 0, end: len, style: base.clone() });
    }
    StyledText { text, runs, paras: Vec::new() }
}

/// A byte of Windows-1252 text.
fn cp1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}', '\u{90}', '‘', '’', '“',
        '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9F => HIGH[(b - 0x80) as usize],
        _ => b as char,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FontSpec;

    #[test]
    fn reads_styles_fonts_and_colors() {
        let base = RunStyle {
            font: FontSpec {
                name: "DejaVu Sans".into(),
                size: 10.0,
                bold: false,
                italic: false,
                underline: false,
                strike: false,
                pdf_font: None,
            },
            color: Color::BLACK,
            back: None,
            script: crate::rich::Script::Normal,
        };
        let src = r"{\rtf1\ansi{\fonttbl{\f0 Arial;}{\f1 DejaVu Serif;}}{\colortbl;\red255\green0\blue0;}\f0\fs24 Plain {\b bold} \cf1 red\cf0  and {\f1\i serif}\par caf\'e9 \u8364?}";
        let st = parse(src, &base);
        assert_eq!(st.text, "Plain bold red and serif\ncafé €");
        let at = |s: &str| st.runs.iter().find(|r| st.text[r.start..r.end].contains(s)).map(|r| r.style.clone()).unwrap();
        assert!(at("bold").font.bold);
        assert_eq!(at("red").color, Color { r: 255, g: 0, b: 0 });
        assert_eq!(at("serif").font.name, "DejaVu Serif");
        assert!(at("serif").font.italic);
        assert_eq!(at("Plain").font.size, 12.0);
    }
}
