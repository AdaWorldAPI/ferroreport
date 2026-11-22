//! HTML export: every page as an inline SVG, drawn from the same print items as the PDF.
//!
//! The pages look like the PDF (same positions, sizes, colors, line breaks) and the text is
//! real text: it can be selected, searched and read by a screen reader. Each line of text is
//! given its measured width, so the layout holds even where the browser has to fall back to
//! another font. Meant for previews in a browser; the PDF remains the document of record.

use crate::error::{Error, Result};
use crate::fonts::FontSet;
use crate::image::{ImageData, Pixels};
use crate::model::{Color, LineStyle, Pen};
use crate::print::{PathOp, PrintDocument, PrintItem, PrintPage};
use base64::Engine;
use std::collections::HashSet;
use std::fmt::Write as _;

/// Options for the HTML export.
#[derive(Debug, Clone, Default)]
pub struct HtmlOptions {
    /// Title of the page (the report's name when empty).
    pub title: Option<String>,
    /// Only the pages (`<svg>` elements), without the surrounding HTML document: to place
    /// them inside a page of your own.
    pub fragment: bool,
}

const STYLE: &str = "body{margin:0;background:#e8e8e8}\
svg.page{display:block;margin:16px auto;background:#fff;box-shadow:0 1px 4px rgba(0,0,0,.25);max-width:100%;height:auto}\
svg.page text{white-space:pre}\
@media print{body{background:none}svg.page{margin:0;box-shadow:none;break-after:page}}";

fn n(v: f32) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".into()
    } else {
        s.to_string()
    }
}

fn nd(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".into()
    } else {
        s.to_string()
    }
}

fn hex(c: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

fn escape(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // Control characters are not allowed in XML text.
            c if (c as u32) < 0x20 && c != '\t' => out.push(' '),
            c => out.push(c),
        }
    }
}

/// Stroke attributes for a pen (`shape`: the outline of a rectangle or ellipse, which
/// JasperReports dashes differently from a line).
fn stroke(pen: &Pen, shape: bool, out: &mut String) {
    let _ = write!(out, " stroke=\"{}\" stroke-width=\"{}\"", hex(pen.color), n(pen.width));
    let w = pen.width.max(0.1);
    match (pen.style, shape) {
        (LineStyle::Dashed, false) => {
            let _ = write!(out, " stroke-dasharray=\"{} {}\"", n(5.0 * w), n(3.0 * w));
        }
        (LineStyle::Dotted, false) => {
            let _ = write!(out, " stroke-dasharray=\"{} {}\"", n(w), n(w));
        }
        (LineStyle::Dashed, true) => {
            let _ = write!(out, " stroke-dasharray=\"{} {}\"", n(4.0 * w), n(4.0 * w));
        }
        (LineStyle::Dotted, true) => {
            let _ = write!(out, " stroke-linecap=\"round\" stroke-dasharray=\"0 {}\"", n(2.0 * w));
        }
        _ => {}
    }
}

/// A font family for CSS: the report's family first, then one every system has.
fn css_family(family: &str) -> String {
    let lower = family.to_ascii_lowercase();
    let generic = if lower.contains("mono") || lower.contains("courier") {
        "monospace"
    } else if lower.contains("serif") && !lower.contains("sans") || lower.contains("times") {
        "serif"
    } else {
        "sans-serif"
    };
    let clean: String = family.chars().filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_').collect();
    format!("'{clean}',{generic}")
}

