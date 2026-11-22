//! Streaming renders must give the same documents as rendering in memory, hand pages over
//! while rows are still arriving, and stop at the limits set for them.

use ferroreport::{Format, JsonRows, Limits, Renderer, RowIter};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(name)
}

fn now() -> chrono::NaiveDateTime {
    chrono::NaiveDate::from_ymd_opt(2026, 1, 15).unwrap().and_hms_opt(10, 30, 0).unwrap()
}

struct Loaded {
    dir: PathBuf,
    report: ferroreport::Report,
    data: String,
    params: serde_json::Value,
}

fn load(name: &str) -> Loaded {
    let dir = fixture(name);
    let report = ferroreport::parse(&std::fs::read_to_string(dir.join("report.jrxml")).unwrap()).unwrap();
    let data = std::fs::read_to_string(dir.join("data.json")).unwrap_or_else(|_| "[]".into());
    let params = std::fs::read_to_string(dir.join("params.json")).map(|p| serde_json::from_str(&p).unwrap()).unwrap_or_default();
    Loaded { dir, report, data, params }
}

impl Loaded {
    fn renderer(&self) -> Renderer {
        Renderer::new().resource_dir(&self.dir).now(now())
    }

    fn in_memory(&self, format: Format) -> Vec<u8> {
        let rows = serde_json::from_str(&self.data).unwrap();
        let records = ferroreport::data::records_from_json(&self.report, &rows).unwrap();
        let params = ferroreport::params_from_json(&self.report, &self.params).unwrap();
        let mut r = self.renderer();
        let doc = r.fill(&self.report, &records, params).unwrap();
        r.export(&doc, format).unwrap()
    }

    fn streamed(&self, format: Format) -> (Vec<u8>, ferroreport::FillStats) {
        let rows: RowIter = Box::new(JsonRows::new(&self.report, self.data.as_bytes()));
        let params = ferroreport::params_from_json(&self.report, &self.params).unwrap();
        let mut out = Vec::new();
        let stats = self.renderer().render_to(&self.report, rows, params, format, &mut out).unwrap();
        (out, stats)
    }
}

fn pages_of(pdf: &[u8]) -> usize {
    let text = String::from_utf8_lossy(pdf);
    text.matches("/Type /Page\n").count() + text.matches("/Type /Page ").count() + text.matches("/Type /Page/").count()
}

#[test]
fn every_fixture_streams_to_the_same_document() {
    let mut checked = 0;
    for entry in std::fs::read_dir(fixture("")).unwrap() {
        let dir = entry.unwrap().path();
        if !dir.join("expected.json").exists() {
            continue;
        }
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        let f = load(&name);
        let want: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("expected.json")).unwrap()).unwrap();
        let (pdf, stats) = f.streamed(Format::Pdf);
        assert_eq!(stats.pages as u64, want["pages"].as_u64().unwrap(), "{name}: pages");
        assert_eq!(pages_of(&pdf), stats.pages.max(1), "{name}: page objects");
        assert!(pdf.ends_with(b"%%EOF"), "{name}");
        assert_eq!(f.streamed(Format::Csv).0, f.in_memory(Format::Csv), "{name}: CSV");
        assert_eq!(f.streamed(Format::Xlsx).0, f.in_memory(Format::Xlsx), "{name}: XLSX");
        let html = String::from_utf8(f.streamed(Format::Html).0).unwrap();
        assert_eq!(html.matches("<svg class=\"page\"").count(), stats.pages, "{name}: HTML pages");
        assert_eq!(
            html.matches("</svg>\n").count() >= stats.pages,
            html.ends_with("</body>\n</html>\n"),
            "{name}: HTML is closed"
        );
        checked += 1;
    }
    assert!(checked >= 30, "only {checked} fixtures checked");
}

#[test]
fn a_page_whose_late_value_is_drawn_over_waits_for_it() {
    // The total in the title has a stamp drawn over it and the page count sits in a frame
    // with a line across: drawing them last would change the page, so nothing is written
    // early and the PDF is the one the in-memory path gives.
    let f = load("34-late-values-overlap");
    assert_eq!(f.streamed(Format::Pdf).0, f.in_memory(Format::Pdf));
}

