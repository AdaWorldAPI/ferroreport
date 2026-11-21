//! Excel (XLSX) export, following JasperReports' `JRXlsxExporter` with its defaults: all pages
//! on one sheet named after the report, the page cut into a grid of cells at element edges
//! (page margins kept), each text in the cells it covers (merged), with its font, colors,
//! borders and alignment. Values are written as text, like Jasper's default; numbers can be
//! written as numbers instead.

use crate::fonts::FontSet;
use crate::grid::{self, Cell, CellStyle, Nature, Sides, What};
use crate::image::{ImageData, Pixels};
use crate::model::{BoxStyle, Color, HAlign, LineStyle, Pen, Rotation, ScaleImage, VAlign};
use crate::print::{CellInfo, ElementInfo, PrintDocument, PrintItem, Shape};
use crate::rich::Script;
use std::collections::HashMap;
use std::fmt::Write as _;

/// Options for [`write_xlsx`].
#[derive(Debug, Clone, Default)]
pub struct XlsxOptions {
    /// Write numeric values as numbers with their format pattern, instead of the formatted
    /// text (JasperReports' `net.sf.jasperreports.export.xls.detect.cell.type`).
    pub detect_cell_type: bool,
}

const NATURE: Nature = Nature { span: true, texts_only: false };

/// The files of the workbook of a filled document.
fn workbook_parts(doc: &PrintDocument, fonts: &FontSet, options: &XlsxOptions) -> Vec<(String, Vec<u8>)> {
    let pages: Vec<_> = doc.pages.iter().map(|p| grid::nodes(&p.items)).collect();
    let width = doc.page_width.round() as i32;
    let height = doc.page_height.round() as i32;
    let mut xcuts = grid::x_cuts(&pages, width, NATURE);
    let grids: Vec<_> = pages.iter().map(|nodes| grid::layout(nodes, &mut xcuts, height, NATURE)).collect();

    let mut styles = Styles::new();
    let mut pictures = Pictures { fonts, anchors: String::new(), media: Vec::new(), names: HashMap::new() };
    let mut rows = String::new();
    let mut merges = String::new();
    let mut row_no = 0usize;
    let names: Vec<String> = (0..xcuts.spans()).map(column_name).collect();
    for (pages_done, g) in grids.iter().enumerate() {
        let first = row_no;
        for (ri, cells) in g.cells.iter().enumerate() {
            row_no += 1;
            let _ = writeln!(rows, "<row r=\"{row_no}\" customHeight=\"1\" ht=\"{}\">", g.rows.size(ri));
            let mut xfs = vec![0usize; cells.len()];
            let row_text = row_no.to_string();
            for (ci, cell) in cells.iter().enumerate() {
                let (xf, content) = match cell {
                    Cell::Empty(style) => (styles.shape_xf(style, style.boxed), String::new()),
                    Cell::Element { node, rows: rs, cols: cs, style } => {
                        if let What::Shape(e, _, drawn) = &node.what {
                            pictures.add(e, drawn, node.w, node.h, ci, first + ri, *cs, *rs, pages_done);
                        }
                        if *rs > 1 || *cs > 1 {
                            let _ = writeln!(
                                merges,
                                "<mergeCell ref=\"{}{row_text}:{}{}\"/>",
                                names[ci],
                                names[ci + cs - 1],
                                first + ri + rs
                            );
                        }
                        styles.element(node, style, options)
                    }
                    Cell::Occupied(r0, c0) => {
                        // Written with the style of the element covering it (set earlier in
                        // this row, or in a row above).
                        let xf = if *r0 == ri { xfs[*c0] } else { g.anchor_xf(*r0, *c0, &mut styles, options) };
                        (xf, String::new())
                    }
                };
                xfs[ci] = xf;
                rows.push_str("  <c r=\"");
                rows.push_str(&names[ci]);
                rows.push_str(&row_text);
                rows.push_str("\" s=\"");
                rows.push_str(&itoa(xf));
                rows.push_str(if content.starts_with("<v>") { "\">" } else { "\" t=\"inlineStr\">" });
                rows.push_str(&content);
                rows.push_str("</c>");
            }
            rows.push_str("</row>\n");
        }
    }

    let mut sheet = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<worksheet\n xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"\n xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\n<sheetPr>\n<outlinePr summaryBelow=\"0\"/>\n</sheetPr><dimension ref=\"A1\"/><sheetViews><sheetView workbookViewId=\"0\"/></sheetViews>\n<sheetFormatPr defaultRowHeight=\"15\"/>\n<cols>\n",
    );
    for i in 0..xcuts.spans() {
        let w = xcuts.size(i) as f32 / 6.0;
        let _ = writeln!(sheet, "<col min=\"{0}\" max=\"{0}\" customWidth=\"1\" width=\"{1}\"/>", i + 1, java_float(w));
    }
    sheet.push_str("</cols>\n<sheetData>\n");
    sheet.push_str(&rows);
    sheet.push_str("</sheetData>\n");
    if !merges.is_empty() {
        sheet.push_str("<mergeCells>\n");
        sheet.push_str(&merges);
        sheet.push_str("</mergeCells>\n");
    }
    sheet.push_str("<pageMargins left=\"0.0\" right=\"0.0\" top=\"0.0\" bottom=\"0.00\" header=\"0.0\" footer=\"0.0\"/>\n");
    let orientation = if doc.landscape { "landscape" } else { "portrait" };
    match paper_size(width, height) {
        Some(n) => {
            let _ = writeln!(sheet, "<pageSetup orientation=\"{orientation}\" paperSize=\"{n}\"/>");
        }
        None => {
            let _ = writeln!(sheet, "<pageSetup orientation=\"{orientation}\"/>");
        }
    }
    sheet.push_str("<drawing r:id=\"rIdDr1\"/></worksheet>");

    let name = sheet_name(&doc.name);
    let mut zip = Parts::default();
    zip.add("[Content_Types].xml", CONTENT_TYPES.as_bytes());
    zip.add("_rels/.rels", ROOT_RELS.as_bytes());
    zip.add(
        "docProps/app.xml",
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\">\n  <Application>ferroreport {}</Application>\n</Properties>\n",
            env!("CARGO_PKG_VERSION")
        )
        .as_bytes(),
    );
    zip.add("docProps/core.xml", CORE.as_bytes());
    zip.add(
        "xl/workbook.xml",
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<workbook\n xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"\n xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\n<workbookPr filterPrivacy=\"1\" codeName=\"ThisWorkbook\"/>\n<sheets>\n  <sheet name=\"{0}\" sheetId=\"1\" r:id=\"rId1\"/>\n</sheets>\n<definedNames>\n<definedName name=\"JR_PAGE_ANCHOR_0_1\">'{1}'!$A$1</definedName>\n</definedNames>\n</workbook>\n",
            escape(&name),
            escape(&name.replace('\'', "''"))
        )
        .as_bytes(),
    );
    zip.add("xl/_rels/workbook.xml.rels", WORKBOOK_RELS.as_bytes());
    zip.add("xl/styles.xml", styles.xml().as_bytes());
    zip.add("xl/worksheets/sheet1.xml", sheet.as_bytes());
    zip.add("xl/worksheets/_rels/sheet1.xml.rels", SHEET_RELS.as_bytes());
    zip.add("xl/drawings/drawing1.xml", pictures.drawing().as_bytes());
    zip.add("xl/drawings/_rels/drawing1.xml.rels", pictures.rels().as_bytes());
    for (name, bytes) in &pictures.media {
        zip.add(&format!("xl/media/{name}"), bytes);
    }
    zip.0
}

