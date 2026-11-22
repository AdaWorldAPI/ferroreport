//! Word export (DOCX), laid out like JasperReports' `JRDocxExporter`: each page is a table
//! whose columns and rows are cut at the edges of the page's elements, every element takes the
//! cells between its edges, and pages after the first start on a new page.
//!
//! Texts keep their fonts, colors, alignment, padding, borders and backgrounds; pictures,
//! charts and barcodes are embedded as pictures; rectangles and lines become cell backgrounds
//! and borders. Pages are written as they are filled.

use crate::fonts::FontSet;
use crate::grid::{self, Cell, CellStyle, Nature, Node, What};
use crate::image::{ImageData, Pixels};
use crate::model::{Color, HAlign, LineStyle, Pen, Rotation, VAlign};
use crate::print::{CellInfo, PrintItem, Shape};
use crate::xlsx::escape;
use crate::zip::ZipStream;
use std::collections::HashMap;
use std::fmt::Write as _;

const NATURE: Nature = Nature { span: true, texts_only: false };

/// Pixels per point of the pictures drawn for charts and barcodes.
const PICTURE_SCALE: f32 = 2.0;

fn io(e: std::io::Error) -> crate::Error {
    crate::Error::Io(e.to_string())
}

/// Writes a Word document page by page.
pub struct DocxStream<W: std::io::Write> {
    zip: ZipStream<W>,
    page_width: f32,
    page_height: f32,
    landscape: bool,
    title: String,
    pages: usize,
    /// Pictures: file name and bytes.
    media: Vec<(String, Vec<u8>)>,
    /// Pictures of images already written, by image identifier.
    names: HashMap<usize, String>,
    buf: String,
}

impl<W: std::io::Write> DocxStream<W> {
    pub fn new(out: W, title: &str, page_width: f32, page_height: f32, landscape: bool) -> crate::Result<DocxStream<W>> {
        let mut zip = ZipStream::new(out);
        zip.begin("word/document.xml").map_err(io)?;
        zip.write(DOCUMENT_HEAD.as_bytes()).map_err(io)?;
        Ok(DocxStream {
            zip,
            page_width,
            page_height,
            landscape,
            title: title.to_string(),
            pages: 0,
            media: Vec::new(),
            names: HashMap::new(),
            buf: String::new(),
        })
    }

