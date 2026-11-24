//! OpenDocument text (ODT) export, laid out like JasperReports' `JROdtExporter` and like this
//! crate's DOCX export: each page is a table whose columns and rows are cut at the edges of
//! the page's elements, every element takes the cells between its edges, and pages after the
//! first start on a new page.
//!
//! Texts keep their fonts, colors, alignment, padding, borders and backgrounds; pictures,
//! charts and barcodes are embedded as pictures; rectangles and lines become cell backgrounds
//! and borders. The document is kept in memory until the end, where its styles are written
//! before it.

use crate::fonts::FontSet;
use crate::grid::{self, Cell, CellStyle, Node, What};
use crate::model::{HAlign, Pen, Rotation, VAlign};
use crate::odf::{self, pt, Pictures, Styles};
use crate::print::{CellInfo, PrintItem, Shape};
use crate::xlsx::NATURE;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

pub(crate) const MIMETYPE: &str = "application/vnd.oasis.opendocument.text";

/// Writes a text document page by page.
pub struct OdtStream<W: std::io::Write> {
    zip: crate::zip::ZipStream<W>,
    page_width: f32,
    page_height: f32,
    landscape: bool,
    title: String,
    pages: usize,
    styles: Styles,
    pictures: Pictures,
    body: String,
    /// The links and anchors of the page being written, by where their element starts.
    links: HashMap<(i32, i32), Arc<crate::print::LinkInfo>>,
    frames: usize,
}

impl<W: std::io::Write> OdtStream<W> {
    pub fn new(out: W, title: &str, page_width: f32, page_height: f32, landscape: bool) -> crate::Result<OdtStream<W>> {
        Ok(OdtStream {
            zip: odf::begin(out, MIMETYPE)?,
            page_width,
            page_height,
            landscape,
            title: title.to_string(),
            pages: 0,
            styles: Styles::default(),
            pictures: Pictures::default(),
            body: String::new(),
            links: HashMap::new(),
            frames: 0,
        })
    }

    /// Ends the document and writes the rest of the package.
    pub fn finish(mut self) -> crate::Result<W> {
        let mut content = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-content {}>\n", odf::NAMESPACES);
        // The paragraph a document ends with (one follows the last table), as small as it can be.
        let end = self.styles.style("P", "paragraph", "", END_PARAGRAPH);
        self.styles.write(&mut content);
        content.push_str("<office:body>\n<office:text>\n");
        content.push_str(&self.body);
        let _ = writeln!(content, "<text:p text:style-name=\"{end}\"/>");
        content.push_str("</office:text>\n</office:body>\n</office:document-content>\n");
        let styles = odf::styles_xml(self.page_width, self.page_height, self.landscape, DEFAULT_PARAGRAPH);
        odf::finish(self.zip, MIMETYPE, &content, &styles, &self.title, &self.pictures)
    }