/// The files of a workbook, in the order they are zipped.
#[derive(Default)]
struct Parts(Vec<(String, Vec<u8>)>);

impl Parts {
    fn add(&mut self, name: &str, data: &[u8]) {
        self.0.push((name.to_string(), data.to_vec()));
    }
}

/// Exports a filled document to an XLSX workbook.
pub fn write_xlsx(doc: &PrintDocument, fonts: &FontSet, options: &XlsxOptions) -> Vec<u8> {
    let mut zip = crate::zip::ZipWriter::default();
    for (name, data) in workbook_parts(doc, fonts, options) {
        zip.add(&name, &data);
    }
    zip.finish()
}

impl grid::PageGrid<'_> {
    /// The style of the element whose corner cell is at (row, column).
    fn anchor_xf(&self, r: usize, c: usize, styles: &mut Styles, options: &XlsxOptions) -> usize {
        match &self.cells[r][c] {
            Cell::Element { node, style, .. } => styles.element(node, style, options).0,
            Cell::Empty(style) => styles.shape_xf(style, style.boxed),
            Cell::Occupied(..) => 0,
        }
    }
}

/// The border a line becomes (`JRXlsxExporter.exportLine`): one side of its cells, or their
/// diagonal for a line that is neither horizontal nor vertical.
fn line_border(w: i32, h: i32, pen: Pen, top_down: bool) -> Border {
    let mut b = [grid::NO_PEN; 4];
    let (top, left, bottom, right) = (0, 1, 2, 3);
    let side = if w as f32 / h as f32 > 1.0 {
        if h > 1 {
            return Border::Diagonal { up: !top_down, pen };
        }
        if top_down {
            top
        } else {
            bottom
        }
    } else {
        if w > 1 {
            return Border::Diagonal { up: !top_down, pen };
        }
        if top_down {
            left
        } else {
            right
        }
    };
    b[side] = pen;
    Border::Sides(Some(b))
}