#[test]
fn page_x_of_y_does_not_hold_pages_back() {
    let f = load("33-page-x-of-y");
    let (pdf, stats) = f.streamed(Format::Pdf);
    assert_eq!(stats.pages, 5);
    // Every page but the last was written before the page count was known, and got it (and
    // the first page its grand total) as a content stream of its own.
    let text = String::from_utf8_lossy(&pdf);
    assert_eq!(text.matches("/Contents [").count(), 4, "pages with values added later");

    // Pages leave while rows are still being read.
    struct Watch {
        read: Rc<Cell<usize>>,
        first_page_after: Option<usize>,
        pages: usize,
        late: usize,
    }
    impl ferroreport::PageSink for Watch {
        fn accepts_late(&self) -> bool {
            true
        }
        fn page(
            &mut self,
            _page: ferroreport::print::PrintPage,
            _fonts: &ferroreport::fonts::FontSet,
        ) -> ferroreport::Result<()> {
            self.first_page_after.get_or_insert(self.read.get());
            self.pages += 1;
            Ok(())
        }
        fn late(
            &mut self,
            _page: usize,
            _item: ferroreport::print::PrintItem,
            _fonts: &ferroreport::fonts::FontSet,
        ) -> ferroreport::Result<()> {
            self.late += 1;
            Ok(())
        }
    }
    let read = Rc::new(Cell::new(0));
    let counter = read.clone();
    let rows: RowIter = Box::new(JsonRows::new(&f.report, f.data.as_bytes()).inspect(move |_| counter.set(counter.get() + 1)));
    let mut watch = Watch { read, first_page_after: None, pages: 0, late: 0 };
    let mut r = f.renderer();
    let stats = r.fill_to(&f.report, rows, Default::default(), &mut watch).unwrap();
    assert_eq!((stats.pages, stats.rows, watch.pages), (5, 190, 5));
    assert!(watch.late >= 5, "four page counts, the grand total and group totals that end on a later page: {}", watch.late);
    assert!(watch.first_page_after.unwrap() < 100, "first page left after {:?} of 190 rows", watch.first_page_after);
}

#[test]
fn limits_stop_a_render() {
    let f = load("33-page-x-of-y");
    let render = |limits: Limits| {
        let rows: RowIter = Box::new(JsonRows::new(&f.report, f.data.as_bytes()));
        f.renderer().limits(limits).render_to(&f.report, rows, Default::default(), Format::Pdf, Vec::new())
    };
    assert!(render(Limits { max_pages: Some(5), ..Default::default() }).is_ok());
    let err = render(Limits { max_pages: Some(4), ..Default::default() }).unwrap_err();
    assert!(matches!(err, ferroreport::Error::Limit(_)), "{err}");
    let err = render(Limits { timeout: Some(std::time::Duration::ZERO), ..Default::default() }).unwrap_err();
    assert!(matches!(err, ferroreport::Error::Limit(_)), "{err}");
}

#[test]
fn bad_rows_stop_a_render_with_their_position() {
    let f = load("33-page-x-of-y");
    let data = r#"[{"id": 1, "name": "a", "salary": 1}, {"id": "x", "name": "b", "salary": 2}]"#;
    let rows: RowIter = Box::new(JsonRows::new(&f.report, data.as_bytes()));
    let err = f.renderer().render_to(&f.report, rows, Default::default(), Format::Pdf, Vec::new()).unwrap_err();
    assert!(err.to_string().contains("row 1"), "{err}");
}

#[test]
fn html_pages_carry_the_report_as_text_and_escape_it() {
    let f = load("02-invoice");
    let html = String::from_utf8(f.streamed(Format::Html).0).unwrap();
    assert!(html.starts_with("<!DOCTYPE html>") && html.contains("<title>invoice</title>"));
    // Text is text (selectable, searchable), with its font and measured width.
    assert!(html.contains(">Ferro Industrial SRL</text>"), "{}", &html[..600]);
    assert!(html.contains("font-family=\"'DejaVu Sans',sans-serif\"") && html.contains("textLength="));
    // Data cannot inject markup.
    let data = r#"[{"sku": "<script>alert(1)</script>", "description": "a & b", "qty": 1, "price": "1.00", "discount": "0"}]"#;
    let rows: RowIter = Box::new(JsonRows::new(&f.report, data.as_bytes()));
    let params = ferroreport::params_from_json(&f.report, &f.params).unwrap();
    let mut out = Vec::new();
    f.renderer().render_to(&f.report, rows, params, Format::Html, &mut out).unwrap();
    let html = String::from_utf8(out).unwrap();
    // (the narrow SKU column breaks the text over lines, so only its start is on one of them)
    assert!(!html.contains("<script") && html.contains("&lt;scr") && html.contains("a &amp; b"), "{html}");
    // A fragment is just the pages.
    let rows: RowIter = Box::new(JsonRows::new(&f.report, f.data.as_bytes()));
    let params = ferroreport::params_from_json(&f.report, &f.params).unwrap();
    let mut out = Vec::new();
    let options = ferroreport::html::HtmlOptions { fragment: true, ..Default::default() };
    f.renderer().html_options(options).render_to(&f.report, rows, params, Format::Html, &mut out).unwrap();
    assert!(out.starts_with(b"<svg class=\"page\""));
}

