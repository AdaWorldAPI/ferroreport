//! OpenDocument spreadsheet (ODS) export, laid out like JasperReports' `JROdsExporter` and
//! like this crate's XLSX export: all pages on one sheet named after the report, the page cut
//! into a grid of cells at element edges (page margins kept), each text in the cells it
//! covers (merged), with its font, colors, borders, padding and alignment. Values are written
//! as text, like Jasper's default; with [`XlsxOptions::detect_cell_type`] numbers are written
//! as numbers, with a number style made from their format pattern.
//!
//! The columns come from every page, so a workbook is made in two passes, as in
//! [`crate::xlsx`]; the sheet is kept in memory until the end, where its styles are written
//! before it.

use crate::fonts::FontSet;
use crate::grid::{self, Cell, CellStyle, Sides, What};
use crate::model::{Color, HAlign, Rotation, VAlign};
use crate::odf::{self, pt, Pictures, Styles};
use crate::print::{CellInfo, PrintDocument, PrintItem, Shape};
use crate::xlsx::{escape, Border, XlsxColumns, XlsxOptions, MAX_COLUMNS, MAX_ROWS, NATURE};
use std::collections::HashMap;
use std::fmt::Write as _;

pub(crate) const MIMETYPE: &str = "application/vnd.oasis.opendocument.spreadsheet";

/// Writes a spreadsheet page by page. `columns` must have seen every page that will be
/// written (see [`XlsxColumns`]).
pub struct OdsStream<W: std::io::Write> {
    zip: crate::zip::ZipStream<W>,
    xcuts: grid::Cuts,
    height: i32,
    page_width: f32,
    page_height: f32,
    landscape: bool,
    options: XlsxOptions,
    styles: Styles,
    pictures: Pictures,
    /// The columns and rows of the sheet, written so far.
    body: String,
    row_no: usize,
    pages_done: usize,
    name: String,
    /// Named ranges: name and the cell (row, column, from 0).
    anchors: Vec<(String, usize, usize)>,
    frames: usize,
}

impl<W: std::io::Write> OdsStream<W> {
    pub fn new(
        out: W,
        columns: XlsxColumns,
        name: &str,
        page_width: f32,
        page_height: f32,
        landscape: bool,
        options: XlsxOptions,
    ) -> crate::Result<OdsStream<W>> {
        let mut xcuts = columns.cuts;
        xcuts.close(page_width.round() as i32);
        if xcuts.spans() > MAX_COLUMNS {
            return Err(crate::Error::Limit(format!(
                "the report needs {} columns and a sheet holds {MAX_COLUMNS}",
                xcuts.spans()
            )));
        }
        let mut styles = Styles::default();
        let plain = empty_style(&mut styles, &CellStyle::default());
        let mut body = String::new();
        for i in 0..xcuts.spans() {
            let co = column_style(&mut styles, xcuts.size(i) as f32);
            let _ = writeln!(body, "<table:table-column table:style-name=\"{co}\" table:default-cell-style-name=\"{plain}\"/>");
        }
        Ok(OdsStream {
            zip: odf::begin(out, MIMETYPE)?,
            xcuts,
            height: page_height.round() as i32,
            page_width,
            page_height,
            landscape,
            options,
            styles,
            pictures: Pictures::default(),
            body,
            row_no: 0,
            pages_done: 0,
            name: crate::xlsx::sheet_name(name),
            anchors: Vec::new(),
            frames: 0,
        })
    }

