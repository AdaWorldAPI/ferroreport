//! # ferroreport
//!
//! Renders JasperReports JRXML templates to PDF without a JVM.
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
pub mod data;
pub mod error;
pub mod expr;
pub mod fill;
pub mod fonts;
pub mod format;
pub mod image;
pub mod jrxml;
pub mod model;
pub mod pdf;
pub mod print;
pub mod text;
pub mod value;
mod xml;

pub use data::Record;
pub use error::{Error, Result};
pub use format::Locale;
pub use model::Report;
pub use print::PrintDocument;
pub use value::Value;

use chrono::NaiveDateTime;
use fonts::FontSet;
use std::collections::HashMap;

/// Parses a JRXML template.
pub fn parse(template: &str) -> Result<Report> {
    jrxml::parse_report(template)
}

/// Renders one report: parse -> fill -> export.
pub struct Renderer {
    fonts: FontSet,
    images: image::ImageStore,
    locale: &'static Locale,
    now: Option<NaiveDateTime>,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    pub fn new() -> Renderer {
        Renderer { fonts: FontSet::new(), images: image::ImageStore::new(), locale: &format::locale::EN_US, now: None }
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
        let env = expr::EvalEnv {
            locale: self.locale,
            groovy: report.language == model::Language::Groovy,
            now: self.now.unwrap_or_else(|| chrono::Utc::now().naive_utc()),
        };
        fill::fill(report, records, params, &mut self.fonts, &mut self.images, &env)
    }

    /// Exports a filled document to PDF.
    pub fn to_pdf(&self, doc: &PrintDocument) -> Result<Vec<u8>> {
        pdf::write_pdf(doc, &self.fonts, &pdf::PdfOptions { title: Some(doc.name.clone()), compress: true })
    }

    /// Renders a template with rows from a JSON array and parameters from a JSON object.
    pub fn render_json(&mut self, template: &str, rows: &serde_json::Value, params: &serde_json::Value) -> Result<Vec<u8>> {
        let report = parse(template)?;
        let records = data::records_from_json(&report, rows)?;
        let params = params_from_json(&report, params)?;
        let doc = self.fill(&report, &records, params)?;
        self.to_pdf(&doc)
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