    fn page_xml(&mut self, items: &[PrintItem], fonts: &FontSet) {
        let nodes = grid::nodes(items);
        let width = self.page_width.round() as i32;
        let height = self.page_height.round() as i32;
        // Each page has its own columns.
        let mut xcuts = grid::x_cuts(std::slice::from_ref(&nodes), width, NATURE);
        let g = grid::layout(&nodes, &mut xcuts, height, NATURE);
        self.links = odf::page_links(items);
        let mut out = std::mem::take(&mut self.body);
        let start = out.len();
        // The first page sets the page style; the others start on a new page.
        let (attributes, brk) =
            if self.pages == 0 { (" style:master-page-name=\"Standard\"", "") } else { ("", " fo:break-before=\"page\"") };
        let props = format!(
            "<style:table-properties style:width=\"{}\" table:align=\"left\" fo:margin-left=\"0pt\" fo:margin-top=\"0pt\" fo:margin-bottom=\"0pt\"{brk}/>",
            pt(width as f32)
        );
        let ta = self.styles.style("ta", "table", attributes, &props);
        let _ = writeln!(out, "<table:table table:name=\"Page{}\" table:style-name=\"{ta}\">", self.pages + 1);
        for c in 0..xcuts.spans() {
            let props = format!("<style:table-column-properties style:column-width=\"{}\"/>", pt(xcuts.size(c) as f32));
            let co = self.styles.style("co", "table-column", "", &props);
            let _ = writeln!(out, "<table:table-column table:style-name=\"{co}\"/>");
        }
        // Rows below the last thing on the page are left out: pages are separated by page
        // breaks, and a table as tall as the page would push what follows onto another.
        let last = g
            .cells
            .iter()
            .rposition(|row| row.iter().any(|c| !matches!(c, Cell::Empty(s) if *s == CellStyle::default())))
            .unwrap_or(0);
        for (ri, cells) in g.cells.iter().enumerate().take(last + 1) {
            let props = format!(
                "<style:table-row-properties style:row-height=\"{}\" style:use-optimal-row-height=\"false\" fo:keep-together=\"always\"/>",
                pt(g.rows.size(ri) as f32)
            );
            let ro = self.styles.style("ro", "table-row", "", &props);
            let _ = writeln!(out, "<table:table-row table:style-name=\"{ro}\">");
            for (ci, cell) in cells.iter().enumerate() {
                match cell {
                    Cell::Empty(style) => {
                        let ce = cell_style(&mut self.styles, style, None, None);
                        let _ = writeln!(out, "<table:table-cell table:style-name=\"{ce}\"/>");
                    }
                    Cell::Element { node, rows, cols, style } => {
                        out.push_str("<table:table-cell");
                        if *cols > 1 {
                            let _ = write!(out, " table:number-columns-spanned=\"{cols}\"");
                        }
                        if *rows > 1 {
                            let _ = write!(out, " table:number-rows-spanned=\"{rows}\"");
                        }
                        let w = (xcuts.offsets[ci + cols] - xcuts.offsets[ci]) as f32;
                        let h = (g.rows.offsets[ri + rows] - g.rows.offsets[ri]) as f32;
                        self.element(&mut out, node, style, w, h, fonts);
                        out.push_str("</table:table-cell>\n");
                    }
                    Cell::Occupied(..) => out.push_str("<table:covered-table-cell/>\n"),
                }
            }
            out.push_str("</table:table-row>\n");
        }
        out.push_str("</table:table>\n");
        // The page's own bookmark (what links to a page lead to), in its first paragraph.
        let mark = format!("<text:bookmark text:name=\"JR_PAGE_ANCHOR_0_{}\"/>", self.pages + 1);
        if let Some(at) = out[start..].find("<text:p").map(|i| start + i) {
            if let Some(close) = out[at..].find('>').map(|i| at + i) {
                if out[..close].ends_with('/') {
                    // An empty paragraph: open it to hold the bookmark.
                    out.replace_range(close - 1..close + 1, &format!(">{mark}</text:p>"));
                } else {
                    out.insert_str(close + 1, &mark);
                }
            }
        }
        self.body = out;
    }

    /// The style and content of an element's cell (its start tag is open).
    fn element(&mut self, out: &mut String, node: &Node<'_>, style: &CellStyle, w: f32, h: f32, fonts: &FontSet) {
        match &node.what {
            What::Text(t) => {
                let ce = cell_style(&mut self.styles, style, Some(t), None);
                let _ = write!(out, " table:style-name=\"{ce}\" office:value-type=\"string\">");
                let p = paragraph_style(&mut self.styles, t);
                let link = self.links.get(&(node.x, node.y)).cloned();
                let mark = link.as_ref().and_then(|l| l.anchor.as_deref()).map(crate::xlsx::excel_name);
                odf::paragraphs(&mut self.styles, out, t, Some(&p), !t.runs.is_empty(), false, mark.as_deref(), link.as_deref());
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
                // A picture sits in its cell where the image is aligned.
                let (halign, valign) = match &e.shape {
                    Shape::Image { halign, valign, .. } => (*halign, *valign),
                    _ => (HAlign::Left, VAlign::Top),
                };
                // An ellipse or a slanted line is drawn into a picture (JasperReports' ODT
                // export draws them as shapes).
                let drawing = match &e.shape {
                    Shape::Ellipse(_) => true,
                    Shape::Line { .. } => node.w > 1 && node.h > 1,
                    _ => false,
                };
                if drawing {
                    // The picture has the shape's fill and pen.
                    style.boxed = None;
                    if style.back == e.back {
                        style.back = None;
                    }
                }
                let padding = e.boxed.filter(|_| matches!(e.shape, Shape::Image { .. }));
                let ce = cell_style(&mut self.styles, &style, None, Some((valign, padding)));
                let _ = write!(out, " table:style-name=\"{ce}\">");
                let picture = if drawing {
                    self.pictures.drawing(drawn, fonts, [e.x, e.y, e.w, e.h]).map(|href| {
                        (href, odf::Placed { x: 0.0, y: 0.0, w: (node.w as f32).min(w), h: (node.h as f32).min(h), clip: None })
                    })
                } else {
                    odf::element_picture(&mut self.pictures, fonts, e, drawn, w, h)
                };
                if let Some((href, placed)) = picture {
                    let gr = odf::picture_style(&mut self.styles, &placed, true);
                    let props = format!("<style:paragraph-properties fo:text-align=\"{}\"/>", odf::text_align(halign));
                    let p = self.styles.style("P", "paragraph", "", &props);
                    let _ = write!(out, "<text:p text:style-name=\"{p}\">");
                    self.frames += 1;
                    odf::frame(out, &gr, self.frames, &href, &placed, Some("as-char"));
                    out.push_str("</text:p>");
                }
            }
        }
    }
}