/// The borders of a cell.
enum Border {
    Sides(Option<Sides>),
    Diagonal { up: bool, pen: Pen },
}

/// A small number as text, without the formatting machinery.
fn itoa(mut n: usize) -> String {
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    String::from_utf8_lossy(&buf[i..]).into_owned()
}

/// Column letters: 0 → A, 25 → Z, 26 → AA.
fn column_name(mut i: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (i % 26) as u8);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

/// Excel sheet names: at most 31 characters, none of `[]:*?/\`.
fn sheet_name(name: &str) -> String {
    let clean: String = name.chars().map(|c| if "[]:*?/\\".contains(c) { '_' } else { c }).take(31).collect();
    if clean.trim().is_empty() {
        "Sheet1".to_string()
    } else {
        clean
    }
}

/// Excel's paper size code for a page, when it is a standard size.
fn paper_size(w: i32, h: i32) -> Option<u32> {
    let (a, b) = (w.min(h), w.max(h));
    let near = |x: i32, y: i32| (a - x).abs() <= 1 && (b - y).abs() <= 1;
    [(612, 792, 1), (612, 1008, 5), (522, 756, 7), (842, 1190, 8), (595, 842, 9), (420, 595, 11), (729, 1032, 12), (516, 729, 13)]
        .iter()
        .find(|&&(x, y, _)| near(x, y))
        .map(|&(_, _, n)| n)
}

/// Like Java's `Float.toString` for the values written here (`2.0`, `6.6666665`).
fn java_float(v: f32) -> String {
    let s = format!("{v}");
    if s.contains('.') || s.contains('e') || s.contains("inf") || s.contains("NaN") {
        s
    } else {
        format!("{s}.0")
    }
}

fn hex(c: Color) -> String {
    format!("{:02X}{:02X}{:02X}", c.r, c.g, c.b)
}

pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\n' | '\t' | '\r' => out.push(c),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
}

/// The workbook's fonts, fills, borders, number formats and cell formats.
struct Styles {
    fonts: Vec<String>,
    borders: Vec<String>,
    fills: Vec<String>,
    numfmts: Vec<String>,
    xfs: Vec<String>,
    index: HashMap<String, usize>,
    /// Positions of fonts, borders and number formats by their XML.
    lookup: HashMap<String, usize>,
    /// The format of empty cells without a frame around them, the most common one.
    plain: Option<usize>,
}

impl Styles {
    fn new() -> Styles {
        Styles {
            fonts: vec![
                "<font><sz val=\"11\"/><color theme=\"1\"/><name val=\"Calibri\"/><family val=\"2\"/><scheme val=\"minor\"/></font>"
                    .into(),
            ],
            borders: vec!["<border><left/><right/><top/><bottom/><diagonal/></border>".into()],
            fills: vec![
                "<fill><patternFill patternType=\"none\"/></fill>".into(),
                "<fill><patternFill patternType=\"solid\"><fgColor rgb=\"FFFFFF\"/></patternFill></fill>".into(),
            ],
            numfmts: Vec::new(),
            xfs: vec!["<xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyAlignment=\"1\" applyProtection=\"1\" applyNumberFormat=\"1\" applyFont=\"1\" applyFill=\"1\" applyBorder=\"1\"/>".into()],
            index: HashMap::new(),
            lookup: HashMap::new(),
            plain: None,
        }
    }

    /// The position of `xml` in `list` (one of the lists above), adding it when new.
    fn intern(list: &mut Vec<String>, lookup: &mut HashMap<String, usize>, xml: String) -> usize {
        if let Some(&i) = lookup.get(&xml) {
            return i;
        }
        let i = match list.iter().position(|x| *x == xml) {
            Some(i) => i,
            None => {
                list.push(xml.clone());
                list.len() - 1
            }
        };
        lookup.insert(xml, i);
        i
    }

