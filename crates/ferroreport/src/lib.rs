//! # ferroreport
//!
//! Renders JasperReports JRXML templates to PDF, Excel (XLSX) and CSV without a JVM.
//!
//! ```no_run
//! let template = std::fs::read_to_string("invoice.jrxml")?;
//! let data: serde_json::Value = serde_json::from_str(&std::fs::read_to_string("rows.json")?)?;
//! let pdf = ferroreport::Renderer::new().render_json(&template, &data, &Default::default())?;
//! std::fs::write("invoice.pdf", pdf)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! The expression language is interpreted from an allow-list of operations: templates cannot
//! load classes, deserialize objects or touch the file system, so rendering templates from
//! untrusted sources is safe.

pub mod barcode;
pub mod chart;
pub mod crosstab;
pub mod csv;
pub mod data;
pub mod docx;
pub mod error;
pub mod expr;
pub mod fill;
pub mod fonts;
pub mod format;
mod grid;
pub mod html;
pub mod image;
pub mod jrxml;
pub mod model;
pub mod pdf;
pub mod print;
mod raster;
pub mod rich;
pub mod rtf;
pub mod standard;
pub mod svg;
pub mod table;
pub mod text;
pub mod value;
pub mod xlsx;
mod xml;
mod zip;

pub use data::{JsonRows, Record};
pub use error::{Error, Result};
pub use fill::{FillStats, Limits, PageSink, RowIter};
pub use format::Locale;
pub use model::Report;
pub use print::PrintDocument;
pub use value::Value;

use chrono::NaiveDateTime;
use fonts::FontSet;
use std::collections::HashMap;

/// Parses a JRXML template. Style templates it uses (`<template>`, `.jrtx` files) are not
/// read: use [`Renderer::parse`], which finds them where it finds images and subreports.
pub fn parse(template: &str) -> Result<Report> {
    jrxml::parse_report(template)
}

/// An output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    #[default]
    Pdf,
    /// Excel workbook, laid out like JasperReports' XLSX exporter.
    Xlsx,
    /// Comma-separated values, laid out like JasperReports' CSV exporter.
    Csv,
    /// A web page with every page of the report drawn as it is in the PDF, for previews.
    Html,
    /// Word document, laid out like JasperReports' DOCX exporter.
    Docx,
}

impl Format {
    /// `pdf`, `xlsx`, `csv`, `html` or `docx` (any case).
    pub fn parse(name: &str) -> Option<Format> {
        match name.to_ascii_lowercase().as_str() {
            "pdf" => Some(Format::Pdf),
            "xlsx" | "excel" => Some(Format::Xlsx),
            "csv" => Some(Format::Csv),
            "html" | "htm" => Some(Format::Html),
            "docx" | "word" => Some(Format::Docx),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Pdf => "pdf",
            Format::Xlsx => "xlsx",
            Format::Csv => "csv",
            Format::Html => "html",
            Format::Docx => "docx",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Format::Pdf => "application/pdf",
            Format::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            Format::Csv => "text/csv; charset=utf-8",
            Format::Html => "text/html; charset=utf-8",
            Format::Docx => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        }
    }
}

/// Renders one report: parse -> fill -> export.
pub struct Renderer {
    fonts: FontSet,
    images: image::ImageStore,
    locale: &'static Locale,
    now: Option<NaiveDateTime>,
    limits: Limits,
    xlsx: xlsx::XlsxOptions,
    csv: csv::CsvOptions,
    html: html::HtmlOptions,
    tagged: bool,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    pub fn new() -> Renderer {
        Renderer {
            fonts: FontSet::new(),
            images: image::ImageStore::new(),
            locale: &format::locale::EN_US,
            now: None,
            limits: Limits::default(),
            xlsx: Default::default(),
            tagged: false,
            csv: Default::default(),
            html: Default::default(),
        }
    }

    /// Sets the locale used for number and date formatting (`en_US`, `es_DO`, `es_ES`, ...).
    pub fn locale(mut self, tag: &str) -> Renderer {
        self.locale = Locale::from_tag(tag);
        self
    }

    /// Fixes the value of `new java.util.Date()` (useful for reproducible output).
    pub fn now(mut self, now: NaiveDateTime) -> Renderer {
        self.now = Some(now);
        self
    }

    /// Stops a render that goes over `limits` (pages, time) with [`Error::Limit`].
    pub fn limits(mut self, limits: Limits) -> Renderer {
        self.limits = limits;
        self
    }