/// An image as a `data:` URI (JPEG as it is, anything else as PNG).
fn data_uri(image: &ImageData) -> Result<String> {
    let b64 = base64::engine::general_purpose::STANDARD;
    match &image.pixels {
        Pixels::Jpeg { data, .. } => Ok(format!("data:image/jpeg;base64,{}", b64.encode(data))),
        Pixels::Svg(svg) => Ok(format!("data:image/svg+xml;base64,{}", b64.encode(&svg.source))),
        Pixels::Raw { samples, gray, alpha } => {
            let channels = if *gray { 1 } else { 3 };
            let (color, bytes) = match alpha {
                None => (if *gray { png::ColorType::Grayscale } else { png::ColorType::Rgb }, samples.clone()),
                Some(a) => {
                    let mut out = Vec::with_capacity(samples.len() + a.len());
                    for (px, alpha) in samples.chunks(channels).zip(a) {
                        out.extend_from_slice(px);
                        out.push(*alpha);
                    }
                    (if *gray { png::ColorType::GrayscaleAlpha } else { png::ColorType::Rgba }, out)
                }
            };
            let mut file = Vec::new();
            {
                let mut enc = png::Encoder::new(&mut file, image.width, image.height);
                enc.set_color(color);
                enc.set_depth(png::BitDepth::Eight);
                let mut writer = enc.write_header().map_err(|e| Error::Io(format!("image: {e}")))?;
                writer.write_image_data(&bytes).map_err(|e| Error::Io(format!("image: {e}")))?;
            }
            Ok(format!("data:image/png;base64,{}", b64.encode(file)))
        }
    }
}

/// Writes the pages of a report as HTML.
struct Pages {
    width: f32,
    height: f32,
    /// Images already written (each is defined once and reused).
    images: HashSet<usize>,
    count: usize,
    /// The links of the page being written: drawn last, over what they cover.
    links: String,
}

/// An anchor as an `id` or a `#fragment`: letters, digits and `-_.` as they are, the rest
/// as `_` followed by its code.
fn anchor_id(name: &str) -> String {
    let mut out = String::from("a-");
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
            out.push(c);
        } else {
            let _ = write!(out, "_{:x}_", c as u32);
        }
    }
    out
}

impl Pages {
    fn page(&mut self, items: &[PrintItem], fonts: &FontSet, out: &mut String) -> Result<()> {
        self.count += 1;
        let _ = writeln!(
            out,
            "<svg class=\"page\" id=\"page-{c}\" xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" width=\"{w}pt\" height=\"{h}pt\" role=\"img\" aria-label=\"Page {c}\">",
            c = self.count,
            w = n(self.width),
            h = n(self.height)
        );
        self.items(items, fonts, out)?;
        out.push_str(&std::mem::take(&mut self.links));
        out.push_str("</svg>\n");
        Ok(())
    }