    /// Writes the rows of one page.
    pub fn add(&mut self, items: &[PrintItem], fonts: &FontSet) -> crate::Result<()> {
        let nodes = grid::nodes(items);
        let g = grid::layout(&nodes, &mut self.xcuts, self.height, NATURE);
        if self.row_no + g.cells.len() > MAX_ROWS {
            return Err(crate::Error::Limit(format!(
                "the report has more rows than a sheet holds ({MAX_ROWS}): page {} would go past the last one. \
                 Export it as CSV, or in parts",
                self.pages_done + 1
            )));
        }
        let links = odf::page_links(items);
        // Anchors as names of the cell where their element starts, like the XLSX export.
        let mut found = Vec::new();
        crate::xlsx::links(items, &mut found);
        for link in &found {
            let Some(anchor) = &link.anchor else { continue };
            let name = crate::xlsx::excel_name(anchor);
            if name.is_empty() || self.anchors.iter().any(|(n, ..)| *n == name) {
                continue;
            }
            let col = self.xcuts.index(link.x.round() as i32).min(self.xcuts.spans().saturating_sub(1));
            let row = self.row_no + g.rows.index(link.y.round() as i32).min(g.rows.spans().saturating_sub(1));
            self.anchors.push((name, row, col));
        }
        self.anchors.push((format!("JR_PAGE_ANCHOR_0_{}", self.pages_done + 1), self.row_no, 0));
        // The style of each element's corner cell, for the cells it covers.
        let mut corner: HashMap<(usize, usize), String> = HashMap::new();
        let mut row = Vec::new();
        for (ri, cells) in g.cells.iter().enumerate() {
            let ro = row_style(&mut self.styles, g.rows.size(ri) as f32);
            let _ = writeln!(self.body, "<table:table-row table:style-name=\"{ro}\">");
            row.clear();
            for (ci, cell) in cells.iter().enumerate() {
                let xml = match cell {
                    Cell::Empty(style) => {
                        format!("<table:table-cell table:style-name=\"{}\"/>", empty_style(&mut self.styles, style))
                    }
                    Cell::Element { node, rows, cols, style } => {
                        let mut xml = String::new();
                        let ce = self.element(&mut xml, node, style, *rows, *cols, &links, fonts);
                        corner.insert((ri, ci), ce);
                        xml
                    }
                    Cell::Occupied(r0, c0) => match corner.get(&(*r0, *c0)) {
                        Some(ce) => format!("<table:covered-table-cell table:style-name=\"{ce}\"/>"),
                        None => "<table:covered-table-cell/>".to_string(),
                    },
                };
                row.push(xml);
            }
            write_cells(&mut self.body, &row);
            self.body.push_str("</table:table-row>\n");
        }
        self.row_no += g.cells.len();
        self.pages_done += 1;
        Ok(())
    }

    /// The corner cell of an element (written to `out`), and its style.
    #[allow(clippy::too_many_arguments)]
    fn element(
        &mut self,
        out: &mut String,
        node: &grid::Node<'_>,
        style: &CellStyle,
        rows: usize,
        cols: usize,
        links: &HashMap<(i32, i32), std::sync::Arc<crate::print::LinkInfo>>,
        fonts: &FontSet,
    ) -> String {
        let span = match (rows, cols) {
            (1, 1) => String::new(),
            _ => format!(" table:number-columns-spanned=\"{cols}\" table:number-rows-spanned=\"{rows}\""),
        };
        match &node.what {
            What::Text(t) => {
                let number = t.number.filter(|n| self.options.detect_cell_type && n.is_finite());
                let data = match (number, &t.pattern) {
                    (Some(_), Some(p)) => number_style(&mut self.styles, p),
                    _ => None,
                };
                let ce = text_style(&mut self.styles, t, style, data.as_ref().map(|(n, _)| n.as_str()));
                let value = match number {
                    Some(n) if data.as_ref().is_some_and(|(_, percent)| *percent) => {
                        format!(" office:value-type=\"percentage\" office:value=\"{}\"", odf::number(n))
                    }
                    Some(n) => format!(" office:value-type=\"float\" office:value=\"{}\"", odf::number(n)),
                    None if t.text.is_empty() => String::new(),
                    None => " office:value-type=\"string\"".to_string(),
                };
                let _ = write!(out, "<table:table-cell table:style-name=\"{ce}\"{span}{value}>");
                if !t.text.is_empty() {
                    let link = links.get(&(node.x, node.y)).filter(|l| l.target.is_some());
                    odf::paragraphs(&mut self.styles, out, t, None, !t.runs.is_empty(), true, None, link.map(|l| l.as_ref()));
                }
                out.push_str("</table:table-cell>");
                ce
            }
            What::Shape(e, _, drawn) => {
                let border = match &e.shape {
                    Shape::Rectangle(pen) | Shape::Ellipse(pen) => Border::Sides(Some([pen.unwrap_or(grid::NO_PEN); 4])),
                    Shape::Line { pen, top_down } => crate::xlsx::line_border(node.w, node.h, *pen, *top_down),
                    Shape::Picture { .. } | Shape::Frame | Shape::Image { .. } => Border::Sides(style.boxed),
                };
                let ce = shape_style(&mut self.styles, style.back, border);
                let _ = write!(out, "<table:table-cell table:style-name=\"{ce}\"{span}>");
                if let Some((href, placed)) =
                    odf::element_picture(&mut self.pictures, fonts, e, drawn, node.w as f32, node.h as f32)
                {
                    let gr = odf::picture_style(&mut self.styles, &placed, false);
                    self.frames += 1;
                    odf::frame(out, &gr, self.frames, &href, &placed, None);
                }
                out.push_str("</table:table-cell>");
                ce
            }
        }
    }