    /// Writes PDFs that say what their content is (tagged PDF), for screen readers and for
    /// software that extracts text: each text element is a paragraph and each image, chart
    /// and barcode a figure, in reading order; lines, borders and backgrounds are marked as
    /// decoration. An image is described by its `hyperlinkTooltipExpression`. The document
    /// carries its title and the language of the renderer's locale.
    pub fn tagged(mut self, tagged: bool) -> Renderer {
        self.tagged = tagged;
        self
    }

    fn pdf_options(&self, title: &str, page_offsets: [[f32; 2]; 2]) -> pdf::PdfOptions {
        pdf::PdfOptions {
            title: Some(title.to_string()),
            compress: true,
            tagged: self.tagged,
            lang: Some(self.locale.tag.replace('_', "-")),
            page_offsets,
        }
    }

    /// Options for Excel output from [`Renderer::export`] and the `render` methods.
    pub fn xlsx_options(mut self, options: xlsx::XlsxOptions) -> Renderer {
        self.xlsx = options;
        self
    }

    /// Options for CSV output from [`Renderer::export`] and the `render` methods.
    pub fn csv_options(mut self, options: csv::CsvOptions) -> Renderer {
        self.csv = options;
        self
    }

    /// Options for HTML output from [`Renderer::export`] and the `render` methods.
    pub fn html_options(mut self, options: html::HtmlOptions) -> Renderer {
        self.html = options;
        self
    }

    /// Adds a directory where relative resource paths (images, subreports) are looked up. Paths in a
    /// template can never leave these directories.
    pub fn resource_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Renderer {
        self.images.add_root(dir);
        self
    }

    /// Adds a resource (image or subreport template) held in memory, under the relative path
    /// templates use for it. Useful where there is no file system (WebAssembly) or when
    /// templates come from a database.
    pub fn resource(mut self, name: &str, data: impl Into<std::sync::Arc<[u8]>>) -> Renderer {
        self.images.add_file(name, data);
        self
    }

    /// Like [`Renderer::resource`], for a renderer already built.
    pub fn add_resource(&mut self, name: &str, data: impl Into<std::sync::Arc<[u8]>>) {
        self.images.add_file(name, data);
    }

    /// Registers an additional TrueType/OpenType font family.
    pub fn register_font(&mut self, family: &str, bold: bool, italic: bool, data: Vec<u8>) -> Result<()> {
        self.fonts.register(family, bold, italic, data)
    }

    /// Fills a parsed report into a print document.
    pub fn fill(&mut self, report: &Report, records: &[Record], params: HashMap<String, Value>) -> Result<PrintDocument> {
        let env = self.env(report);
        fill::fill_slice(report, records, params, &mut self.fonts, &mut self.images, &env, self.limits)
    }

