//! What the OpenDocument exporters ([`crate::ods`], [`crate::odt`]) share: the package (a ZIP
//! whose first entry is the uncompressed `mimetype`, a manifest, `content.xml`, `styles.xml`),
//! automatic styles, fonts, text with its spans, links, and pictures.
//!
//! JasperReports' `JROdsExporter` and `JROdtExporter` lay a page out on the same grid as its
//! XLSX and DOCX exporters; so do these, with the grid of [`crate::grid`].
//!
//! The automatic styles of `content.xml` come before its body, and are only all known once
//! the last page has been laid out. A document is therefore written in one of three ways
//! ([`Content`]): kept in memory and written whole at the end; measured, the body made and
//! thrown away so that only its styles are kept; or streamed, its styles found by measuring
//! it first, each page compressed and written as it comes.

use crate::fonts::FontSet;
use crate::image::{ImageData, Pixels};
use crate::model::{BoxStyle, Color, HAlign, LineStyle, Pen, ScaleImage, VAlign};
use crate::print::{CellInfo, LinkInfo, LinkTarget, PrintItem, Shape};
use crate::rich::Script;
use crate::xlsx::escape;
use crate::zip::ZipStream;
use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::sync::Arc;

pub(crate) fn io(e: std::io::Error) -> crate::Error {
    crate::Error::Io(e.to_string())
}

/// Pixels per point of the pictures drawn for charts and barcodes.
pub(crate) const PICTURE_SCALE: f32 = 2.0;

/// A length in points, as OpenDocument writes it (`12pt`, `0.5pt`).
pub(crate) fn pt(v: f32) -> String {
    let v = (v * 1000.0).round() / 1000.0;
    format!("{}pt", if v == 0.0 { 0.0 } else { v })
}