    /// Ends the document and writes the rest of the package.
    pub fn finish(mut self) -> crate::Result<W> {
        let (w, h) = (twips(self.page_width), twips(self.page_height));
        let orient = if self.landscape { "landscape" } else { "portrait" };
        let tail = format!(
            "  <w:sectPr>\n   <w:pgSz w:w=\"{w}\" w:h=\"{h}\" w:orient=\"{orient}\"/>\n   <w:pgMar w:top=\"0\" w:right=\"0\" w:bottom=\"0\" w:left=\"0\" w:header=\"0\" w:footer=\"0\" w:gutter=\"0\"/>\n   <w:docGrid w:linePitch=\"360\"/>\n  </w:sectPr>\n </w:body>\n</w:document>\n"
        );
        // The paragraph a document ends with, as small as it can be.
        self.zip
            .write(b"  <w:p><w:pPr><w:pStyle w:val=\"EMPTY_CELL_STYLE\"/><w:spacing w:line=\"20\" w:lineRule=\"exact\"/></w:pPr></w:p>\n")
            .map_err(io)?;
        self.zip.write(tail.as_bytes()).map_err(io)?;
        self.zip.end().map_err(io)?;

        let mut rels = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n <Relationship Id=\"rIdStyles\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\n <Relationship Id=\"rIdSettings\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings\" Target=\"settings.xml\"/>\n",
        );
        for (name, _) in &self.media {
            let _ = writeln!(
                rels,
                " <Relationship Id=\"{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"media/{name}\"/>",
                rel_id(name)
            );
        }
        rels.push_str("</Relationships>\n");
        self.zip.add("word/_rels/document.xml.rels", rels.as_bytes()).map_err(io)?;
        for (name, bytes) in std::mem::take(&mut self.media) {
            self.zip.add(&format!("word/media/{name}"), &bytes).map_err(io)?;
        }
        self.zip.add("word/styles.xml", STYLES.as_bytes()).map_err(io)?;
        self.zip.add("word/settings.xml", SETTINGS.as_bytes()).map_err(io)?;
        self.zip.add("[Content_Types].xml", CONTENT_TYPES.as_bytes()).map_err(io)?;
        self.zip.add("_rels/.rels", ROOT_RELS.as_bytes()).map_err(io)?;
        let core = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>{}</dc:title><dc:creator>ferroreport</dc:creator></cp:coreProperties>\n",
            escape(&self.title)
        );
        self.zip.add("docProps/core.xml", core.as_bytes()).map_err(io)?;
        self.zip.add("docProps/app.xml", APP.as_bytes()).map_err(io)?;
        self.zip.finish().map_err(io)
    }

    fn page_xml(&mut self, items: &[PrintItem], fonts: &FontSet) {
        let nodes = grid::nodes(items);
        let width = self.page_width.round() as i32;
        let height = self.page_height.round() as i32;
        // Each page has its own columns.
        let mut xcuts = grid::x_cuts(std::slice::from_ref(&nodes), width, NATURE);
        let g = grid::layout(&nodes, &mut xcuts, height, NATURE);
        let mut out = std::mem::take(&mut self.buf);
        out.clear();
        // Pages after the first start on a new page: a paragraph of almost no height breaks
        // it (and keeps the tables of two pages from being taken as one).
        if self.pages > 0 {
            out.push_str("  <w:p><w:pPr><w:pStyle w:val=\"EMPTY_CELL_STYLE\"/><w:spacing w:line=\"20\" w:lineRule=\"exact\"/></w:pPr><w:r><w:br w:type=\"page\"/></w:r></w:p>\n");
        }
        let _ =
            write!(out, "  <w:tbl>\n   <w:tblPr><w:tblW w:w=\"{}\" w:type=\"dxa\"/><w:tblLayout w:type=\"fixed\"/>", width * 20);
        out.push_str("<w:tblCellMar><w:left w:w=\"0\" w:type=\"dxa\"/><w:right w:w=\"0\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr>\n   <w:tblGrid>");
        for c in 0..xcuts.spans() {
            let _ = write!(out, "<w:gridCol w:w=\"{}\"/>", xcuts.size(c) * 20);
        }
        out.push_str("</w:tblGrid>\n");
        // Rows below the last thing on the page are left out: pages are separated by page
        // breaks, and a table as tall as the page would push what follows onto another.
        let last = g
            .cells
            .iter()
            .rposition(|row| row.iter().any(|c| !matches!(c, Cell::Empty(s) if *s == CellStyle::default())))
            .unwrap_or(0);
        for (ri, cells) in g.cells.iter().enumerate().take(last + 1) {
            let _ = writeln!(out, "   <w:tr><w:trPr><w:trHeight w:hRule=\"exact\" w:val=\"{}\"/></w:trPr>", g.rows.size(ri) * 20);
            let mut ci = 0;
            while ci < cells.len() {
                let span = match &cells[ci] {
                    Cell::Empty(style) => {
                        out.push_str("    <w:tc><w:tcPr>");
                        cell_style(&mut out, style, None);
                        out.push_str("</w:tcPr>");
                        empty_paragraph(&mut out);
                        out.push_str("</w:tc>\n");
                        1
                    }
                    Cell::Element { node, rows, cols, style } => {
                        out.push_str("    <w:tc><w:tcPr>");
                        if *cols > 1 {
                            let _ = write!(out, "<w:gridSpan w:val=\"{cols}\"/>");
                        }
                        if *rows > 1 {
                            out.push_str("<w:vMerge w:val=\"restart\"/>");
                        }
                        let w = (xcuts.offsets[ci + cols] - xcuts.offsets[ci]) as f32;
                        let h = (g.rows.offsets[ri + rows] - g.rows.offsets[ri]) as f32;
                        self.element(&mut out, node, style, w, h, fonts);
                        out.push_str("</w:tc>\n");
                        *cols
                    }
                    Cell::Occupied(r0, c0) if *c0 == ci && *r0 < ri => {
                        // Below an element's first row: the rest of a vertical merge.
                        let (cols, style) = match &g.cells[*r0][*c0] {
                            Cell::Element { cols, style, .. } => (*cols, style.clone()),
                            _ => (1, CellStyle::default()),
                        };
                        out.push_str("    <w:tc><w:tcPr>");
                        if cols > 1 {
                            let _ = write!(out, "<w:gridSpan w:val=\"{cols}\"/>");
                        }
                        out.push_str("<w:vMerge/>");
                        cell_style(&mut out, &style, None);
                        out.push_str("</w:tcPr><w:p><w:pPr><w:pStyle w:val=\"EMPTY_CELL_STYLE\"/></w:pPr></w:p></w:tc>\n");
                        cols
                    }
                    // Covered by an element starting in a column to the left: spanned.
                    Cell::Occupied(..) => 1,
                };
                ci += span.max(1);
            }
            out.push_str("   </w:tr>\n");
        }
        out.push_str("  </w:tbl>\n");
        self.buf = out;
    }

    /// The properties and content of an element's cell (its `<w:tcPr>` is open).
    fn element(&mut self, out: &mut String, node: &Node<'_>, style: &CellStyle, w: f32, h: f32, fonts: &FontSet) {
        match &node.what {
            What::Text(t) => {
                cell_style(out, style, Some(t));
                out.push_str("</w:tcPr>");
                text_paragraphs(out, t);
            }
            What::Shape(e, _, drawn) => {
                let mut style = style.clone();
                match &e.shape {
                    // A rectangle: its background, its pen as the borders.
                    Shape::Rectangle(pen) => {
                        if let Some(p) = pen.filter(|p| p.width > 0.0) {
                            style.boxed = Some([p; 4]);
                        }
                    }
                    // A line: a border on the side it runs along.
                    Shape::Line { pen, .. } => {
                        let none = Pen { width: 0.0, ..*pen };
                        style.boxed = Some(if node.w >= node.h { [*pen, none, none, none] } else { [none, *pen, none, none] });
                        style.back = None;
                    }
                    _ => {}
                }
                cell_style(out, &style, None);
                // A picture sits in its cell where the image is aligned.
                let (jc, v) = match &e.shape {
                    Shape::Image { halign, valign, .. } => (*halign, *valign),
                    _ => (HAlign::Left, VAlign::Top),
                };
                let v = match v {
                    VAlign::Middle => "center",
                    VAlign::Bottom => "bottom",
                    VAlign::Top => "top",
                };
                let _ = write!(out, "<w:vAlign w:val=\"{v}\"/>");
                out.push_str("</w:tcPr>");
                let jc = match jc {
                    HAlign::Center => "<w:jc w:val=\"center\"/>",
                    HAlign::Right => "<w:jc w:val=\"right\"/>",
                    _ => "",
                };
                let picture = match &e.shape {
                    Shape::Image { data: Some(data), .. } => self.media_name(data).map(|n| (n, fit(data, &e.shape, w, h))),
                    Shape::Picture { .. } if w > 0.0 && h > 0.0 => {
                        let area = [e.x, e.y, e.w, e.h];
                        crate::raster::render_png(drawn, fonts, area, PICTURE_SCALE).map(|png| {
                            let name = format!("img_{}.png", self.media.len());
                            self.media.push((name.clone(), png));
                            (name, (e.w, e.h))
                        })
                    }
                    _ => None,
                };
                match picture {
                    Some((name, (pw, ph))) => {
                        let _ = write!(out, "<w:p><w:pPr><w:pStyle w:val=\"Base\"/>{jc}</w:pPr><w:r>");
                        drawing(out, &name, self.media.len() + 1, pw, ph);
                        out.push_str("</w:r></w:p>");
                    }
                    None => empty_paragraph(out),
                }
            }
        }
    }

    /// The media file of an image, written once however often it is used.
    fn media_name(&mut self, data: &ImageData) -> Option<String> {
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
        let name = format!("img_{}.{ext}", self.media.len());
        self.media.push((name.clone(), bytes));
        self.names.insert(data.id, name.clone());
        Some(name)
    }
}