    /// The format and content of an element's corner cell.
    fn element(&mut self, node: &grid::Node<'_>, style: &CellStyle, options: &XlsxOptions) -> (usize, String) {
        match &node.what {
            What::Text(t) => self.text(t, style, options),
            What::Shape(e, ..) => {
                let border = match &e.shape {
                    Shape::Rectangle(pen) | Shape::Ellipse(pen) => Border::Sides(Some([pen.unwrap_or(grid::NO_PEN); 4])),
                    Shape::Line { pen, top_down } => line_border(node.w, node.h, *pen, *top_down),
                    Shape::Picture { .. } | Shape::Frame | Shape::Image { .. } => Border::Sides(style.boxed),
                };
                let border = match border {
                    Border::Sides(b) => self.border(b),
                    Border::Diagonal { up, pen } => self.diagonal(up, pen),
                };
                (self.xf(0, style.back.unwrap_or(Color::WHITE), border, 0, "wrapText=\"true\"", false), String::new())
            }
        }
    }

    fn diagonal(&mut self, up: bool, pen: Pen) -> usize {
        let diagonal = match border_style(pen) {
            Some(s) => format!("<diagonal style=\"{s}\"><color rgb=\"{}\"/></diagonal>", hex(pen.color)),
            None => "<diagonal></diagonal>".to_string(),
        };
        let attr = if up { "diagonalUp" } else { "diagonalDown" };
        let xml = format!("<border {attr}=\"1\"><left></left><right></right><top></top><bottom></bottom>{diagonal}</border>");
        Self::intern(&mut self.borders, &mut self.lookup, xml)
    }

    fn border(&mut self, boxed: Option<Sides>) -> usize {
        let Some([top, left, bottom, right]) = boxed else { return 0 };
        let side = |name: &str, p: Pen| match border_style(p) {
            Some(s) => format!("<{name} style=\"{s}\"><color rgb=\"{}\"/></{name}>", hex(p.color)),
            None => format!("<{name}><color rgb=\"{}\"/></{name}>", hex(p.color)),
        };
        let xml = format!(
            "<border>{}{}{}{}<diagonal></diagonal></border>",
            side("left", left),
            side("right", right),
            side("top", top),
            side("bottom", bottom)
        );
        Self::intern(&mut self.borders, &mut self.lookup, xml)
    }

    fn xf(&mut self, font: usize, back: Color, border: usize, numfmt: usize, alignment: &str, locked: bool) -> usize {
        let key = format!("{font}|{}|{border}|{numfmt}|{alignment}|{locked}", hex(back));
        if let Some(&i) = self.index.get(&key) {
            return i;
        }
        self.fills
            .push(format!("<fill><patternFill patternType=\"solid\"><fgColor rgb=\"{}\"/></patternFill></fill>", hex(back)));
        let fill = self.fills.len() - 1;
        self.xfs.push(format!(
            "<xf numFmtId=\"{numfmt}\" fontId=\"{font}\" fillId=\"{fill}\" borderId=\"{border}\" xfId=\"0\" applyAlignment=\"1\" applyProtection=\"1\" applyNumberFormat=\"1\" applyFont=\"1\" applyFill=\"1\" applyBorder=\"1\"><alignment {alignment}/><protection hidden=\"false\" locked=\"{locked}\"/></xf>"
        ));
        let i = self.xfs.len() - 1;
        self.index.insert(key, i);
        i
    }

    /// Empty cells and cells of shapes (lines, rectangles, pictures).
    fn shape_xf(&mut self, style: &CellStyle, boxed: Option<Sides>) -> usize {
        let plain = style.back.is_none() && boxed.is_none();
        if let (true, Some(xf)) = (plain, self.plain) {
            return xf;
        }
        let border = self.border(boxed);
        let xf = self.xf(0, style.back.unwrap_or(Color::WHITE), border, 0, "wrapText=\"true\"", false);
        if plain {
            self.plain = Some(xf);
        }
        xf
    }