    fn env(&self, report: &Report) -> expr::EvalEnv<'static> {
        expr::EvalEnv {
            locale: self.locale,
            groovy: report.language == model::Language::Groovy,
            now: self.now.unwrap_or_else(|| chrono::Utc::now().naive_utc()),
        }
    }

    /// Fills a report whose rows arrive one at a time, handing each page to `sink` as soon as
    /// it is finished. Use it to write a format of your own, or to send pages somewhere
    /// as they are made.
    pub fn fill_to(
        &mut self,
        report: &Report,
        rows: RowIter,
        params: HashMap<String, Value>,
        sink: &mut dyn PageSink,
    ) -> Result<FillStats> {
        let env = self.env(report);
        fill::fill_to(report, rows, params, &mut self.fonts, &mut self.images, &env, self.limits, sink)
    }

    /// Renders a report whose rows arrive one at a time, writing the output as it goes:
    /// PDF and CSV pages leave memory as soon as they are finished, so a report of a
    /// million rows needs about as much memory as one of a thousand. (An Excel workbook
    /// needs its rows twice to be written that way: see [`Renderer::render_rows_to`]. Here
    /// they can only be read once, so the workbook is built in memory.)
    ///
    /// ```no_run
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let report = ferroreport::parse(&std::fs::read_to_string("sales.jrxml")?)?;
    /// let rows = ferroreport::JsonRows::new(&report, std::fs::File::open("sales.json")?);
    /// let out = std::io::BufWriter::new(std::fs::File::create("sales.pdf")?);
    /// ferroreport::Renderer::new().render_to(&report, Box::new(rows), Default::default(), ferroreport::Format::Pdf, out)?;
    /// # Ok(()) }
    /// ```
    pub fn render_to<W: std::io::Write>(
        &mut self,
        report: &Report,
        rows: RowIter,
        params: HashMap<String, Value>,
        format: Format,
        mut out: W,
    ) -> Result<FillStats> {
        let env = self.env(report);
        match format {
            Format::Pdf => {
                let options = self.pdf_options(&report.name, report.pdf_page_offsets);
                let mut sink = pdf::PdfStream::new(out, report.page_width, report.page_height, options)?;
                let stats = fill::fill_to(report, rows, params, &mut self.fonts, &mut self.images, &env, self.limits, &mut sink)?;
                sink.finish(&self.fonts)?;
                Ok(stats)
            }
            Format::Csv => {
                let mut sink = csv::CsvStream::new(out, report.page_width, report.page_height, self.csv.clone());
                let stats = fill::fill_to(report, rows, params, &mut self.fonts, &mut self.images, &env, self.limits, &mut sink)?;
                sink.finish()?;
                Ok(stats)
            }
            Format::Html => {
                let mut sink =
                    html::HtmlStream::new(out, &report.name, report.page_width, report.page_height, self.html.clone())?;
                let stats = fill::fill_to(report, rows, params, &mut self.fonts, &mut self.images, &env, self.limits, &mut sink)?;
                sink.finish()?;
                Ok(stats)
            }
            Format::Docx => {
                let mut sink = docx::DocxStream::new(out, &report.name, report.page_width, report.page_height, report.landscape)?;
                let stats = fill::fill_to(report, rows, params, &mut self.fonts, &mut self.images, &env, self.limits, &mut sink)?;
                sink.finish()?;
                Ok(stats)
            }
            Format::Xlsx => {
                let mut count = 0;
                let rows = rows.inspect(|_| count += 1);
                let doc = fill::fill_rows(report, Box::new(rows), params, &mut self.fonts, &mut self.images, &env, self.limits)?;
                let bytes = self.to_xlsx(&doc, &self.xlsx)?;
                out.write_all(&bytes).and_then(|_| out.flush()).map_err(|e| Error::Io(e.to_string()))?;
                Ok(FillStats { pages: doc.pages.len(), rows: count })
            }
        }
    }

    /// Exports a filled document to PDF.
    pub fn to_pdf(&self, doc: &PrintDocument) -> Result<Vec<u8>> {
        pdf::write_pdf(doc, &self.fonts, &self.pdf_options(&doc.name, doc.pdf_page_offsets))
    }

    /// Exports a filled document to an Excel workbook (XLSX), laid out like JasperReports'
    /// XLSX exporter.
    pub fn to_xlsx(&self, doc: &PrintDocument, options: &xlsx::XlsxOptions) -> Result<Vec<u8>> {
        xlsx::write_xlsx(doc, &self.fonts, options)
    }

    /// Renders a report whose rows can be read again from the start: `rows` gives a new
    /// iterator over them each time it is called (open the file again, run the query again).
    ///
    /// PDF, CSV and HTML read them once, like [`Renderer::render_to`]. An Excel workbook
    /// needs them twice when it is large: the columns of its sheet come from every page, so
    /// the report is filled once to find them and once more to write the rows, which are
    /// compressed and sent on as they are made. A workbook of a million rows then takes
    /// about the memory of one of a thousand. Small workbooks are still filled once.
    ///
    /// ```no_run
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let report = ferroreport::parse(&std::fs::read_to_string("sales.jrxml")?)?;
    /// let mut rows = || -> ferroreport::Result<ferroreport::RowIter> {
    ///     let file = std::fs::File::open("sales.json").map_err(|e| ferroreport::Error::Io(e.to_string()))?;
    ///     Ok(Box::new(ferroreport::JsonRows::new(&report, file)))
    /// };
    /// let out = std::io::BufWriter::new(std::fs::File::create("sales.xlsx")?);
    /// ferroreport::Renderer::new().render_rows_to(&report, &mut rows, Default::default(), ferroreport::Format::Xlsx, out)?;
    /// # Ok(()) }
    /// ```
    pub fn render_rows_to<'a, W: std::io::Write>(
        &mut self,
        report: &Report,
        rows: &mut dyn FnMut() -> Result<RowIter<'a>>,
        params: HashMap<String, Value>,
        format: Format,
        out: W,
    ) -> Result<FillStats> {
        if format != Format::Xlsx {
            return self.render_to(report, rows()?, params, format, out);
        }
        // One environment for both passes: `new Date()` must say the same in each.
        let env = self.env(report);
        let mut first = xlsx::FirstPass::new();
        let stats =
            fill::fill_to(report, rows()?, params.clone(), &mut self.fonts, &mut self.images, &env, self.limits, &mut first)?;
        let (columns, kept) = first.done();
        let mut sheet = xlsx::XlsxStream::new(
            out,
            columns,
            &report.name,
            report.page_width,
            report.page_height,
            report.landscape,
            self.xlsx.clone(),
        )?;
        let stats = match kept {
            // A report that fits in memory is written from the pages of the first pass.
            Some(pages) => {
                for page in &pages {
                    sheet.add(&page.items, &self.fonts)?;
                }
                stats
            }
            None => fill::fill_to(report, rows()?, params, &mut self.fonts, &mut self.images, &env, self.limits, &mut sheet)?,
        };
        sheet.finish()?;
        Ok(stats)
    }

    /// Exports a filled document to CSV, laid out like JasperReports' CSV exporter.
    pub fn to_csv(&self, doc: &PrintDocument, options: &csv::CsvOptions) -> String {
        csv::write_csv(doc, options)
    }

    /// Parses a JRXML template, reading the style templates it uses (`<template>`, `.jrtx`
    /// files) from the renderer's resources: its resource folders and the files added with
    /// [`Renderer::add_resource`].
    pub fn parse(&self, template: &str) -> Result<Report> {
        jrxml::parse_report_with(template, &|location| self.images.read_resource(location))
    }

    /// Exports a filled document in the given format.
    pub fn export(&self, doc: &PrintDocument, format: Format) -> Result<Vec<u8>> {
        Ok(match format {
            Format::Pdf => self.to_pdf(doc)?,
            Format::Xlsx => self.to_xlsx(doc, &self.xlsx)?,
            Format::Csv => self.to_csv(doc, &self.csv).into_bytes(),
            Format::Html => html::write_html(doc, &self.fonts, &self.html)?.into_bytes(),
            Format::Docx => docx::write_docx(doc, &self.fonts)?,
        })
    }

    /// Renders a template with rows from a JSON array and parameters from a JSON object.
    pub fn render_json(&mut self, template: &str, rows: &serde_json::Value, params: &serde_json::Value) -> Result<Vec<u8>> {
        self.render_json_as(template, rows, params, Format::Pdf)
    }

    /// Like [`Renderer::render_json`], to another format.
    pub fn render_json_as(
        &mut self,
        template: &str,
        rows: &serde_json::Value,
        params: &serde_json::Value,
        format: Format,
    ) -> Result<Vec<u8>> {
        let report = self.parse(template)?;
        self.render_report_json(&report, rows, params, format)
    }

    /// Like [`Renderer::render_json_as`], with a template already parsed: parse a template
    /// once with [`parse`] and render it as many times as needed.
    pub fn render_report_json(
        &mut self,
        report: &Report,
        rows: &serde_json::Value,
        params: &serde_json::Value,
        format: Format,
    ) -> Result<Vec<u8>> {
        let records = data::records_from_json(report, rows)?;
        let params = params_from_json(report, params)?;
        let mut out = Vec::new();
        if format == Format::Xlsx {
            // A workbook may read the rows twice.
            let mut rows = || -> Result<RowIter> { Ok(Box::new(records.iter().cloned().map(Ok))) };
            self.render_rows_to(report, &mut rows, params, format, &mut out)?;
        } else {
            self.render_to(report, Box::new(records.into_iter().map(Ok)), params, format, &mut out)?;
        }
        Ok(out)
    }

    /// Font families that were substituted during the last fills.
    pub fn font_substitutions(&self) -> &[(String, String)] {
        &self.fonts.substitutions
    }
}

/// Converts a JSON object into report parameters typed by their declarations.
pub fn params_from_json(report: &Report, json: &serde_json::Value) -> Result<HashMap<String, Value>> {
    let mut out = HashMap::new();
    let Some(obj) = json.as_object() else { return Ok(out) };
    for (k, v) in obj {
        let class = report.parameters.iter().find(|p| &p.name == k).map(|p| p.class).unwrap_or(value::JavaType::Object);
        out.insert(k.clone(), data::json_to_value(v, class)?);
    }
    Ok(out)
}