impl<W: std::io::Write> crate::fill::PageSink for DocxStream<W> {
    fn page(&mut self, page: crate::print::PrintPage, fonts: &FontSet) -> crate::Result<()> {
        self.page_xml(&page.items, fonts);
        self.pages += 1;
        let buf = std::mem::take(&mut self.buf);
        let r = self.zip.write(buf.as_bytes()).map_err(io);
        self.buf = buf;
        r
    }
}

/// Exports a filled document to a Word document.
pub fn write_docx(doc: &crate::print::PrintDocument, fonts: &FontSet) -> crate::Result<Vec<u8>> {
    use crate::fill::PageSink;
    let mut sink = DocxStream::new(Vec::new(), &doc.name, doc.page_width, doc.page_height, doc.landscape)?;
    for page in &doc.pages {
        sink.page(page.clone(), fonts)?;
    }
    sink.finish()
}

fn twips(pt: f32) -> i32 {
    (pt * 20.0).round() as i32
}

fn hex(c: Color) -> String {
    format!("{:02X}{:02X}{:02X}", c.r, c.g, c.b)
}

fn rel_id(name: &str) -> String {
    format!("rId_{}", name.replace('.', "_"))
}

fn border(out: &mut String, side: &str, pen: &Pen) {
    if pen.width <= 0.0 {
        let _ = write!(out, "<w:{side} w:val=\"nil\"/>");
        return;
    }
    let val = match pen.style {
        LineStyle::Solid => "single",
        LineStyle::Dashed => "dashed",
        LineStyle::Dotted => "dotted",
        LineStyle::Double => "double",
    };
    // Eighths of a point, as Word wants them (at least a quarter point).
    let size = ((pen.width * 8.0).round() as i32).clamp(2, 96);
    let _ = write!(out, "<w:{side} w:val=\"{val}\" w:sz=\"{size}\" w:space=\"0\" w:color=\"{}\"/>", hex(pen.color));
}