impl<W: std::io::Write> crate::fill::PageSink for OdtStream<W> {
    fn page(&mut self, page: crate::print::PrintPage, fonts: &FontSet) -> crate::Result<()> {
        self.page_xml(&page.items, fonts);
        self.pages += 1;
        Ok(())
    }
}

/// Exports a filled document to an OpenDocument text document.
pub fn write_odt(doc: &crate::print::PrintDocument, fonts: &FontSet) -> crate::Result<Vec<u8>> {
    use crate::fill::PageSink;
    let mut sink = OdtStream::new(Vec::new(), &doc.name, doc.page_width, doc.page_height, doc.landscape)?;
    for page in &doc.pages {
        sink.page(page.clone(), fonts)?;
    }
    sink.finish()
}

/// Background, borders, and for a text its padding, vertical alignment and direction (for a
/// picture, its vertical alignment and padding).
fn cell_style(
    styles: &mut Styles,
    style: &CellStyle,
    text: Option<&CellInfo>,
    picture: Option<(VAlign, Option<crate::model::BoxStyle>)>,
) -> String {
    let mut props = String::from("<style:table-cell-properties");
    if let Some(back) = text.and_then(|t| t.back).or(style.back) {
        let _ = write!(props, " fo:background-color=\"{}\"", odf::hex(back));
    }
    props.push_str(&odf::borders(style.boxed.as_ref()));
    match (text, picture) {
        (Some(t), _) => {
            props.push_str(&odf::padding(&t.boxed));
            let _ = write!(props, " style:vertical-align=\"{}\"", odf::vertical_align(t.valign));
            match t.rotation {
                Rotation::Left => props.push_str(" style:writing-mode=\"bt-lr\""),
                Rotation::Right => props.push_str(" style:writing-mode=\"tb-rl\""),
                _ => {}
            }
        }
        (None, Some((valign, padding))) => {
            match padding {
                Some(b) => props.push_str(&odf::padding(&b)),
                None => props.push_str(" fo:padding=\"0pt\""),
            }
            let _ = write!(props, " style:vertical-align=\"{}\"", odf::vertical_align(valign));
        }
        (None, None) => props.push_str(" fo:padding=\"0pt\""),
    }
    props.push_str("/>");
    styles.style("ce", "table-cell", "", &props)
}

/// A text's paragraph style: its alignment and font.
fn paragraph_style(styles: &mut Styles, t: &CellInfo) -> String {
    let mut props = format!(
        "<style:paragraph-properties fo:text-align=\"{}\" fo:margin-left=\"0pt\" fo:margin-right=\"0pt\" fo:margin-top=\"0pt\" fo:margin-bottom=\"0pt\" fo:text-indent=\"0pt\" fo:line-height=\"100%\"/>",
        odf::text_align(t.halign)
    );
    props.push_str(&styles.text_properties(&t.font, t.color, None, crate::rich::Script::Normal));
    styles.style("P", "paragraph", "", &props)
}

/// The default paragraph: no space around it, and as small as it can be (the paragraphs of
/// empty cells use it, and must not make their rows taller).
const DEFAULT_PARAGRAPH: &str = "<style:paragraph-properties fo:margin-top=\"0pt\" fo:margin-bottom=\"0pt\" fo:line-height=\"100%\"/><style:text-properties fo:font-size=\"1pt\" style:font-size-asian=\"1pt\" style:font-size-complex=\"1pt\"/>";

