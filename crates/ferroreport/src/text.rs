//! Line breaking and vertical text metrics, following Jasper's AWT-based text measurer.

use crate::fonts::FaceData;
use crate::model::LineSpacing;

/// One laid-out line of text.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub text: String,
    /// Advance width without trailing whitespace.
    pub width: f32,
    /// Byte offset of the line in the [`normalize`]d text, used to continue a text that is
    /// split across pages.
    pub start: usize,
}

/// Line endings normalised to `\n` and tabs to spaces (same byte length).
pub fn normalize(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains(['\r', '\t']) {
        text.replace("\r\n", "\n").replace('\r', "\n").replace('\t', " ").into()
    } else {
        text.into()
    }
}

/// Which of JasperReports' two text measurements to use.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Measure {
    /// Filling: decides stretch height, truncation and page splits.
    Fill,
    /// Exporting: decides the drawn line breaks and alignment.
    Render,
}

/// The text of the first `k` lines of [`break_lines`] (the part that fits in a box).
pub fn text_before_line(text: &str, lines: &[Line], k: usize) -> String {
    let normalized = normalize(text);
    match lines.get(k) {
        Some(l) => {
            let head = &normalized[..l.start];
            head.strip_suffix('\n').unwrap_or(head).trim_end_matches(' ').to_string()
        }
        None => normalized.into_owned(),
    }
}

fn measure(face: &FaceData, size: f32, mode: Measure, s: &str) -> f32 {
    match mode {
        Measure::Fill => face.fill_width(s, size),
        Measure::Render => face.text_width(s, size),
    }
}

/// Breaks `text` into lines no wider than `max_width` at `size` points.
///
/// Paragraphs are split on newlines. Within a paragraph, breaks happen after whitespace and
/// hyphens; trailing whitespace hangs past the edge. A word longer than the line is broken
/// between characters, keeping at least one character per line.
pub fn break_lines(text: &str, face: &FaceData, size: f32, max_width: f32, mode: Measure) -> Vec<Line> {
    let mut lines = Vec::new();
    let normalized = normalize(text);
    let mut base = 0;
    for paragraph in normalized.split('\n') {
        break_paragraph(paragraph, base, face, size, max_width, mode, &mut lines);
        base += paragraph.len() + 1;
    }
    lines
}

/// The text left after the first `k` lines of [`break_lines`], for continuing on a new page.
pub fn text_after_lines(text: &str, lines: &[Line], k: usize) -> String {
    let normalized = normalize(text);
    match lines.get(k) {
        Some(l) => normalized[l.start..].to_string(),
        None => String::new(),
    }
}

/// Splits a paragraph into break units: each unit ends at a break opportunity.
fn units(paragraph: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let chars: Vec<(usize, char)> = paragraph.char_indices().collect();
    for (k, &(i, c)) in chars.iter().enumerate() {
        let next = chars.get(k + 1).map(|x| x.1);
        let end = i + c.len_utf8();
        let is_break = match c {
            ' ' => next.is_some_and(|n| n != ' '),
            '-' | '\u{2010}' | '\u{2013}' => {
                // Break after a hyphen only when it is preceded by a letter/digit and followed by one.
                let prev_alnum = k > 0 && chars[k - 1].1.is_alphanumeric();
                prev_alnum && next.is_some_and(|n| n.is_alphanumeric())
            }
            _ => false,
        };
        if is_break {
            out.push(&paragraph[start..end]);
            start = end;
        }
    }
    if start < paragraph.len() || out.is_empty() {
        out.push(&paragraph[start..]);
    }
    out
}

fn break_paragraph(
    paragraph: &str,
    base: usize,
    face: &FaceData,
    size: f32,
    max_width: f32,
    mode: Measure,
    lines: &mut Vec<Line>,
) {
    let fits = |s: &str| measure(face, size, mode, s.trim_end_matches(' ')) <= max_width + 1e-3;
    let push = |lines: &mut Vec<Line>, from: usize, to: usize| {
        let text = paragraph[from..to].trim_end_matches(' ');
        lines.push(Line { text: text.to_string(), width: face.text_width(text, size), start: base + from });
    };
    // Most texts are one line that fits: widths only grow with length, so when the whole
    // paragraph fits there is nothing to break.
    if fits(paragraph) {
        push(lines, 0, paragraph.len());
        return;
    }
    // `line_start..pos` is the current line, as byte offsets into `paragraph`.
    let mut line_start = 0;
    let mut pos = 0;
    for unit in units(paragraph) {
        let unit_end = pos + unit.len();
        if fits(&paragraph[line_start..unit_end]) {
            pos = unit_end;
            continue;
        }
        if pos > line_start {
            push(lines, line_start, pos);
            line_start = pos;
        }
        // The unit alone may still be too wide: break it between characters.
        while !fits(&paragraph[line_start..unit_end]) {
            let mut w = 0.0;
            let mut cut = line_start;
            for (i, c) in paragraph[line_start..unit_end].char_indices() {
                let cw = measure(face, size, mode, c.encode_utf8(&mut [0; 4]));
                if i > 0 && w + cw > max_width + 1e-3 {
                    break;
                }
                w += cw;
                cut = line_start + i + c.len_utf8();
            }
            push(lines, line_start, cut);
            line_start = cut;
            if line_start >= unit_end {
                break;
            }
        }
        pos = unit_end;
    }
    push(lines, line_start, paragraph.len().max(line_start));
}