    /// The format and content of a text cell.
    fn text(&mut self, t: &CellInfo, style: &CellStyle, options: &XlsxOptions) -> (usize, String) {
        let f = &t.font;
        let font = Self::intern(
            &mut self.fonts,
            &mut self.lookup,
            format!(
                "<font><sz val=\"{}\"/><color rgb=\"{}\"/><name val=\"{}\"/><b val=\"{}\"/><i val=\"{}\"/><u val=\"{}\"/><strike val=\"{}\"/><family val=\"2\"/></font>",
                java_float(f.size),
                hex(t.color),
                escape(&f.name),
                f.bold,
                f.italic,
                if f.underline { "single" } else { "none" },
                f.strike
            ),
        );
        let border = self.border(style.boxed);
        // Rotated text is aligned in the turned box: a left-rotated text starts at the bottom.
        let along = |start: &'static str, end: &'static str| match t.halign {
            HAlign::Left => start,
            HAlign::Center => "center",
            HAlign::Right => end,
            HAlign::Justified => "justify",
        };
        let across = |top: &'static str, bottom: &'static str| match t.valign {
            VAlign::Top => top,
            VAlign::Middle => "center",
            VAlign::Bottom => bottom,
        };
        let (horizontal, vertical) = match t.rotation {
            Rotation::Left => (across("left", "right"), along("bottom", "top")),
            Rotation::Right => (across("right", "left"), along("top", "bottom")),
            _ => (along("left", "right"), across("top", "bottom")),
        };
        let rotation = match t.rotation {
            Rotation::Left => " textRotation=\"90\"",
            Rotation::Right => " textRotation=\"180\"",
            _ => "",
        };
        let alignment = format!("wrapText=\"true\" horizontal=\"{horizontal}\" vertical=\"{vertical}\"{rotation}");
        let number = t.number.filter(|n| options.detect_cell_type && n.is_finite());
        let numfmt = match (number, &t.pattern) {
            (Some(_), Some(p)) => self.numfmt(p),
            _ => 0,
        };
        let back = style.back.unwrap_or(Color::WHITE);
        let xf = self.xf(font, back, border, numfmt, &alignment, true);
        let content = match number {
            Some(n) => format!("<v>{}</v>", java_double(n)),
            None if t.text.is_empty() => String::new(),
            None if t.runs.is_empty() => format!("<is><r>\n<t xml:space=\"preserve\">{}</t></r>\n</is>", escape(&t.text)),
            None => rich_runs(t),
        };
        (xf, content)
    }

    fn numfmt(&mut self, pattern: &str) -> usize {
        let code = excel_pattern(pattern);
        let i = Self::intern(&mut self.numfmts, &mut self.lookup, code);
        164 + i
    }

    fn xml(&self) -> String {
        let mut s = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\n<numFmts>\n<numFmt numFmtId=\"0\" formatCode=\"General\"/>\n",
        );
        for (i, code) in self.numfmts.iter().enumerate() {
            let _ = writeln!(s, "<numFmt numFmtId=\"{}\" formatCode=\"{}\"/>", 164 + i, escape(code));
        }
        s.push_str("</numFmts>\n<fonts>\n");
        for f in &self.fonts {
            s.push_str(f);
            s.push('\n');
        }
        s.push_str("</fonts>\n<fills>\n");
        for f in &self.fills {
            s.push_str(f);
            s.push('\n');
        }
        s.push_str("</fills>\n<borders>\n");
        for b in &self.borders {
            s.push_str(b);
            s.push('\n');
        }
        s.push_str("</borders>\n<cellStyleXfs count=\"1\"><xf/></cellStyleXfs>\n<cellXfs>\n");
        for x in &self.xfs {
            s.push_str(x);
            s.push('\n');
        }
        s.push_str("</cellXfs>\n<dxfs count=\"0\"/><tableStyles count=\"0\" defaultTableStyle=\"TableStyleMedium9\" defaultPivotStyle=\"PivotStyleLight16\"/>\n</styleSheet>\n");
        s
    }
}

/// A styled or HTML text as rich-text runs, each with its font (`XlsxRunHelper`).
fn rich_runs(t: &CellInfo) -> String {
    let mut out = String::from("<is>");
    for run in &t.runs {
        let Some(text) = t.text.get(run.start..run.end) else { continue };
        let st = &run.style;
        let f = &st.font;
        let _ = write!(
            out,
            "<r>\n       <rPr>\n        <rFont val=\"{}\"/>\n        <color rgb=\"{}\" />\n        <sz val=\"{}\" />\n",
            escape(&f.name),
            hex(st.color),
            java_float(f.size)
        );
        for (on, tag) in [
            (f.bold, "<b val=\"true\"/>"),
            (f.italic, "<i val=\"true\"/>"),
            (f.underline, "<u val=\"single\"/>"),
            (f.strike, "<strike val=\"true\"/>"),
            (st.script == Script::Super, "<vertAlign val=\"superscript\" />"),
            (st.script == Script::Sub, "<vertAlign val=\"subscript\" />"),
        ] {
            if on {
                let _ = writeln!(out, "        {tag}");
            }
        }
        let _ = writeln!(out, "       </rPr>\n<t xml:space=\"preserve\">{}</t></r>", escape(text));
    }
    out.push_str("</is>");
    out
}

