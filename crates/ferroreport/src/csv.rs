//! CSV export, following JasperReports' `JRCsvExporter`: texts are laid out on a grid cut at
//! their edges, and every row and column where a text starts becomes a CSV row and field.

use crate::grid::{self, Cell, Nature, What};
use crate::print::PrintDocument;

/// Options for [`write_csv`].
#[derive(Debug, Clone)]
pub struct CsvOptions {
    /// Field delimiter (`,` by default).
    pub delimiter: String,
    /// Record separator (`\n` by default).
    pub record_delimiter: String,
}

impl Default for CsvOptions {
    fn default() -> Self {
        CsvOptions { delimiter: ",".into(), record_delimiter: "\n".into() }
    }
}

const NATURE: Nature = Nature { span: false, texts_only: true };

/// Exports a filled document to CSV text.
pub fn write_csv(doc: &PrintDocument, options: &CsvOptions) -> String {
    let pages: Vec<_> = doc.pages.iter().map(|p| grid::nodes(&p.items)).collect();
    let width = doc.page_width.round() as i32;
    let height = doc.page_height.round() as i32;
    let mut out = String::new();
    for nodes in &pages {
        // Each page has its own columns: those where one of its texts starts.
        let mut xcuts = grid::x_cuts(std::slice::from_ref(nodes), width, NATURE);
        let g = grid::layout(nodes, &mut xcuts, height, NATURE);
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
    out
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