/// Vertical metrics of a block of text in a given font and line spacing.
///
/// JasperReports 6 gives the first line its natural height (ascent + descent + leading) and
/// every following line `descent + leading + h`, where `h` is `factor * ascent` for
/// proportional spacings (Single = 1, 1½ = 1.5, Double = 2, Proportional = size), the given
/// size for Fixed, and `max(size, ascent)` for AtLeast. When drawing, lines are spaced
/// evenly by the average of those heights, with the first baseline one ascent below the top.
/// These rules were derived from Jasper's PDF output and reproduce it exactly.
#[derive(Debug, Clone, Copy)]
pub struct VerticalMetrics {
    pub ascent: f32,
    pub first_line_height: f32,
    pub other_line_height: f32,
}

pub fn vertical_metrics(face: &FaceData, size: f32, spacing: LineSpacing) -> VerticalMetrics {
    let natural = face.line_height(size);
    let ascent = face.ascent(size);
    let below = natural - ascent;
    let h = match spacing {
        LineSpacing::Single => ascent,
        LineSpacing::OneAndHalf => 1.5 * ascent,
        LineSpacing::Double => 2.0 * ascent,
        LineSpacing::Proportional(f) => f * ascent,
        LineSpacing::AtLeast(s) => s.max(ascent),
        LineSpacing::Fixed(s) => s,
    };
    VerticalMetrics { ascent, first_line_height: natural, other_line_height: below + h }
}

impl VerticalMetrics {
    /// Height of a block with `n` lines.
    pub fn block_height(&self, n: usize) -> f32 {
        if n == 0 {
            0.0
        } else {
            self.first_line_height + self.other_line_height * (n - 1) as f32
        }
    }

    /// Distance between consecutive baselines in a block of `n` lines.
    pub fn advance(&self, n: usize) -> f32 {
        if n == 0 {
            0.0
        } else {
            self.block_height(n) / n as f32
        }
    }

    /// How many lines fit in `available` points.
    pub fn lines_that_fit(&self, available: f32) -> usize {
        if self.first_line_height > available + 1e-3 {
            return 0;
        }
        1 + ((available - self.first_line_height + 1e-3) / self.other_line_height).floor().max(0.0) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::FontSet;
    use crate::model::FontSpec;

    fn face() -> (FontSet, usize) {
        let mut fonts = FontSet::new();
        let id = fonts.resolve(&FontSpec {
            name: "DejaVu Sans".into(),
            size: 10.0,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
        });
        (fonts, id)
    }

    #[test]
    fn wraps_on_spaces_and_splits_long_words() {
        let (fonts, id) = face();
        let f = fonts.face(id);
        let lines = break_lines("hello world foo", f, 10.0, f.text_width("hello world", 10.0) + 0.5, Measure::Render);
        assert_eq!(lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["hello world", "foo"]);
        let lines = break_lines("abcdefghij", f, 10.0, f.text_width("abcd", 10.0) + 0.1, Measure::Render);
        assert_eq!(lines[0].text, "abcd");
        assert_eq!(lines.len(), 3);
        let lines = break_lines("a\n\nb", f, 10.0, 100.0, Measure::Render);
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn fit_counts() {
        let (fonts, id) = face();
        let m = vertical_metrics(fonts.face(id), 10.0, LineSpacing::Single);
        assert_eq!(m.lines_that_fit(18.0), 1);
        assert_eq!(m.lines_that_fit(23.3), 2);
        assert_eq!(m.lines_that_fit(5.0), 0);
    }

    #[test]
    fn spacing_matches_jasper() {
        let (fonts, id) = face();
        // Baseline advances measured in JasperReports 6.20 PDFs (DejaVu Sans 10pt).
        let m = vertical_metrics(fonts.face(id), 10.0, LineSpacing::OneAndHalf);
        assert!((m.advance(2) - 13.96).abs() < 0.01);
        assert!((m.advance(3) - 14.74).abs() < 0.01);
        assert!((m.advance(4) - 15.12).abs() < 0.01);
        let m = vertical_metrics(fonts.face(id), 10.0, LineSpacing::Fixed(20.0));
        assert!((m.advance(2) - 17.0).abs() < 0.01);
        let m = vertical_metrics(fonts.face(id), 10.0, LineSpacing::Proportional(0.8));
        assert!((m.advance(4) - 10.25).abs() < 0.01);
    }
}
