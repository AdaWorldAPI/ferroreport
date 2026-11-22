//! Python bindings: the `ferroreport._ferroreport` extension module. The public API lives in
//! `python/ferroreport/__init__.py`.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use std::collections::HashMap;

fn err(e: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(e.to_string())
}

/// Renders a JRXML template (source text) with parameters given as JSON text. Rows come as
/// JSON text, or from `data_path` (a JSON array or one object per line), read as they are
/// needed. The document is returned, or written to `output_path` as it is made. Returns
/// (bytes, pages, rows).
#[pyfunction]
#[pyo3(signature = (template, data_json, params_json, locale, resource_dirs, resources, now=None, format=None,
                    max_pages=None, timeout=None, csv_safe=false, xlsx_numbers=false, data_path=None, output_path=None,
                    tagged=false))]
#[allow(clippy::too_many_arguments)]
fn render(
    py: Python<'_>,
    template: String,
    data_json: String,
    params_json: String,
    locale: String,
    resource_dirs: Vec<String>,
    resources: HashMap<String, Vec<u8>>,
    now: Option<String>,
    format: Option<String>,
    max_pages: Option<usize>,
    timeout: Option<f64>,
    csv_safe: bool,
    xlsx_numbers: bool,
    data_path: Option<String>,
    output_path: Option<String>,
    tagged: bool,
) -> PyResult<(Py<PyBytes>, usize, usize)> {
    let format = match format {
        Some(f) => ferroreport::Format::parse(&f).ok_or_else(|| err(format!("unknown format `{f}` (pdf, xlsx, csv or html)")))?,
        None => ferroreport::Format::Pdf,
    };
    let (bytes, stats) = py
        .detach(move || -> Result<(Vec<u8>, ferroreport::FillStats), String> {
            let params: serde_json::Value = serde_json::from_str(&params_json).map_err(|e| format!("params: {e}"))?;
            let limits =
                ferroreport::Limits { max_pages, timeout: timeout.filter(|t| *t > 0.0).map(std::time::Duration::from_secs_f64) };
            let mut r = ferroreport::Renderer::new()
                .locale(&locale)
                .limits(limits)
                .xlsx_options(ferroreport::xlsx::XlsxOptions { detect_cell_type: xlsx_numbers })
                .tagged(tagged)
                .csv_options(ferroreport::csv::CsvOptions { guard_formulas: csv_safe, ..Default::default() });
            if let Some(now) = now {
                let t = ferroreport::data::parse_datetime(&now).ok_or_else(|| format!("now: cannot parse `{now}`"))?;
                r = r.now(t);
            }
            for dir in resource_dirs {
                r = r.resource_dir(dir);
            }
            for (name, data) in resources {
                r.add_resource(&name, data);
            }
            let report = ferroreport::parse(&template).map_err(|e| e.to_string())?;
            let params = ferroreport::params_from_json(&report, &params).map_err(|e| e.to_string())?;
            // The rows from the start, each time they are asked for: a large workbook reads
            // them twice (`Renderer::render_rows_to`).
            let mut rows = || -> ferroreport::Result<ferroreport::RowIter> {
                Ok(match &data_path {
                    Some(path) => {
                        let file = std::fs::File::open(path).map_err(|e| ferroreport::Error::Io(format!("{path}: {e}")))?;
                        Box::new(ferroreport::JsonRows::new(&report, file))
                    }
                    None => Box::new(ferroreport::JsonRows::new(&report, data_json.as_bytes())),
                })
            };
            match output_path {
                Some(path) => {
                    // Written beside the output and moved into place, so a failed render
                    // leaves no half-written file.
                    let part = format!("{path}.part");
                    let file = std::fs::File::create(&part).map_err(|e| format!("{path}: {e}"))?;
                    let out = std::io::BufWriter::with_capacity(1 << 16, file);
                    match r.render_rows_to(&report, &mut rows, params, format, out) {
                        Ok(stats) => {
                            std::fs::rename(&part, &path).map_err(|e| format!("{path}: {e}"))?;
                            Ok((Vec::new(), stats))
                        }
                        Err(e) => {
                            let _ = std::fs::remove_file(&part);
                            Err(e.to_string())
                        }
                    }
                }
                None => {
                    let mut out = Vec::new();
                    let stats = r.render_rows_to(&report, &mut rows, params, format, &mut out).map_err(|e| e.to_string())?;
                    Ok((out, stats))
                }
            }
        })
        .map_err(err)?;
    Ok((PyBytes::new(py, &bytes).unbind(), stats.pages, stats.rows))
}

/// Parses a template and reports what this engine does not support in it.
#[pyfunction]
fn inspect(py: Python<'_>, template: String) -> PyResult<Py<PyAny>> {
    let report = ferroreport::parse(&template).map_err(err)?;
    let out = pyo3::types::PyDict::new(py);
    out.set_item("name", &report.name)?;
    out.set_item("parameters", report.parameters.iter().map(|p| p.name.clone()).collect::<Vec<_>>())?;
    out.set_item("fields", report.fields.iter().map(|f| f.name.clone()).collect::<Vec<_>>())?;
    out.set_item("warnings", &report.warnings)?;
    Ok(out.into_any().unbind())
}

#[pymodule]
fn _ferroreport(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(render, m)?)?;
    m.add_function(wrap_pyfunction!(inspect, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