pub(crate) fn hex(c: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

/// Like Java's `Double.toString` for ordinary values, which spreadsheets read back exactly.
pub(crate) fn number(v: f64) -> String {
    format!("{v}")
}

/// The styles of a document: automatic styles by their properties, each written once under
/// a name made of a prefix and a number.
#[derive(Default)]
pub(crate) struct Styles {
    xml: String,
    names: HashMap<String, String>,
    counts: HashMap<&'static str, usize>,
    /// Font families used, for the font declarations.
    pub fonts: BTreeSet<String>,
    /// Set once the styles have been written ahead of the body: a style first asked for
    /// after that (the rows read the second time were not those read the first) can no
    /// longer go in `content.xml`, and goes to `styles.xml` as a common style instead.
    frozen: bool,
    late_xml: String,
    late_fonts: BTreeSet<String>,
}

impl Styles {
    /// The name of the `<style:style>` of `family` with these properties.
    pub fn style(&mut self, prefix: &'static str, family: &str, attributes: &str, properties: &str) -> String {
        let key = format!("{family}|{attributes}|{properties}");
        if let Some(n) = self.names.get(&key) {
            return n.clone();
        }
        let name = self.next(prefix);
        let xml = if self.frozen { &mut self.late_xml } else { &mut self.xml };
        let _ =
            writeln!(xml, "<style:style style:name=\"{name}\" style:family=\"{family}\"{attributes}>{properties}</style:style>");
        self.names.insert(key, name.clone());
        name
    }

    /// The name of a data style (`<number:…-style>`): `element` is its element name, `body`
    /// what it holds.
    pub fn data_style(&mut self, element: &str, body: &str) -> String {
        let key = format!("{element}|{body}");
        if let Some(n) = self.names.get(&key) {
            return n.clone();
        }
        let name = self.next("N");
        let xml = if self.frozen { &mut self.late_xml } else { &mut self.xml };
        let _ = writeln!(xml, "<number:{element} style:name=\"{name}\">{body}</number:{element}>");
        self.names.insert(key, name.clone());
        name
    }

    fn next(&mut self, prefix: &'static str) -> String {
        let n = self.counts.entry(prefix).or_insert(0);
        *n += 1;
        format!("{prefix}{n}")
    }

    /// Text properties of a font, its color and background (`<style:text-properties>`).
    pub fn text_properties(
        &mut self,
        font: &crate::model::FontSpec,
        color: Color,
        back: Option<Color>,
        script: Script,
    ) -> String {
        if self.fonts.insert(font.name.clone()) && self.frozen {
            self.late_fonts.insert(font.name.clone());
        }
        let size = pt(font.size);
        let mut p = format!(
            "<style:text-properties style:font-name=\"{}\" fo:font-size=\"{size}\" style:font-size-asian=\"{size}\" style:font-size-complex=\"{size}\" fo:color=\"{}\"",
            escape(&font.name),
            hex(color)
        );
        let (weight, style) = (if font.bold { "bold" } else { "normal" }, if font.italic { "italic" } else { "normal" });
        let _ = write!(
            p,
            " fo:font-weight=\"{weight}\" style:font-weight-asian=\"{weight}\" style:font-weight-complex=\"{weight}\" fo:font-style=\"{style}\" style:font-style-asian=\"{style}\" style:font-style-complex=\"{style}\""
        );
        if font.underline {
            p.push_str(" style:text-underline-style=\"solid\" style:text-underline-width=\"auto\" style:text-underline-color=\"font-color\"");
        }
        if font.strike {
            p.push_str(" style:text-line-through-style=\"solid\"");
        }
        if let Some(b) = back {
            let _ = write!(p, " fo:background-color=\"{}\"", hex(b));
        }
        match script {
            Script::Super => p.push_str(" style:text-position=\"super 58%\""),
            Script::Sub => p.push_str(" style:text-position=\"sub 58%\""),
            Script::Normal => {}
        }
        p.push_str("/>");
        p
    }

    /// The `<office:font-face-decls>` and `<office:automatic-styles>` of `content.xml`.
    pub fn write(&self, out: &mut String) {
        font_faces(out, &self.fonts);
        out.push_str("<office:automatic-styles>\n");
        out.push_str(&self.xml);
        out.push_str("</office:automatic-styles>\n");
    }

    /// From now on, new styles are late ones (see [`Styles::late`]).
    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    /// The fonts and styles first asked for after [`Styles::freeze`], for `styles.xml`
    /// (empty when the body asked for nothing new).
    pub fn late(&self) -> (String, &str) {
        let mut fonts = String::new();
        if !self.late_fonts.is_empty() {
            font_faces(&mut fonts, &self.late_fonts);
        }
        (fonts, &self.late_xml)
    }
}

fn font_faces(out: &mut String, fonts: &BTreeSet<String>) {
    out.push_str("<office:font-face-decls>\n");
    for f in fonts {
        let _ = writeln!(
            out,
            "<style:font-face style:name=\"{0}\" svg:font-family=\"&apos;{0}&apos;\"/>",
            escape(&f.replace('\'', ""))
        );
    }
    out.push_str("</office:font-face-decls>\n");
}

/// One side of a border (`fo:border-top="0.5pt solid #000000"`).
pub(crate) fn border(p: &Pen) -> String {
    if p.width <= 0.0 {
        return "none".into();
    }
    let style = match p.style {
        LineStyle::Solid => "solid",
        LineStyle::Dashed => "dashed",
        LineStyle::Dotted => "dotted",
        LineStyle::Double => "double",
    };
    format!("{} {style} {}", pt(p.width), hex(p.color))
}

/// Border attributes for a cell's four sides (top, left, bottom, right).
pub(crate) fn borders(sides: Option<&[Pen; 4]>) -> String {
    let Some(b) = sides else { return " fo:border=\"none\"".into() };
    if b.iter().all(|p| p.width <= 0.0) {
        return " fo:border=\"none\"".into();
    }
    let mut s = String::new();
    for (name, pen) in ["top", "left", "bottom", "right"].iter().zip(b) {
        let _ = write!(s, " fo:border-{name}=\"{}\"", border(pen));
        if pen.width > 0.0 && pen.style == LineStyle::Double {
            // The two lines and the gap between them.
            let w = pt((pen.width / 3.0).max(0.25));
            let _ = write!(s, " style:border-line-width-{name}=\"{w} {w} {w}\"");
        }
    }
    s
}

pub(crate) fn padding(b: &BoxStyle) -> String {
    format!(
        " fo:padding-top=\"{}\" fo:padding-left=\"{}\" fo:padding-bottom=\"{}\" fo:padding-right=\"{}\"",
        pt(b.padding.top),
        pt(b.padding.left),
        pt(b.padding.bottom),
        pt(b.padding.right)
    )
}

pub(crate) fn text_align(h: HAlign) -> &'static str {
    match h {
        HAlign::Left => "start",
        HAlign::Center => "center",
        HAlign::Right => "end",
        HAlign::Justified => "justify",
    }
}

