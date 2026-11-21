//! Styled text (`markup="styled"` and `markup="html"`): parsing, line breaking and drawing.
//!
//! Layout follows JasperReports 6: while filling, each line gets the height of its tallest run
//! (AWT `TextLayout`); the PDF exporter then lays the lines out again with iText, spacing them by
//! `factor × largest font size of the line`, where `factor` is the filled height divided by the
//! sum of those font sizes, with the first baseline at the first line's ascent.

use crate::fonts::{FaceId, FontSet};
use crate::model::{Color, FontSpec, HAlign, LineSpacing, TextElement, VAlign};
use crate::print::{PrintItem, PrintLine, PrintText};
use crate::text::Measure;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Markup {
    None,
    Styled,
    Html,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Script {
    Normal,
    Super,
    Sub,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunStyle {
    pub font: FontSpec,
    pub color: Color,
    pub back: Option<Color>,
    pub script: Script,
}

impl RunStyle {
    /// Size the glyphs are drawn at (super/subscripts are two thirds of the font size).
    fn draw_size(&self) -> f32 {
        match self.script {
            Script::Normal => self.font.size,
            _ => self.font.size * 2.0 / 3.0,
        }
    }

    /// Baseline shift (up is positive).
    fn rise(&self) -> f32 {
        match self.script {
            Script::Normal => 0.0,
            Script::Super => self.font.size / 3.0,
            Script::Sub => -self.font.size / 3.0,
        }
    }
}

/// A run of text with one style: `start..end` are byte offsets into [`StyledText::text`].
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub start: usize,
    pub end: usize,
    pub style: RunStyle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StyledText {
    pub text: String,
    pub runs: Vec<Run>,
}

impl StyledText {
    pub fn plain(text: &str, style: &RunStyle) -> StyledText {
        StyledText { text: text.to_string(), runs: vec![Run { start: 0, end: text.len(), style: style.clone() }] }
    }

    fn style_at(&self, byte: usize) -> &RunStyle {
        self.runs
            .iter()
            .find(|r| byte >= r.start && byte < r.end)
            .or_else(|| self.runs.iter().rev().find(|r| r.start <= byte))
            .or(self.runs.first())
            .map(|r| &r.style)
            .expect("styled text has at least one run")
    }

    /// The part from byte `from` on, keeping styles.
    pub fn tail(&self, from: usize) -> StyledText {
        let text = self.text[from..].to_string();
        let mut runs: Vec<Run> = self
            .runs
            .iter()
            .filter(|r| r.end > from)
            .map(|r| Run { start: r.start.max(from) - from, end: r.end - from, style: r.style.clone() })
            .collect();
        if runs.is_empty() {
            runs.push(Run { start: 0, end: 0, style: self.style_at(from).clone() });
        }
        StyledText { text, runs }
    }

    /// Writes the text back as `markup="styled"` source, for a text continued on another page.
    pub fn to_styled_markup(&self, base: &RunStyle) -> String {
        let mut out = String::new();
        for r in &self.runs {
            let s = &r.style;
            let mut attrs = String::new();
            if s.font.name != base.font.name {
                attrs += &format!(" fontName=\"{}\"", escape(&s.font.name));
            }
            if s.font.size != base.font.size {
                attrs += &format!(" size=\"{}\"", s.font.size);
            }
            for (on, base_on, name) in [
                (s.font.bold, base.font.bold, "isBold"),
                (s.font.italic, base.font.italic, "isItalic"),
                (s.font.underline, base.font.underline, "isUnderline"),
                (s.font.strike, base.font.strike, "isStrikeThrough"),
            ] {
                if on != base_on {
                    attrs += &format!(" {name}=\"{on}\"");
                }
            }
            if s.color != base.color {
                attrs += &format!(" forecolor=\"#{:02X}{:02X}{:02X}\"", s.color.r, s.color.g, s.color.b);
            }
            if let Some(b) = s.back {
                attrs += &format!(" backcolor=\"#{:02X}{:02X}{:02X}\"", b.r, b.g, b.b);
            }
            let mut text = escape(&self.text[r.start..r.end]);
            text = match s.script {
                Script::Super => format!("<sup>{text}</sup>"),
                Script::Sub => format!("<sub>{text}</sub>"),
                Script::Normal => text,
            };
            if attrs.is_empty() {
                out += &text;
            } else {
                out += &format!("<style{attrs}>{text}</style>");
            }
        }
        out
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

// ---------------------------------------------------------------------------------------------
// Parsing

fn parse_color(v: &str) -> Option<Color> {
    let v = v.trim();
    let hex = v.strip_prefix('#').unwrap_or(v);
    if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        let n = u32::from_str_radix(hex, 16).ok()?;
        return Some(Color { r: (n >> 16) as u8, g: (n >> 8) as u8, b: n as u8 });
    }
    if hex.len() == 3 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        let d = |i: usize| u8::from_str_radix(&hex[i..i + 1].repeat(2), 16).ok();
        return Some(Color { r: d(0)?, g: d(1)?, b: d(2)? });
    }
    let named = match v.to_ascii_lowercase().as_str() {
        "black" => 0x000000,
        "white" => 0xFFFFFF,
        "red" => 0xFF0000,
        "green" => 0x008000,
        "blue" => 0x0000FF,
        "yellow" => 0xFFFF00,
        "gray" | "grey" => 0x808080,
        "orange" => 0xFFA500,
        "darkgray" | "darkgrey" => 0x404040,
        "lightgray" | "lightgrey" => 0xC0C0C0,
        "cyan" => 0x00FFFF,
        "magenta" => 0xFF00FF,
        "pink" => 0xFFAFAF,
        _ => return None,
    };
    Some(Color { r: (named >> 16) as u8, g: (named >> 8) as u8, b: named as u8 })
}

fn decode_entity(name: &str) -> Option<char> {
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{A0}',
        "copy" => '©',
        "reg" => '®',
        "euro" => '€',
        "ntilde" => 'ñ',
        "Ntilde" => 'Ñ',
        "aacute" => 'á',
        "eacute" => 'é',
        "iacute" => 'í',
        "oacute" => 'ó',
        "uacute" => 'ú',
        "bull" => '•',
        _ => {
            let n = name.strip_prefix('#')?;
            let code = match n.strip_prefix(['x', 'X']) {
                Some(h) => u32::from_str_radix(h, 16).ok()?,
                None => n.parse().ok()?,
            };
            char::from_u32(code)?
        }
    })
}

