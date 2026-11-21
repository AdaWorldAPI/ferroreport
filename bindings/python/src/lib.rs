//! Python bindings: the `ferroreport._ferroreport` extension module. The public API lives in
//! `python/ferroreport/__init__.py`.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use std::collections::HashMap;

fn err(e: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(e.to_string())
}

/// Renders a JRXML template (source text) with rows and parameters given as JSON text.
#[pyfunction]
#[pyo3(signature = (template, data_json, params_json, locale, resource_dirs, resources, now=None))]
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
) -> PyResult<Py<PyBytes>> {
    let pdf = py
        .detach(move || -> Result<Vec<u8>, String> {
            let rows: serde_json::Value = serde_json::from_str(&data_json).map_err(|e| format!("data: {e}"))?;
            let params: serde_json::Value = serde_json::from_str(&params_json).map_err(|e| format!("params: {e}"))?;
            let mut r = ferroreport::Renderer::new().locale(&locale);
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
            r.render_json(&template, &rows, &params).map_err(|e| e.to_string())
        })
        .map_err(err)?;
    Ok(PyBytes::new(py, &pdf).unbind())
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