pub(crate) fn vertical_align(v: VAlign) -> &'static str {
    match v {
        VAlign::Top => "top",
        VAlign::Middle => "middle",
        VAlign::Bottom => "bottom",
    }
}

/// Text as OpenDocument keeps it: spaces after the first of a run, and leading ones, as
/// `<text:s/>` (a reader would collapse them), tabs as `<text:tab/>`.
pub(crate) fn text(out: &mut String, s: &str, at_start: bool) {
    let mut spaces = 0usize;
    let mut first = at_start;
    let mut word = String::new();
    let flush = |out: &mut String, word: &mut String, spaces: &mut usize, first: &mut bool| {
        if *spaces > 0 {
            let mut n = *spaces;
            if !*first {
                out.push(' ');
                n -= 1;
            }
            match n {
                0 => {}
                1 => out.push_str("<text:s/>"),
                n => {
                    let _ = write!(out, "<text:s text:c=\"{n}\"/>");
                }
            }
            *spaces = 0;
            *first = false;
        }
        if !word.is_empty() {
            out.push_str(&escape(word));
            word.clear();
            *first = false;
        }
    };
    for c in s.chars() {
        match c {
            ' ' => {
                if !word.is_empty() {
                    flush(out, &mut word, &mut spaces, &mut first);
                }
                spaces += 1;
            }
            '\t' => {
                flush(out, &mut word, &mut spaces, &mut first);
                out.push_str("<text:tab/>");
                first = false;
            }
            '\r' => {}
            c => {
                if spaces > 0 {
                    flush(out, &mut word, &mut spaces, &mut first);
                }
                word.push(c);
            }
        }
    }
    flush(out, &mut word, &mut spaces, &mut first);
}

/// Where a link leads, as an `xlink:href`.
pub(crate) fn href(target: &LinkTarget) -> String {
    match target {
        LinkTarget::Url(url) => url.clone(),
        LinkTarget::Anchor(a) => format!("#{}", crate::xlsx::excel_name(a)),
        LinkTarget::Page(p) => format!("#JR_PAGE_ANCHOR_0_{p}"),
        LinkTarget::RemoteAnchor { file, anchor } => format!("{file}#{anchor}"),
        LinkTarget::RemotePage { file, page } => format!("{file}#JR_PAGE_ANCHOR_0_{page}"),
    }
}

/// The links and anchors of a page, by where their element starts.
pub(crate) fn page_links(items: &[PrintItem]) -> HashMap<(i32, i32), Arc<LinkInfo>> {
    let mut found = Vec::new();
    crate::xlsx::links(items, &mut found);
    found.into_iter().map(|l| ((l.x.round() as i32, l.y.round() as i32), l)).collect()
}

/// The paragraphs of a text. `lines_as_paragraphs`: each line its own paragraph (spreadsheet
/// cells), otherwise lines are broken inside one paragraph. Runs are `<text:span>`s with
/// their text style; a plain text has none when `spans` is false (its cell carries the font).
#[allow(clippy::too_many_arguments)]
pub(crate) fn paragraphs(
    styles: &mut Styles,
    out: &mut String,
    t: &CellInfo,
    paragraph_style: Option<&str>,
    spans: bool,
    lines_as_paragraphs: bool,
    bookmark: Option<&str>,
    link: Option<&LinkInfo>,
) {
    // Pieces of text with their text style (None: the paragraph's own).
    let mut pieces: Vec<(&str, Option<String>)> = Vec::new();
    if t.runs.is_empty() {
        let style = spans.then(|| {
            let props = styles.text_properties(&t.font, t.color, None, Script::Normal);
            styles.style("T", "text", "", &props)
        });
        pieces.push((t.text.as_str(), style));
    } else {
        for run in &t.runs {
            let Some(piece) = t.text.get(run.start..run.end) else { continue };
            let st = &run.style;
            let props = styles.text_properties(&st.font, st.color, st.back, st.script);
            pieces.push((piece, Some(styles.style("T", "text", "", &props))));
        }
    }
    let open = match paragraph_style {
        Some(p) => format!("<text:p text:style-name=\"{p}\">"),
        None => "<text:p>".to_string(),
    };
    let link_open = link.and_then(|l| l.target.as_ref().map(|target| (target, l.tooltip.as_deref()))).map(|(target, tip)| {
        let mut a = format!("<text:a xlink:type=\"simple\" xlink:href=\"{}\"", escape(&href(target)));
        if let Some(tip) = tip {
            let _ = write!(a, " office:title=\"{}\"", escape(tip));
        }
        a.push('>');
        a
    });
    let start = |out: &mut String, first: bool| {
        out.push_str(&open);
        if first {
            if let Some(b) = bookmark {
                let _ = write!(out, "<text:bookmark text:name=\"{}\"/>", escape(b));
            }
        }
        if let Some(a) = &link_open {
            out.push_str(a);
        }
    };
    let end = |out: &mut String| {
        if link_open.is_some() {
            out.push_str("</text:a>");
        }
        out.push_str("</text:p>");
    };
    start(out, true);
    let mut line_start = true;
    for (piece, style) in pieces {
        for (i, line) in piece.split('\n').enumerate() {
            if i > 0 {
                if lines_as_paragraphs {
                    end(out);
                    start(out, false);
                } else {
                    out.push_str("<text:line-break/>");
                }
                line_start = true;
            }
            if line.is_empty() {
                continue;
            }
            match &style {
                Some(s) => {
                    let _ = write!(out, "<text:span text:style-name=\"{s}\">");
                    text(out, line, line_start);
                    out.push_str("</text:span>");
                }
                None => text(out, line, line_start),
            }
            line_start = false;
        }
    }
    end(out);
}