    /// An anchor where the element is, and a link over it.
    fn link(&mut self, link: &crate::print::LinkInfo) {
        use crate::print::LinkTarget;
        let rect = format!("x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"", n(link.x), n(link.y), n(link.w), n(link.h));
        if let Some(name) = &link.anchor {
            let _ = writeln!(self.links, "<rect id=\"{}\" {rect} fill=\"none\"/>", anchor_id(name));
        }
        let href = match &link.target {
            Some(LinkTarget::Url(url)) => {
                // Only links a browser opens as pages: no scripts from the data.
                let lower = url.trim_start().to_ascii_lowercase();
                if lower.starts_with("javascript:") || lower.starts_with("data:") || lower.starts_with("vbscript:") {
                    return;
                }
                url.clone()
            }
            Some(LinkTarget::Anchor(a)) => format!("#{}", anchor_id(a)),
            Some(LinkTarget::Page(p)) => format!("#page-{p}"),
            Some(LinkTarget::RemoteAnchor { file, anchor }) => format!("{file}#{}", anchor_id(anchor)),
            Some(LinkTarget::RemotePage { file, page }) => format!("{file}#page-{page}"),
            None => return,
        };
        self.links.push_str("<a href=\"");
        escape(&href, &mut self.links);
        self.links.push('"');
        if matches!(link.target, Some(LinkTarget::Url(_))) {
            self.links.push_str(" target=\"_blank\" rel=\"noopener noreferrer\"");
        }
        self.links.push('>');
        if let Some(t) = &link.tooltip {
            self.links.push_str("<title>");
            escape(t, &mut self.links);
            self.links.push_str("</title>");
        }
        let _ = writeln!(self.links, "<rect {rect} fill=\"transparent\"/></a>");
    }

    fn items(&mut self, items: &[PrintItem], fonts: &FontSet, out: &mut String) -> Result<()> {
        for item in items {
            match item {
                PrintItem::Element(_) => {}
                PrintItem::Link(link) => self.link(link),
                PrintItem::Cell(_, inner) => self.items(std::slice::from_ref(inner), fonts, out)?,
                PrintItem::Group(inner) | PrintItem::Paragraphs(_, inner) => self.items(inner, fonts, out)?,
                PrintItem::Transform([a, b, c, d, e, f], inner) => {
                    let _ =
                        writeln!(out, "<g transform=\"matrix({} {} {} {} {} {})\">", n(*a), n(*b), n(*c), n(*d), n(*e), n(*f));
                    self.items(inner, fonts, out)?;
                    out.push_str("</g>\n");
                }
                PrintItem::Rect { x, y, w, h, fill, stroke: pen, radius, .. } => {
                    if fill.is_none() && pen.is_none() {
                        continue;
                    }
                    let _ = write!(out, "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"", n(*x), n(*y), n(*w), n(*h));
                    if *radius > 0.0 {
                        let _ = write!(out, " rx=\"{}\"", n(radius.min(w / 2.0).min(h / 2.0)));
                    }
                    let _ = write!(out, " fill=\"{}\"", fill.map(hex).unwrap_or_else(|| "none".into()));
                    if let Some(pen) = pen {
                        stroke(pen, true, out);
                    }
                    out.push_str("/>\n");
                }
                PrintItem::Ellipse { x, y, w, h, fill, stroke: pen } => {
                    if fill.is_none() && pen.is_none() {
                        continue;
                    }
                    let _ = write!(
                        out,
                        "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\" fill=\"{}\"",
                        n(x + w / 2.0),
                        n(y + h / 2.0),
                        n(w / 2.0),
                        n(h / 2.0),
                        fill.map(hex).unwrap_or_else(|| "none".into())
                    );
                    if let Some(pen) = pen {
                        stroke(pen, true, out);
                    }
                    out.push_str("/>\n");
                }
                PrintItem::Line { x1, y1, x2, y2, pen } => {
                    if pen.style == LineStyle::Double {
                        // Two thin strokes on either side of the center line.
                        let (dx, dy) = if (y1 - y2).abs() < 1e-3 { (0.0, pen.width / 3.0) } else { (pen.width / 3.0, 0.0) };
                        for s in [-1.0f32, 1.0] {
                            let _ = writeln!(
                                out,
                                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"{}\"/>",
                                n(x1 + s * dx),
                                n(y1 + s * dy),
                                n(x2 + s * dx),
                                n(y2 + s * dy),
                                hex(pen.color),
                                n(pen.width / 3.0)
                            );
                        }
                    } else {
                        let _ = write!(out, "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"", n(*x1), n(*y1), n(*x2), n(*y2));
                        stroke(pen, false, out);
                        out.push_str("/>\n");
                    }
                }
                PrintItem::Text(t) => {
                    let face = fonts.face(t.face);
                    for line in &t.lines {
                        if line.text.is_empty() {
                            continue;
                        }
                        // Spaces at the ends are positions, not glyphs: browsers do not count them
                        // when they fit a line to its width, so they are taken out of both.
                        let space = face.char_width(' ', t.size) + line.word_spacing;
                        let lead = line.text.len() - line.text.trim_start_matches(' ').len();
                        let text = line.text.trim_matches(' ');
                        if text.is_empty() {
                            continue;
                        }
                        let spaces = text.matches(' ').count() as f32;
                        let _ = write!(
                            out,
                            "<text x=\"{}\" y=\"{}\" font-family=\"{}\" font-size=\"{}\" fill=\"{}\"",
                            n(line.x + lead as f32 * space),
                            n(line.baseline),
                            css_family(&face.family),
                            n(t.size),
                            hex(t.color)
                        );
                        if face.bold {
                            out.push_str(" font-weight=\"bold\"");
                        }
                        if face.italic {
                            out.push_str(" font-style=\"italic\"");
                        }
                        match (t.underline, t.strike) {
                            (true, true) => out.push_str(" text-decoration=\"underline line-through\""),
                            (true, false) => out.push_str(" text-decoration=\"underline\""),
                            (false, true) => out.push_str(" text-decoration=\"line-through\""),
                            _ => {}
                        }
                        if line.word_spacing.abs() > 1e-4 {
                            let _ = write!(out, " word-spacing=\"{}\"", n(line.word_spacing));
                        }
                        // The width the PDF gives the line (the glyphs' own widths plus the extra
                        // after each space) keeps it in place whatever font draws it.
                        let width = face.text_width(text, t.size) + line.word_spacing * spaces;
                        if width > 0.0 && text.chars().count() > 1 {
                            let _ = write!(out, " textLength=\"{}\" lengthAdjust=\"spacing\"", n(width));
                        }
                        out.push('>');
                        escape(text, out);
                        out.push_str("</text>\n");
                    }
                }
                PrintItem::Image { x, y, w, h, clip, image } => {
                    let id = format!("ferro-im{}", image.id);
                    if self.images.insert(image.id) {
                        let _ = writeln!(
                            out,
                            "<defs><image id=\"{id}\" width=\"1\" height=\"1\" preserveAspectRatio=\"none\" href=\"{}\"/></defs>",
                            data_uri(image)?
                        );
                    }
                    let place =
                        format!("<use href=\"#{id}\" transform=\"translate({} {}) scale({} {})\"/>", n(*x), n(*y), n(*w), n(*h));
                    match clip {
                        Some([cx, cy, cw, ch]) => {
                            let _ = writeln!(
                                out,
                                "<svg x=\"{cx}\" y=\"{cy}\" width=\"{cw}\" height=\"{ch}\" viewBox=\"{cx} {cy} {cw} {ch}\" overflow=\"hidden\">{place}</svg>",
                                cx = n(*cx),
                                cy = n(*cy),
                                cw = n(*cw),
                                ch = n(*ch)
                            );
                        }
                        None => {
                            out.push_str(&place);
                            out.push('\n');
                        }
                    }
                }
                PrintItem::Path { ops, fill, stroke: line, dash } => {
                    if ops.is_empty() || (fill.is_none() && line.is_none()) {
                        continue;
                    }
                    out.push_str("<path d=\"");
                    path_data(ops, false, out);
                    let _ = write!(out, "\" fill=\"{}\"", fill.map(hex).unwrap_or_else(|| "none".into()));
                    if let Some((color, width)) = line {
                        let _ = write!(out, " stroke=\"{}\" stroke-width=\"{}\"", hex(*color), n(*width));
                        if let Some([on, off]) = dash {
                            let _ = write!(out, " stroke-dasharray=\"{} {}\"", n(*on), n(*off));
                        }
                    }
                    out.push_str("/>\n");
                }
                PrintItem::Shapes { x, y, scale, color, rects, path } => {
                    let _ = write!(
                        out,
                        "<g transform=\"translate({} {}) scale({})\" fill=\"{}\">",
                        n(*x),
                        n(*y),
                        nd(*scale),
                        hex(*color)
                    );
                    if !rects.is_empty() {
                        // One path for all the bars keeps a barcode small.
                        out.push_str("<path d=\"");
                        for [rx, ry, rw, rh] in rects.iter() {
                            let _ = write!(out, "M{} {}h{}v{}h{}z", nd(*rx), nd(*ry), nd(*rw), nd(*rh), nd(-*rw));
                        }
                        out.push_str("\"/>");
                    }
                    if !path.is_empty() {
                        out.push_str("<path d=\"");
                        path_data(path, true, out);
                        out.push_str("\"/>");
                    }
                    out.push_str("</g>\n");
                }
            }
        }
        Ok(())
    }
}

/// SVG path data. In a barcode drawing (`initial`), a quadratic step is the cubic whose first
/// control point is the current point, as the PDF exporter writes it.
fn path_data(ops: &[PathOp], initial: bool, out: &mut String) {
    let mut cur = (0.0f64, 0.0f64);
    for op in ops {
        match *op {
            PathOp::Move(a, b) => {
                let _ = write!(out, "M{} {}", nd(a), nd(b));
                cur = (a, b);
            }
            PathOp::Line(a, b) => {
                let _ = write!(out, "L{} {}", nd(a), nd(b));
                cur = (a, b);
            }
            PathOp::Quad(a, b, x, y) => {
                if initial {
                    let _ = write!(out, "C{} {} {} {} {} {}", nd(cur.0), nd(cur.1), nd(a), nd(b), nd(x), nd(y));
                } else {
                    let _ = write!(out, "Q{} {} {} {}", nd(a), nd(b), nd(x), nd(y));
                }
                cur = (x, y);
            }
            PathOp::Cubic(a, b, c, d, x, y) => {
                let _ = write!(out, "C{} {} {} {} {} {}", nd(a), nd(b), nd(c), nd(d), nd(x), nd(y));
                cur = (x, y);
            }
            PathOp::Close => out.push('Z'),
        }
    }
}

fn head(title: &str, out: &mut String) {
    out.push_str("<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>");
    escape(title, out);
    let _ = write!(
        out,
        "</title>\n<meta name=\"generator\" content=\"ferroreport {}\">\n<style>{STYLE}</style>\n</head>\n<body>\n",
        env!("CARGO_PKG_VERSION")
    );
}

/// Exports a filled document to an HTML page.
pub fn write_html(doc: &PrintDocument, fonts: &FontSet, options: &HtmlOptions) -> Result<String> {
    let mut out = String::new();
    if !options.fragment {
        head(options.title.as_deref().unwrap_or(&doc.name), &mut out);
    }
    let mut pages =
        Pages { width: doc.page_width, height: doc.page_height, images: HashSet::new(), count: 0, links: String::new() };
    for page in &doc.pages {
        pages.page(&page.items, fonts, &mut out)?;
    }
    if !options.fragment {
        out.push_str("</body>\n</html>\n");
    }
    Ok(out)
}

/// Writes HTML to `out` page by page.
pub struct HtmlStream<W: std::io::Write> {
    out: W,
    pages: Pages,
    fragment: bool,
    buf: String,
}

fn io(e: std::io::Error) -> Error {
    Error::Io(e.to_string())
}

impl<W: std::io::Write> HtmlStream<W> {
    pub fn new(mut out: W, name: &str, page_width: f32, page_height: f32, options: HtmlOptions) -> Result<HtmlStream<W>> {
        if !options.fragment {
            let mut text = String::new();
            head(options.title.as_deref().unwrap_or(name), &mut text);
            out.write_all(text.as_bytes()).map_err(io)?;
        }
        let pages = Pages { width: page_width, height: page_height, images: HashSet::new(), count: 0, links: String::new() };
        Ok(HtmlStream { out, pages, fragment: options.fragment, buf: String::new() })
    }

    pub fn finish(mut self) -> Result<W> {
        if !self.fragment {
            self.out.write_all(b"</body>\n</html>\n").map_err(io)?;
        }
        self.out.flush().map_err(io)?;
        Ok(self.out)
    }
}

impl<W: std::io::Write> crate::fill::PageSink for HtmlStream<W> {
    fn wants_cells(&self) -> bool {
        false
    }

    fn page(&mut self, page: PrintPage, fonts: &FontSet) -> Result<()> {
        self.buf.clear();
        self.pages.page(&page.items, fonts, &mut self.buf)?;
        self.out.write_all(self.buf.as_bytes()).map_err(io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_text_are_written_cleanly() {
        assert_eq!((n(12.0), n(0.5), n(-0.001), n(2.71849)), ("12".into(), "0.5".into(), "0".into(), "2.72".into()));
        let mut out = String::new();
        escape("a<b & \"c\"\u{1}", &mut out);
        assert_eq!(out, "a&lt;b &amp; &quot;c&quot; ");
        assert_eq!(css_family("DejaVu Sans"), "'DejaVu Sans',sans-serif");
        assert_eq!(css_family("DejaVu Serif"), "'DejaVu Serif',serif");
        assert_eq!(css_family("DejaVu Sans Mono"), "'DejaVu Sans Mono',monospace");
        assert_eq!(css_family("x';alert(1)//"), "'xalert1',sans-serif");
    }
}