    /// Ends the sheet and writes the rest of the package.
    pub fn finish(mut self) -> crate::Result<W> {
        if self.pages_done == 0 {
            // A report without pages: one empty row the height of a page, as the XLSX export
            // writes.
            let ro = row_style(&mut self.styles, self.page_height);
            let plain = empty_style(&mut self.styles, &CellStyle::default());
            let _ = writeln!(
                self.body,
                "<table:table-row table:style-name=\"{ro}\"><table:table-cell table:style-name=\"{plain}\"/></table:table-row>"
            );
        }
        let ta = self.styles.style(
            "ta",
            "table",
            " style:master-page-name=\"Standard\"",
            "<style:table-properties table:display=\"true\" style:writing-mode=\"lr-tb\"/>",
        );
        let mut content = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-content {}>\n", odf::NAMESPACES);
        self.styles.write(&mut content);
        let sheet = escape(&self.name);
        let _ = writeln!(
            content,
            "<office:body>\n<office:spreadsheet>\n<table:table table:name=\"{sheet}\" table:style-name=\"{ta}\">"
        );
        content.push_str(&self.body);
        content.push_str("</table:table>\n");
        if self.pages_done > 0 && !self.anchors.is_empty() {
            content.push_str("<table:named-expressions>\n");
            let quoted = escape(&self.name.replace('\'', "''"));
            for (name, row, col) in &self.anchors {
                let cell = format!("$&apos;{quoted}&apos;.${}${}", crate::xlsx::column_name(*col), row + 1);
                let _ = writeln!(
                    content,
                    "<table:named-range table:name=\"{}\" table:base-cell-address=\"{cell}\" table:cell-range-address=\"{cell}\"/>",
                    escape(name)
                );
            }
            content.push_str("</table:named-expressions>\n");
        }
        content.push_str("</office:spreadsheet>\n</office:body>\n</office:document-content>\n");
        let styles = odf::styles_xml(self.page_width, self.page_height, self.landscape, "");
        odf::finish(self.zip, MIMETYPE, &content, &styles, &self.name, &self.pictures)
    }
}

impl<W: std::io::Write> crate::fill::PageSink for OdsStream<W> {
    fn page(&mut self, page: crate::print::PrintPage, fonts: &FontSet) -> crate::Result<()> {
        self.add(&page.items, fonts)
    }
}

/// Exports a filled document to an OpenDocument spreadsheet.
pub fn write_ods(doc: &PrintDocument, fonts: &FontSet, options: &XlsxOptions) -> crate::Result<Vec<u8>> {
    let mut columns = XlsxColumns::new();
    for page in &doc.pages {
        columns.add(&page.items);
    }
    let mut out =
        OdsStream::new(Vec::new(), columns, &doc.name, doc.page_width, doc.page_height, doc.landscape, options.clone())?;
    for page in &doc.pages {
        out.add(&page.items, fonts)?;
    }
    out.finish()
}

/// The cells of a row, runs of the same empty cell written once with a repeat count.
fn write_cells(out: &mut String, cells: &[String]) {
    let mut i = 0;
    while i < cells.len() {
        let cell = &cells[i];
        let mut n = 1;
        if cell.ends_with("/>") {
            while i + n < cells.len() && cells[i + n] == *cell {
                n += 1;
            }
        }
        if n > 1 {
            let _ = writeln!(out, "{} table:number-columns-repeated=\"{n}\"/>", &cell[..cell.len() - 2]);
        } else {
            out.push_str(cell);
            out.push('\n');
        }
        i += n;
    }
}

fn column_style(styles: &mut Styles, width: f32) -> String {
    let props = format!("<style:table-column-properties fo:break-before=\"auto\" style:column-width=\"{}\"/>", pt(width));
    styles.style("co", "table-column", "", &props)
}

fn row_style(styles: &mut Styles, height: f32) -> String {
    let props = format!(
        "<style:table-row-properties style:row-height=\"{}\" fo:break-before=\"auto\" style:use-optimal-row-height=\"false\"/>",
        pt(height)
    );
    styles.style("ro", "table-row", "", &props)
}