/// Background, borders, and for a text its padding, vertical alignment and direction.
fn cell_style(out: &mut String, style: &CellStyle, text: Option<&CellInfo>) {
    if let Some(back) = text.and_then(|t| t.back).or(style.back) {
        let _ = write!(out, "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>", hex(back));
    }
    if let Some(b) = &style.boxed {
        if b.iter().any(|p| p.width > 0.0) {
            out.push_str("<w:tcBorders>");
            border(out, "top", &b[0]);
            border(out, "left", &b[1]);
            border(out, "bottom", &b[2]);
            border(out, "right", &b[3]);
            out.push_str("</w:tcBorders>");
        }
    }
    let Some(t) = text else { return };
    let p = t.boxed.padding;
    let _ = write!(
        out,
        "<w:tcMar><w:top w:w=\"{}\" w:type=\"dxa\"/><w:left w:w=\"{}\" w:type=\"dxa\"/><w:bottom w:w=\"{}\" w:type=\"dxa\"/><w:right w:w=\"{}\" w:type=\"dxa\"/></w:tcMar>",
        twips(p.top),
        twips(p.left),
        twips(p.bottom),
        twips(p.right)
    );
    match t.rotation {
        Rotation::Left => out.push_str("<w:textDirection w:val=\"btLr\"/>"),
        Rotation::Right => out.push_str("<w:textDirection w:val=\"tbRl\"/>"),
        _ => {}
    }
    let v = match t.valign {
        VAlign::Middle => "center",
        VAlign::Bottom => "bottom",
        VAlign::Top => "top",
    };
    let _ = write!(out, "<w:vAlign w:val=\"{v}\"/>");
}