/// Where a picture goes in the area it is shown in, and what of it is cut away.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Placed {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// What is cut from the picture at its own size: top, right, bottom, left.
    pub clip: Option<[f32; 4]>,
}

/// Places a picture of natural size `nw` × `nh` in an area `aw` × `ah`.
pub(crate) fn place(nw: f32, nh: f32, aw: f32, ah: f32, scale: ScaleImage, halign: HAlign, valign: VAlign) -> Placed {
    let xa = match halign {
        HAlign::Center => 0.5,
        HAlign::Right => 1.0,
        _ => 0.0,
    };
    let ya = match valign {
        VAlign::Middle => 0.5,
        VAlign::Bottom => 1.0,
        VAlign::Top => 0.0,
    };
    match scale {
        _ if nw <= 0.0 || nh <= 0.0 => Placed { x: 0.0, y: 0.0, w: aw, h: ah, clip: None },
        ScaleImage::FillFrame => Placed { x: 0.0, y: 0.0, w: aw, h: ah, clip: None },
        ScaleImage::RetainShape => {
            let k = (aw / nw).min(ah / nh);
            let (w, h) = (nw * k, nh * k);
            Placed { x: xa * (aw - w), y: ya * (ah - h), w, h, clip: None }
        }
        ScaleImage::Clip => {
            let (w, h) = (nw.min(aw), nh.min(ah));
            let (cut_w, cut_h) = ((nw - aw).max(0.0), (nh - ah).max(0.0));
            let clip = (cut_w > 0.0 || cut_h > 0.0).then_some([ya * cut_h, (1.0 - xa) * cut_w, (1.0 - ya) * cut_h, xa * cut_w]);
            Placed { x: xa * (aw - w), y: ya * (ah - h), w, h, clip }
        }
    }
}

/// The pictures of a document, each image once, in `Pictures/`.
#[derive(Default)]
pub(crate) struct Pictures {
    pub files: Vec<(String, Vec<u8>)>,
    names: HashMap<usize, String>,
    /// Pictures made so far.
    count: usize,
    /// Only measuring: whether a picture can be made is found out, but it is not kept, and
    /// charts and barcodes are not drawn.
    measure: bool,
}

impl Pictures {
    pub fn measuring() -> Pictures {
        Pictures { measure: true, ..Pictures::default() }
    }

    fn keep(&mut self, ext: &str, bytes: Vec<u8>) -> String {
        let name = format!("Pictures/img_{}.{ext}", self.count);
        self.count += 1;
        if !self.measure {
            self.files.push((name.clone(), bytes));
        }
        name
    }

    /// The file of an image, written once however often it is used.
    pub fn image(&mut self, data: &ImageData) -> Option<String> {
        if let Some(n) = self.names.get(&data.id) {
            return Some(n.clone());
        }
        let (ext, bytes) = match &data.pixels {
            Pixels::Jpeg { data, .. } => ("jpg", data.clone()),
            Pixels::Raw { samples, gray, alpha } => {
                ("png", crate::xlsx::encode_png(data.width, data.height, samples, *gray, alpha.as_deref())?)
            }
            Pixels::Svg(svg) => {
                let (w, h) = crate::svg::picture_size(&svg.tree);
                ("png", crate::svg::png(&svg.tree, w, h)?)
            }
        };
        let name = self.keep(ext, bytes);
        self.names.insert(data.id, name.clone());
        Some(name)
    }