fn background(back: Option<Color>) -> String {
    match back {
        Some(c) => format!(" fo:background-color=\"{}\"", odf::hex(c)),
        None => String::new(),
    }
}

/// Empty cells: the background and borders the grid gave them (frames).
fn empty_style(styles: &mut Styles, style: &CellStyle) -> String {
    shape_style(styles, style.back, Border::Sides(style.boxed))
}

/// Cells of shapes and pictures: a background and borders, or a diagonal for a slanted line.
fn shape_style(styles: &mut Styles, back: Option<Color>, border: Border) -> String {
    let lines = match border {
        Border::Sides(b) => odf::borders(b.as_ref()),
        Border::Diagonal { up, pen } => {
            let attr = if up { "style:diagonal-bl-tr" } else { "style:diagonal-tl-br" };
            format!(" fo:border=\"none\" {attr}=\"{}\"", odf::border(&pen))
        }
    };
    let props = format!(
        "<style:table-cell-properties fo:wrap-option=\"wrap\" style:shrink-to-fit=\"false\"{}{lines} fo:padding=\"0pt\"/>",
        background(back)
    );
    styles.style("ce", "table-cell", "", &props)
}

/// The style of a text's cell: its box, alignment and font (the font of a number's cell is
/// the one it is shown in), and the number style of its value.
fn text_style(styles: &mut Styles, t: &CellInfo, style: &CellStyle, data: Option<&str>) -> String {
    let rotation = match t.rotation {
        Rotation::Left => " style:rotation-angle=\"90\"",
        Rotation::Right => " style:rotation-angle=\"270\"",
        Rotation::UpsideDown => " style:rotation-angle=\"180\"",
        _ => "",
    };
    // Rotated text is aligned in the turned box (a left-rotated text starts at the bottom), as in
    // the XLSX export.
    let along = |start: &'static str, end: &'static str| match t.halign {
        HAlign::Left => start,
        HAlign::Center => "middle",
        HAlign::Right => end,
        HAlign::Justified => "middle",
    };
    let across = |top: &'static str, bottom: &'static str| match t.valign {
        VAlign::Top => top,
        VAlign::Middle => "center",
        VAlign::Bottom => bottom,
    };
    let (horizontal, vertical) = match t.rotation {
        Rotation::Left => (across("start", "end"), along("bottom", "top")),
        Rotation::Right => (across("end", "start"), along("top", "bottom")),
        Rotation::UpsideDown => (
            match t.halign {
                HAlign::Left => "end",
                HAlign::Right => "start",
                h => odf::text_align(h),
            },
            match t.valign {
                VAlign::Top => "bottom",
                VAlign::Bottom => "top",
                VAlign::Middle => "middle",
            },
        ),
        _ => (odf::text_align(t.halign), odf::vertical_align(t.valign)),
    };
    let sides: Option<Sides> = style.boxed;
    let mut props = format!(
        "<style:table-cell-properties fo:wrap-option=\"wrap\" style:shrink-to-fit=\"false\"{}{}{} style:vertical-align=\"{vertical}\" style:text-align-source=\"fix\" style:repeat-content=\"false\"{rotation}/>",
        background(style.back),
        odf::borders(sides.as_ref()),
        odf::padding(&t.boxed),
    );
    let _ = write!(props, "<style:paragraph-properties fo:text-align=\"{horizontal}\" fo:margin-left=\"0pt\"/>");
    props.push_str(&styles.text_properties(&t.font, t.color, None, crate::rich::Script::Normal));
    let attributes = match data {
        Some(n) => format!(" style:data-style-name=\"{n}\""),
        None => String::new(),
    };
    styles.style("ce", "table-cell", &attributes, &props)
}

/// A number style made from a Java number pattern (`#,##0.00`, `0.0%`, `0.###E0`), and
/// whether it shows a percentage. `None` for what it cannot say (per mille).
fn number_style(styles: &mut Styles, pattern: &str) -> Option<(String, bool)> {
    let p = parse_pattern(pattern)?;
    let mut body = String::new();
    let literal = |body: &mut String, s: &str| {
        if !s.is_empty() {
            let _ = write!(body, "<number:text>{}</number:text>", escape(s));
        }
    };
    literal(&mut body, &p.prefix);
    match p.exponent {
        Some(e) => {
            let _ = write!(
                body,
                "<number:scientific-number number:decimal-places=\"{}\" number:min-decimal-places=\"{}\" number:min-integer-digits=\"{}\" number:min-exponent-digits=\"{e}\"/>",
                p.max_decimals, p.min_decimals, p.min_integer
            );
        }
        None => {
            let _ = write!(
                body,
                "<number:number number:decimal-places=\"{}\" number:min-decimal-places=\"{}\" number:min-integer-digits=\"{}\"{}/>",
                p.max_decimals,
                p.min_decimals,
                p.min_integer,
                if p.grouping { " number:grouping=\"true\"" } else { "" }
            );
        }
    }
    literal(&mut body, &p.suffix);
    let element = if p.percent { "percentage-style" } else { "number-style" };
    Some((styles.data_style(element, &body), p.percent))
}

