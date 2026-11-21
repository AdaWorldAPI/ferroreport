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