    /// A chart or a barcode, drawn into a picture of `area` (x, y, width, height).
    pub fn drawing(&mut self, drawn: &[PrintItem], fonts: &FontSet, area: [f32; 4]) -> Option<String> {
        if self.measure {
            return crate::raster::fits(area, PICTURE_SCALE).then(|| self.keep("png", Vec::new()));
        }
        let png = crate::raster::render_png(drawn, fonts, area, PICTURE_SCALE)?;
        Some(self.keep("png", png))
    }
}

/// The resolution a JPEG file gives (its JFIF density), or 96 dots per inch.
fn jpeg_dpi(bytes: &[u8]) -> f32 {
    match bytes.get(..18) {
        Some(h) if h[..4] == [0xFF, 0xD8, 0xFF, 0xE0] && &h[6..11] == b"JFIF\0" => {
            let x = u16::from_be_bytes([h[14], h[15]]) as f32;
            match h[13] {
                1 if x > 0.0 => x,
                2 if x > 0.0 => x * 2.54,
                _ => 96.0,
            }
        }
        _ => 96.0,
    }
}

/// The natural size of an image, in points.
pub(crate) fn natural_size(data: &ImageData) -> (f32, f32) {
    match &data.pixels {
        Pixels::Svg(svg) => (svg.tree.size().width(), svg.tree.size().height()),
        _ => (data.width as f32, data.height as f32),
    }
}

/// The picture of a non-text element and how it sits in the element: the file, and where
/// it goes relative to the element's top-left corner. `None` for other shapes.
pub(crate) fn element_picture(
    pictures: &mut Pictures,
    fonts: &FontSet,
    e: &crate::print::ElementInfo,
    drawn: &[PrintItem],
    w: f32,
    h: f32,
) -> Option<(String, Placed)> {
    match &e.shape {
        Shape::Image { data: Some(data), scale, halign, valign } => {
            // Inside the padding.
            let p = e.boxed.map(|b| b.padding).unwrap_or_default();
            let (aw, ah) = ((w - p.left - p.right).max(0.0), (h - p.top - p.bottom).max(0.0));
            let (nw, nh) = natural_size(data);
            let mut placed = place(nw, nh, aw, ah, *scale, *halign, *valign);
            placed.x += p.left;
            placed.y += p.top;
            if let Some(clip) = placed.clip {
                // Readers measure the cut (`fo:clip`) against the image's size at the resolution
                // its file gives, 96 dots per inch when it gives none.
                if let Pixels::Jpeg { data: bytes, .. } = &data.pixels {
                    let k = 72.0 / jpeg_dpi(bytes);
                    return Some((pictures.image(data)?, Placed { clip: Some(clip.map(|v| v * k)), ..placed }));
                }
                // A decoded image: what shows of it is drawn into a picture of its own.
                let area = [e.x + placed.x, e.y + placed.y, placed.w, placed.h];
                let name = pictures.drawing(drawn, fonts, area)?;
                return Some((name, Placed { clip: None, ..placed }));
            }
            Some((pictures.image(data)?, placed))
        }
        Shape::Picture { .. } if w > 0.0 && h > 0.0 => {
            let name = pictures.drawing(drawn, fonts, [e.x, e.y, e.w, e.h])?;
            Some((name, Placed { x: 0.0, y: 0.0, w: e.w.min(w), h: e.h.min(h), clip: None }))
        }
        _ => None,
    }
}

/// A graphic style for a picture: no border, no wrap, and what is cut from it.
pub(crate) fn picture_style(styles: &mut Styles, placed: &Placed, anchored_in_text: bool) -> String {
    let mut props = String::from("<style:graphic-properties fo:border=\"none\" fo:padding=\"0pt\" fo:margin-left=\"0pt\" fo:margin-right=\"0pt\" fo:margin-top=\"0pt\" fo:margin-bottom=\"0pt\" draw:stroke=\"none\" draw:fill=\"none\" style:wrap=\"none\" style:mirror=\"none\" draw:color-mode=\"standard\"");
    if anchored_in_text {
        props.push_str(" style:vertical-pos=\"top\" style:vertical-rel=\"baseline\"");
    }
    if let Some([t, r, b, l]) = placed.clip {
        let _ = write!(props, " fo:clip=\"rect({}, {}, {}, {})\"", pt(t), pt(r), pt(b), pt(l));
    }
    props.push_str("/>");
    styles.style("G", "graphic", "", &props)
}

