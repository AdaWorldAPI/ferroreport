//! CSV export, following JasperReports' `JRCsvExporter`: texts are laid out on a grid cut at
//! their edges, and every row and column where a text starts becomes a CSV row and field.

use crate::grid::{self, Cell, Nature, What};
use crate::print::{PrintDocument, PrintItem};

/// Options for [`write_csv`].
#[derive(Debug, Clone)]
pub struct CsvOptions {
    /// Field delimiter (`,` by default).
    pub delimiter: String,
    /// Record separator (`\n` by default).
    pub record_delimiter: String,
    /// Put an apostrophe before fields a spreadsheet would run as a formula (those starting
    /// with `=`, `+`, `-`, `@`, a tab or a carriage return, unless they are plain numbers).
    /// Off by default, like JasperReports; turn it on when the data comes from other people.
    pub guard_formulas: bool,
}

impl Default for CsvOptions {
    fn default() -> Self {
        CsvOptions { delimiter: ",".into(), record_delimiter: "\n".into(), guard_formulas: false }
    }
}

const NATURE: Nature = Nature { span: false, texts_only: true };

/// Exports a filled document to CSV text.
pub fn write_csv(doc: &PrintDocument, options: &CsvOptions) -> String {
    let mut out = String::new();
    for page in &doc.pages {
        page_csv(&page.items, doc.page_width, doc.page_height, options, &mut out);
    }
    out
}

/// Appends the CSV rows of one page.
pub fn page_csv(items: &[PrintItem], page_width: f32, page_height: f32, options: &CsvOptions, out: &mut String) {
    let mut nodes = grid::nodes(items);
    // A text with nothing in it takes no place in the CSV grid, as in JasperReports.
    nodes.retain(|n| !matches!(&n.what, What::Text(t) if t.text.is_empty()));
    let width = page_width.round() as i32;
    let height = page_height.round() as i32;
    // Each page has its own columns: those where one of its texts starts.
    let mut xcuts = grid::x_cuts(std::slice::from_ref(&nodes), width, NATURE);
    let g = grid::layout(&nodes, &mut xcuts, height, NATURE);
    for (ri, cells) in g.cells.iter().enumerate() {
        if !g.rows.used[ri] {
            continue;
        }
        let mut fields = Vec::new();
        for (ci, cell) in cells.iter().enumerate() {
            if !xcuts.used[ci] {
                continue;
            }
            let text = match cell {
                Cell::Element { node, .. } => match &node.what {
                    What::Text(t) if options.guard_formulas && is_formula(&t.text) => {
                        quote(&format!("'{}", t.text), &options.delimiter)
                    }
                    What::Text(t) => quote(&t.text, &options.delimiter),
                    What::Shape(..) => String::new(),
                },
                _ => String::new(),
            };
            fields.push(text);
        }
        out.push_str(&fields.join(&options.delimiter));
        out.push_str(&options.record_delimiter);
    }
}

/// Whether a spreadsheet would treat `text` as a formula when it opens the CSV.
fn is_formula(text: &str) -> bool {
    match text.chars().next() {
        Some('=' | '@' | '\t' | '\r') => true,
        // A negative or signed number is data, not a formula.
        Some('+' | '-') => {
            text[1..].trim().replace([',', '.', ' ', '%'], "").chars().any(|c| !c.is_ascii_digit()) || text.len() == 1
        }
        _ => false,
    }
}

/// Writes CSV to `out` page by page.
pub struct CsvStream<W: std::io::Write> {
    out: W,
    options: CsvOptions,
    page_width: f32,
    page_height: f32,
    buf: String,
}

impl<W: std::io::Write> CsvStream<W> {
    pub fn new(out: W, page_width: f32, page_height: f32, options: CsvOptions) -> CsvStream<W> {
        CsvStream { out, options, page_width, page_height, buf: String::new() }
    }

    pub fn finish(mut self) -> crate::Result<W> {
        self.out.flush().map_err(|e| crate::Error::Io(e.to_string()))?;
        Ok(self.out)
    }
}

impl<W: std::io::Write> crate::fill::PageSink for CsvStream<W> {
    fn page(&mut self, page: crate::print::PrintPage, _fonts: &crate::fonts::FontSet) -> crate::Result<()> {
        self.buf.clear();
        page_csv(&page.items, self.page_width, self.page_height, &self.options, &mut self.buf);
        self.out.write_all(self.buf.as_bytes()).map_err(|e| crate::Error::Io(e.to_string()))
    }
}

/// Quotes a field holding the delimiter, a quote or a line break (`JRCsvExporter.prepareText`).
fn quote(text: &str, delimiter: &str) -> String {
    let needs = (!delimiter.is_empty() && text.contains(delimiter)) || text.contains(['"', '\n', '\r']);
    if needs {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::is_formula;

    #[test]
    fn spots_formulas_but_not_numbers() {
        for f in ["=SUM(A1:A9)", "@cmd", "+cmd|' /C calc'!A0", "-2+3+cmd|' /C calc'!A0", "\tx", "-"] {
            assert!(is_formula(f), "{f}");
        }
        for ok in ["-12.50", "+1,200.00", "- 5", "-15%", "Total", "a=b", ""] {
            assert!(!is_formula(ok), "{ok}");
        }
    }
}