fn empty_paragraph(out: &mut String) {
    out.push_str("<w:p><w:pPr><w:pStyle w:val=\"EMPTY_CELL_STYLE\"/></w:pPr></w:p>");
}

fn run_props(out: &mut String, font: &crate::model::FontSpec, color: Color, back: Option<Color>, script: crate::rich::Script) {
    let name = escape(&font.name);
    let _ = write!(out, "<w:rPr><w:rFonts w:ascii=\"{name}\" w:hAnsi=\"{name}\" w:eastAsia=\"{name}\" w:cs=\"{name}\"/>");
    if font.bold {
        out.push_str("<w:b/>");
    }
    if font.italic {
        out.push_str("<w:i/>");
    }
    if font.strike {
        out.push_str("<w:strike/>");
    }
    let _ = write!(out, "<w:color w:val=\"{}\"/>", hex(color));
    let _ = write!(out, "<w:sz w:val=\"{}\"/>", (font.size * 2.0).round().max(1.0) as i32);
    if let Some(b) = back {
        let _ = write!(out, "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>", hex(b));
    }
    if font.underline {
        out.push_str("<w:u w:val=\"single\"/>");
    }
    match script {
        crate::rich::Script::Super => out.push_str("<w:vertAlign w:val=\"superscript\"/>"),
        crate::rich::Script::Sub => out.push_str("<w:vertAlign w:val=\"subscript\"/>"),
        crate::rich::Script::Normal => {}
    }
    out.push_str("</w:rPr>");
}

/// A piece of text as runs, line breaks as `<w:br/>`.
fn text_run(out: &mut String, text: &str, props: &str) {
    for (i, line) in text.split('\n').enumerate() {
        out.push_str("<w:r>");
        out.push_str(props);
        if i > 0 {
            out.push_str("<w:br/>");
        }
        if !line.is_empty() {
            let _ = write!(out, "<w:t xml:space=\"preserve\">{}</w:t>", escape(line));
        }
        out.push_str("</w:r>");
    }
}

fn text_paragraphs(out: &mut String, t: &CellInfo) {
    let jc = match t.halign {
        HAlign::Center => Some("center"),
        HAlign::Right => Some("right"),
        HAlign::Justified => Some("both"),
        _ => None,
    };
    out.push_str("<w:p><w:pPr><w:pStyle w:val=\"Base\"/>");
    if let Some(jc) = jc {
        let _ = write!(out, "<w:jc w:val=\"{jc}\"/>");
    }
    out.push_str("</w:pPr>");
    if t.runs.is_empty() {
        let mut props = String::new();
        run_props(&mut props, &t.font, t.color, None, crate::rich::Script::Normal);
        text_run(out, &t.text, &props);
    } else {
        for run in &t.runs {
            let Some(piece) = t.text.get(run.start..run.end) else { continue };
            let mut props = String::new();
            run_props(&mut props, &run.style.font, run.style.color, run.style.back, run.style.script);
            text_run(out, piece, &props);
        }
    }
    out.push_str("</w:p>");
}