struct Tag {
    name: String,
    closing: bool,
    self_closing: bool,
    attrs: Vec<(String, String)>,
}

fn parse_tag(src: &str) -> Option<Tag> {
    let inner = src.strip_prefix('<')?.strip_suffix('>')?;
    // A tag name follows `<` (or `</`) directly, as in XML.
    if !inner.trim_start_matches('/').starts_with(|c: char| c.is_ascii_alphabetic()) || inner.contains('<') {
        return None;
    }
    let inner = inner.trim();
    let (closing, inner) = match inner.strip_prefix('/') {
        Some(rest) => (true, rest.trim_start()),
        None => (false, inner),
    };
    let (self_closing, inner) = match inner.strip_suffix('/') {
        Some(rest) => (true, rest.trim_end()),
        None => (false, inner),
    };
    let name_end = inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len());
    let name = inner[..name_end].to_ascii_lowercase();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == ':') {
        return None;
    }
    let mut attrs = Vec::new();
    let mut rest = inner[name_end..].trim_start();
    while !rest.is_empty() {
        let eq = rest.find('=');
        let key_end = rest.find(|c: char| c.is_whitespace() || c == '=').unwrap_or(rest.len());
        let key = rest[..key_end].to_string();
        rest = rest[key_end..].trim_start();
        if eq.is_some() && rest.starts_with('=') {
            rest = rest[1..].trim_start();
            let (value, after) = match rest.chars().next() {
                Some(q @ ('"' | '\'')) => {
                    let close = rest[1..].find(q)? + 1;
                    (&rest[1..close], &rest[close + 1..])
                }
                _ => {
                    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                    (&rest[..end], &rest[end..])
                }
            };
            attrs.push((key, decode_text(value)));
            rest = after.trim_start();
        } else {
            attrs.push((key, String::new()));
        }
    }
    Some(Tag { name, closing, self_closing, attrs })
}