/// A `<draw:frame>` holding a picture.
/// `anchor`: how it is anchored in text (`as-char`); `None` in a spreadsheet cell, where
/// it is placed from the cell's corner.
pub(crate) fn frame(out: &mut String, style: &str, n: usize, href: &str, placed: &Placed, anchor: Option<&str>) {
    let position = match anchor {
        Some(a) => format!(" text:anchor-type=\"{a}\""),
        None => format!(" svg:x=\"{}\" svg:y=\"{}\"", pt(placed.x), pt(placed.y)),
    };
    let _ = write!(
        out,
        "<draw:frame draw:style-name=\"{style}\" draw:name=\"Picture {n}\" draw:z-index=\"{n}\"{position} svg:width=\"{}\" svg:height=\"{}\"><draw:image xlink:href=\"{href}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/></draw:frame>",
        pt(placed.w.max(0.01)),
        pt(placed.h.max(0.01)),
    );
}

/// The namespaces of `content.xml` and `styles.xml`.
pub(crate) const NAMESPACES: &str = "xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\" xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:meta=\"urn:oasis:names:tc:opendocument:xmlns:meta:1.0\" xmlns:number=\"urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0\" xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\" office:version=\"1.3\"";

/// The page: its size, no margins (the report's margins are part of the grid, as in
/// JasperReports), portrait or landscape. Styles the body asked for after its styles were
/// written ([`Styles::late`]) are common styles here.
pub(crate) fn styles_xml(page_width: f32, page_height: f32, landscape: bool, default_paragraph: &str, late: &Styles) -> String {
    let orientation = if landscape { "landscape" } else { "portrait" };
    let (late_fonts, late_styles) = late.late();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-styles {NAMESPACES}>\n{late_fonts}<office:styles>\n<style:default-style style:family=\"paragraph\">{default_paragraph}</style:default-style>\n<style:style style:name=\"Graphics\" style:family=\"graphic\"/>\n{late_styles}</office:styles>\n<office:automatic-styles>\n<style:page-layout style:name=\"pm1\"><style:page-layout-properties fo:page-width=\"{}\" fo:page-height=\"{}\" style:print-orientation=\"{orientation}\" fo:margin-top=\"0pt\" fo:margin-bottom=\"0pt\" fo:margin-left=\"0pt\" fo:margin-right=\"0pt\"/></style:page-layout>\n</office:automatic-styles>\n<office:master-styles>\n<style:master-page style:name=\"Standard\" style:page-layout-name=\"pm1\"/>\n</office:master-styles>\n</office:document-styles>\n",
        pt(page_width),
        pt(page_height),
    )
}

/// The start of `content.xml`, before its styles.
fn content_head() -> String {
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-content {NAMESPACES}>\n")
}

/// How hard a streamed `content.xml` is compressed. The compressor for pieces makes larger
/// files than the one for whole files at the same level; at 6 it makes about the size a kept
/// `content.xml` has, still in a small part of the time the pages take to lay out.
const STREAMED_LEVEL: u8 = 6;

/// Where the body of `content.xml` goes as it is made.
pub(crate) enum Content<W: std::io::Write> {
    /// Nothing is written until the end, when the styles are known: the body is kept until
    /// then, or, when `body` is `None`, only measured (made and thrown away, for its styles).
    Held { out: W, body: Option<String> },
    /// The styles were known before the first page (a first pass measured them) and are
    /// written: each piece of the body is compressed and written as it comes.
    Streamed(Box<ZipStream<W>>),
}

impl<W: std::io::Write> Content<W> {
    /// A body kept in memory until the end.
    pub fn kept(out: W) -> Content<W> {
        Content::Held { out, body: Some(String::new()) }
    }

    /// A body only measured.
    pub fn measured(out: W) -> Content<W> {
        Content::Held { out, body: None }
    }