/// The XLSX border style of a pen (`XlsxBorderInfo.borderStyle`).
fn border_style(p: Pen) -> Option<&'static str> {
    if p.width <= 0.0 {
        return None;
    }
    Some(match p.style {
        LineStyle::Double => "double",
        LineStyle::Dotted => "dotted",
        LineStyle::Dashed if p.width >= 1.0 => "mediumDashed",
        LineStyle::Dashed => "dashed",
        LineStyle::Solid if p.width >= 2.0 => "thick",
        LineStyle::Solid if p.width >= 1.0 => "medium",
        LineStyle::Solid if p.width >= 0.5 => "thin",
        LineStyle::Solid => "hair",
    })
}

/// A Java number pattern as an Excel format code. The pattern languages share digits,
/// grouping, decimals, percent and literal text; Java's currency sign has no equivalent.
fn excel_pattern(p: &str) -> String {
    p.replace('¤', "$")
}

/// Like Java's `Double.toString` for ordinary values (`3885.0`, `0.25`).
fn java_double(v: f64) -> String {
    let s = format!("{v}");
    if s.contains('.') || s.contains('e') {
        s
    } else {
        format!("{s}.0")
    }
}

const CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\n  <Default Extension=\"gif\" ContentType=\"image/gif\"/>\n  <Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/>\n  <Default Extension=\"jpg\" ContentType=\"image/jpeg\"/>\n  <Default Extension=\"png\" ContentType=\"image/png\"/>\n  <Default Extension=\"tiff\" ContentType=\"image/tiff\"/>\n  <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\n  <Default Extension=\"xml\" ContentType=\"application/xml\"/>\n  <Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>\n  <Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>\n  <Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>\n  <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\n  <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\n  <Override PartName=\"/xl/drawings/drawing1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.drawing+xml\"/>\n</Types>\n";

const ROOT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/> \n<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/>\n<Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>\n</Relationships>\n";

const CORE: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<coreProperties xmlns=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n</coreProperties>\n";

const WORKBOOK_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n <Relationship Id=\"rIdSt\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\n <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\n</Relationships>\n";

const SHEET_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n <Relationship Id=\"rIdDr1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing\" Target=\"../drawings/drawing1.xml\"/>\n</Relationships>\n";

/// Images placed over the cells they cover (`JRXlsxExporter.exportImage`). Charts and
/// barcodes are drawn into images first.
struct Pictures<'f> {
    fonts: &'f FontSet,
    anchors: String,
    /// Media files: name and bytes, each image once.
    media: Vec<(String, Vec<u8>)>,
    names: HashMap<usize, String>,
}