/// The size an image is shown at in its cell (points).
fn fit(data: &ImageData, shape: &Shape, w: f32, h: f32) -> (f32, f32) {
    let (iw, ih) = match &data.pixels {
        Pixels::Svg(svg) => (svg.tree.size().width(), svg.tree.size().height()),
        _ => (data.width as f32, data.height as f32),
    };
    match shape {
        Shape::Image { scale: crate::model::ScaleImage::FillFrame, .. } => (w, h),
        Shape::Image { scale: crate::model::ScaleImage::Clip, .. } => (iw, ih),
        _ => {
            let k = if iw > 0.0 && ih > 0.0 { (w / iw).min(h / ih) } else { 1.0 };
            (iw * k, ih * k)
        }
    }
}

fn drawing(out: &mut String, name: &str, id: usize, w: f32, h: f32) {
    let (cx, cy) = (((w * 12700.0).round() as i64).max(1), ((h * 12700.0).round() as i64).max(1));
    let rid = rel_id(name);
    let _ = write!(
        out,
        "<w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:docPr id=\"{id}\" name=\"Picture {id}\"/><a:graphic xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:pic xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:nvPicPr><pic:cNvPr id=\"{id}\" name=\"{name}\"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"{rid}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing>"
    );
}

const DOCUMENT_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">\n <w:body>\n";

const STYLES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\n <w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"DejaVu Sans\" w:hAnsi=\"DejaVu Sans\" w:eastAsia=\"DejaVu Sans\" w:cs=\"DejaVu Sans\"/><w:sz w:val=\"20\"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:before=\"0\" w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>\n <w:style w:type=\"paragraph\" w:styleId=\"Base\" w:default=\"1\"><w:name w:val=\"Base\"/><w:qFormat/></w:style>\n <w:style w:type=\"paragraph\" w:styleId=\"EMPTY_CELL_STYLE\"><w:name w:val=\"EMPTY_CELL_STYLE\"/><w:basedOn w:val=\"Base\"/><w:rPr><w:sz w:val=\"1\"/></w:rPr></w:style>\n</w:styles>\n";

const SETTINGS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:settings xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:zoom w:percent=\"100\"/><w:defaultTabStop w:val=\"720\"/><w:compat/></w:settings>\n";

const CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"png\" ContentType=\"image/png\"/><Default Extension=\"jpg\" ContentType=\"image/jpeg\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/><Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/><Override PartName=\"/word/settings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml\"/><Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/><Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/></Types>\n";

const ROOT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/><Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/></Relationships>\n";

const APP: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>ferroreport</Application></Properties>\n";

#[cfg(test)]
mod tests {
    /// One table per page, a page break before each page after the first, the texts in
    /// their cells, late values ("page 2 of 5") included.
    #[test]
    fn writes_a_table_per_page() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/33-page-x-of-y");
        let template = std::fs::read_to_string(dir.join("report.jrxml")).unwrap();
        let rows: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("data.json")).unwrap()).unwrap();
        let mut renderer = crate::Renderer::new();
        let report = renderer.parse(&template).unwrap();
        let docx = renderer.render_report_json(&report, &rows, &serde_json::json!({}), crate::Format::Docx).unwrap();
        let records = crate::data::records_from_json(&report, &rows).unwrap();
        let stats = renderer
            .render_to(&report, Box::new(records.into_iter().map(Ok)), Default::default(), crate::Format::Pdf, Vec::new())
            .unwrap();
        let pages = stats.pages;
        let files = crate::zip::tests::read(&docx);
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        for part in ["word/document.xml", "word/styles.xml", "[Content_Types].xml", "_rels/.rels", "word/_rels/document.xml.rels"]
        {
            assert!(names.contains(&part), "{part}");
        }
        let xml = String::from_utf8(files.iter().find(|(n, _)| n == "word/document.xml").unwrap().1.clone()).unwrap();
        let tables = xml.matches("<w:tbl>").count();
        assert!(tables > 1);
        assert_eq!(xml.matches("w:type=\"page\"").count(), tables - 1);
        assert_eq!(tables, pages);
        // The total of "Page 1 of 5", known only at the end, is in the text.
        assert!(xml.contains(&format!("> {pages}</w:t>")), "late page totals are in the text");
    }
}