    /// Starts the package and `content.xml` with all its styles, then `body_start`. Styles
    /// asked for after this go to `styles.xml`.
    pub fn streamed(out: W, mimetype: &str, styles: &mut Styles, body_start: &str) -> crate::Result<Content<W>> {
        let mut zip = begin(out, mimetype)?;
        let mut head = content_head();
        styles.write(&mut head);
        head.push_str(body_start);
        styles.freeze();
        zip.begin_at("content.xml", STREAMED_LEVEL).map_err(io)?;
        zip.write(head.as_bytes()).map_err(io)?;
        Ok(Content::Streamed(Box::new(zip)))
    }

    /// Whether the body is only measured.
    pub fn measuring(&self) -> bool {
        matches!(self, Content::Held { body: None, .. })
    }

    /// From here on, only measure the body: what was kept of it is dropped.
    pub fn stop_keeping(&mut self) {
        if let Content::Held { body, .. } = self {
            *body = None;
        }
    }

    /// The next piece of the body; `piece` is left empty.
    pub fn push(&mut self, piece: &mut String) -> crate::Result<()> {
        match self {
            Content::Held { body: Some(body), .. } => body.push_str(piece),
            Content::Held { body: None, .. } => {}
            Content::Streamed(zip) => zip.write(piece.as_bytes()).map_err(io)?,
        }
        piece.clear();
        Ok(())
    }

    /// The output, given back by a body that was only measured.
    pub fn into_output(self) -> Option<W> {
        match self {
            Content::Held { out, body: None } => Some(out),
            _ => None,
        }
    }

    /// Ends `content.xml` with `body_end` and writes the rest of the package. A kept body is
    /// written here, between its styles (with `body_start`) and `body_end`.
    #[allow(clippy::too_many_arguments)]
    pub fn finish(
        self,
        mimetype: &str,
        styles: &Styles,
        body_start: &str,
        body_end: &str,
        styles_xml: &str,
        title: &str,
        pictures: &Pictures,
    ) -> crate::Result<W> {
        let mut zip = match self {
            Content::Held { out, body: Some(body) } => {
                let mut content = content_head();
                styles.write(&mut content);
                content.push_str(body_start);
                content.push_str(&body);
                drop(body);
                content.push_str(body_end);
                let mut zip = begin(out, mimetype)?;
                zip.add("content.xml", content.as_bytes()).map_err(io)?;
                zip
            }
            Content::Held { body: None, .. } => {
                return Err(crate::Error::Io("a document that was only measured cannot be finished".into()))
            }
            Content::Streamed(mut zip) => {
                zip.write(body_end.as_bytes()).map_err(io)?;
                zip.end().map_err(io)?;
                *zip
            }
        };
        zip.add("styles.xml", styles_xml.as_bytes()).map_err(io)?;
        for (name, bytes) in &pictures.files {
            zip.add(name, bytes).map_err(io)?;
        }
        let meta = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-meta xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:meta=\"urn:oasis:names:tc:opendocument:xmlns:meta:1.0\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" office:version=\"1.3\"><office:meta><meta:generator>ferroreport {}</meta:generator><dc:title>{}</dc:title></office:meta></office:document-meta>\n",
            env!("CARGO_PKG_VERSION"),
            escape(title)
        );
        zip.add("meta.xml", meta.as_bytes()).map_err(io)?;
        let mut manifest = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.3\">\n <manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.3\" manifest:media-type=\"{mimetype}\"/>\n <manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/>\n <manifest:file-entry manifest:full-path=\"styles.xml\" manifest:media-type=\"text/xml\"/>\n <manifest:file-entry manifest:full-path=\"meta.xml\" manifest:media-type=\"text/xml\"/>\n"
        );
        for (name, _) in &pictures.files {
            let media = if name.ends_with(".jpg") { "image/jpeg" } else { "image/png" };
            let _ = writeln!(manifest, " <manifest:file-entry manifest:full-path=\"{name}\" manifest:media-type=\"{media}\"/>");
        }
        manifest.push_str("</manifest:manifest>\n");
        zip.add("META-INF/manifest.xml", manifest.as_bytes()).map_err(io)?;
        zip.finish().map_err(io)
    }
}