impl Pictures<'_> {
    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        e: &ElementInfo,
        drawn: &[PrintItem],
        w: i32,
        h: i32,
        col: usize,
        row: usize,
        cols: usize,
        rows: usize,
        page: usize,
    ) {
        // The image sits inside the padding, or inside the border when that is wider.
        let b = e.boxed.unwrap_or_else(|| BoxStyle::plain(e.fore));
        let pad = |p: f32, pen: Pen| (p.round() as i32).max(border_correction(pen));
        let (top, left) = (pad(b.padding.top, b.top), pad(b.padding.left, b.left));
        let (bottom, right) = (pad(b.padding.bottom, b.bottom), pad(b.padding.right, b.right));
        let aw = (w - left - right).max(0) as f64;
        let ah = (h - top - bottom).max(0) as f64;
        let (name, scale, halign, valign, nw, nh) = match &e.shape {
            Shape::Image { data: Some(data), scale, halign, valign } => {
                let Some(name) = self.media_name(data, page) else { return };
                (name, *scale, *halign, *valign, data.width as f64, data.height as f64)
            }
            Shape::Picture { symbol } if aw > 0.0 && ah > 0.0 => {
                // A barcode's picture is its symbol, which keeps its shape in the cells.
                let (pw, ph, scale) = match symbol {
                    Some((sw, sh)) if *sw > 0.0 && *sh > 0.0 => {
                        let k = (aw / sw).min(ah / sh);
                        (sw * k, sh * k, ScaleImage::RetainShape)
                    }
                    _ => (aw, ah, ScaleImage::FillFrame),
                };
                let area = [e.x + left as f32, e.y + top as f32, pw as f32, ph as f32];
                let Some(png) = crate::raster::render_png(drawn, self.fonts, area, PICTURE_SCALE) else { return };
                let name = format!("img_0_{page}_{}.png", self.media.len());
                self.media.push((name.clone(), png));
                (name, scale, HAlign::Left, VAlign::Top, pw, ph)
            }
            _ => return,
        };
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
        // Insets of the image from the cell area, in thousandths of a percent (negative when
        // the image overflows and is clipped).
        let share = |gap: f64, of: f64| if of > 0.0 { (gap / of * 100_000.0) as i64 } else { 0 };
        let (l, t, r, bt) = match scale {
            ScaleImage::FillFrame => (0, 0, 0, 0),
            ScaleImage::Clip => (
                share(xa * (aw - nw), aw),
                share(ya * (ah - nh), ah),
                share((1.0 - xa) * (aw - nw), aw),
                share((1.0 - ya) * (ah - nh), ah),
            ),
            ScaleImage::RetainShape => {
                // The fitted size is in whole points.
                if nh > 0.0 && ah > 0.0 && nw / nh > aw / ah {
                    let fit = (aw * nh / nw).trunc();
                    (0, share(ya * (ah - fit), ah), 0, share((1.0 - ya) * (ah - fit), ah))
                } else {
                    let fit = if nh > 0.0 { (ah * nw / nh).trunc() } else { aw };
                    (share(xa * (aw - fit), aw), 0, share((1.0 - xa) * (aw - fit), aw), 0)
                }
            }
        };
        let emu = |pt: i32| pt * 12700;
        let id = self.media.len() * 1000 + self.anchors.len() % 1000 + 1;
        let _ = write!(
            self.anchors,
            "<xdr:twoCellAnchor editAs=\"oneCell\">\n<xdr:from><xdr:col>{col}</xdr:col><xdr:colOff>{}</xdr:colOff><xdr:row>{row}</xdr:row><xdr:rowOff>{}</xdr:rowOff></xdr:from>\n<xdr:to><xdr:col>{}</xdr:col><xdr:colOff>{}</xdr:colOff><xdr:row>{}</xdr:row><xdr:rowOff>{}</xdr:rowOff></xdr:to>\n<xdr:pic>\n<xdr:nvPicPr><xdr:cNvPr id=\"{id}\" name=\"Picture\">\n</xdr:cNvPr><xdr:cNvPicPr/></xdr:nvPicPr>\n<xdr:blipFill>\n<a:blip r:embed=\"{name}\"/><a:srcRect/><a:stretch><a:fillRect l=\"{l}\" t=\"{t}\" r=\"{r}\" b=\"{bt}\"/></a:stretch>\n</xdr:blipFill>\n<xdr:spPr>\n  <a:xfrm rot=\"0\">\n    <a:off x=\"0\" y=\"0\"/>\n    <a:ext cx=\"0\" cy=\"0\"/>  </a:xfrm>\n<a:prstGeom prst=\"rect\"></a:prstGeom>\n</xdr:spPr>\n</xdr:pic>\n<xdr:clientData/>\n</xdr:twoCellAnchor>\n",
            emu(left),
            emu(top),
            col + cols,
            -emu(right),
            row + rows,
            -emu(bottom),
        );
    }

    /// The media file of an image, written once however often it is used.
    fn media_name(&mut self, data: &ImageData, page: usize) -> Option<String> {
        if let Some(n) = self.names.get(&data.id) {
            return Some(n.clone());
        }
        let (ext, bytes) = match &data.pixels {
            Pixels::Jpeg { data, .. } => ("jpg", data.clone()),
            Pixels::Raw { samples, gray, alpha } => {
                ("png", encode_png(data.width, data.height, samples, *gray, alpha.as_deref())?)
            }
        };
        let name = format!("img_0_{page}_{}.{ext}", self.media.len());
        self.media.push((name.clone(), bytes));
        self.names.insert(data.id, name.clone());
        Some(name)
    }

    fn drawing(&self) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<xdr:wsDr\n xmlns:xdr=\"http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing\"\n xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"\n xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\n{}</xdr:wsDr>",
            self.anchors
        )
    }

    fn rels(&self) -> String {
        let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n");
        for (name, _) in &self.media {
            let _ = writeln!(
                s,
                " <Relationship Id=\"{name}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"../media/{name}\"/>"
            );
        }
        s.push_str("</Relationships>\n");
        s
    }
}