/// The parts of a Java `DecimalFormat` pattern that a number style can say.
#[derive(Debug, PartialEq)]
struct Pattern {
    prefix: String,
    suffix: String,
    min_integer: usize,
    min_decimals: usize,
    max_decimals: usize,
    grouping: bool,
    exponent: Option<usize>,
    percent: bool,
}

fn parse_pattern(pattern: &str) -> Option<Pattern> {
    // The positive part (before an unquoted `;`).
    let chars: Vec<char> = pattern.chars().collect();
    let (mut prefix, mut number, mut suffix) = (String::new(), String::new(), String::new());
    let mut part = 0; // 0 prefix, 1 number, 2 suffix
    let mut percent = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ';' => break,
            '\'' => {
                let mut s = String::new();
                if chars.get(i + 1) == Some(&'\'') {
                    s.push('\'');
                    i += 2;
                } else {
                    i += 1;
                    while i < chars.len() && chars[i] != '\'' {
                        s.push(chars[i]);
                        i += 1;
                    }
                    i += 1;
                }
                if part == 1 {
                    part = 2;
                }
                if part == 0 { &mut prefix } else { &mut suffix }.push_str(&s);
                continue;
            }
            '#' | '0'..='9' | ',' | '.' if part < 2 => {
                part = 1;
                number.push(c);
            }
            'E' if part == 1 => number.push(c),
            '\u{2030}' => return None,
            c => {
                if part == 1 {
                    part = 2;
                }
                let c = match c {
                    '%' => {
                        percent = true;
                        '%'
                    }
                    '\u{a4}' => '$',
                    c => c,
                };
                if part == 0 { &mut prefix } else { &mut suffix }.push(c);
            }
        }
        i += 1;
    }
    if !number.chars().any(|c| c == '#' || c.is_ascii_digit()) {
        return None;
    }
    let (mantissa, exponent) = match number.split_once('E') {
        Some((m, e)) => (m, Some(e.chars().filter(|c| c.is_ascii_digit()).count().max(1))),
        None => (number.as_str(), None),
    };
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    Some(Pattern {
        prefix,
        suffix,
        min_integer: int.chars().filter(|c| c.is_ascii_digit()).count(),
        min_decimals: frac.chars().filter(|c| c.is_ascii_digit()).count(),
        max_decimals: frac.chars().filter(|c| *c == '#' || c.is_ascii_digit()).count(),
        grouping: int.contains(','),
        exponent,
        percent,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fixture's spreadsheet, and its parts by name.
    fn render(name: &str, rows: Option<serde_json::Value>, numbers: bool) -> (Vec<u8>, Vec<(String, String)>) {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(name);
        let read = |f: &str| std::fs::read_to_string(dir.join(f));
        let rows = rows.unwrap_or_else(|| read("data.json").map(|t| serde_json::from_str(&t).unwrap()).unwrap_or_default());
        let params: serde_json::Value =
            read("params.json").map(|t| serde_json::from_str(&t).unwrap()).unwrap_or(serde_json::json!({}));
        let mut r = crate::Renderer::new()
            .resource_dir(&dir)
            .xlsx_options(XlsxOptions { detect_cell_type: numbers })
            .now(crate::data::parse_datetime("2026-01-15T10:30:00").unwrap());
        let report = r.parse(&read("report.jrxml").unwrap()).unwrap();
        let ods = r.render_report_json(&report, &rows, &params, crate::Format::Ods).unwrap();
        let parts =
            crate::zip::tests::read(&ods).into_iter().map(|(n, d)| (n, String::from_utf8_lossy(&d).into_owned())).collect();
        (ods, parts)
    }

    fn part<'a>(parts: &'a [(String, String)], name: &str) -> &'a str {
        parts.iter().find(|(n, _)| n == name).map(|(_, d)| d.as_str()).unwrap_or_else(|| panic!("no {name}"))
    }

    /// The package: `mimetype` first and stored as it is, then the parts a reader needs.
    pub(crate) fn check_package(bytes: &[u8], parts: &[(String, String)], mimetype: &str) {
        assert_eq!(&bytes[..4], b"PK\x03\x04");
        assert_eq!(u16::from_le_bytes([bytes[8], bytes[9]]), 0, "mimetype is stored");
        assert_eq!(&bytes[30..38], b"mimetype");
        assert_eq!(&bytes[38..38 + mimetype.len()], mimetype.as_bytes());
        assert_eq!(parts[0].0, "mimetype");
        for name in ["content.xml", "styles.xml", "meta.xml", "META-INF/manifest.xml"] {
            assert!(parts.iter().any(|(n, _)| n == name), "{name}");
        }
        assert!(part(parts, "META-INF/manifest.xml").contains(&format!("manifest:media-type=\"{mimetype}\"")));
    }

    #[test]
    fn writes_every_page_on_one_sheet() {
        let (bytes, parts) = render("02-invoice", None, false);
        check_package(&bytes, &parts, MIMETYPE);
        let content = part(&parts, "content.xml");
        assert_eq!(content.matches("<table:table ").count(), 1);
        assert!(content.contains("table:name=\"invoice\""));
        // Texts in the cells they cover, with their fonts and borders.
        assert!(content.contains("<text:p>Ferro Industrial SRL</text:p>"));
        assert!(content.contains("table:number-columns-spanned="));
        assert!(content.contains("<table:covered-table-cell"));
        assert!(content.contains("fo:font-weight=\"bold\""));
        assert!(content.contains("fo:border-bottom="));
        // Both pages, each with its anchor; numbers as text by default.
        assert!(content.contains("table:name=\"JR_PAGE_ANCHOR_0_2\""));
        assert!(content.contains("<text:p>Page 2 of<text:s/>2</text:p>") || content.contains("<text:p><text:s/>2</text:p>"));
        assert!(content.contains("office:value-type=\"string\"><text:p>3,885.00</text:p>"));
        assert!(!content.contains("office:value-type=\"float\""));
        // The page of the report, its margins in the grid.
        let styles = part(&parts, "styles.xml");
        assert!(styles.contains("fo:page-width=\"595pt\" fo:page-height=\"842pt\""));
    }

    #[test]
    fn numbers_are_numbers_with_their_pattern() {
        let (_, parts) = render("02-invoice", None, true);
        let content = part(&parts, "content.xml");
        assert!(content.contains("office:value-type=\"float\" office:value=\"3885\"><text:p>3,885.00</text:p>"));
        let style = content.split("<number:number-style ").nth(1).unwrap();
        assert!(style.contains("number:decimal-places=\"2\" number:min-decimal-places=\"2\""));
        assert!(style.contains("number:grouping=\"true\""));
        assert!(content.contains("style:data-style-name=\"N"));
    }

    #[test]
    fn a_report_without_pages_is_one_empty_row() {
        let (bytes, parts) = render("02-invoice", Some(serde_json::json!([])), false);
        check_package(&bytes, &parts, MIMETYPE);
        let content = part(&parts, "content.xml");
        assert_eq!(content.matches("<table:table-row ").count(), 1);
        assert!(!content.contains("named-range"));
        assert!(content.contains("style:row-height=\"842pt\""));
    }

    #[test]
    fn java_patterns() {
        let p = parse_pattern("#,##0.00").unwrap();
        assert_eq!((p.min_integer, p.min_decimals, p.max_decimals, p.grouping), (1, 2, 2, true));
        let p = parse_pattern("0.0%").unwrap();
        assert!(p.percent);
        assert_eq!((p.suffix.as_str(), p.min_decimals), ("%", 1));
        let p = parse_pattern("¤ #,##0.##;(¤ #,##0.##)").unwrap();
        assert_eq!((p.prefix.as_str(), p.min_decimals, p.max_decimals), ("$ ", 0, 2));
        let p = parse_pattern("'RD$' #,##0").unwrap();
        assert_eq!(p.prefix, "RD$ ");
        let p = parse_pattern("0.###E0").unwrap();
        assert_eq!(p.exponent, Some(1));
        assert!(parse_pattern("#,##0\u{2030}").is_none());
        assert!(parse_pattern("dd/MM/yyyy").is_none());
    }
}