/// Starts a package: the `mimetype` first and uncompressed, as readers look for it there.
fn begin<W: std::io::Write>(out: W, mimetype: &str) -> crate::Result<ZipStream<W>> {
    let mut zip = ZipStream::new(out);
    zip.add_stored("mimetype", mimetype.as_bytes()).map_err(io)?;
    Ok(zip)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> String {
        let mut out = String::new();
        text(&mut out, s, true);
        out
    }

    #[test]
    fn spaces_are_kept() {
        assert_eq!(t("a b"), "a b");
        assert_eq!(t(" 2"), "<text:s/>2");
        assert_eq!(t("a   b"), "a <text:s text:c=\"2\"/>b");
        assert_eq!(t("a\tb <"), "a<text:tab/>b &lt;");
        assert_eq!(t("x  "), "x <text:s/>");
    }

    #[test]
    fn pictures_are_placed() {
        let fit = place(100.0, 50.0, 50.0, 50.0, ScaleImage::RetainShape, HAlign::Left, VAlign::Middle);
        assert_eq!(fit, Placed { x: 0.0, y: 12.5, w: 50.0, h: 25.0, clip: None });
        let clip = place(100.0, 20.0, 50.0, 40.0, ScaleImage::Clip, HAlign::Center, VAlign::Bottom);
        assert_eq!(clip, Placed { x: 0.0, y: 20.0, w: 50.0, h: 20.0, clip: Some([0.0, 25.0, 0.0, 25.0]) });
    }

    /// Measured first and then written as it is made, every fixture gives the files it
    /// gives when it is kept in memory and written at the end.
    #[test]
    fn every_fixture_streams_to_the_same_files() {
        use crate::fill::PageSink;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
        let mut checked = 0;
        for entry in std::fs::read_dir(&root).unwrap() {
            let dir = entry.unwrap().path();
            if !dir.join("expected.json").exists() || dir.join("data.sql").exists() {
                continue;
            }
            let read = |f: &str| std::fs::read_to_string(dir.join(f));
            let mut r =
                crate::Renderer::new().resource_dir(&dir).now(crate::data::parse_datetime("2026-01-15T10:30:00").unwrap());
            let report = r.parse(&read("report.jrxml").unwrap()).unwrap();
            let rows = read("data.json").map(|t| serde_json::from_str(&t).unwrap()).unwrap_or(serde_json::json!([]));
            let params = read("params.json").map(|t| serde_json::from_str(&t).unwrap()).unwrap_or(serde_json::json!({}));
            let records = crate::data::records_from_json(&report, &rows).unwrap();
            let params = crate::params_from_json(&report, &params).unwrap();
            let doc = r.fill(&report, &records, params).unwrap();
            let name = dir.file_name().unwrap().to_string_lossy().to_string();
            let page = (doc.page_width, doc.page_height, doc.landscape);
            let files = |bytes: &[u8]| crate::zip::tests::read(bytes);

            for numbers in [false, true] {
                let options = crate::xlsx::XlsxOptions { detect_cell_type: numbers };
                let kept = crate::ods::write_ods(&doc, &r.fonts, &options).unwrap();
                let mut columns = crate::xlsx::XlsxColumns::new();
                for p in &doc.pages {
                    columns.add(&p.items);
                }
                let mut measure =
                    crate::ods::OdsStream::measuring(Vec::new(), columns.clone(), &doc.name, page, options.clone()).unwrap();
                for p in &doc.pages {
                    measure.add(&p.items, &r.fonts).unwrap();
                }
                let (styles, out) = measure.into_styles().unwrap();
                let mut sheet = crate::ods::OdsStream::streamed(out, columns, &doc.name, page, options, styles).unwrap();
                for p in &doc.pages {
                    sheet.add(&p.items, &r.fonts).unwrap();
                }
                assert!(files(&sheet.finish().unwrap()) == files(&kept), "{name}: ODS (numbers: {numbers})");
            }

            let kept = crate::odt::write_odt(&doc, &r.fonts).unwrap();
            let mut measure = crate::odt::OdtStream::new(Vec::new(), &doc.name, page.0, page.1, page.2).unwrap();
            measure.stop_keeping();
            for p in &doc.pages {
                measure.page(p.clone(), &r.fonts).unwrap();
            }
            let (styles, out) = measure.into_styles().unwrap();
            let mut text = crate::odt::OdtStream::streamed(out, &doc.name, page, styles).unwrap();
            for p in &doc.pages {
                text.page(p.clone(), &r.fonts).unwrap();
            }
            assert!(files(&text.finish().unwrap()) == files(&kept), "{name}: ODT");
            checked += 1;
        }
        assert!(checked >= 30, "only {checked} fixtures checked");
    }

    #[test]
    fn lengths() {
        assert_eq!(pt(12.0), "12pt");
        assert_eq!(pt(0.5), "0.5pt");
        assert_eq!(pt(1.0 / 3.0), "0.333pt");
    }
}