/// Pixels per point of the pictures drawn for charts and barcodes.
const PICTURE_SCALE: f32 = 2.0;

/// Room an image leaves for its border (`ExcelAbstractExporter.getImageBorderCorrection`).
fn border_correction(pen: Pen) -> i32 {
    if pen.width <= 0.0 {
        0
    } else if pen.width >= 2.0 {
        2
    } else {
        1
    }
}

/// Encodes decoded samples (gray or RGB, optional alpha) as PNG.
fn encode_png(width: u32, height: u32, samples: &[u8], gray: bool, alpha: Option<&[u8]>) -> Option<Vec<u8>> {
    let channels = if gray { 1 } else { 3 };
    let (color, data) = match alpha {
        None => (if gray { png::ColorType::Grayscale } else { png::ColorType::Rgb }, samples.to_vec()),
        Some(a) => {
            let mut out = Vec::with_capacity(samples.len() + a.len());
            for (px, &alpha) in samples.chunks(channels).zip(a) {
                out.extend_from_slice(px);
                out.push(alpha);
            }
            (if gray { png::ColorType::GrayscaleAlpha } else { png::ColorType::Rgba }, out)
        }
    };
    let mut bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut bytes, width, height);
        enc.set_color(color);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().ok()?;
        w.write_image_data(&data).ok()?;
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Our workbook parts for a fixture, next to the ones JasperReports 6.20 wrote for it
    /// (kept in `fixtures/<name>/jasper-xlsx/`, so this runs without Java).
    fn check_fixture(name: &str) {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(name);
        let read = |f: &str| std::fs::read_to_string(dir.join(f));
        let template = read("report.jrxml").unwrap();
        let rows: serde_json::Value =
            read("data.json").map(|t| serde_json::from_str(&t).unwrap()).unwrap_or(serde_json::json!([]));
        let params: serde_json::Value =
            read("params.json").map(|t| serde_json::from_str(&t).unwrap()).unwrap_or(serde_json::json!({}));
        let mut r = crate::Renderer::new().resource_dir(&dir).now(crate::data::parse_datetime("2026-01-15T10:30:00").unwrap());
        let report = crate::parse(&template).unwrap();
        let records = crate::data::records_from_json(&report, &rows).unwrap();
        let params = crate::params_from_json(&report, &params).unwrap();
        let doc = r.fill(&report, &records, params).unwrap();
        let parts = workbook_parts(&doc, &r.fonts, &XlsxOptions::default());
        for (part, file) in [("xl/worksheets/sheet1.xml", "sheet1.xml"), ("xl/styles.xml", "styles.xml")] {
            let ours = parts.iter().find(|(n, _)| n == part).map(|(_, d)| String::from_utf8_lossy(d).to_string());
            let theirs = read(&format!("jasper-xlsx/{file}")).unwrap();
            assert!(ours.as_deref() == Some(theirs.as_str()), "{name}: {part} differs from JasperReports'");
        }
        let csv = crate::csv::write_csv(&doc, &Default::default());
        assert_eq!(csv, read("jasper-xlsx/report.csv").unwrap(), "{name}: CSV differs from JasperReports'");
    }

    #[test]
    fn invoice_matches_jasper() {
        check_fixture("02-invoice");
    }

    #[test]
    fn shapes_match_jasper() {
        check_fixture("08-styles-and-shapes");
    }

    #[test]
    fn markup_matches_jasper() {
        check_fixture("21-markup");
    }

    #[test]
    fn table_groups_match_jasper() {
        check_fixture("28-table-groups");
    }

    #[test]
    fn crosstab_matches_jasper() {
        check_fixture("30-crosstab");
    }

    #[test]
    fn column_names() {
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(25), "Z");
        assert_eq!(column_name(26), "AA");
        assert_eq!(column_name(701), "ZZ");
        assert_eq!(column_name(702), "AAA");
    }

    #[test]
    fn floats_print_like_java() {
        assert_eq!(java_float(40.0 / 6.0), "6.6666665");
        assert_eq!(java_float(2.0), "2.0");
        assert_eq!(java_float(9.0), "9.0");
    }
}