/// A workbook too large to keep is made in two passes over its rows, and comes out the same
/// as when its pages are all in memory.
#[test]
fn a_large_workbook_is_written_in_two_passes_and_is_the_same() {
    let large = load("09-large");
    let few: Vec<serde_json::Value> = serde_json::from_str(&large.data).unwrap();
    let rows: Vec<serde_json::Value> = (0..40_000).map(|i| few[i % few.len()].clone()).collect();
    let lines: String = rows.iter().map(|r| format!("{r}\n")).collect();
    let records = ferroreport::data::records_from_json(&large.report, &serde_json::Value::Array(rows)).unwrap();

    let mut r = large.renderer();
    let doc = r.fill(&large.report, &records, Default::default()).unwrap();
    let kept = r.to_xlsx(&doc, &Default::default()).unwrap();
    drop(doc);

    let asked = Cell::new(0);
    let mut again = || -> ferroreport::Result<RowIter> {
        asked.set(asked.get() + 1);
        Ok(Box::new(JsonRows::new(&large.report, lines.as_bytes())))
    };
    let mut streamed = Vec::new();
    let stats =
        large.renderer().render_rows_to(&large.report, &mut again, Default::default(), Format::Xlsx, &mut streamed).unwrap();
    assert_eq!(asked.get(), 2, "a report this size reads its rows twice");
    assert_eq!(stats.rows, 40_000);
    assert!(streamed == kept, "the workbook written in two passes differs from the one written from memory");

    // A small one is filled once, and other formats always are.
    for (format, rows) in [(Format::Xlsx, &lines[..2_000]), (Format::Csv, &lines[..])] {
        let rows = &rows[..rows.rfind('\n').unwrap() + 1];
        asked.set(0);
        let mut again = || -> ferroreport::Result<RowIter> {
            asked.set(asked.get() + 1);
            Ok(Box::new(JsonRows::new(&large.report, rows.as_bytes())))
        };
        large.renderer().render_rows_to(&large.report, &mut again, Default::default(), format, std::io::sink()).unwrap();
        assert_eq!(asked.get(), 1, "{format:?}");
    }
}

/// A tagged PDF carries a structure tree over the same pages: a paragraph per text and a
/// figure per picture, with the document's language and title.
#[test]
fn a_tagged_pdf_describes_its_content() {
    let count = |pdf: &[u8], what: &str| pdf.windows(what.len()).filter(|w| *w == what.as_bytes()).count();
    for (name, figures) in [("33-page-x-of-y", 0), ("15-images", 10)] {
        let fx = load(name);
        let plain = fx.streamed(Format::Pdf).0;
        let mut out = Vec::new();
        let rows: RowIter = Box::new(JsonRows::new(&fx.report, fx.data.as_bytes()));
        let params = ferroreport::params_from_json(&fx.report, &fx.params).unwrap();
        fx.renderer().tagged(true).render_to(&fx.report, rows, params, Format::Pdf, &mut out).unwrap();
        assert_eq!(pages_of(&out), pages_of(&plain), "{name}");
        for key in
            ["/Type /StructTreeRoot", "/MarkInfo", "/Lang (en-US)", "/DisplayDocTitle true", "/ParentTreeNextKey", "/S /Document"]
        {
            assert_eq!(count(&out, key), 1, "{name}: {key}");
        }
        assert!(count(&out, "/S/P/P ") > 0, "{name}: paragraphs");
        assert_eq!(count(&out, "/S /Figure"), figures, "{name}: figures");
        assert_eq!(count(&out, "/Alt ("), figures, "{name}: every figure says what it shows");
        assert_eq!(count(&out, "/StructParents"), count(&out, "/S /Sect"), "{name}: every page with content has its section");
        assert_eq!(count(&plain, "/StructTreeRoot"), 0);
    }
}