fn decode_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find(';').filter(|&e| e <= 10).and_then(|e| decode_entity(&after[..e]).map(|c| (c, e))) {
            Some((c, e)) => {
                out.push(c);
                rest = &after[e + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// HTML `<font size>` in points (Java's HTML style sheet).
fn html_font_size(v: &str, current: f32) -> f32 {
    const SIZES: [f32; 7] = [8.0, 10.0, 12.0, 14.0, 18.0, 24.0, 36.0];
    let v = v.trim();
    let index = if let Some(d) = v.strip_prefix('+') {
        3 + d.parse::<i32>().unwrap_or(0)
    } else if let Some(d) = v.strip_prefix('-') {
        3 - d.parse::<i32>().unwrap_or(0)
    } else {
        match v.parse::<i32>() {
            Ok(n) => n,
            Err(_) => return current,
        }
    };
    SIZES[(index.clamp(1, 7) - 1) as usize]
}

fn truthy(v: &str) -> bool {
    v.trim().eq_ignore_ascii_case("true")
}

/// Parses styled or HTML markup. Like JasperReports, styled text that is not well formed is
/// shown as it is, tags included.
pub fn parse(src: &str, base: &RunStyle, markup: Markup) -> StyledText {
    match parse_markup(src, base, markup) {
        Some(st) => st,
        None => StyledText::plain(src, base),
    }
}

fn parse_markup(src: &str, base: &RunStyle, markup: Markup) -> Option<StyledText> {
    let html = markup == Markup::Html;
    let mut text = String::new();
    let mut runs: Vec<Run> = Vec::new();
    let mut stack: Vec<(String, RunStyle)> = Vec::new();
    let mut style = base.clone();
    let push_text = |text: &mut String, runs: &mut Vec<Run>, s: &str, style: &RunStyle| {
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
    let mut rest = src;
    let mut last_space = true;
    while !rest.is_empty() {
        let (chunk, tag) = match rest.find('<') {
            Some(0) => {
                let end = rest.find('>')? + 1;
                let tag = &rest[..end];
                rest = &rest[end..];
                ("", Some(tag))
            }
            Some(i) => {
                let c = &rest[..i];
                rest = &rest[i..];
                (c, None)
            }
            None => {
                let c = rest;
                rest = "";
                (c, None)
            }
        };
        if !chunk.is_empty() {
            let mut decoded = decode_text(chunk);
            if html {
                // HTML collapses white space.
                let mut collapsed = String::with_capacity(decoded.len());
                for c in decoded.chars() {
                    if c.is_whitespace() && c != '\u{A0}' {
                        if !last_space {
                            collapsed.push(' ');
                            last_space = true;
                        }
                    } else {
                        collapsed.push(c);
                        last_space = false;
                    }
                }
                decoded = collapsed;
            }
            push_text(&mut text, &mut runs, &decoded, &style);
            continue;
        }
        let Some(tag) = tag else { continue };
        if tag.starts_with("<!--") || tag.starts_with("<!") || tag.starts_with("<?") {
            continue;
        }
        let t = match parse_tag(tag) {
            Some(t) => t,
            None if html => continue,
            None => return None,
        };
        let line_break = |text: &mut String, runs: &mut Vec<Run>, style: &RunStyle| {
            push_text(text, runs, "\n", style);
        };
        match (t.name.as_str(), t.closing) {
            ("br", _) => {
                line_break(&mut text, &mut runs, &style);
                last_space = true;
                continue;
            }
            ("p" | "div" | "li" | "ul" | "ol", true) if html => {
                if !text.ends_with('\n') && !text.is_empty() {
                    line_break(&mut text, &mut runs, &style);
                }
                last_space = true;
            }
            (_, true) => {
                // Close the innermost element with this name (and anything left open inside it).
                if let Some(pos) = stack.iter().rposition(|(n, _)| *n == t.name) {
                    style = stack[pos].1.clone();
                    stack.truncate(pos);
                } else if !html {
                    return None;
                }
                continue;
            }
            _ => {}
        }
        if t.closing {
            continue;
        }
        let saved = style.clone();
        match t.name.as_str() {
            "b" | "strong" => style.font.bold = true,
            "i" | "em" => style.font.italic = true,
            "u" => style.font.underline = true,
            "s" | "strike" | "del" => style.font.strike = true,
            "sup" => style.script = Script::Super,
            "sub" => style.script = Script::Sub,
            "li" => {
                if !text.is_empty() && !text.ends_with('\n') {
                    line_break(&mut text, &mut runs, &style);
                }
                push_text(&mut text, &mut runs, "\u{2022} ", &style);
                last_space = true;
            }
            "p" | "div" if html && !text.is_empty() && !text.ends_with('\n') => {
                line_break(&mut text, &mut runs, &style);
                last_space = true;
            }
            "style" | "font" => {
                for (k, v) in &t.attrs {
                    match (t.name.as_str(), k.as_str()) {
                        ("style", "fontName") | ("font", "face" | "fontFace") => {
                            style.font.name = v.split(',').next().unwrap_or(v).trim().to_string()
                        }
                        ("style", "size") => style.font.size = v.trim().parse().unwrap_or(style.font.size),
                        ("font", "size") => {
                            style.font.size = if html {
                                html_font_size(v, style.font.size)
                            } else {
                                v.trim().parse().unwrap_or(style.font.size)
                            }
                        }
                        ("style", "isBold") => style.font.bold = truthy(v),
                        ("style", "isItalic") => style.font.italic = truthy(v),
                        ("style", "isUnderline") => style.font.underline = truthy(v),
                        ("style", "isStrikeThrough") => style.font.strike = truthy(v),
                        ("style", "forecolor") | ("font", "color") => style.color = parse_color(v).unwrap_or(style.color),
                        ("style", "backcolor") => style.back = parse_color(v),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        if !t.self_closing && t.name != "br" {
            stack.push((t.name.clone(), saved));
        } else {
            style = saved;
        }
    }
    if !html && !stack.is_empty() {
        return None;
    }
    if html {
        // Leading and trailing white space of an HTML body are not shown.
        let trimmed_end = text.trim_end_matches([' ', '\n']).len();
        text.truncate(trimmed_end);
        runs.retain_mut(|r| {
            r.end = r.end.min(trimmed_end);
            r.start < r.end
        });
    }
    if runs.is_empty() {
        runs.push(Run { start: 0, end: text.len(), style: base.clone() });
    }
    Some(StyledText { text, runs })
}

// ---------------------------------------------------------------------------------------------
// Layout

/// Per character: byte offset, width, and the style's face.
struct Glyphs {
    offsets: Vec<usize>,
    /// Prefix sums of advances, `prefix[i]` = width of the first `i` characters.
    prefix: Vec<f32>,
}

fn resolve(fonts: &mut FontSet, style: &RunStyle) -> FaceId {
    fonts.resolve(&style.font)
}

fn glyphs(st: &StyledText, fonts: &mut FontSet, mode: Measure) -> Glyphs {
    let mut offsets = Vec::with_capacity(st.text.len() + 1);
    let mut prefix = Vec::with_capacity(st.text.len() + 1);
    prefix.push(0.0f32);
    let mut acc = 0.0f32;
    for run in &st.runs {
        let id = resolve(fonts, &run.style);
        let face = fonts.face(id);
        let size = run.style.draw_size();
        for (i, c) in st.text[run.start..run.end].char_indices() {
            offsets.push(run.start + i);
            let c = if c == '\t' { ' ' } else { c };
            acc += match mode {
                Measure::Fill => face.fill_char_width(c, size),
                Measure::Render => face.char_width(c, size),
            };
            prefix.push(acc);
        }
    }
    offsets.push(st.text.len());
    Glyphs { offsets, prefix }
}

impl Glyphs {
    fn index_of(&self, byte: usize) -> usize {
        self.offsets.partition_point(|&o| o < byte)
    }
    fn width(&self, from: usize, to: usize) -> f32 {
        self.prefix[self.index_of(to)] - self.prefix[self.index_of(from)]
    }
}

/// A line as byte range `start..end` (trailing spaces excluded from `end`), with the start of
/// the next line in `next`.
#[derive(Debug, Clone, Copy)]
pub struct RLine {
    pub start: usize,
    pub end: usize,
    pub next: usize,
    pub width: f32,
}

fn break_units(text: &str, from: usize, to: usize) -> Vec<usize> {
    // Ends of break units (same rules as plain text).
    let para = &text[from..to];
    let chars: Vec<(usize, char)> = para.char_indices().collect();
    let mut ends = Vec::new();
    for (k, &(i, c)) in chars.iter().enumerate() {
        let next = chars.get(k + 1).map(|x| x.1);
        let is_break = match c {
            ' ' | '\t' => next.is_some_and(|n| n != ' ' && n != '\t'),
            '-' | '\u{2010}' | '\u{2013}' => {
                let prev_alnum = k > 0 && chars[k - 1].1.is_alphanumeric();
                prev_alnum && next.is_some_and(|n| n.is_alphanumeric())
            }
            _ => false,
        };
        if is_break {
            ends.push(from + i + c.len_utf8());
        }
    }
    if ends.last() != Some(&to) {
        ends.push(to);
    }
    ends
}

fn trim_end(text: &str, from: usize, to: usize) -> usize {
    from + text[from..to].trim_end_matches([' ', '\t']).len()
}

pub fn break_rich(st: &StyledText, fonts: &mut FontSet, max_width: f32, mode: Measure) -> Vec<RLine> {
    let g = glyphs(st, fonts, mode);
    let render = if mode == Measure::Render { None } else { Some(glyphs(st, fonts, Measure::Render)) };
    let render_w = |a: usize, b: usize| render.as_ref().unwrap_or(&g).width(a, b);
    let text = &st.text;
    let mut lines = Vec::new();
    let mut para_start = 0;
    loop {
        let para_end = text[para_start..].find('\n').map(|i| para_start + i).unwrap_or(text.len());
        let fits = |a: usize, b: usize| g.width(a, trim_end(text, a, b)) <= max_width + 1e-3;
        let mut line_start = para_start;
        let mut pos = para_start;
        let push = |lines: &mut Vec<RLine>, a: usize, b: usize| {
            let e = trim_end(text, a, b);
            lines.push(RLine { start: a, end: e, next: b, width: render_w(a, e) });
        };
        for unit_end in break_units(text, para_start, para_end) {
            if fits(line_start, unit_end) {
                pos = unit_end;
                continue;
            }
            if pos > line_start {
                push(&mut lines, line_start, pos);
                line_start = pos;
            }
            while !fits(line_start, unit_end) {
                let mut cut = line_start;
                for (i, c) in text[line_start..unit_end].char_indices() {
                    let end = line_start + i + c.len_utf8();
                    if i > 0 && g.width(line_start, end) > max_width + 1e-3 {
                        break;
                    }
                    cut = end;
                }
                push(&mut lines, line_start, cut);
                line_start = cut;
                if line_start >= unit_end {
                    break;
                }
            }
            pos = unit_end;
        }
        push(&mut lines, line_start, para_end.max(line_start));
        if let Some(l) = lines.last_mut() {
            l.next = (para_end + 1).min(text.len());
        }
        if para_end >= text.len() {
            break;
        }
        para_start = para_end + 1;
    }
    lines
}

/// Vertical metrics of one line: tallest ascent and descent, largest font size.
#[derive(Debug, Clone, Copy)]
struct LineMetrics {
    ascent: f32,
    below: f32,
    max_size: f32,
}

fn line_metrics(st: &StyledText, fonts: &mut FontSet, line: &RLine) -> LineMetrics {
    let (a, b) = (line.start, line.next.max(line.end));
    let mut m = LineMetrics { ascent: 0.0, below: 0.0, max_size: 0.0 };
    let mut any = false;
    for r in &st.runs {
        let overlaps = if a == b { r.start <= a && (a < r.end || r.end == a) } else { r.start < b && r.end > a };
        if !overlaps || (r.start == r.end && a != b) {
            continue;
        }
        // Line breaks themselves do not count.
        if st.text[r.start.max(a)..r.end.min(b)].chars().all(|c| c == '\n') && a != b {
            continue;
        }
        any = true;
        let id = resolve(fonts, &r.style);
        let face = fonts.face(id);
        let size = r.style.font.size;
        m.ascent = m.ascent.max(face.ascent(size));
        m.below = m.below.max(face.line_height(size) - face.ascent(size));
        m.max_size = m.max_size.max(size);
    }
    if !any {
        let style = st.style_at(a).clone();
        let id = resolve(fonts, &style);
        let face = fonts.face(id);
        let size = style.font.size;
        m = LineMetrics { ascent: face.ascent(size), below: face.line_height(size) - face.ascent(size), max_size: size };
    }
    m
}

fn line_height(m: &LineMetrics, first: bool, spacing: LineSpacing) -> f32 {
    if first {
        return m.ascent + m.below;
    }
    let h = match spacing {
        LineSpacing::Single => m.ascent,
        LineSpacing::OneAndHalf => 1.5 * m.ascent,
        LineSpacing::Double => 2.0 * m.ascent,
        LineSpacing::Proportional(f) => f * m.ascent,
        LineSpacing::AtLeast(s) => s.max(m.ascent),
        LineSpacing::Fixed(s) => s,
    };
    m.below + h
}

/// Filled height of the first `n` lines.
fn fill_heights(st: &StyledText, fonts: &mut FontSet, lines: &[RLine], spacing: LineSpacing) -> Vec<LineMetricsH> {
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let m = line_metrics(st, fonts, l);
            LineMetricsH { m, height: line_height(&m, i == 0, spacing) }
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct LineMetricsH {
    m: LineMetrics,
    height: f32,
}

/// Height the text needs in a box `width` wide (padding excluded), as filled.
pub fn needed_height(st: &StyledText, fonts: &mut FontSet, width: f32, spacing: LineSpacing) -> f32 {
    let lines = break_rich(st, fonts, width, Measure::Fill);
    fill_heights(st, fonts, &lines, spacing).iter().map(|l| l.height).sum()
}

/// How many filled lines fit in `available` points, and the byte where the rest starts.
pub fn lines_that_fit(
    st: &StyledText,
    fonts: &mut FontSet,
    width: f32,
    spacing: LineSpacing,
    available: f32,
) -> (usize, usize, usize) {
    let lines = break_rich(st, fonts, width, Measure::Fill);
    let hs = fill_heights(st, fonts, &lines, spacing);
    let mut total = 0.0;
    let mut k = 0;
    for h in &hs {
        if total + h.height > available + 1e-3 {
            break;
        }
        total += h.height;
        k += 1;
    }
    let rest = lines.get(k).map(|l| l.start).unwrap_or(st.text.len());
    (k, lines.len(), rest)
}

/// Lays out styled text in a box and returns what to draw, with the height the text needs.
#[allow(clippy::too_many_arguments)]
pub fn layout(
    fonts: &mut FontSet,
    te: &TextElement,
    st: &StyledText,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    clip: bool,
) -> (PrintItem, f32) {
    let p = te.boxed.padding;
    let (cx, cy, cw, ch) = (x + p.left, y + p.top, (w - p.left - p.right).max(0.0), h - p.top - p.bottom);
    let spacing = te.style.line_spacing;

    // Filling: heights, how much fits.
    let fill_lines = break_rich(st, fonts, cw, Measure::Fill);
    let fill = fill_heights(st, fonts, &fill_lines, spacing);
    let needed = p.top + p.bottom + fill.iter().map(|l| l.height).sum::<f32>();
    let mut fit = fill.len();
    if clip {
        let mut total = 0.0;
        fit = 0;
        for l in &fill {
            if total + l.height > ch + 1e-3 {
                break;
            }
            total += l.height;
            fit += 1;
        }
    }
    let shown = if fit < fill_lines.len() {
        let cut = fill_lines[fit].start;
        let head = st.text[..cut].trim_end_matches(['\n', ' ']).len();
        let mut s = st.clone();
        s.text.truncate(head);
        s.runs.retain_mut(|r| {
            r.end = r.end.min(head);
            r.start < r.end
        });
        if s.runs.is_empty() {
            s.runs.push(Run { start: 0, end: 0, style: st.style_at(0).clone() });
        }
        s
    } else {
        st.clone()
    };
    let fill = &fill[..fit.min(fill.len())];
    let text_height: f32 = fill.iter().map(|l| l.height).sum();
    let size_sum: f32 = fill.iter().map(|l| l.m.max_size).sum();
    let factor = if size_sum > 0.0 { text_height / size_sum } else { 1.0 };
    let first_leading = fill.first().map(|l| l.m.ascent).unwrap_or(0.0);

    // Exporting: line breaks with iText widths, baselines spaced by factor × largest size.
    let mut lines = break_rich(&shown, fonts, cw, Measure::Render);
    if clip {
        lines.truncate(fit.max(if fit == 0 { 0 } else { 1 }));
    }
    let voff = match te.style.valign {
        VAlign::Top => 0.0,
        VAlign::Middle => (ch - text_height) / 2.0,
        VAlign::Bottom => ch - text_height,
    };
    let mut items = Vec::new();
    let mut baseline = cy + voff;
    let g = glyphs(&shown, fonts, Measure::Render);
    let mut prev_spacing = 0.0f32;
    for (k, l) in lines.iter().enumerate() {
        let m = line_metrics(&shown, fonts, l);
        baseline += if k == 0 { first_leading } else { factor * m.max_size };
        // Justification: extra space per space character (not on a paragraph's last line).
        let ends_paragraph = l.next >= shown.text.len() || shown.text[..l.next].ends_with('\n');
        let starts_paragraph = l.start == 0 || shown.text[..l.start].ends_with('\n');
        let spaces = shown.text[l.start..l.end].matches(' ').count();
        let mut word_spacing = 0.0;
        if te.style.halign == HAlign::Justified {
            let hanging = match shown.text[l.start..l.end].chars().last() {
                Some(c @ (',' | '.' | ';' | ':')) => {
                    let st_ = shown.style_at(l.end - 1).clone();
                    {
                        let id = resolve(fonts, &st_);
                        0.4 * fonts.face(id).char_width(c, st_.draw_size())
                    }
                }
                _ => 0.0,
            };
            word_spacing = if !ends_paragraph && spaces > 0 {
                (cw - (l.width - hanging)) / spaces as f32
            } else if ends_paragraph && k > 0 && !starts_paragraph {
                prev_spacing
            } else {
                0.0
            };
            prev_spacing = word_spacing;
        }
        let line_width = l.width + word_spacing * spaces as f32;
        let mut px = match te.style.halign {
            HAlign::Center => cx + (cw - l.width) / 2.0,
            HAlign::Right => cx + cw - l.width,
            _ => cx,
        };
        let _ = line_width;
        // One drawn segment per run on this line.
        for r in &shown.runs {
            let (a, b) = (r.start.max(l.start), r.end.min(l.end));
            if a >= b {
                continue;
            }
            let seg = &shown.text[a..b];
            let seg_spaces = seg.matches(' ').count() as f32;
            let width = g.width(a, b);
            let id = resolve(fonts, &r.style);
            let size = r.style.draw_size();
            let seg_w = width + word_spacing * seg_spaces;
            if let Some(back) = r.style.back {
                let face = fonts.face(id);
                let (asc, desc) = face.typo_metrics(size);
                let top = baseline - r.style.rise() - asc;
                items.push(PrintItem::Rect {
                    x: px,
                    y: top,
                    w: seg_w,
                    h: asc + desc,
                    fill: Some(back),
                    stroke: None,
                    radius: 0.0,
                    path: false,
                });
            }
            items.push(PrintItem::Text(PrintText {
                face: id,
                size,
                color: r.style.color,
                underline: r.style.font.underline,
                strike: r.style.font.strike,
                lines: vec![PrintLine {
                    text: seg.to_string(),
                    x: px,
                    baseline: baseline - r.style.rise(),
                    width: seg_w,
                    word_spacing,
                }],
            }));
            px += seg_w;
        }
    }
    (PrintItem::Group(items), needed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> RunStyle {
        RunStyle {
            font: FontSpec {
                name: "DejaVu Sans".into(),
                size: 10.0,
                bold: false,
                italic: false,
                underline: false,
                strike: false,
            },
            color: Color::BLACK,
            back: None,
            script: Script::Normal,
        }
    }

    #[test]
    fn parses_styled_tags() {
        let st = parse(r##"A <style isBold="true" forecolor="#CC0000">b &amp; c</style> d"##, &base(), Markup::Styled);
        assert_eq!(st.text, "A b & c d");
        assert_eq!(st.runs.len(), 3);
        assert!(st.runs[1].style.font.bold);
        assert_eq!(st.runs[1].style.color, Color { r: 0xCC, g: 0, b: 0 });
        // Not well formed: shown as it is.
        let st = parse("a < b <style>", &base(), Markup::Styled);
        assert_eq!(st.text, "a < b <style>");
    }

    #[test]
    fn parses_html() {
        let st = parse("<b>B</b>  and\n <font size=\"4\" color=\"#0066CC\">x</font><br>next", &base(), Markup::Html);
        assert_eq!(st.text, "B and x\nnext");
        assert!(st.runs[0].style.font.bold);
        let x = st.runs.iter().find(|r| &st.text[r.start..r.end] == "x").unwrap();
        assert_eq!(x.style.font.size, 14.0);
    }

    #[test]
    fn round_trips_to_styled_markup() {
        let b = base();
        let st = parse(r#"plain <style isItalic="true" size="12">it &lt;</style><sup>2</sup>"#, &b, Markup::Styled);
        let again = parse(&st.to_styled_markup(&b), &b, Markup::Styled);
        assert_eq!(st, again);
    }
}