const END_PARAGRAPH: &str = "<style:paragraph-properties fo:margin-top=\"0pt\" fo:margin-bottom=\"0pt\" fo:line-height=\"1pt\"/><style:text-properties fo:font-size=\"1pt\" style:font-size-asian=\"1pt\" style:font-size-complex=\"1pt\"/>";

#[cfg(test)]
mod tests {
    fn render(name: &str, rows: Option<serde_json::Value>) -> (Vec<u8>, Vec<(String, String)>, usize) {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(name);
        let read = |f: &str| std::fs::read_to_string(dir.join(f));
        let rows = rows.unwrap_or_else(|| read("data.json").map(|t| serde_json::from_str(&t).unwrap()).unwrap_or_default());
        let params: serde_json::Value =
            read("params.json").map(|t| serde_json::from_str(&t).unwrap()).unwrap_or(serde_json::json!({}));
        let mut r = crate::Renderer::new().resource_dir(&dir);
        let report = r.parse(&read("report.jrxml").unwrap()).unwrap();
        let records = crate::data::records_from_json(&report, &rows).unwrap();
        let p = crate::params_from_json(&report, &params).unwrap();
        let mut odt = Vec::new();
        let stats = r.render_to(&report, Box::new(records.into_iter().map(Ok)), p, crate::Format::Odt, &mut odt).unwrap();
        let parts =
            crate::zip::tests::read(&odt).into_iter().map(|(n, d)| (n, String::from_utf8_lossy(&d).into_owned())).collect();
        (odt, parts, stats.pages)
    }

    fn content(parts: &[(String, String)]) -> &str {
        parts.iter().find(|(n, _)| n == "content.xml").map(|(_, d)| d.as_str()).unwrap()
    }

    /// One table per page, a page break before each page after the first, the texts in
    /// their cells, late values ("page 2 of 5") included.
    #[test]
    fn writes_a_table_per_page() {
        let (bytes, parts, pages) = render("33-page-x-of-y", None);
        crate::ods::tests::check_package(&bytes, &parts, super::MIMETYPE);
        let xml = content(&parts);
        let tables = xml.matches("<table:table ").count();
        assert!(tables > 1);
        assert_eq!(tables, pages);
        assert_eq!(xml.matches("fo:break-before=\"page\"").count(), 1, "one table style breaks the page");
        assert_eq!(xml.matches("style:master-page-name=\"Standard\"").count(), 1);
        for p in 1..=pages {
            assert!(xml.contains(&format!("<text:bookmark text:name=\"JR_PAGE_ANCHOR_0_{p}\"/>")), "page {p}");
        }
        // The total of "Page 1 of 5", known only at the end, is in the text.
        assert!(xml.contains(&format!("<text:s/>{pages}</text:p>")), "late page totals are in the text");
    }

    #[test]
    fn texts_keep_their_style_and_pictures_are_embedded() {
        let (_, parts, _) = render("02-invoice", None);
        let xml = content(&parts);
        assert!(xml.contains(">Ferro Industrial SRL</text:p>"));
        assert!(xml.contains("<table:covered-table-cell/>"));
        assert!(xml.contains("fo:font-size=\"18pt\""));
        assert!(xml.contains("fo:text-align=\"end\""));
        let (_, parts, _) = render("15-images", None);
        let pictures: Vec<_> = parts.iter().filter(|(n, _)| n.starts_with("Pictures/")).collect();
        assert!(!pictures.is_empty());
        let manifest = &parts.iter().find(|(n, _)| n == "META-INF/manifest.xml").unwrap().1;
        for (name, _) in pictures {
            assert!(content(&parts).contains(&format!("xlink:href=\"{name}\"")), "{name}");
            assert!(manifest.contains(&format!("manifest:full-path=\"{name}\"")), "{name}");
        }
    }

    #[test]
    fn a_report_without_pages_is_an_empty_document() {
        let (bytes, parts, pages) = render("02-invoice", Some(serde_json::json!([])));
        assert_eq!(pages, 0);
        crate::ods::tests::check_package(&bytes, &parts, super::MIMETYPE);
        let xml = content(&parts);
        assert!(!xml.contains("<table:table "));
        assert!(xml.contains("<office:text>"));
    }
}
